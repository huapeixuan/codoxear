use serde_json::{json, Value};
use std::path::Path;
use web_push::{
    ContentEncoding, IsahcWebPushClient, SubscriptionInfo, VapidSignatureBuilder, WebPushClient,
    WebPushMessageBuilder,
};

use crate::voice_worker::vapid::VAPID_PRIVATE_KEY_FILE;

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

#[derive(Debug, Clone)]
pub struct RustWebPushSender {
    app_dir: std::path::PathBuf,
}

impl RustWebPushSender {
    pub fn new(app_dir: std::path::PathBuf) -> Self {
        Self { app_dir }
    }
}

impl WebPushSender for RustWebPushSender {
    fn send_json(
        &self,
        target: &WebPushTarget,
        payload: &Value,
        ttl_seconds: u32,
        vapid_subject: &str,
    ) -> Result<(), WebPushSendError> {
        let message = build_webpush_message(
            &self.app_dir.join(VAPID_PRIVATE_KEY_FILE),
            target,
            payload,
            ttl_seconds,
            vapid_subject,
        )?;
        send_webpush_message(message)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebPushSendError {
    StaleSubscription(u16),
    Transport(String),
}

pub fn build_webpush_message(
    vapid_pem: &Path,
    target: &WebPushTarget,
    payload: &Value,
    ttl_seconds: u32,
    vapid_subject: &str,
) -> Result<web_push::WebPushMessage, WebPushSendError> {
    let subscription_info = SubscriptionInfo::new(&target.endpoint, &target.p256dh, &target.auth);
    let file = std::fs::File::open(vapid_pem)
        .map_err(|err| WebPushSendError::Transport(format!("read VAPID PEM: {err}")))?;
    let mut signature = VapidSignatureBuilder::from_pem(file, &subscription_info)
        .map_err(|err| WebPushSendError::Transport(format!("build VAPID signature: {err}")))?;
    signature.add_claim("sub", vapid_subject);
    let signature = signature
        .build()
        .map_err(|err| WebPushSendError::Transport(format!("sign VAPID JWT: {err}")))?;
    let body = serde_json::to_vec(payload)
        .map_err(|err| WebPushSendError::Transport(format!("encode push payload: {err}")))?;
    let mut builder = WebPushMessageBuilder::new(&subscription_info);
    builder.set_ttl(ttl_seconds);
    builder.set_payload(ContentEncoding::Aes128Gcm, &body);
    builder.set_vapid_signature(signature);
    builder
        .build()
        .map_err(|err| WebPushSendError::Transport(format!("build push message: {err}")))
}

pub fn send_webpush_message(message: web_push::WebPushMessage) -> Result<(), WebPushSendError> {
    std::thread::spawn(move || {
        let client = IsahcWebPushClient::new()
            .map_err(|err| WebPushSendError::Transport(err.to_string()))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| WebPushSendError::Transport(format!("create push runtime: {err}")))?;
        match runtime.block_on(client.send(message)) {
            Ok(()) => Ok(()),
            Err(web_push::WebPushError::EndpointNotFound(_)) => {
                Err(WebPushSendError::StaleSubscription(404))
            }
            Err(web_push::WebPushError::EndpointNotValid(_)) => {
                Err(WebPushSendError::StaleSubscription(410))
            }
            Err(err) => Err(WebPushSendError::Transport(err.to_string())),
        }
    })
    .join()
    .map_err(|_| WebPushSendError::Transport("push sender thread panicked".to_string()))?
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
