use axum::{Json, http::StatusCode, response::IntoResponse};
use serde_json::{Value, json};
use std::env;

const OPENAI_REALTIME_URL: &str = "https://api.openai.com/v1/realtime/client_secrets";

pub async fn create_session() -> impl IntoResponse {
    let api_key = match env::var("APP__OPENAI_REALTIME__API_KEY") {
        Ok(k) => k,
        Err(_) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "APP__OPENAI_REALTIME__API_KEY missing" })),
            );
        }
    };

    let body = json!({
        "session": {
            "type": "realtime",
            "model": "gpt-realtime-2",
            "audio": {
                "input": {
                    "turn_detection": {
                        "type": "server_vad",
                        "create_response": true,
                        "interrupt_response": true
                    },
                    "transcription": {
                        "model": "gpt-realtime-whisper",
                        "language": "fr"
                    }
                },
                "output": { "voice": "marin" }
            }
        }
    });

    let client = reqwest::Client::new();
    let resp = match client
        .post(OPENAI_REALTIME_URL)
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
    (StatusCode::OK, Json(payload))
}
