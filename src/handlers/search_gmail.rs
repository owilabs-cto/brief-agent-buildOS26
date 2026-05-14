use crate::AppState;
use crate::tools::gmail_fixtures;
use axum::{Json, extract::State};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct SearchGmailRequest {
    pub vc_name: String,
}

pub async fn search_gmail(
    State(state): State<AppState>,
    Json(req): Json<SearchGmailRequest>,
) -> Json<Value> {
    let now = Utc::now();
    let threads: Vec<Value> = state
        .fixtures
        .iter()
        .filter(|t| gmail_fixtures::matches(t, &req.vc_name))
        .map(|t| {
            let mut cloned = t.clone();
            let flag = gmail_fixtures::recency_flag(&cloned, now);
            if let Some(obj) = cloned.as_object_mut() {
                obj.insert("recency_flag".to_string(), json!(flag));
            }
            cloned
        })
        .collect();

    let count = threads.len();
    Json(json!({
        "threads": threads,
        "queried_at": now.to_rfc3339(),
        "source": "fixtures",
        "count": count,
    }))
}
