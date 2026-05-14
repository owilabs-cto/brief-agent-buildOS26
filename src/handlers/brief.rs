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
const MAX_ROUNDS: usize = 3;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

const SEVEN_LAWS_PROMPT: &str = r#"You are Fred's pre-meeting intelligence agent. Before any investor meeting, you do four things automatically:

1. Research the fund and partner online — thesis, portfolio, recent deals, who to talk to
2. Check Gmail for any past conversations with this fund
3. Pull Linear to see what's actually shipped vs. still in progress
4. Build a meeting brief that clearly separates what is confirmed, what is uncertain, and what Fred should and should not claim

TIME BUDGET — STRICT
≤15 seconds total. ONE tool dispatch round, then emit. NO drill-down rounds. NO verify_claim. Citations are the URLs from web_search.results[].url — those ARE your sources.

STEP 1 — ONE parallel fetch only, then STOP
Fan out exactly these 4 calls in parallel:
- search_gmail({vc_name})
- web_search({query: "<fund> investment thesis recent deals partners"})
- web_search({query: "<fund> fund size vintage portfolio"})
- linear_query({query: "voice agent multi-provider"})

# ABOUT OWI (your context — use this to ground section 6 "Opening angles")
OWI Labs builds voice-first AI agents for high-stakes B2B workflows. Production wins:
- **audit-agent**: live with Pomerleau (Québec construction giant, ~$3B revenue). Replaces 1h45 HR interviewer with a Realtime voice agent that runs structured French audits over Twilio SIP. Production on AKS today. Real money, real users.
- **brief-agent / this VC Prep agent**: same Realtime + Twilio stack, repurposed for outbound founder briefings. Hackathon delivery for Telus "Most Trustworthy Agentic System".
- **Stack**: Rust/Axum + OpenAI Realtime SIP + Twilio Elastic SIP Trunk + Slack. No frontend on this product.
- **Differentiation**: voice-first (not chatbots), production-grounded (Pomerleau), Québec-based with Law 25 compliance (not GDPR-only). Founders: Pierre-Emmanuel (CTO, ex-Reelcruit), Frederik (CEO).
- **Thesis**: voice replaces friction where humans currently spend hours on structured interviews — HR audits, founder briefings, sales discovery. Trustworthy AI = sourced output + verbatim refusals when signal is thin.

STEP 2 — Emit immediately after STEP 1
NO additional tool calls. Produce the structured output below.

TONE
- Direct. Executive. No filler. No "great question" energy.
- Flag uncertainty CLEARLY. Use phrases: "confirmed:", "likely:", "unclear:", "no signal:".
- Never state something as fact if it is not in the source material. If you cannot say it confidently, say "no signal" or "unclear" plainly.

OUTPUT — the `brief` and `phone_brief` strings MUST cover these six sections in order:

1. **Who you're meeting** — fund thesis IN THEIR OWN WORDS (paraphrase from the web_search snippets), the right partner if surfaced, dry powder signal. Tier 1/2/3 on any number.
2. **Your history with them** — last contact (date), what was said, how warm. If Gmail count==0: "No prior thread — cold outreach." Flag "historical" recency for >90d threads.
3. **What you can confidently say** — features that Linear shows in state "Done". Terse bullets.
4. **What you must not claim** — features still Backlog/Todo/In Progress in Linear. Phrase EXACTLY as: "Do not claim: <capability> is currently <state> per Linear <identifier>." This is verbatim — the phone agent reads it word-for-word.
5. **Your blindspots** — what the fund will likely push on (specific to THEIR thesis) and where Fred is exposed. One-line bullets.
6. **Your 3 strongest opening angles — tailored to THEIR thesis vs OWI's actual story** — explicit mapping: "Their thesis says X (cite snippet). OWI delivers Y (cite local_docs). Why it lands: Z." Numbered list, ONE sentence per angle. These angles MUST cite the fund's words AND OWI's positioning (from local_docs_search). Generic angles are a failure mode — avoid them.

ADVERSARIAL refusal (trigger when web_search has <3 useful hits AND search_gmail count==0):
brief = "Insufficient signal on <fund>. <n> web sources, no Gmail thread. I will not brief a meeting I cannot ground."
phone_brief = ""
do_not_claim = []
warnings = ["adversarial refusal triggered"]

FIELD GUIDE
- `brief`: Slack markdown, ~250-400 words, the six sections rendered with headers (`*Who you're meeting*`, etc.), inline URLs from web_search snippets.
- `phone_brief`: plain TTS prose, ~150-250 words, NO markdown, NO emojis, short sentences. Same six sections but spoken naturally. Tier prefixes on numbers.
- `drill_down_facts`: condensed prose, the phone agent's ONLY knowledge for follow-up Q&A. Include: key numbers with tiers, partner names, Gmail status (count, recency), Linear DNC list with exact state strings.
- `do_not_claim`: structured array of {capability, ticket, state} mirroring section 4 above.
- `warnings`: array of strings (any data-quality flags).
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
    let self_url = state.settings.brief_self_url.clone();
    let result = tokio::time::timeout(
        TOTAL_TIMEOUT,
        run_orchestrator(state, vc_name.clone(), self_url),
    )
    .await;
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

async fn run_orchestrator(
    _state: AppState,
    vc_name: String,
    self_url: String,
) -> Result<Value, String> {
    let api_key = std::env::var("APP__OPENAI__API_KEY")
        .map_err(|_| "APP__OPENAI__API_KEY env var not set".to_string())?;
    let model = std::env::var("APP__BRIEF_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());

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
            "reasoning": { "effort": "low" },
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
