use serde_json::{json, Value};

pub const DEFAULT_PUSH_NOTIFICATION_TEXT: &str = "回复完成";
pub const WEB_PUSH_TTL_SECONDS: u32 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPushTarget {
    pub id: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPushPayload {
    pub session_id: String,
    pub session_display_name: String,
    pub message_id: String,
    pub notification_text: String,
    pub timestamp_millis: i64,
}

impl WebPushPayload {
    pub fn final_response_default(
        session_id: &str,
        session_display_name: &str,
        message_id: &str,
        timestamp_millis: i64,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            session_display_name: session_display_name.to_string(),
            message_id: message_id.to_string(),
            notification_text: DEFAULT_PUSH_NOTIFICATION_TEXT.to_string(),
            timestamp_millis,
        }
    }

    pub fn to_json_value(&self) -> Value {
        json!({
            "session_id": self.session_id,
            "session_display_name": self.session_display_name,
            "message_id": self.message_id,
            "notification_text": self.notification_text,
            "timestamp": self.timestamp_millis as f64 / 1000.0,
        })
    }
}

pub trait WebPushSender: Send + Sync {
    fn send_json(
        &self,
        target: &WebPushTarget,
        payload: &Value,
        ttl_seconds: u32,
        vapid_subject: &str,
    ) -> Result<(), WebPushSendError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebPushSendError {
    StaleSubscription(u16),
    Transport(String),
}

pub fn should_drop_subscription(endpoint: &str, error: Option<&WebPushSendError>) -> bool {
    if is_invalid_endpoint(endpoint) {
        return true;
    }
    matches!(error, Some(WebPushSendError::StaleSubscription(404 | 410)))
}

fn is_invalid_endpoint(endpoint: &str) -> bool {
    endpoint
        .trim()
        .to_lowercase()
        .split('/')
        .nth(2)
        .map(|host| host.ends_with(".invalid"))
        .unwrap_or(false)
}
