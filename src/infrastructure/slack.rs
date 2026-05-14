use anyhow::{Context, Result, bail};
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::error;

type HmacSha256 = Hmac<Sha256>;

const MAX_REQUEST_AGE_SECS: i64 = 60 * 5;

#[derive(Clone)]
pub struct SlackClient {
    http: Client,
    bot_token: String,
    signing_secret: String,
}

#[derive(Debug, Serialize)]
struct PostMessage<'a> {
    channel: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_ts: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct UpdateMessage<'a> {
    channel: &'a str,
    ts: &'a str,
    text: &'a str,
}

impl SlackClient {
    pub fn new(http: Client, bot_token: String, signing_secret: String) -> Self {
        Self {
            http,
            bot_token,
            signing_secret,
        }
    }

    pub fn verify_signature(&self, timestamp: &str, body: &str, signature: &str) -> bool {
        if self.signing_secret.is_empty() {
            return false;
        }
        let ts: i64 = match timestamp.parse() {
            Ok(v) => v,
            Err(_) => return false,
        };
        let now = chrono::Utc::now().timestamp();
        if (now - ts).abs() > MAX_REQUEST_AGE_SECS {
            return false;
        }

        let mut mac = match HmacSha256::new_from_slice(self.signing_secret.as_bytes()) {
            Ok(m) => m,
            Err(_) => return false,
        };
        mac.update(format!("v0:{timestamp}:{body}").as_bytes());
        let expected = format!(
            "v0={}",
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        expected.as_bytes().ct_eq(signature.as_bytes()).unwrap_u8() == 1
    }

    pub async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<String> {
        let body = PostMessage {
            channel,
            text,
            thread_ts,
        };
        let resp = self
            .http
            .post("https://slack.com/api/chat.postMessage")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await
            .context("slack chat.postMessage request failed")?;

        let json: Value = resp
            .json()
            .await
            .context("failed to parse slack chat.postMessage response")?;

        if json["ok"].as_bool() != Some(true) {
            let err = json["error"].as_str().unwrap_or("unknown");
            error!(subsystem = "slack", error = err, "chat.postMessage failed");
            bail!("slack chat.postMessage error: {err}");
        }

        Ok(json["ts"].as_str().unwrap_or("").to_string())
    }

    pub async fn update_message(&self, channel: &str, ts: &str, text: &str) -> Result<()> {
        let body = UpdateMessage { channel, ts, text };
        let resp = self
            .http
            .post("https://slack.com/api/chat.update")
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .await
            .context("slack chat.update request failed")?;

        let json: Value = resp
            .json()
            .await
            .context("failed to parse slack chat.update response")?;

        if json["ok"].as_bool() != Some(true) {
            let err = json["error"].as_str().unwrap_or("unknown");
            error!(subsystem = "slack", error = err, "chat.update failed");
            bail!("slack chat.update error: {err}");
        }
        Ok(())
    }
}
