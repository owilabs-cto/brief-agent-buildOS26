use axum::{
    Form,
    extract::State,
    http::StatusCode,
};
use chrono::Utc;
use serde::Deserialize;
use tracing::{info, warn};

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct TwilioStatusForm {
    #[serde(rename = "CallSid")]
    pub call_sid: Option<String>,
    #[serde(rename = "CallStatus")]
    pub call_status: Option<String>,
    #[serde(rename = "CallDuration")]
    pub call_duration: Option<String>,
}

pub async fn handle_status(
    State(state): State<AppState>,
    Form(form): Form<TwilioStatusForm>,
) -> StatusCode {
    let call_status = form.call_status.as_deref().unwrap_or("").to_string();
    let call_sid = form.call_sid.as_deref().unwrap_or("").to_string();
    info!(
        subsystem = "twilio-status",
        %call_status,
        %call_sid,
        "twilio status callback"
    );

    let entry = state
        .brief_contexts
        .iter()
        .next()
        .map(|e| (*e.key(), e.value().clone()));

    let Some((audit_call_id, mut ctx)) = entry else {
        return StatusCode::OK;
    };

    let now_unix = Utc::now().timestamp();

    let (footer, terminal) = match call_status.as_str() {
        "initiated" | "ringing" => (
            ":telephone_receiver: Ringing…".to_string(),
            false,
        ),
        "answered" | "in-progress" => {
            ctx.answered_at_unix = Some(now_unix);
            state.brief_contexts.insert(audit_call_id, ctx.clone());
            (":green_circle: Connected · 0:00".to_string(), false)
        }
        "completed" => {
            let duration = form
                .call_duration
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| ctx.answered_at_unix.map(|a| now_unix - a))
                .unwrap_or(0);
            let mins = duration / 60;
            let secs = duration % 60;
            (
                format!(":white_check_mark: Call ended · {mins}:{secs:02}"),
                true,
            )
        }
        "busy" | "no-answer" | "failed" | "canceled" => (
            format!(":x: Call {call_status}"),
            true,
        ),
        _ => return StatusCode::OK,
    };

    let new_text = format!(
        "{prefix}\n\n_{stats} · {footer}_",
        prefix = ctx.slack_body_prefix,
        stats = ctx.slack_stats_prefix,
    );
    if let Err(error) = state
        .slack
        .update_message(&ctx.slack_channel, &ctx.slack_message_ts, &new_text)
        .await
    {
        warn!(subsystem = "twilio-status", error = %error, "slack update_message failed");
    }

    if terminal {
        state.brief_contexts.remove(&audit_call_id);
    }

    StatusCode::OK
}
