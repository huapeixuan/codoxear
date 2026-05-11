use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_SUMMARIZATION_MODEL: &str = "gpt-4.1-mini";
const DEFAULT_TTS_MODEL: &str = "gpt-4o-mini-tts";
const DEFAULT_TTS_BASE_URL: &str = "https://api.openai.com/v1";

pub fn load_voice_settings_snapshot(app_dir: &Path) -> Value {
    let settings = read_json(&app_dir.join("voice_settings.json"))
        .and_then(|value| clean_voice_settings(&value))
        .unwrap_or_else(default_voice_settings);
    let subscriptions = read_subscriptions(app_dir);
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
    let mut items: Vec<Value> = read_subscriptions(app_dir)
        .into_iter()
        .map(|record| {
            json!({
                "id": record.get("id").cloned().unwrap_or(Value::String(String::new())),
                "endpoint": record
                    .get("subscription")
                    .and_then(|value| value.get("endpoint"))
                    .cloned()
                    .unwrap_or(Value::String(String::new())),
                "notifications_enabled": record
                    .get("notifications_enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                "device_class": record
                    .get("device_class")
                    .and_then(Value::as_str)
                    .unwrap_or("desktop"),
                "created_ts": record.get("created_ts").cloned().unwrap_or(Value::Null),
                "updated_ts": record.get("updated_ts").cloned().unwrap_or(Value::Null),
                "last_success_ts": record.get("last_success_ts").cloned().unwrap_or(Value::Null),
                "last_failure_ts": record.get("last_failure_ts").cloned().unwrap_or(Value::Null),
                "last_error": record.get("last_error").cloned().unwrap_or(Value::String(String::new())),
                "user_agent": record.get("user_agent").cloned().unwrap_or(Value::String(String::new())),
                "device_label": record.get("device_label").cloned().unwrap_or(Value::String(String::new())),
            })
        })
        .collect();
    items.sort_by(|left, right| {
        value_f64(right.get("updated_ts").unwrap_or(&Value::Null))
            .partial_cmp(&value_f64(left.get("updated_ts").unwrap_or(&Value::Null)))
            .unwrap_or(Ordering::Equal)
    });
    json!({"vapid_public_key": "", "subscriptions": items})
}

pub fn notification_state_for_message(app_dir: &Path, message_id: &str) -> Option<Value> {
    let ledger = read_ledger(app_dir);
    let row = ledger.get(message_id)?.as_object()?;
    Some(json!({
        "message_id": message_id,
        "message_class": row.get("message_class").cloned().unwrap_or(Value::Null),
        "summary_status": row.get("summary_status").cloned().unwrap_or(Value::Null),
        "push_status": row.get("push_status").cloned().unwrap_or(Value::Null),
        "notification_text": compact_text(row.get("notification_text").and_then(Value::as_str).unwrap_or("")),
    }))
}

pub fn notification_feed_since(app_dir: &Path, since: f64) -> Vec<Value> {
    let ledger = read_ledger(app_dir);
    let mut out = Vec::new();
    for row in ledger.values() {
        let Some(object) = row.as_object() else {
            continue;
        };
        if object.get("message_class").and_then(Value::as_str) != Some("final_response") {
            continue;
        }
        let updated_ts = value_f64(object.get("updated_ts").unwrap_or(&Value::Null));
        if updated_ts <= since {
            continue;
        }
        let summary_status = clean_string(object.get("summary_status")).unwrap_or_default();
        if !matches!(summary_status.as_str(), "sent" | "skipped" | "error") {
            continue;
        }
        let text = compact_text(
            object
                .get("notification_text")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        if text.is_empty() {
            continue;
        }
        out.push(json!({
            "message_id": clean_string(object.get("message_id")).unwrap_or_default(),
            "session_id": clean_string(object.get("session_id")).unwrap_or_default(),
            "session_display_name": clean_string(object.get("session_display_name")).unwrap_or_else(|| "Session".to_string()),
            "notification_text": text,
            "updated_ts": updated_ts,
        }));
    }
    out.sort_by(|left, right| {
        let left_key = (
            value_f64(left.get("updated_ts").unwrap_or(&Value::Null)),
            left.get("message_id").and_then(Value::as_str).unwrap_or(""),
        );
        let right_key = (
            value_f64(right.get("updated_ts").unwrap_or(&Value::Null)),
            right
                .get("message_id")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        left_key.partial_cmp(&right_key).unwrap_or(Ordering::Equal)
    });
    out
}

fn read_subscriptions(app_dir: &Path) -> Vec<Value> {
    let Some(Value::Array(items)) = read_json(&app_dir.join("push_subscriptions.json")) else {
        return Vec::new();
    };
    let mut records = Map::new();
    for item in items {
        let Some(record) = clean_subscription_record(&item) else {
            continue;
        };
        if let Some(id) = record.get("id").and_then(Value::as_str) {
            records.insert(id.to_string(), Value::Object(record));
        }
    }
    records.into_values().collect()
}

fn read_ledger(app_dir: &Path) -> Map<String, Value> {
    let Some(Value::Object(object)) = read_json(&app_dir.join("voice_delivery_ledger.json")) else {
        return Map::new();
    };
    let mut cleaned = Map::new();
    for (message_id, row) in object {
        if message_id.is_empty() {
            continue;
        }
        let Some(record) = clean_ledger_row(&message_id, &row) else {
            continue;
        };
        cleaned.insert(message_id, Value::Object(record));
    }
    cleaned
}

fn clean_voice_settings(raw: &Value) -> Option<Map<String, Value>> {
    let object = raw.as_object()?;
    let base_url = normalize_base_url(object.get("tts_base_url"))?;
    let mut settings = Map::new();
    settings.insert(
        "tts_enabled_for_narration".to_string(),
        json!(py_bool(object.get("tts_enabled_for_narration"))),
    );
    settings.insert(
        "tts_enabled_for_final_response".to_string(),
        json!(py_bool(object.get("tts_enabled_for_final_response"))),
    );
    settings.insert("tts_base_url".to_string(), json!(base_url));
    settings.insert(
        "tts_api_key".to_string(),
        json!(clean_string(object.get("tts_api_key")).unwrap_or_default()),
    );
    settings.insert(
        "summarization_model".to_string(),
        json!(clean_string(object.get("summarization_model"))
            .unwrap_or_else(|| DEFAULT_SUMMARIZATION_MODEL.to_string())),
    );
    settings.insert(
        "tts_model".to_string(),
        json!(
            clean_string(object.get("tts_model")).unwrap_or_else(|| DEFAULT_TTS_MODEL.to_string())
        ),
    );
    Some(settings)
}

fn default_voice_settings() -> Map<String, Value> {
    let mut settings = Map::new();
    settings.insert("tts_enabled_for_narration".to_string(), json!(false));
    settings.insert("tts_enabled_for_final_response".to_string(), json!(false));
    settings.insert("tts_base_url".to_string(), json!(DEFAULT_TTS_BASE_URL));
    settings.insert("tts_api_key".to_string(), json!(""));
    settings.insert(
        "summarization_model".to_string(),
        json!(DEFAULT_SUMMARIZATION_MODEL),
    );
    settings.insert("tts_model".to_string(), json!(DEFAULT_TTS_MODEL));
    settings
}

fn normalize_base_url(raw: Option<&Value>) -> Option<String> {
    let value = clean_string(raw).unwrap_or_else(|| DEFAULT_TTS_BASE_URL.to_string());
    if value.starts_with("http://") || value.starts_with("https://") {
        Some(value.trim_end_matches('/').to_string())
    } else {
        None
    }
}

fn clean_subscription_record(raw: &Value) -> Option<Map<String, Value>> {
    let object = raw.as_object()?;
    let subscription = clean_subscription(object.get("subscription")?)?;
    let now = now_seconds();
    let created_ts = object.get("created_ts").map(value_f64).unwrap_or(now);
    let updated_ts = object
        .get("updated_ts")
        .map(value_f64)
        .unwrap_or(created_ts);
    let user_agent = clean_string(object.get("user_agent")).unwrap_or_default();
    let device_class = clean_device_class(object.get("device_class"), &user_agent);
    let mut record = Map::new();
    record.insert("id".to_string(), json!(subscription_id(&subscription)));
    record.insert("subscription".to_string(), Value::Object(subscription));
    record.insert(
        "notifications_enabled".to_string(),
        json!(object
            .get("notifications_enabled")
            .map(|value| py_bool(Some(value)))
            .unwrap_or(true)),
    );
    record.insert("created_ts".to_string(), json!(created_ts));
    record.insert("updated_ts".to_string(), json!(updated_ts));
    record.insert(
        "last_success_ts".to_string(),
        number_or_null(object.get("last_success_ts")),
    );
    record.insert(
        "last_failure_ts".to_string(),
        number_or_null(object.get("last_failure_ts")),
    );
    record.insert(
        "last_error".to_string(),
        json!(clean_string(object.get("last_error")).unwrap_or_default()),
    );
    record.insert("user_agent".to_string(), json!(user_agent));
    record.insert(
        "device_label".to_string(),
        json!(clean_string(object.get("device_label")).unwrap_or_default()),
    );
    record.insert("device_class".to_string(), json!(device_class));
    Some(record)
}

fn clean_subscription(raw: &Value) -> Option<Map<String, Value>> {
    let object = raw.as_object()?;
    let endpoint = clean_string(object.get("endpoint"))?;
    if endpoint.is_empty() {
        return None;
    }
    let keys = object.get("keys")?.as_object()?;
    let p256dh = clean_string(keys.get("p256dh"))?;
    let auth = clean_string(keys.get("auth"))?;
    if p256dh.is_empty() || auth.is_empty() {
        return None;
    }
    let mut key_map = Map::new();
    key_map.insert("p256dh".to_string(), json!(p256dh));
    key_map.insert("auth".to_string(), json!(auth));
    let mut subscription = Map::new();
    subscription.insert("endpoint".to_string(), json!(endpoint));
    subscription.insert("keys".to_string(), Value::Object(key_map));
    Some(subscription)
}

fn clean_ledger_row(message_id: &str, row: &Value) -> Option<Map<String, Value>> {
    let object = row.as_object()?;
    let session_id = clean_string(object.get("session_id"))?;
    let message_class = clean_string(object.get("message_class"))?;
    if session_id.is_empty() || !matches!(message_class.as_str(), "narration" | "final_response") {
        return None;
    }
    let now = now_seconds();
    let mut out = Map::new();
    out.insert("message_id".to_string(), json!(message_id));
    out.insert("session_id".to_string(), json!(session_id));
    out.insert(
        "session_display_name".to_string(),
        json!(clean_string(object.get("session_display_name")).unwrap_or_default()),
    );
    out.insert("message_class".to_string(), json!(message_class));
    out.insert(
        "preview_text".to_string(),
        json!(clean_string(object.get("preview_text")).unwrap_or_default()),
    );
    out.insert(
        "notification_text".to_string(),
        json!(clean_string(object.get("notification_text")).unwrap_or_default()),
    );
    out.insert(
        "summary_text".to_string(),
        json!(clean_string(object.get("summary_text")).unwrap_or_default()),
    );
    out.insert(
        "summary_status".to_string(),
        json!(clean_string(object.get("summary_status")).unwrap_or_else(|| "pending".to_string())),
    );
    out.insert(
        "narrated_status".to_string(),
        json!(clean_string(object.get("narrated_status")).unwrap_or_else(|| "pending".to_string())),
    );
    out.insert(
        "push_status".to_string(),
        json!(clean_string(object.get("push_status")).unwrap_or_else(|| "pending".to_string())),
    );
    out.insert(
        "voice".to_string(),
        json!(clean_string(object.get("voice")).unwrap_or_default()),
    );
    out.insert(
        "created_ts".to_string(),
        json!(object.get("created_ts").map(value_f64).unwrap_or(now)),
    );
    out.insert(
        "updated_ts".to_string(),
        json!(object.get("updated_ts").map(value_f64).unwrap_or(now)),
    );
    out.insert(
        "last_error".to_string(),
        json!(clean_string(object.get("last_error")).unwrap_or_default()),
    );
    Some(out)
}

fn subscription_id(subscription: &Map<String, Value>) -> String {
    let endpoint = subscription
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut hasher = Sha256::new();
    hasher.update(endpoint.as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()[..24]
        .to_string()
}

fn clean_device_class(raw: Option<&Value>, user_agent: &str) -> String {
    let value = clean_string(raw).unwrap_or_default().to_lowercase();
    if value == "mobile" || value == "desktop" {
        return value;
    }
    device_class_from_user_agent(user_agent)
}

fn device_class_from_user_agent(raw: &str) -> String {
    let ua = raw.trim().to_lowercase();
    if ["mobile", "android", "iphone", "ipad", "ipod"]
        .iter()
        .any(|needle| ua.contains(needle))
    {
        "mobile".to_string()
    } else {
        "desktop".to_string()
    }
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn clean_string(value: Option<&Value>) -> Option<String> {
    let trimmed = match value? {
        Value::Null => String::new(),
        Value::String(raw) => raw.trim().to_string(),
        other => other.to_string().trim().trim_matches('"').to_string(),
    };
    (!trimmed.is_empty()).then_some(trimmed)
}

fn compact_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn py_bool(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().unwrap_or(0.0) != 0.0,
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(value)) => !value.is_empty(),
        Some(Value::Object(value)) => !value.is_empty(),
    }
}

fn value_f64(value: &Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(0.0),
        Value::String(raw) => raw.parse::<f64>().unwrap_or(0.0),
        Value::Bool(value) => u8::from(*value) as f64,
        _ => 0.0,
    }
}

fn number_or_null(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => json!(number.as_f64().unwrap_or(0.0)),
        _ => Value::Null,
    }
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}
