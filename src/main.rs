#[allow(dead_code)]
mod handlers;
#[allow(dead_code)]
mod openai;
#[allow(dead_code)]
mod tools;

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub fixtures: Arc<Vec<Value>>,
    pub docs: Arc<Vec<tools::local_docs_loader::LoadedDoc>>,
    pub http: reqwest::Client,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let fixtures_dir = PathBuf::from("./fixtures/gmail");
    let fixtures = tools::gmail_fixtures::load_fixtures(&fixtures_dir)
        .context("load gmail fixtures")?;
    info!(count = fixtures.len(), dir = %fixtures_dir.display(), "loaded gmail fixtures");

    let docs = tools::local_docs_loader::load_all().context("load local docs")?;
    info!(count = docs.len(), "loaded local docs");

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .context("build http client")?;

    let state = AppState {
        fixtures: Arc::new(fixtures),
        docs: Arc::new(docs),
        http,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/session", post(handlers::session::create_session))
        .route("/tools/search_gmail", post(handlers::search_gmail::search_gmail))
        .route("/tools/verify_claim", post(handlers::verify_claim::verify_claim))
        .route("/tools/web_search", post(handlers::web_search::web_search))
        .route("/tools/web_fetch", post(handlers::web_fetch::web_fetch))
        .route("/tools/linear_query", post(handlers::linear_query::linear_query))
        .route("/tools/local_docs_search", post(handlers::local_docs_search::local_docs_search))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = "0.0.0.0:3030".parse().context("bind addr")?;
    let listener = TcpListener::bind(addr).await.context("bind listener")?;
    info!(%addr, "brief-agent listening");
    axum::serve(listener, app).await.context("serve")?;
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}
