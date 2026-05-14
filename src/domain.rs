//! Domain types for the brief → call pipeline.

use serde::{Deserialize, Serialize};

/// State stashed in the in-memory registry between Slack-command time
/// and the OpenAI Realtime webhook firing. Looked up by `audit_call_id`
/// (UUID stamped on the SIP URI).
#[derive(Clone, Debug)]
pub struct BriefContext {
    pub fund_name: String,
    pub phone_brief: String,
    pub drill_down_facts: String,
    pub do_not_claim_lines: Vec<String>,
    pub slack_channel: String,
    pub slack_message_ts: String,
    /// Pre-rendered Slack body up to and including the DNC block, but
    /// without the trailing `_<stats> · <status>_` footer. The footer is
    /// re-rendered on each Twilio status callback and concatenated to
    /// this prefix.
    pub slack_body_prefix: String,
    /// Pre-rendered stats prefix for the footer, e.g.
    /// `Brief in 14s · 12 tool calls · 8 grounded`. The status suffix is
    /// appended at footer render time.
    pub slack_stats_prefix: String,
    pub answered_at_unix: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DoNotClaimItem {
    pub capability: String,
    pub ticket: String,
    pub state: String,
}
