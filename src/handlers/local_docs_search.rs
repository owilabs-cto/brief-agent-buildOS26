use crate::AppState;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use std::env;

const OPENAI_URL: &str = "https://api.openai.com/v1/chat/completions";
const MODEL: &str = "gpt-5.5";

const SYSTEM_PROMPT: &str = "Extract verbatim passages from the docs relevant to the user query. \
Return up to 5 passages as JSON. Each passage has file_path, section_title, \
content (≤200 words, verbatim from the doc), and relevance_score 0-1. \
If nothing relevant, return an empty array.";

#[derive(Deserialize)]
pub struct LocalDocsSearchRequest {
    pub query: String,
}

pub async fn local_docs_search(
    State(state): State<AppState>,
    Json(req): Json<LocalDocsSearchRequest>,
) -> impl IntoResponse {
    let api_key = match env::var("APP__OPENAI__API_KEY") {
        Ok(k) => k,
        Err(_) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "APP__OPENAI__API_KEY missing" })),
            );
        }
    };

    let mut user_content = format!("Query: {}\n\n===DOCS===\n", req.query);
    for d in state.docs.iter() {
        user_content.push_str(&d.path);
        user_content.push('\n');
        user_content.push_str(&d.content);
        user_content.push_str("\n\n");
    }

    let response_format = json!({
        "type": "json_schema",
        "json_schema": {
            "name": "passages_envelope",
            "strict": true,
            "schema": {
                "type": "object",
                "additionalProperties": false,
                "required": ["passages"],
                "properties": {
                    "passages": {
                        "type": "array",
                        "maxItems": 5,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["file_path", "section_title", "content", "relevance_score"],
                            "properties": {
                                "file_path": { "type": "string" },
                                "section_title": { "type": "string" },
                                "content": { "type": "string" },
                                "relevance_score": { "type": "number", "minimum": 0, "maximum": 1 }
                            }
                        }
                    }
                }
            }
        }
    });

    let body = json!({
        "model": MODEL,
        "reasoning_effort": "low",
        "response_format": response_format,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": user_content }
        ]
    });

    let resp = match state
        .http
        .post(OPENAI_URL)
        .bearer_auth(&api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("openai request failed: {e}") })),
            );
        }
    };

    let status = resp.status();
    let payload: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("openai response not json: {e}") })),
            );
        }
    };

    if !status.is_success() {
        return (StatusCode::BAD_GATEWAY, Json(payload));
    }

    let content_str = payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("{}");

    let passages = serde_json::from_str::<Value>(content_str)
        .ok()
        .and_then(|v| v.get("passages").cloned())
        .unwrap_or_else(|| json!([]));

    (
        StatusCode::OK,
        Json(json!({
            "passages": passages,
            "queried_at": Utc::now().to_rfc3339(),
            "doc_count_loaded": state.docs.len(),
            "model": MODEL,
        })),
    )
}
