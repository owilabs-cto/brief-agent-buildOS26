use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{error, info, warn};

use crate::AppState;
use crate::domain::BriefContext;
use crate::infrastructure::voice::openai_realtime::client::OpenAIRealtimeClient;

#[derive(Debug, Deserialize)]
struct SlackSlashCommand {
    text: String,
    user_id: String,
    user_name: String,
    channel_id: String,
}

pub async fn handle_brief_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, StatusCode> {
    if !state.settings.slack.enabled {
        warn!(subsystem = "slack", "brief invoked while slack disabled");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let body_str = std::str::from_utf8(&body).map_err(|_| StatusCode::BAD_REQUEST)?;

    let timestamp = headers
        .get("x-slack-request-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let signature = headers
        .get("x-slack-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !state.slack.verify_signature(timestamp, body_str, signature) {
        warn!(subsystem = "slack", "invalid slack signature on /brief");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let cmd: SlackSlashCommand = serde_urlencoded::from_str(body_str).map_err(|e| {
        error!(subsystem = "slack", error = %e, "failed to parse slash command body");
        StatusCode::BAD_REQUEST
    })?;

    let fund = cmd.text.trim().to_string();
    if fund.is_empty() {
        return Ok(Json(json!({
            "response_type": "ephemeral",
            "text": "Usage: `/brief <VC fund name>` — e.g. `/brief Telus Ventures`.",
        })));
    }

    if state.settings.destination_phone.is_empty() {
        return Ok(Json(json!({
            "response_type": "ephemeral",
            "text": "APP__DESTINATION_PHONE is not set on the server. Set it before invoking `/brief`.",
        })));
    }

    info!(
        subsystem = "slack",
        action = "brief",
        user_id = %cmd.user_id,
        user_name = %cmd.user_name,
        channel_id = %cmd.channel_id,
        fund = %fund,
        "/brief slash command received"
    );

    let state_clone = state.clone();
    let channel = cmd.channel_id.clone();
    let user_id = cmd.user_id.clone();
    let fund_clone = fund.clone();
    tokio::spawn(async move {
        if let Err(error) = run_brief_pipeline(state_clone, channel, user_id, fund_clone).await {
            error!(subsystem = "slack", error = %error, "brief pipeline failed");
        }
    });

    Ok(Json(json!({
        "response_type": "in_channel",
        "text": format!(
            ":brain: Brief en cours sur *{fund}*… j'appelle dans ~30s. (demande de <@{}>)",
            cmd.user_id
        ),
    })))
}

async fn run_brief_pipeline(
    state: AppState,
    channel: String,
    user_id: String,
    fund: String,
) -> anyhow::Result<()> {
    let brief_url = format!("{}/brief", state.settings.brief_self_url);
    let started = std::time::Instant::now();
    let brief_resp = state
        .http
        .post(&brief_url)
        .json(&json!({ "vc_name": fund }))
        .send()
        .await?;
    let brief_status = brief_resp.status();
    let brief_value: Value = brief_resp.json().await?;
    let elapsed = started.elapsed().as_secs();

    if !brief_status.is_success() {
        let msg = format!(
            ":x: Brief failed for `{fund}` ({brief_status}). Logs have details. Requested by <@{user_id}>."
        );
        let _ = state.slack.post_message(&channel, &msg, None).await;
        return Ok(());
    }

    let warnings = brief_value["warnings"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let phone_brief = brief_value["phone_brief"].as_str().unwrap_or("").to_string();
    let adversarial = warnings
        .iter()
        .any(|w| w.as_str() == Some("adversarial refusal triggered"))
        || phone_brief.trim().is_empty();

    if adversarial {
        let brief_text = brief_value["brief"]
            .as_str()
            .unwrap_or("Adversarial refusal triggered, no call placed.");
        let msg = format!(
            ":no_entry_sign: *Brief refused: {fund}*\n\n{brief_text}\n\n_Requested by <@{user_id}>. No call placed._"
        );
        let _ = state.slack.post_message(&channel, &msg, None).await;
        return Ok(());
    }

    let brief_text = brief_value["brief"].as_str().unwrap_or("").to_string();
    let drill_down_facts = brief_value["drill_down_facts"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let do_not_claim = brief_value["do_not_claim"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let tool_calls_total = brief_value["tool_calls_total"].as_u64().unwrap_or(0);
    let grounded_claims = brief_value["grounded_claims"].as_u64().unwrap_or(0);
    let audit_trail = brief_value["audit_trail"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let brief_truncated = truncate_for_slack(&brief_text, 3000);
    let dnc_block = render_dnc_block(&do_not_claim);
    let slack_body_prefix = format!("*Brief: {fund}*\n\n{brief_truncated}\n{dnc_block}");
    let slack_stats_prefix = format!(
        "Brief in {elapsed}s · {tool_calls_total} tool calls · {grounded_claims} grounded"
    );
    let initial_status = ":telephone_receiver: Placing call…";
    let main_text = format!("{slack_body_prefix}\n\n_{slack_stats_prefix} · {initial_status}_");
    let main_ts = state.slack.post_message(&channel, &main_text, None).await?;

    let audit_md = truncate_for_slack(&render_audit_trail(&audit_trail), 3500);
    if !audit_md.is_empty() {
        let _ = state
            .slack
            .post_message(&channel, &audit_md, Some(&main_ts))
            .await;
    }

    let dnc_lines: Vec<String> = do_not_claim
        .iter()
        .map(|item| {
            let cap = item["capability"].as_str().unwrap_or("?");
            let tkt = item["ticket"].as_str().unwrap_or("?");
            let st = item["state"].as_str().unwrap_or("?");
            format!("Do not claim: {cap} is currently {st} per Linear {tkt}.")
        })
        .collect();

    let client = OpenAIRealtimeClient::new(
        state.http.clone(),
        state.settings.openai_realtime.clone(),
        state.settings.twilio.clone(),
    );
    let outbound = match client
        .twilio_create_outbound_call(&state.settings.destination_phone)
        .await
    {
        Ok(v) => v,
        Err(error) => {
            error!(error = %error, "twilio outbound failed");
            let new_body = format!(
                "{slack_body_prefix}\n\n_{slack_stats_prefix} · :x: Call placement failed_"
            );
            let _ = state.slack.update_message(&channel, &main_ts, &new_body).await;
            return Err(error);
        }
    };

    let ctx = BriefContext {
        fund_name: fund.clone(),
        phone_brief,
        drill_down_facts,
        do_not_claim_lines: dnc_lines,
        slack_channel: channel.clone(),
        slack_message_ts: main_ts.clone(),
        slack_body_prefix,
        slack_stats_prefix,
        answered_at_unix: None,
    };
    state.brief_contexts.insert(outbound.audit_call_id, ctx);

    info!(
        audit_call_id = %outbound.audit_call_id,
        twilio_sid = %outbound.parent_sid,
        fund = %fund,
        "outbound call placed"
    );

    Ok(())
}

fn render_dnc_block(items: &[Value]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n*:warning: DO NOT CLAIM*\n");
    for item in items {
        let cap = item["capability"].as_str().unwrap_or("?");
        let tkt = item["ticket"].as_str().unwrap_or("?");
        let st = item["state"].as_str().unwrap_or("?");
        out.push_str(&format!("• `{cap}` — Linear `{tkt}` is *{st}*\n"));
    }
    out
}

fn truncate_for_slack(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(50)).collect();
    out.push_str("\n\n…_(truncated — see full brief in thread)_");
    out
}

fn render_audit_trail(steps: &[Value]) -> String {
    if steps.is_empty() {
        return String::new();
    }
    let mut out = String::from("*Audit trail*\n");
    for s in steps {
        let step = s["step"].as_u64().unwrap_or(0);
        let tool = s["tool"].as_str().unwrap_or("?");
        let summary = s["result_summary"].as_str().unwrap_or("");
        let duration_ms = s["duration_ms"].as_u64().unwrap_or(0);
        if tool == "verify_claim" {
            let verdict = s["verdict"].as_str().unwrap_or("?");
            let tier = s["tier"]
                .as_i64()
                .map(|t| format!("Tier {t}"))
                .unwrap_or_else(|| "no tier".to_string());
            let claim = s["claim"].as_str().unwrap_or("");
            out.push_str(&format!(
                "{step}. `{tool}` — *{verdict}* ({tier}, {duration_ms}ms): _{claim}_\n"
            ));
        } else {
            out.push_str(&format!(
                "{step}. `{tool}` — {summary} ({duration_ms}ms)\n"
            ));
        }
    }
    out
}
