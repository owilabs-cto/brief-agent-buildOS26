use crate::AppState;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};
use tracing::{info, warn};

const OPENAI_URL: &str = "https://api.openai.com/v1/responses";
const DEFAULT_MODEL: &str = "gpt-5.5-2026-04-23";
const DEFAULT_SELF_URL: &str = "http://localhost:3030";
const MAX_ROUNDS: usize = 8;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(90);
const HTTP_TIMEOUT: Duration = Duration::from_secs(90);

const SEVEN_LAWS_PROMPT: &str = r#"PURPOSE
You are OWI's VC Prep orchestrator. The user gives you a fund name. You must emit structured output {brief, phone_brief, drill_down_facts, do_not_claim, warnings} after grounding every statement via verify_claim against real sources.

STEP 1 — Parallel fetch
Input is fund name only. Fan out in parallel:
- search_gmail({vc_name})
- web_search({query: "<fund> investment thesis vintage AUM"})
- web_search({query: "<fund> portfolio 2025 2026"})
- linear_query({query: "voice agent multi-provider"})

STEP 2 — Drill-down
If web_search exposes partner names, web_search for top 1–3 partners. If no partners surface, skip.

STEP 3 — Verify
For EVERY claim you intend to include in brief / phone_brief / drill_down_facts, call verify_claim with a source_bundle assembled from your web_search / web_fetch / search_gmail / local_docs_search hits. Only verdict="grounded" claims survive.

STEP 4 — Emit
Produce the structured output. `brief` for Slack (markdown OK, emojis OK). `phone_brief` for TTS (plain prose, no markdown, no emojis, short sentences). `drill_down_facts` condensed prose, the phone agent's ONLY knowledge base for follow-ups.

# THE 7 LAWS — verbatim

LAW #0 — SOURCING ABSOLUTE
Every statement you produce, structured-output or downstream-spoken, MUST be backed by a verify_claim call that returned verdict="grounded" with non-empty cited_sources[]. If you cannot cite, you do not state. No exception. If a section has no grounded claims, omit it entirely.

LAW #1 — FUND SIZE / FINANCIAL TIER
Never state a fund size, dry powder figure, vintage year, or any financial number without an explicit confidence tier (Tier 1/2/3) from verify_claim. The Tier is stated BEFORE the number in phone_brief (read aloud). Example: "Tier 2 confidence: roughly 33 percent dry powder remaining."

LAW #2 — THESIS DRIFT
Cross-reference the fund's stated thesis against their last 5 portfolio investments via web_search. If the pattern contradicts the stated thesis, surface the discrepancy explicitly in brief (Slack) AND phone_brief (call) BEFORE any opening-angle recommendation.

LAW #3 — GMAIL RECENCY
Every Gmail thread cited has a recency_flag. Threads flagged "historical" (>90 days) must be presented as such: "historical thread, verify before referencing." If search_gmail returns count==0, say so plainly: "no prior Gmail thread found, treat as cold outreach." Never invent relationship history.

LAW #4 — PARTNER RANKING BY THESIS FIT (conditional)
If web_search exposes partner names, rank partners by thesis fit not by seniority. Cross-reference each partner's bio + recent deals via web_search. State why you ranked them. If no partners surface publicly, skip — brief covers fund only.

LAW #5 — DO NOT CLAIM (Linear cross-check)
Before mentioning ANY OWI capability the founder might be tempted to claim is "live" or "shipped" during the call, call linear_query to verify. If linear_query returns the relevant ticket in state Backlog, Todo, or In Progress (NOT Done), include in do_not_claim array AND surface in brief as a red warning callout AND state verbatim in phone_brief: "Do not claim: <capability> is currently <state> per Linear <identifier>."

LAW #6 — DRY POWDER BY VINTAGE
When estimating dry powder, ground the estimate in fund vintage + publicly-known deployment curves (Carta vintage data, public press statements). State Tier 3 explicitly: "Tier 3 estimate, based on vintage X and typical deployment curve from <source>."

ADVERSARIAL behavior (explicit)
If web_search returns <3 useful results AND search_gmail returns count==0 AND local_docs_search returns no relevant passages: DO NOT produce a brief. Return structured output with brief = "Insufficient signal on <fund>. I have <n> web sources, no Gmail thread, no OWI doc match. I will not brief a meeting I cannot ground.", phone_brief = "", do_not_claim = [], warnings = ["adversarial refusal triggered"].
"#;

#[derive(Deserialize)]
pub struct BriefRequest {
    pub vc_name: String,
}

pub async fn brief(
    State(state): State<AppState>,
    Json(req): Json<BriefRequest>,
) -> axum::response::Response {
    let vc_name = req.vc_name.trim().to_string();
    if vc_name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "vc_name required" })),
        )
            .into_response();
    }

    let started = Instant::now();
    let result =
        tokio::time::timeout(TOTAL_TIMEOUT, run_orchestrator(state, vc_name.clone())).await;
    let elapsed_secs = started.elapsed().as_secs();

    match result {
        Ok(Ok(mut value)) => {
            if let Some(obj) = value.as_object_mut() {
                obj.insert("elapsed_secs".to_string(), json!(elapsed_secs));
                obj.insert("vc_name".to_string(), json!(vc_name));
            }
            info!(vc_name, elapsed_secs, "brief complete");
            (StatusCode::OK, Json(value)).into_response()
        }
        Ok(Err(e)) => {
            warn!(vc_name, error = %e, "brief orchestrator failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": e, "vc_name": vc_name })),
            )
                .into_response()
        }
        Err(_) => {
            warn!(vc_name, elapsed_secs, "brief orchestrator timeout");
            (
                StatusCode::GATEWAY_TIMEOUT,
                Json(json!({ "error": "orchestrator timeout", "vc_name": vc_name })),
            )
                .into_response()
        }
    }
}

fn http_client() -> &'static Client {
    static C: OnceLock<Client> = OnceLock::new();
    C.get_or_init(|| {
        Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("build brief http client")
    })
}

async fn run_orchestrator(_state: AppState, vc_name: String) -> Result<Value, String> {
    let api_key = std::env::var("APP__OPENAI__API_KEY")
        .map_err(|_| "APP__OPENAI__API_KEY env var not set".to_string())?;
    let model = std::env::var("APP__BRIEF_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let self_url =
        std::env::var("APP__BRIEF_SELF_URL").unwrap_or_else(|_| DEFAULT_SELF_URL.to_string());

    let mut input: Vec<Value> = vec![json!({
        "role": "user",
        "content": format!("Prepare a sourced VC brief for: {vc_name}. The fund's name is exactly: \"{vc_name}\". Fan out via STEP 1 in parallel.")
    })];
    let mut audit_trail: Vec<Value> = Vec::new();
    let mut tool_calls_total = 0u32;
    let mut grounded_claims = 0u32;
    let mut step_counter = 1u32;
    let mut previous_response_id: Option<String> = None;

    for round in 0..MAX_ROUNDS {
        let mut body = json!({
            "model": model,
            "instructions": SEVEN_LAWS_PROMPT,
            "input": input,
            "tools": tool_schemas(),
            "parallel_tool_calls": true,
            "reasoning": { "effort": "medium" },
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "vc_brief",
                    "schema": final_output_schema(),
                    "strict": true
                }
            }
        });
        if let Some(prev) = &previous_response_id {
            body["previous_response_id"] = json!(prev);
        }

        info!(
            round,
            tool_calls_total, grounded_claims, "orchestrator round"
        );

        let resp = http_client()
            .post(OPENAI_URL)
            .bearer_auth(&api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("openai POST failed: {e}"))?;

        let status = resp.status();
        let envelope: Value = resp
            .json()
            .await
            .map_err(|e| format!("openai response not json: {e}"))?;

        if !status.is_success() {
            return Err(format!("openai returned {status}: {envelope}"));
        }

        previous_response_id = envelope["id"].as_str().map(str::to_string);
        let outputs = envelope["output"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let function_calls: Vec<Value> = outputs
            .iter()
            .filter(|item| item["type"] == "function_call")
            .cloned()
            .collect();

        if function_calls.is_empty() {
            let text = outputs
                .iter()
                .find(|item| item["type"] == "message")
                .and_then(|m| m["content"].as_array())
                .and_then(|arr| arr.first())
                .and_then(|c| c["text"].as_str())
                .ok_or_else(|| {
                    format!("final message missing text. envelope: {envelope}")
                })?;
            let mut parsed: Value = serde_json::from_str(text)
                .map_err(|e| format!("final json parse failed: {e}. raw: {text}"))?;
            if let Some(obj) = parsed.as_object_mut() {
                obj.insert("audit_trail".to_string(), json!(audit_trail));
                obj.insert("tool_calls_total".to_string(), json!(tool_calls_total));
                obj.insert("grounded_claims".to_string(), json!(grounded_claims));
            }
            return Ok(parsed);
        }

        let mut handles = Vec::with_capacity(function_calls.len());
        for fc in &function_calls {
            let name = fc["name"].as_str().unwrap_or("").to_string();
            let call_id = fc["call_id"].as_str().unwrap_or("").to_string();
            let args_str = fc["arguments"].as_str().unwrap_or("{}").to_string();
            let self_url = self_url.clone();
            handles.push(tokio::spawn(async move {
                let args: Value = serde_json::from_str(&args_str).unwrap_or(json!({}));
                let url = format!("{self_url}/tools/{name}");
                let started = Instant::now();
                let res = http_client().post(&url).json(&args).send().await;
                let result_value: Value = match res {
                    Ok(r) => match r.json::<Value>().await {
                        Ok(v) => v,
                        Err(e) => json!({ "error": format!("tool response not json: {e}") }),
                    },
                    Err(e) => json!({ "error": format!("tool dispatch failed: {e}") }),
                };
                let duration_ms = started.elapsed().as_millis() as u64;
                (name, call_id, args, result_value, duration_ms)
            }));
        }

        let mut next_input: Vec<Value> = Vec::with_capacity(handles.len());
        for h in handles {
            let (name, call_id, args, result_value, duration_ms) = h
                .await
                .map_err(|e| format!("tool task join failed: {e}"))?;

            let mut trail_entry = json!({
                "step": step_counter,
                "tool": name,
                "args": args,
                "result_summary": summarize_result(&name, &result_value),
                "ts": Utc::now().to_rfc3339(),
                "duration_ms": duration_ms,
            });
            step_counter += 1;
            tool_calls_total += 1;

            if name == "verify_claim" {
                let verdict = result_value["verdict"].as_str().unwrap_or("").to_string();
                let tier = result_value["tier"].clone();
                let cited = result_value["cited_sources"].clone();
                if let Some(obj) = trail_entry.as_object_mut() {
                    obj.insert("verdict".to_string(), json!(verdict));
                    obj.insert("tier".to_string(), tier);
                    obj.insert("cited_sources".to_string(), cited);
                    if let Some(claim) = args.get("claim") {
                        obj.insert("claim".to_string(), claim.clone());
                    }
                }
                if verdict == "grounded" {
                    grounded_claims += 1;
                }
            }
            audit_trail.push(trail_entry);

            next_input.push(json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": serde_json::to_string(&result_value).unwrap_or_else(|_| "{}".to_string()),
            }));
        }

        input = next_input;
    }

    Err(format!(
        "orchestrator exceeded {MAX_ROUNDS} rounds without final answer"
    ))
}

fn summarize_result(name: &str, value: &Value) -> String {
    match name {
        "search_gmail" => {
            let count = value["count"].as_u64().unwrap_or(0);
            format!("{count} thread(s)")
        }
        "web_search" => {
            let count = value["result_count"].as_u64().unwrap_or(0);
            let source = value["source"].as_str().unwrap_or("?");
            format!("{count} results ({source})")
        }
        "web_fetch" => {
            let title = value["title"].as_str().unwrap_or("");
            format!("fetched: {title}")
        }
        "linear_query" => {
            let count = value["issues"].as_array().map(|a| a.len()).unwrap_or(0);
            format!("{count} issue(s)")
        }
        "local_docs_search" => {
            let count = value["passages"].as_array().map(|a| a.len()).unwrap_or(0);
            format!("{count} passage(s)")
        }
        "verify_claim" => {
            let verdict = value["verdict"].as_str().unwrap_or("?");
            let tier = value["tier"]
                .as_i64()
                .map(|t| format!("Tier {t}"))
                .unwrap_or_else(|| "no tier".to_string());
            format!("{verdict} ({tier})")
        }
        _ => value.to_string().chars().take(120).collect(),
    }
}

fn tool_schemas() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "name": "search_gmail",
            "description": "Search Gmail fixtures for prior threads involving the named VC fund. Returns threads with recency_flag ('historical' if >90 days old) and count.",
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "required": ["vc_name"],
                "properties": {
                    "vc_name": { "type": "string" }
                }
            },
            "strict": true
        }),
        json!({
            "type": "function",
            "name": "web_search",
            "description": "Open-web search (Tavily with DuckDuckGo HTML fallback). Returns up to 8 results with title, url, snippet.",
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "required": ["query", "max_results"],
                "properties": {
                    "query": { "type": "string" },
                    "max_results": { "type": ["integer", "null"], "minimum": 1, "maximum": 8 }
                }
            },
            "strict": true
        }),
        json!({
            "type": "function",
            "name": "web_fetch",
            "description": "Fetch a single https URL and return up to 2000 chars of cleaned text plus title.",
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "required": ["url"],
                "properties": {
                    "url": { "type": "string" }
                }
            },
            "strict": true
        }),
        json!({
            "type": "function",
            "name": "linear_query",
            "description": "Query OWI's Linear (scoped to the multi-provider + plan-002 projects) by free-text. Returns issues with state Backlog / Todo / In Progress / Done. Use this for LAW #5 DO NOT CLAIM checks.",
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" }
                }
            },
            "strict": true
        }),
        json!({
            "type": "function",
            "name": "local_docs_search",
            "description": "Search OWI's local doc corpus (CONTEXT-MAP.md, audit-agent docs, plan-de-croissance, transcripts). Returns up to 5 verbatim passages.",
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" }
                }
            },
            "strict": true
        }),
        json!({
            "type": "function",
            "name": "verify_claim",
            "description": "Verify a single claim against a bundle of source excerpts. Returns verdict (grounded|ungrounded|contradicted), tier (1|2|3|null), cited_sources, confidence, reason. Call this for EVERY statement that will appear in brief, phone_brief, or drill_down_facts.",
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "required": ["claim", "source_bundle"],
                "properties": {
                    "claim": { "type": "string" },
                    "source_bundle": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["type", "identifier", "excerpt"],
                            "properties": {
                                "type": { "type": "string" },
                                "identifier": { "type": "string" },
                                "excerpt": { "type": "string" }
                            }
                        }
                    }
                }
            },
            "strict": true
        }),
    ]
}

fn final_output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["brief", "phone_brief", "drill_down_facts", "do_not_claim", "warnings"],
        "properties": {
            "brief": { "type": "string" },
            "phone_brief": { "type": "string" },
            "drill_down_facts": { "type": "string" },
            "do_not_claim": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["capability", "ticket", "state"],
                    "properties": {
                        "capability": { "type": "string" },
                        "ticket": { "type": "string" },
                        "state": { "type": "string" }
                    }
                }
            },
            "warnings": {
                "type": "array",
                "items": { "type": "string" }
            }
        }
    })
}
