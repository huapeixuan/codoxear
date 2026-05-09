use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const DEFAULT_SUMMARIZATION_MODEL: &str = "gpt-4.1-mini";
const DEFAULT_TTS_MODEL: &str = "gpt-4o-mini-tts";
const DEFAULT_TTS_BASE_URL: &str = "https://api.openai.com/v1";

pub fn load_voice_settings_snapshot(app_dir: &Path) -> Value {
    let raw = read_json_file(&app_dir.join("voice_settings.json")).unwrap_or(Value::Null);
    let settings = clean_voice_settings(&raw);
    let subscriptions = load_subscription_records(app_dir);
    let enabled_devices = subscriptions
        .iter()
        .filter(|record| {
            record
                .get("notifications_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && record.get("device_class").and_then(Value::as_str) == Some("mobile")
        })
        .count();
    let total_devices = subscriptions
        .iter()
        .filter(|record| record.get("device_class").and_then(Value::as_str) == Some("mobile"))
        .count();

    let mut out = settings;
    out.insert(
        "audio".to_string(),
        json!({
            "queue_depth": 0,
            "active_listener_count": 0,
            "stream_url": "/api/audio/live.m3u8",
            "segment_count": 0,
            "last_error": "",
            "media_sequence": 1,
        }),
    );
    out.insert(
        "notifications".to_string(),
        json!({
            "enabled_devices": enabled_devices,
            "total_devices": total_devices,
            "vapid_public_key": "",
        }),
    );
    Value::Object(out)
}

pub fn load_subscriptions_snapshot(app_dir: &Path) -> Value {
    let mut items = load_subscription_records(app_dir);
    items.sort_by(|a, b| {
        let b_ts = b.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
        let a_ts = a.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
        b_ts.partial_cmp(&a_ts).unwrap_or(std::cmp::Ordering::Equal)
    });
    Value::Object(Map::from_iter([
        ("vapid_public_key".to_string(), json!("")),
        ("subscriptions".to_string(), Value::Array(items)),
    ]))
}

pub fn notification_state_for_message(app_dir: &Path, message_id: &str) -> Option<Value> {
    let row = load_ledger(app_dir).remove(message_id)?;
    Some(json!({
        "message_id": message_id,
        "message_class": row.get("message_class"),
        "summary_status": row.get("summary_status"),
        "push_status": row.get("push_status"),
        "notification_text": compact_text(row.get("notification_text").and_then(Value::as_str).unwrap_or("")),
    }))
}

pub fn notification_feed_since(app_dir: &Path, since_ts: f64) -> Vec<Value> {
    let mut out = Vec::new();
    for row in load_ledger(app_dir).into_values() {
        if row.get("message_class").and_then(Value::as_str) != Some("final_response") {
            continue;
        }
        let updated_ts = row.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
        if updated_ts <= since_ts {
            continue;
        }
        let summary_status = row
            .get("summary_status")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !matches!(summary_status, "sent" | "skipped" | "error") {
            continue;
        }
        let notification_text = compact_text(
            row.get("notification_text")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        if notification_text.is_empty() {
            continue;
        }
        out.push(json!({
            "message_id": row.get("message_id").and_then(Value::as_str).unwrap_or(""),
            "session_id": row.get("session_id").and_then(Value::as_str).unwrap_or(""),
            "session_display_name": non_empty_or(
                row.get("session_display_name").and_then(Value::as_str).unwrap_or("").trim(),
                "Session",
            ),
            "notification_text": notification_text,
            "updated_ts": updated_ts,
        }));
    }
    out.sort_by(|a, b| {
        let a_key = (
            a.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0),
            a.get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        );
        let b_key = (
            b.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0),
            b.get("message_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        );
        a_key
            .partial_cmp(&b_key)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn clean_voice_settings(raw: &Value) -> Map<String, Value> {
    let empty = Map::new();
    let object = raw.as_object().unwrap_or(&empty);
    let base_url = normalize_base_url(value_str(object.get("tts_base_url")));
    Map::from_iter([
        (
            "tts_enabled_for_narration".to_string(),
            json!(object
                .get("tts_enabled_for_narration")
                .and_then(Value::as_bool)
                .unwrap_or(false)),
        ),
        (
            "tts_enabled_for_final_response".to_string(),
            json!(object
                .get("tts_enabled_for_final_response")
                .and_then(Value::as_bool)
                .unwrap_or(false)),
        ),
        ("tts_base_url".to_string(), json!(base_url)),
        (
            "tts_api_key".to_string(),
            json!(value_str(object.get("tts_api_key")).trim()),
        ),
        (
            "summarization_model".to_string(),
            json!(non_empty_or(
                value_str(object.get("summarization_model")).trim(),
                DEFAULT_SUMMARIZATION_MODEL,
            )),
        ),
        (
            "tts_model".to_string(),
            json!(non_empty_or(
                value_str(object.get("tts_model")).trim(),
                DEFAULT_TTS_MODEL,
            )),
        ),
    ])
}

fn load_subscription_records(app_dir: &Path) -> Vec<Value> {
    let Some(Value::Array(rows)) = read_json_file(&app_dir.join("push_subscriptions.json")) else {
        return Vec::new();
    };
    rows.iter().filter_map(clean_subscription_record).collect()
}

fn clean_subscription_record(raw: &Value) -> Option<Value> {
    let object = raw.as_object()?;
    let subscription = clean_subscription(object.get("subscription")?)?;
    let user_agent = value_str(object.get("user_agent")).trim().to_string();
    let device_class = clean_device_class(value_str(object.get("device_class")), &user_agent);
    Some(json!({
        "id": subscription_id(&subscription),
        "endpoint": subscription.get("endpoint").and_then(Value::as_str).unwrap_or(""),
        "notifications_enabled": object.get("notifications_enabled").and_then(Value::as_bool).unwrap_or(true),
        "device_class": device_class,
        "created_ts": number_or_now(object.get("created_ts")),
        "updated_ts": number_or(object.get("updated_ts"), number_or_now(object.get("created_ts"))),
        "last_success_ts": number_value(object.get("last_success_ts")),
        "last_failure_ts": number_value(object.get("last_failure_ts")),
        "last_error": value_str(object.get("last_error")).trim(),
        "user_agent": user_agent,
        "device_label": value_str(object.get("device_label")).trim(),
    }))
}

fn clean_subscription(raw: &Value) -> Option<Map<String, Value>> {
    let object = raw.as_object()?;
    let endpoint = value_str(object.get("endpoint")).trim().to_string();
    let keys = object.get("keys")?.as_object()?;
    let p256dh = value_str(keys.get("p256dh")).trim().to_string();
    let auth = value_str(keys.get("auth")).trim().to_string();
    if endpoint.is_empty() || p256dh.is_empty() || auth.is_empty() {
        return None;
    }
    Some(Map::from_iter([
        ("endpoint".to_string(), json!(endpoint)),
        (
            "keys".to_string(),
            Value::Object(Map::from_iter([
                ("p256dh".to_string(), json!(p256dh)),
                ("auth".to_string(), json!(auth)),
            ])),
        ),
    ]))
}

fn load_ledger(app_dir: &Path) -> Map<String, Value> {
    let Some(Value::Object(rows)) = read_json_file(&app_dir.join("voice_delivery_ledger.json"))
    else {
        return Map::new();
    };
    let mut cleaned = Map::new();
    for (message_id, row) in rows {
        if message_id.is_empty() {
            continue;
        }
        let Some(row) = clean_ledger_row(&message_id, &row) else {
            continue;
        };
        cleaned.insert(message_id, row);
    }
    cleaned
}

fn clean_ledger_row(message_id: &str, raw: &Value) -> Option<Value> {
    let object = raw.as_object()?;
    let session_id = value_str(object.get("session_id")).trim().to_string();
    let message_class = value_str(object.get("message_class")).trim().to_string();
    if session_id.is_empty() || !matches!(message_class.as_str(), "narration" | "final_response") {
        return None;
    }
    Some(json!({
        "message_id": message_id,
        "session_id": session_id,
        "session_display_name": value_str(object.get("session_display_name")).trim(),
        "message_class": message_class,
        "preview_text": value_str(object.get("preview_text")).trim(),
        "notification_text": value_str(object.get("notification_text")).trim(),
        "summary_text": value_str(object.get("summary_text")).trim(),
        "summary_status": non_empty_or(value_str(object.get("summary_status")).trim(), "pending"),
        "narrated_status": non_empty_or(value_str(object.get("narrated_status")).trim(), "pending"),
        "push_status": non_empty_or(value_str(object.get("push_status")).trim(), "pending"),
        "voice": value_str(object.get("voice")).trim(),
        "created_ts": number_or_now(object.get("created_ts")),
        "updated_ts": number_or_now(object.get("updated_ts")),
        "last_error": value_str(object.get("last_error")).trim(),
    }))
}

fn read_json_file(path: &Path) -> Option<Value> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn normalize_base_url(raw: &str) -> String {
    let value = non_empty_or(raw.trim(), DEFAULT_TTS_BASE_URL);
    if value.starts_with("http://") || value.starts_with("https://") {
        value.trim_end_matches('/').to_string()
    } else {
        DEFAULT_TTS_BASE_URL.to_string()
    }
}

fn clean_device_class(raw: &str, user_agent: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "mobile" => "mobile",
        "desktop" => "desktop",
        _ if user_agent.to_ascii_lowercase().contains("mobile")
            || user_agent.to_ascii_lowercase().contains("android")
            || user_agent.to_ascii_lowercase().contains("iphone")
            || user_agent.to_ascii_lowercase().contains("ipad")
            || user_agent.to_ascii_lowercase().contains("ipod") =>
        {
            "mobile"
        }
        _ => "desktop",
    }
}

fn subscription_id(subscription: &Map<String, Value>) -> String {
    let endpoint = subscription
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update(endpoint.as_bytes());
    format!("{:x}", hasher.finalize())[..24].to_string()
}

fn compact_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn value_str(value: Option<&Value>) -> &str {
    value.and_then(Value::as_str).unwrap_or("")
}

fn non_empty_or<'a>(value: &'a str, default: &'a str) -> &'a str {
    if value.is_empty() {
        default
    } else {
        value
    }
}

fn number_value(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_f64)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn number_or(value: Option<&Value>, default: f64) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(default)
}

fn number_or_now(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).unwrap_or(0.0)
}
