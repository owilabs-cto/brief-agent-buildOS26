//! Slim OpenAI Realtime SIP adapter for OWI-106.
//!
//! Covers exactly what the hackathon demo needs:
//! - Place an outbound Twilio call bridged to OpenAI SIP
//! - Accept the incoming SIP webhook with prepared brief as instructions
//! - Twilio status callbacks for Slack footer edits
//!
//! Out of scope (deferred): WSS event loop, end_call tool handler,
//! voicemail detection, audit transcript persistence. The audit-agent
//! production module has all of that; here we keep the surface minimal
//! and rely on Twilio TimeLimit + caller hangup for termination.

pub mod client;
pub mod signature;
pub mod vc_brief_workflow;

use serde::Deserialize;

pub const REALTIME_CALL_INCOMING: &str = "realtime.call.incoming";

#[derive(Debug, Deserialize)]
pub struct WebhookEnvelope {
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: WebhookEnvelopeData,
}

#[derive(Debug, Deserialize)]
pub struct WebhookEnvelopeData {
    pub call_id: String,
    #[serde(default)]
    pub sip_headers: Vec<SipHeader>,
}

#[derive(Debug, Deserialize)]
pub struct SipHeader {
    pub name: String,
    pub value: String,
}

impl WebhookEnvelopeData {
    pub fn sip_header(&self, name: &str) -> Option<&str> {
        self.sip_headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }
}
