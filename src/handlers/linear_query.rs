use crate::AppState;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use std::env;

const LINEAR_URL: &str = "https://api.linear.app/graphql";
const PROJECT_IDS: [&str; 2] = [
    "3b885885-0953-4c5d-b9d3-f5bf8a5aea5c",
    "eead7032-6dcc-4055-9284-ffc81f3e2bcc",
];

const QUERY_ISSUES: &str = r#"
query Issues($filter: IssueFilter!) {
  issues(filter: $filter, first: 30, orderBy: updatedAt) {
    nodes { id identifier title state { name type } project { id name } url updatedAt }
  }
}
"#;

const QUERY_SEARCH: &str = r#"
query Search($term: String!, $filter: IssueFilter!) {
  searchIssues(term: $term, filter: $filter, first: 30) {
    nodes { id identifier title state { name type } project { id name } url updatedAt }
  }
}
"#;

#[derive(Deserialize)]
pub struct LinearQueryRequest {
    pub query: String,
}

pub async fn linear_query(
    State(state): State<AppState>,
    Json(req): Json<LinearQueryRequest>,
) -> impl IntoResponse {
    let api_key = match env::var("APP__LINEAR_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": "APP__LINEAR_API_KEY missing" })),
            );
        }
    };

    let filter = json!({
        "project": { "id": { "in": PROJECT_IDS } }
    });

    let body = if req.query.trim().is_empty() {
        json!({ "query": QUERY_ISSUES, "variables": { "filter": filter } })
    } else {
        json!({
            "query": QUERY_SEARCH,
            "variables": { "term": req.query, "filter": filter }
        })
    };

    let resp = match state
        .http
        .post(LINEAR_URL)
        .header("Authorization", &api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("linear request failed: {e}") })),
            );
        }
    };

    let status = resp.status();
    let payload: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({ "error": format!("linear response not json: {e}") })),
            );
        }
    };

    if !status.is_success() {
        return (StatusCode::BAD_GATEWAY, Json(payload));
    }

    let nodes = payload
        .pointer("/data/issues/nodes")
        .or_else(|| payload.pointer("/data/searchIssues/nodes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let issues: Vec<Value> = nodes.into_iter().map(flatten_node).collect();

    (
        StatusCode::OK,
        Json(json!({
            "issues": issues,
            "query": req.query,
            "queried_at": Utc::now().to_rfc3339(),
            "project_scope": ["multi-provider", "plan-002"],
        })),
    )
}

fn flatten_node(n: Value) -> Value {
    let id = n.get("id").cloned().unwrap_or(Value::Null);
    let identifier = n.get("identifier").cloned().unwrap_or(Value::Null);
    let title = n.get("title").cloned().unwrap_or(Value::Null);
    let url = n.get("url").cloned().unwrap_or(Value::Null);
    let updated_at = n.get("updatedAt").cloned().unwrap_or(Value::Null);
    let state_name = n.pointer("/state/name").cloned().unwrap_or(Value::Null);
    let state_type = n.pointer("/state/type").cloned().unwrap_or(Value::Null);
    let project_name = n.pointer("/project/name").cloned().unwrap_or(Value::Null);
    json!({
        "id": id,
        "identifier": identifier,
        "title": title,
        "state_name": state_name,
        "state_type": state_type,
        "project_name": project_name,
        "url": url,
        "updated_at": updated_at,
    })
}
