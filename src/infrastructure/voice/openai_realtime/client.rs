//! REST surface for OpenAI Realtime SIP + the small Twilio bits we need.
//!
//! Three responsibilities for OWI-106:
//! 1. `POST .../Calls.json` — outbound Twilio call that bridges to OpenAI SIP.
//! 2. `POST /v1/realtime/calls/{call_id}/accept` — wire prepared brief into the session.
//! 3. `POST /v1/realtime/calls/{call_id}/hangup` — clean teardown.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::config::{OpenAiRealtimeConfig, TwilioConfig};

#[derive(Clone)]
pub struct OpenAIRealtimeClient {
    http: reqwest::Client,
    pub realtime: OpenAiRealtimeConfig,
    pub twilio: TwilioConfig,
}

impl OpenAIRealtimeClient {
    pub fn new(
        http: reqwest::Client,
        realtime: OpenAiRealtimeConfig,
        twilio: TwilioConfig,
    ) -> Self {
        Self {
            http,
            realtime,
            twilio,
        }
    }

    /// Place an outbound PSTN call via Twilio. Returns Twilio's parent
    /// CA SID and the orchestrator-generated `audit_call_id` UUID stamped
    /// on the SIP URI (joined back via the SIP-header on the OpenAI
    /// incoming webhook).
    pub async fn twilio_create_outbound_call(
        &self,
        to_e164: &str,
    ) -> Result<TwilioOutboundCall> {
        let audit_call_id = Uuid::new_v4();
        let sip_target = format!(
            "sip:{project}@sip.api.openai.com;transport=tls?x-audit-call-id={audit_call_id}",
            project = self.realtime.project_id,
        );
        let twiml = format!(
            "<Response><Dial answerOnBridge=\"true\" timeLimit=\"{limit}\"><Sip>{sip_target}</Sip></Dial></Response>",
            limit = self.realtime.call_time_limit_secs,
        );
        let status_callback_url = format!(
            "{}/internal/voice/webhook/twilio/status",
            self.realtime.webhook_base_url,
        );

        let form: Vec<(&str, String)> = vec![
            ("To", to_e164.to_string()),
            ("From", self.twilio.from_number.clone()),
            ("Twiml", twiml),
            ("StatusCallback", status_callback_url),
            ("StatusCallbackEvent", "initiated".to_string()),
            ("StatusCallbackEvent", "ringing".to_string()),
            ("StatusCallbackEvent", "answered".to_string()),
            ("StatusCallbackEvent", "completed".to_string()),
        ];

        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{account}/Calls.json",
            account = self.twilio.account_sid,
        );

        let response = self
            .http
            .post(&url)
            .basic_auth(&self.twilio.account_sid, Some(&self.twilio.auth_token))
            .form(&form)
            .send()
            .await
            .context("twilio outbound call request failed")?;

        let status = response.status();
        let payload: Value = response.json().await.context("twilio response not json")?;
        if !status.is_success() {
            bail!("twilio outbound call rejected ({status}): {payload}");
        }
        let parent_sid = payload
            .get("sid")
            .and_then(Value::as_str)
            .context("twilio response missing sid")?
            .to_string();

        Ok(TwilioOutboundCall {
            parent_sid,
            audit_call_id,
        })
    }

    pub async fn accept_call(
        &self,
        openai_call_id: &str,
        body: AcceptCallBody<'_>,
    ) -> Result<()> {
        let url = format!(
            "{base}/v1/realtime/calls/{openai_call_id}/accept",
            base = self.realtime.base_url,
        );
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.realtime.api_key)
            .json(&body)
            .send()
            .await
            .context("openai /accept request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("openai /accept returned {status}: {body}");
        }
        Ok(())
    }

    pub async fn hangup_openai_call(&self, openai_call_id: &str) -> Result<()> {
        let url = format!(
            "{base}/v1/realtime/calls/{openai_call_id}/hangup",
            base = self.realtime.base_url,
        );
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.realtime.api_key)
            .send()
            .await
            .context("openai /hangup request failed")?;
        let status = response.status();
        if status.is_success() || status.as_u16() == 404 || status.as_u16() == 409 {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        bail!("openai /hangup returned {status}: {body}")
    }
}

#[derive(Debug)]
pub struct TwilioOutboundCall {
    pub parent_sid: String,
    pub audit_call_id: Uuid,
}

/// Payload for `POST /v1/realtime/calls/{call_id}/accept`. `voice` and
/// `turn_detection` MUST live under `audio.output` / `audio.input` —
/// extras at top level are silently dropped and the session boots without
/// VAD, so OpenAI BYEs the SIP leg within ~1s.
#[derive(Debug, Serialize)]
pub struct AcceptCallBody<'a> {
    #[serde(rename = "type")]
    pub session_type: &'a str,
    pub model: &'a str,
    pub instructions: &'a str,
    pub audio: AudioConfig,
    pub tools: &'a [Value],
}

#[derive(Debug, Serialize)]
pub struct AudioConfig {
    pub input: AudioInput,
    pub output: AudioOutput,
}

#[derive(Debug, Serialize)]
pub struct AudioInput {
    pub turn_detection: TurnDetection,
    pub transcription: AudioInputTranscription,
}

#[derive(Debug, Serialize)]
pub struct AudioInputTranscription {
    pub model: &'static str,
    pub language: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AudioOutput {
    pub voice: String,
}

#[derive(Debug, Serialize)]
pub struct TurnDetection {
    #[serde(rename = "type")]
    pub kind: String,
    pub create_response: bool,
    pub interrupt_response: bool,
}

impl TurnDetection {
    pub fn server_vad(create_response: bool) -> Self {
        Self {
            kind: "server_vad".to_string(),
            create_response,
            interrupt_response: true,
        }
    }
}
