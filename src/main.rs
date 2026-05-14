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
    let state = AppState { fixtures: Arc::new(fixtures) };

    let app = Router::new()
        .route("/health", get(health))
        .route("/session", post(handlers::session::create_session))
        .route("/tools/search_gmail", post(handlers::search_gmail::search_gmail))
        .route("/tools/verify_claim", post(handlers::verify_claim::verify_claim))
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
