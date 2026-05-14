#[allow(dead_code)]
mod handlers;
#[allow(dead_code)]
mod openai;
#[allow(dead_code)]
mod tools;

use anyhow::{Context, Result};
use axum::{Json, Router, routing::get};
use serde_json::{Value, json};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let app = Router::new()
        .route("/health", get(health))
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
