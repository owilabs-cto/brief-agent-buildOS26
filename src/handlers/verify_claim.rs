use anyhow::{Context, Result};
use axum::{Json, http::StatusCode, response::IntoResponse};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::OnceLock, time::Duration};
use tracing::{info, warn};

const OPENAI_URL: &str = "https://api.openai.com/v1/responses";
const MODEL: &str = "gpt-5.4-mini";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_OUTPUT_TOKENS: u32 = 1024;

const SYSTEM_PROMPT: &str = "You are a claim verifier for a Responsible AI agent. Given a CLAIM and a SOURCE BUNDLE of verbatim excerpts with identifiers, classify the claim STRICTLY.

VERDICT (exactly one):
- \"grounded\": Sources directly support the claim.
- \"ungrounded\": Sources do not address the claim (insufficient evidence).
- \"contradicted\": Sources state something incompatible with the claim.

TIER (only when verdict == \"grounded\", else null):
- 1: Source is a press release, SEC filing, or official document directly stating the claim's exact figure or fact.
- 2: Claim is inferred from quantitative evidence in sources (portfolio counts, deal announcements, known check ranges, dated public statements).
- 3: Claim is estimated with explicit uncertainty (sources support a range or approximation, not a precise number).

Output STRICT JSON only:
{
  \"verdict\": \"grounded\"|\"ungrounded\"|\"contradicted\",
  \"tier\": 1|2|3|null,
  \"cited_sources\": [<identifiers from bundle that support>],
  \"confidence\": <0.0 to 1.0>,
  \"reason\": \"<=80 words, plain text>\"
}

If verdict != \"grounded\": cited_sources = [], tier = null.
Never invent sources. Never echo claim wording in cited_sources.";

#[derive(Debug, Deserialize)]
pub struct Source {
    #[serde(rename = "type")]
    pub source_type: String,
    pub identifier: String,
    pub excerpt: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub claim: String,
    pub source_bundle: Vec<Source>,
}

pub async fn verify_claim(Json(req): Json<VerifyRequest>) -> impl IntoResponse {
    if req.source_bundle.is_empty() {
        info!(
            action = "verify_claim",
            verdict = "ungrounded",
            short_circuit = true,
            claim_len = req.claim.len(),
            "empty source bundle"
        );
        return (
            StatusCode::OK,
            Json(json!({
                "verdict": "ungrounded",
                "tier": Value::Null,
                "cited_sources": [],
                "confidence": 1.0,
                "reason": "no sources provided"
            })),
        )
            .into_response();
    }

    match call_openai(&req).await {
        Ok(parsed) => (StatusCode::OK, Json(parsed)).into_response(),
        Err(e) => {
            warn!(action = "verify_claim", error = %e, "openai call failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

fn shared_client() -> &'static Client {
    static C: OnceLock<Client> = OnceLock::new();
    C.get_or_init(|| {
        Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("build reqwest client")
    })
}

async fn call_openai(req: &VerifyRequest) -> Result<Value> {
    let api_key =
        std::env::var("APP__OPENAI__API_KEY").context("APP__OPENAI__API_KEY env var not set")?;

    let schema = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["verdict", "tier", "cited_sources", "confidence", "reason"],
        "properties": {
            "verdict": { "type": "string", "enum": ["grounded", "ungrounded", "contradicted"] },
            "tier": { "type": ["integer", "null"], "enum": [1, 2, 3, null] },
            "cited_sources": { "type": "array", "items": { "type": "string" } },
            "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
            "reason": { "type": "string" }
        }
    });

    let body = json!({
        "model": MODEL,
        "instructions": SYSTEM_PROMPT,
        "input": build_user_message(req),
        "max_output_tokens": MAX_OUTPUT_TOKENS,
        "text": {
            "format": {
                "type": "json_schema",
                "name": "claim_verdict",
                "schema": schema,
                "strict": true
            }
        },
        "temperature": 0
    });

    let started = std::time::Instant::now();
    let resp = shared_client()
        .post(OPENAI_URL)
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        .context("openai request send failed")?;

    let status = resp.status();
    if !status.is_success() {
        let raw = resp.text().await.unwrap_or_default();
        anyhow::bail!("openai returned {status}: {raw}");
    }

    let envelope: Value = resp
        .json()
        .await
        .context("openai response envelope not valid json")?;
    let duration_ms = started.elapsed().as_millis();

    let raw_text = envelope["output"]
        .as_array()
        .and_then(|arr| arr.iter().find(|item| item["type"] == "message"))
        .and_then(|m| m["content"].as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c["text"].as_str())
        .ok_or_else(|| {
            anyhow::anyhow!("openai response missing message.content[0].text: {envelope}")
        })?;

    let mut verdict_json: Value = serde_json::from_str(raw_text)
        .with_context(|| format!("model output not valid json: {raw_text}"))?;

    let verdict_str = verdict_json["verdict"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if verdict_str != "grounded"
        && let Some(obj) = verdict_json.as_object_mut()
    {
        obj.insert("tier".to_string(), Value::Null);
        obj.insert("cited_sources".to_string(), json!([]));
    }

    info!(
        action = "verify_claim",
        verdict = %verdict_str,
        claim_len = req.claim.len(),
        sources = req.source_bundle.len(),
        duration_ms,
        "verification complete"
    );

    Ok(verdict_json)
}

fn build_user_message(req: &VerifyRequest) -> String {
    let mut s = String::with_capacity(256 + req.claim.len());
    s.push_str("CLAIM:\n");
    s.push_str(&req.claim);
    s.push_str("\n\nSOURCE BUNDLE:\n");
    for (i, src) in req.source_bundle.iter().enumerate() {
        s.push_str(&format!(
            "[{i}] type={} identifier={}\nexcerpt: {}\n\n",
            src.source_type, src.identifier, src.excerpt
        ));
    }
    s
}
