use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

const DEFAULT_SUMMARIZATION_MODEL: &str = "gpt-4.1-mini";
const DEFAULT_TTS_MODEL: &str = "gpt-4o-mini-tts";
const DEFAULT_TTS_BASE_URL: &str = "https://api.openai.com/v1";

pub fn load_voice_settings_snapshot(app_dir: &Path) -> Value {
    let raw = read_json_object(&app_dir.join("voice_settings.json")).unwrap_or_default();
    let mut out = Map::new();
    out.insert(
        "tts_enabled_for_narration".to_string(),
        json!(raw
            .get("tts_enabled_for_narration")
            .and_then(Value::as_bool)
            .unwrap_or(false)),
    );
    out.insert(
        "tts_enabled_for_final_response".to_string(),
        json!(raw
            .get("tts_enabled_for_final_response")
            .and_then(Value::as_bool)
            .unwrap_or(false)),
    );
    out.insert(
        "tts_base_url".to_string(),
        json!(normalize_base_url(raw.get("tts_base_url"))),
    );
    out.insert(
        "tts_api_key".to_string(),
        json!(raw
            .get("tts_api_key")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()),
    );
    out.insert(
        "summarization_model".to_string(),
        json!(clean_string(
            raw.get("summarization_model"),
            DEFAULT_SUMMARIZATION_MODEL
        )),
    );
    out.insert(
        "tts_model".to_string(),
        json!(clean_string(raw.get("tts_model"), DEFAULT_TTS_MODEL)),
    );
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
            "enabled_devices": enabled_mobile_device_count(app_dir),
            "total_devices": total_mobile_device_count(app_dir),
            "vapid_public_key": vapid_public_key(app_dir),
        }),
    );
    Value::Object(out)
}

pub fn load_subscriptions_snapshot(app_dir: &Path) -> Value {
    let mut items = cleaned_subscription_records(app_dir)
        .into_iter()
        .map(|record| {
            let subscription = record.get("subscription").and_then(Value::as_object);
            let mut item = Map::new();
            item.insert("id".to_string(), record_value(&record, "id"));
            item.insert(
                "endpoint".to_string(),
                subscription
                    .and_then(|obj| obj.get("endpoint"))
                    .and_then(Value::as_str)
                    .map(Value::from)
                    .unwrap_or(Value::String(String::new())),
            );
            item.insert(
                "notifications_enabled".to_string(),
                json!(record
                    .get("notifications_enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)),
            );
            item.insert(
                "device_class".to_string(),
                json!(record
                    .get("device_class")
                    .and_then(Value::as_str)
                    .unwrap_or("desktop")),
            );
            for key in [
                "created_ts",
                "updated_ts",
                "last_success_ts",
                "last_failure_ts",
                "last_error",
                "user_agent",
                "device_label",
            ] {
                item.insert(key.to_string(), record_value(&record, key));
            }
            Value::Object(item)
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        let ats = a.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
        let bts = b.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
        bts.partial_cmp(&ats).unwrap_or(std::cmp::Ordering::Equal)
    });
    json!({"vapid_public_key": vapid_public_key(app_dir), "subscriptions": items})
}

pub fn notification_state_for_message(app_dir: &Path, message_id: &str) -> Option<Value> {
    let ledger = cleaned_ledger(app_dir);
    let row = ledger.get(message_id)?.as_object()?;
    let mut out = Map::new();
    out.insert("message_id".to_string(), json!(message_id));
    out.insert(
        "message_class".to_string(),
        record_value(row, "message_class"),
    );
    out.insert(
        "summary_status".to_string(),
        record_value(row, "summary_status"),
    );
    out.insert("push_status".to_string(), record_value(row, "push_status"));
    out.insert(
        "notification_text".to_string(),
        json!(compact_text(
            row.get("notification_text")
                .and_then(Value::as_str)
                .unwrap_or("")
        )),
    );
    Some(Value::Object(out))
}

pub fn notification_feed_since(app_dir: &Path, since: f64) -> Vec<Value> {
    let ledger = cleaned_ledger(app_dir);
    let mut out = Vec::new();
    for row in ledger.values() {
        let Some(row) = row.as_object() else { continue };
        if row.get("message_class").and_then(Value::as_str) != Some("final_response") {
            continue;
        }
        let updated_ts = row.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
        if updated_ts <= since {
            continue;
        }
        let summary_status = row
            .get("summary_status")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !matches!(summary_status, "sent" | "skipped" | "error") {
            continue;
        }
        let text = compact_text(
            row.get("notification_text")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        if text.is_empty() {
            continue;
        }
        out.push(json!({
            "message_id": row.get("message_id").and_then(Value::as_str).unwrap_or(""),
            "session_id": row.get("session_id").and_then(Value::as_str).unwrap_or(""),
            "session_display_name": non_empty_or(row.get("session_display_name").and_then(Value::as_str), "Session"),
            "notification_text": text,
            "updated_ts": updated_ts,
        }));
    }
    out.sort_by(|a, b| {
        let ats = a.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
        let bts = b.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
        ats.partial_cmp(&bts)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.get("message_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .cmp(b.get("message_id").and_then(Value::as_str).unwrap_or(""))
            })
    });
    out
}

fn enabled_mobile_device_count(app_dir: &Path) -> usize {
    cleaned_subscription_records(app_dir)
        .iter()
        .filter(|record| {
            record
                .get("notifications_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && record.get("device_class").and_then(Value::as_str) == Some("mobile")
        })
        .count()
}

fn total_mobile_device_count(app_dir: &Path) -> usize {
    cleaned_subscription_records(app_dir)
        .iter()
        .filter(|record| record.get("device_class").and_then(Value::as_str) == Some("mobile"))
        .count()
}

fn cleaned_subscription_records(app_dir: &Path) -> Vec<Map<String, Value>> {
    let raw = fs::read_to_string(app_dir.join("push_subscriptions.json")).ok();
    let Some(value) = raw.and_then(|raw| serde_json::from_str::<Value>(&raw).ok()) else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items
            .into_iter()
            .filter_map(clean_subscription_record)
            .collect(),
        Value::Object(object) => object
            .into_values()
            .filter_map(clean_subscription_record)
            .collect(),
        _ => Vec::new(),
    }
}

fn clean_subscription_record(raw: Value) -> Option<Map<String, Value>> {
    let raw = raw.as_object()?;
    let subscription = clean_subscription(raw.get("subscription")?)?;
    let user_agent = clean_string(raw.get("user_agent"), "");
    let mut record = Map::new();
    record.insert("id".to_string(), json!(subscription_id(&subscription)));
    record.insert("subscription".to_string(), Value::Object(subscription));
    record.insert(
        "notifications_enabled".to_string(),
        json!(raw
            .get("notifications_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)),
    );
    record.insert(
        "created_ts".to_string(),
        number_or_null(raw.get("created_ts")),
    );
    record.insert(
        "updated_ts".to_string(),
        number_or_null(raw.get("updated_ts")),
    );
    record.insert(
        "last_success_ts".to_string(),
        number_or_null(raw.get("last_success_ts")),
    );
    record.insert(
        "last_failure_ts".to_string(),
        number_or_null(raw.get("last_failure_ts")),
    );
    record.insert(
        "last_error".to_string(),
        json!(clean_string(raw.get("last_error"), "")),
    );
    record.insert("user_agent".to_string(), json!(user_agent.clone()));
    record.insert(
        "device_label".to_string(),
        json!(clean_string(raw.get("device_label"), "")),
    );
    record.insert(
        "device_class".to_string(),
        json!(clean_device_class(raw.get("device_class"), &user_agent)),
    );
    Some(record)
}

fn clean_subscription(raw: &Value) -> Option<Map<String, Value>> {
    let raw = raw.as_object()?;
    let endpoint = clean_string(raw.get("endpoint"), "");
    let keys = raw.get("keys")?.as_object()?;
    let p256dh = clean_string(keys.get("p256dh"), "");
    let auth = clean_string(keys.get("auth"), "");
    if endpoint.is_empty() || p256dh.is_empty() || auth.is_empty() {
        return None;
    }
    let mut clean_keys = Map::new();
    clean_keys.insert("p256dh".to_string(), json!(p256dh));
    clean_keys.insert("auth".to_string(), json!(auth));
    let mut out = Map::new();
    out.insert("endpoint".to_string(), json!(endpoint));
    out.insert("keys".to_string(), Value::Object(clean_keys));
    Some(out)
}

fn cleaned_ledger(app_dir: &Path) -> Map<String, Value> {
    let Some(raw) = read_json_object(&app_dir.join("voice_delivery_ledger.json")) else {
        return Map::new();
    };
    let mut out = Map::new();
    for (message_id, row) in raw {
        if message_id.is_empty() {
            continue;
        }
        let Some(row) = row.as_object() else { continue };
        let session_id = clean_string(row.get("session_id"), "");
        let message_class = clean_string(row.get("message_class"), "");
        if session_id.is_empty()
            || !matches!(message_class.as_str(), "narration" | "final_response")
        {
            continue;
        }
        out.insert(
            message_id.clone(),
            json!({
                "message_id": message_id,
                "session_id": session_id,
                "session_display_name": clean_string(row.get("session_display_name"), ""),
                "message_class": message_class,
                "preview_text": clean_string(row.get("preview_text"), ""),
                "notification_text": clean_string(row.get("notification_text"), ""),
                "summary_text": clean_string(row.get("summary_text"), ""),
                "summary_status": clean_string(row.get("summary_status"), "pending"),
                "narrated_status": clean_string(row.get("narrated_status"), "pending"),
                "push_status": clean_string(row.get("push_status"), "pending"),
                "voice": clean_string(row.get("voice"), ""),
                "created_ts": row.get("created_ts").and_then(Value::as_f64).unwrap_or(0.0),
                "updated_ts": row.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0),
                "last_error": clean_string(row.get("last_error"), ""),
            }),
        );
    }
    out
}

fn vapid_public_key(app_dir: &Path) -> String {
    let key_path = app_dir.join("webpush_vapid_public.pem");
    let Ok(pem) = fs::read_to_string(key_path) else {
        return String::new();
    };
    pem.lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

fn read_json_object(path: &Path) -> Option<Map<String, Value>> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&raw)
        .ok()?
        .as_object()
        .cloned()
}

fn clean_string(value: Option<&Value>, default: &str) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn normalize_base_url(value: Option<&Value>) -> String {
    clean_string(value, DEFAULT_TTS_BASE_URL)
}

fn number_or_null(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_f64)
        .map(Value::from)
        .unwrap_or(Value::Null)
}

fn record_value(record: &Map<String, Value>, key: &str) -> Value {
    record.get(key).cloned().unwrap_or(Value::Null)
}

fn clean_device_class(value: Option<&Value>, user_agent: &str) -> String {
    let candidate = clean_string(value, "").to_lowercase();
    if matches!(candidate.as_str(), "mobile" | "desktop") {
        return candidate;
    }
    let ua = user_agent.to_lowercase();
    if ["mobile", "android", "iphone", "ipad", "ipod"]
        .iter()
        .any(|needle| ua.contains(needle))
    {
        "mobile".to_string()
    } else {
        "desktop".to_string()
    }
}

fn subscription_id(subscription: &Map<String, Value>) -> String {
    let endpoint = subscription
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or("");
    let digest = Sha256::digest(endpoint.as_bytes());
    digest
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn compact_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty_or(value: Option<&str>, default: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}
