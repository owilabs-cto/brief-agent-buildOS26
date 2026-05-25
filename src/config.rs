use std::env;

#[derive(Clone, Debug)]
pub struct OpenAiRealtimeConfig {
    pub api_key: String,
    pub project_id: String,
    pub base_url: String,
    pub model: String,
    pub voice: String,
    pub webhook_secret: String,
    pub webhook_base_url: String,
    pub call_time_limit_secs: u32,
}

#[derive(Clone, Debug)]
pub struct TwilioConfig {
    pub account_sid: String,
    pub auth_token: String,
    pub from_number: String,
}

#[derive(Clone, Debug)]
pub struct SlackConfig {
    pub bot_token: String,
    pub signing_secret: String,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub port: u16,
    pub brief_self_url: String,
    pub destination_phone: String,
    pub openai_realtime: OpenAiRealtimeConfig,
    pub twilio: TwilioConfig,
    pub slack: SlackConfig,
    pub verify_webhook_signature: bool,
}

impl Settings {
    pub fn from_env() -> Self {
        let port: u16 = env::var("APP__PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8080);
        let brief_self_url = env::var("APP__BRIEF_SELF_URL")
            .unwrap_or_else(|_| format!("http://localhost:{port}"));

        let openai_realtime = OpenAiRealtimeConfig {
            api_key: env::var("APP__OPENAI_REALTIME__API_KEY")
                .or_else(|_| env::var("APP__OPENAI__API_KEY"))
                .unwrap_or_default(),
            project_id: env::var("APP__OPENAI_REALTIME__PROJECT_ID").unwrap_or_default(),
            base_url: env::var("APP__OPENAI_REALTIME__BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com".to_string()),
            model: env::var("APP__OPENAI_REALTIME__MODEL")
                .unwrap_or_else(|_| "gpt-realtime-2".to_string()),
            voice: env::var("APP__OPENAI_REALTIME__VOICE")
                .unwrap_or_else(|_| "marin".to_string()),
            webhook_secret: env::var("APP__OPENAI_REALTIME__WEBHOOK_SECRET").unwrap_or_default(),
            webhook_base_url: env::var("APP__OPENAI_REALTIME__WEBHOOK_BASE_URL")
                .unwrap_or_default(),
            call_time_limit_secs: env::var("APP__OPENAI_REALTIME__CALL_TIME_LIMIT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(180),
        };

        let twilio = TwilioConfig {
            account_sid: env::var("APP__TWILIO__ACCOUNT_SID").unwrap_or_default(),
            auth_token: env::var("APP__TWILIO__AUTH_TOKEN").unwrap_or_default(),
            from_number: env::var("APP__TWILIO__FROM_NUMBER").unwrap_or_default(),
        };

        let slack = SlackConfig {
            bot_token: env::var("APP__SLACK__BOT_TOKEN").unwrap_or_default(),
            signing_secret: env::var("APP__SLACK__SIGNING_SECRET").unwrap_or_default(),
            enabled: env::var("APP__SLACK__ENABLED")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(true),
        };

        let destination_phone = env::var("APP__DESTINATION_PHONE").unwrap_or_default();

        let verify_webhook_signature = env::var("APP__OPENAI_REALTIME__VERIFY_SIGNATURE")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(!openai_realtime.webhook_secret.is_empty());

        Self {
            port,
            brief_self_url,
            destination_phone,
            openai_realtime,
            twilio,
            slack,
            verify_webhook_signature,
        }
    }
}
