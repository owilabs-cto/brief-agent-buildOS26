//! Handler for `POST /internal/voice/webhook/openai-realtime`.
//!
//! 1. (Optional, if APP__OPENAI_REALTIME__WEBHOOK_SECRET set) verify HMAC.
//! 2. Parse envelope, extract `x-audit-call-id` from sip_headers[].
//! 3. Look up BriefContext from the in-memory registry.
//! 4. POST `/accept` with vc_brief_workflow instructions + end_call tool.
//!
//! 200 is returned immediately; the /accept POST runs off the response
//! path so OpenAI's webhook timeout doesn't fire.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::AppState;
use crate::infrastructure::voice::openai_realtime::client::{
    AcceptCallBody, AudioConfig, AudioInput, AudioInputTranscription, AudioOutput,
    OpenAIRealtimeClient, TurnDetection,
};
use crate::infrastructure::voice::openai_realtime::vc_brief_workflow::{
    BriefInstructions, build_instructions, end_call_tool_schema,
};
use crate::infrastructure::voice::openai_realtime::{
    REALTIME_CALL_INCOMING, WebhookEnvelope, signature,
};

const AUDIT_CALL_ID_HEADER: &str = "x-audit-call-id";

pub async fn handle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> StatusCode {
    if state.settings.verify_webhook_signature
        && let Err(err) = signature::verify_webhook_signature(
            &state.settings.openai_realtime.webhook_secret,
            &headers,
            &body,
        )
    {
        warn!(subsystem = "openai-realtime", error = %err, "invalid webhook signature");
        return StatusCode::UNAUTHORIZED;
    }

    let envelope: WebhookEnvelope = match serde_json::from_str(&body) {
        Ok(e) => e,
        Err(error) => {
            warn!(subsystem = "openai-realtime", error = %error, body_len = body.len(), "failed to parse webhook envelope");
            return StatusCode::BAD_REQUEST;
        }
    };

    info!(
        subsystem = "openai-realtime",
        event_type = %envelope.event_type,
        call_id = %envelope.data.call_id,
        "webhook received"
    );

    if envelope.event_type != REALTIME_CALL_INCOMING {
        return StatusCode::OK;
    }

    let Some(audit_call_id_str) = envelope.data.sip_header(AUDIT_CALL_ID_HEADER) else {
        warn!(
            subsystem = "openai-realtime",
            openai_call_id = %envelope.data.call_id,
            "sip_headers missing x-audit-call-id — accepting as anonymous call (no brief context)"
        );
        return StatusCode::OK;
    };

    let Ok(audit_call_id) = Uuid::parse_str(audit_call_id_str) else {
        warn!(
            subsystem = "openai-realtime",
            audit_call_id = %audit_call_id_str,
            "x-audit-call-id is not a valid UUID"
        );
        return StatusCode::OK;
    };

    let Some(ctx_entry) = state.brief_contexts.get(&audit_call_id) else {
        warn!(
            subsystem = "openai-realtime",
            %audit_call_id,
            "no BriefContext for this audit_call_id — registry miss"
        );
        return StatusCode::OK;
    };
    let ctx = ctx_entry.clone();
    drop(ctx_entry);

    let openai_call_id = envelope.data.call_id.clone();
    tokio::spawn(accept_and_log(state.clone(), openai_call_id, audit_call_id, ctx));

    StatusCode::OK
}

async fn accept_and_log(
    state: AppState,
    openai_call_id: String,
    audit_call_id: Uuid,
    ctx: crate::domain::BriefContext,
) {
    let client = OpenAIRealtimeClient::new(
        state.http.clone(),
        state.settings.openai_realtime.clone(),
        state.settings.twilio.clone(),
    );

    let instructions = build_instructions(&BriefInstructions {
        fund_name: ctx.fund_name.clone(),
        phone_brief: ctx.phone_brief.clone(),
        drill_down_facts: ctx.drill_down_facts.clone(),
        do_not_claim_lines: ctx.do_not_claim_lines.clone(),
    });
    let tools = vec![end_call_tool_schema()];

    let body = AcceptCallBody {
        session_type: "realtime",
        model: &state.settings.openai_realtime.model,
        instructions: &instructions,
        audio: AudioConfig {
            input: AudioInput {
                turn_detection: TurnDetection::server_vad(true),
                transcription: AudioInputTranscription {
                    model: "gpt-realtime-whisper",
                    language: "en",
                },
            },
            output: AudioOutput {
                voice: state.settings.openai_realtime.voice.clone(),
            },
        },
        tools: &tools,
    };

    match client.accept_call(&openai_call_id, body).await {
        Ok(()) => {
            info!(
                subsystem = "openai-realtime",
                %openai_call_id,
                %audit_call_id,
                fund = %ctx.fund_name,
                "call accepted with prepared brief"
            );
        }
        Err(error) => {
            warn!(
                subsystem = "openai-realtime",
                %openai_call_id,
                %audit_call_id,
                error = %error,
                "/accept failed"
            );
        }
    }
}
