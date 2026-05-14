#[allow(dead_code)]
mod handlers;
#[allow(dead_code)]
mod openai;
#[allow(dead_code)]
mod tools;

pub mod config;
pub mod domain;
pub mod infrastructure;

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    routing::{get, post},
};
use dashmap::DashMap;
use serde_json::{Value, json};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio::net::TcpListener;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::config::Settings;
use crate::domain::BriefContext;
use crate::infrastructure::slack::SlackClient;

#[derive(Clone)]
pub struct AppState {
    pub fixtures: Arc<Vec<Value>>,
    pub docs: Arc<Vec<tools::local_docs_loader::LoadedDoc>>,
    pub http: reqwest::Client,
    pub settings: Settings,
    pub slack: SlackClient,
    pub brief_contexts: Arc<DashMap<Uuid, BriefContext>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let settings = Settings::from_env();
    info!(
        port = settings.port,
        brief_self_url = %settings.brief_self_url,
        slack_enabled = settings.slack.enabled,
        slack_has_secret = !settings.slack.signing_secret.is_empty(),
        twilio_configured = !settings.twilio.account_sid.is_empty(),
        realtime_project_id_set = !settings.openai_realtime.project_id.is_empty(),
        webhook_base_url = %settings.openai_realtime.webhook_base_url,
        verify_webhook_signature = settings.verify_webhook_signature,
        "brief-agent settings loaded"
    );

    let fixtures_dir = PathBuf::from("./fixtures/gmail");
    let fixtures = tools::gmail_fixtures::load_fixtures(&fixtures_dir)
        .context("load gmail fixtures")?;
    info!(count = fixtures.len(), dir = %fixtures_dir.display(), "loaded gmail fixtures");

    let docs = tools::local_docs_loader::load_all().context("load local docs")?;
    info!(count = docs.len(), "loaded local docs");

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .context("build http client")?;

    let slack = SlackClient::new(
        http.clone(),
        settings.slack.bot_token.clone(),
        settings.slack.signing_secret.clone(),
    );

    let port = settings.port;
    let state = AppState {
        fixtures: Arc::new(fixtures),
        docs: Arc::new(docs),
        http,
        settings,
        slack,
        brief_contexts: Arc::new(DashMap::new()),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/brief", post(handlers::brief::brief))
        .route("/session", post(handlers::session::create_session))
        .route("/tools/search_gmail", post(handlers::search_gmail::search_gmail))
        .route("/tools/verify_claim", post(handlers::verify_claim::verify_claim))
        .route("/tools/web_search", post(handlers::web_search::web_search))
        .route("/tools/web_fetch", post(handlers::web_fetch::web_fetch))
        .route("/tools/linear_query", post(handlers::linear_query::linear_query))
        .route("/tools/local_docs_search", post(handlers::local_docs_search::local_docs_search))
        .route("/slack/commands/brief", post(handlers::slack::handle_brief_command))
        .route("/slack/brief", post(handlers::slack::handle_brief_command))
        .route(
            "/internal/voice/webhook/openai-realtime",
            post(handlers::webhook::openai_realtime::handle_webhook),
        )
        .route(
            "/internal/voice/webhook/twilio/status",
            post(handlers::webhook::twilio_status::handle_status),
        )
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("0.0.0.0:{port}").parse().context("bind addr")?;
    let listener = TcpListener::bind(addr).await.context("bind listener")?;
    info!(%addr, "brief-agent listening");
    axum::serve(listener, app).await.context("serve")?;
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true }))
}
