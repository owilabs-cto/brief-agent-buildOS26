//! Handler for `POST /internal/voice/webhook/twilio/status`.
//!
//! Twilio fires this for each subscribed event (`initiated`, `ringing`,
//! `answered`, `completed`). We don't get audit_call_id directly — Twilio
//! only knows about the parent CA SID. So we walk the BriefContexts map
//! looking for a context that's still pending; the Slack channel + ts on
//! it are what we update.
//!
//! For OWI-106 v1 the linking is by side-effect: only one call is
//! in-flight at a time (single founder, single demo). Multi-call concurrent
//! support would need a parent_sid → audit_call_id map populated at
//! outbound-call-place time.

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

    // Find the most recently-inserted brief context. Single-tenant demo:
    // there's only one in-flight call at a time.
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
