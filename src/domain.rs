use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub struct BriefContext {
    pub fund_name: String,
    pub phone_brief: String,
    pub drill_down_facts: String,
    pub do_not_claim_lines: Vec<String>,
    pub slack_channel: String,
    pub slack_message_ts: String,
    pub slack_body_prefix: String,
    pub slack_stats_prefix: String,
    pub answered_at_unix: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DoNotClaimItem {
    pub capability: String,
    pub ticket: String,
    pub state: String,
}
