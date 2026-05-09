use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_SUMMARIZATION_MODEL: &str = "gpt-4.1-mini";
const DEFAULT_TTS_MODEL: &str = "gpt-4o-mini-tts";
const DEFAULT_TTS_BASE_URL: &str = "https://api.openai.com/v1";

pub fn load_voice_settings_snapshot(app_dir: &Path) -> Value {
    let settings = clean_voice_settings(read_json(&app_dir.join("voice_settings.json")));
    let subscriptions = clean_subscriptions(read_json(&app_dir.join("push_subscriptions.json")));
    let enabled_devices = subscriptions
        .iter()
        .filter(|record| {
            record
                .get("notifications_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && record
                    .get("device_class")
                    .and_then(Value::as_str)
                    .unwrap_or("desktop")
                    == "mobile"
        })
        .count();
    let total_devices = subscriptions
        .iter()
        .filter(|record| {
            record
                .get("device_class")
                .and_then(Value::as_str)
                .unwrap_or("desktop")
                == "mobile"
        })
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
    let mut records = clean_subscriptions(read_json(&app_dir.join("push_subscriptions.json")));
    let mut items = records
        .drain(..)
        .map(subscription_public_item)
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        let left_ts = left
            .get("updated_ts")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let right_ts = right
            .get("updated_ts")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        right_ts
            .partial_cmp(&left_ts)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    json!({"vapid_public_key": "", "subscriptions": items})
}

fn subscription_public_item(record: Value) -> Value {
    let mut out = Map::new();
    out.insert(
        "id".to_string(),
        record.get("id").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "endpoint".to_string(),
        record
            .get("subscription")
            .and_then(|subscription| subscription.get("endpoint"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    for key in [
        "notifications_enabled",
        "device_class",
        "created_ts",
        "updated_ts",
        "last_success_ts",
        "last_failure_ts",
        "last_error",
        "user_agent",
        "device_label",
    ] {
        out.insert(
            key.to_string(),
            record.get(key).cloned().unwrap_or(Value::Null),
        );
    }
    Value::Object(out)
}

pub fn notification_state_for_message(app_dir: &Path, message_id: &str) -> Option<Value> {
    let ledger = clean_ledger(read_json(&app_dir.join("voice_delivery_ledger.json")));
    let row = ledger.get(message_id)?;
    let mut out = Map::new();
    out.insert("message_id".to_string(), json!(message_id));
    out.insert(
        "message_class".to_string(),
        row.get("message_class").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "summary_status".to_string(),
        row.get("summary_status").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "push_status".to_string(),
        row.get("push_status").cloned().unwrap_or(Value::Null),
    );
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

pub fn notification_feed_since(app_dir: &Path, since_ts: f64) -> Vec<Value> {
    let ledger = clean_ledger(read_json(&app_dir.join("voice_delivery_ledger.json")));
    let mut rows = ledger
        .values()
        .filter_map(|row| {
            if row.get("message_class").and_then(Value::as_str) != Some("final_response") {
                return None;
            }
            let updated_ts = row.get("updated_ts").and_then(Value::as_f64).unwrap_or(0.0);
            if updated_ts <= since_ts {
                return None;
            }
            let summary_status = row
                .get("summary_status")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !matches!(summary_status, "sent" | "skipped" | "error") {
                return None;
            }
            let notification_text = compact_text(
                row.get("notification_text")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
            if notification_text.is_empty() {
                return None;
            }
            Some(json!({
                "message_id": row.get("message_id").and_then(Value::as_str).unwrap_or(""),
                "session_id": row.get("session_id").and_then(Value::as_str).unwrap_or(""),
                "session_display_name": session_display_name(row),
                "notification_text": notification_text,
                "updated_ts": updated_ts,
            }))
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        let left_ts = left
            .get("updated_ts")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let right_ts = right
            .get("updated_ts")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        left_ts
            .partial_cmp(&right_ts)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.get("message_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .cmp(
                        right
                            .get("message_id")
                            .and_then(Value::as_str)
                            .unwrap_or(""),
                    )
            })
    });
    rows
}

fn read_json(path: &Path) -> Option<Value> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn clean_voice_settings(raw: Option<Value>) -> Map<String, Value> {
    let object = raw
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let base_url = normalize_base_url(object.get("tts_base_url")).unwrap_or_else(|err| {
        tracing::warn!(error = %err, "invalid voice tts_base_url; using default");
        DEFAULT_TTS_BASE_URL.to_string()
    });
    let mut out = Map::new();
    out.insert(
        "tts_enabled_for_narration".to_string(),
        json!(object
            .get("tts_enabled_for_narration")
            .and_then(Value::as_bool)
            .unwrap_or(false)),
    );
    out.insert(
        "tts_enabled_for_final_response".to_string(),
        json!(object
            .get("tts_enabled_for_final_response")
            .and_then(Value::as_bool)
            .unwrap_or(false)),
    );
    out.insert("tts_base_url".to_string(), json!(base_url));
    out.insert(
        "tts_api_key".to_string(),
        json!(clean_text(object.get("tts_api_key"))),
    );
    out.insert(
        "summarization_model".to_string(),
        json!(non_empty_or(
            object.get("summarization_model"),
            DEFAULT_SUMMARIZATION_MODEL
        )),
    );
    out.insert(
        "tts_model".to_string(),
        json!(non_empty_or(object.get("tts_model"), DEFAULT_TTS_MODEL)),
    );
    out
}

fn clean_subscriptions(raw: Option<Value>) -> Vec<Value> {
    let Some(items) = raw.and_then(|value| value.as_array().cloned()) else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| clean_subscription_record(&item))
        .collect()
}

fn clean_subscription_record(raw: &Value) -> Option<Value> {
    let object = raw.as_object()?;
    let subscription = clean_subscription(object.get("subscription")?)?;
    let now_ts = unix_now_f64();
    let created_ts = object
        .get("created_ts")
        .and_then(Value::as_f64)
        .unwrap_or(now_ts);
    let updated_ts = object
        .get("updated_ts")
        .and_then(Value::as_f64)
        .unwrap_or(created_ts);
    let user_agent = clean_text(object.get("user_agent"));
    let mut out = Map::new();
    out.insert("id".to_string(), json!(subscription_id(&subscription)));
    out.insert("subscription".to_string(), subscription);
    out.insert(
        "notifications_enabled".to_string(),
        json!(object
            .get("notifications_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)),
    );
    out.insert("created_ts".to_string(), json!(created_ts));
    out.insert("updated_ts".to_string(), json!(updated_ts));
    out.insert(
        "last_success_ts".to_string(),
        number_or_null(object.get("last_success_ts")),
    );
    out.insert(
        "last_failure_ts".to_string(),
        number_or_null(object.get("last_failure_ts")),
    );
    out.insert(
        "last_error".to_string(),
        json!(clean_text(object.get("last_error"))),
    );
    out.insert("user_agent".to_string(), json!(user_agent.clone()));
    out.insert(
        "device_label".to_string(),
        json!(clean_text(object.get("device_label"))),
    );
    out.insert(
        "device_class".to_string(),
        json!(clean_device_class(object.get("device_class"), &user_agent)),
    );
    Some(Value::Object(out))
}

fn clean_subscription(raw: &Value) -> Option<Value> {
    let object = raw.as_object()?;
    let endpoint = clean_text(object.get("endpoint"));
    let keys = object.get("keys")?.as_object()?;
    let p256dh = clean_text(keys.get("p256dh"));
    let auth = clean_text(keys.get("auth"));
    if endpoint.is_empty() || p256dh.is_empty() || auth.is_empty() {
        return None;
    }
    Some(json!({"endpoint": endpoint, "keys": {"p256dh": p256dh, "auth": auth}}))
}

fn clean_ledger(raw: Option<Value>) -> Map<String, Value> {
    let Some(object) = raw.and_then(|value| value.as_object().cloned()) else {
        return Map::new();
    };
    let mut cleaned = Map::new();
    for (message_id, row) in object {
        if message_id.is_empty() {
            continue;
        }
        let Some(row_obj) = row.as_object() else {
            continue;
        };
        let session_id = clean_text(row_obj.get("session_id"));
        let message_class = clean_text(row_obj.get("message_class"));
        if session_id.is_empty()
            || !matches!(message_class.as_str(), "narration" | "final_response")
        {
            continue;
        }
        let mut out = Map::new();
        out.insert("message_id".to_string(), json!(message_id.clone()));
        out.insert("session_id".to_string(), json!(session_id));
        out.insert(
            "session_display_name".to_string(),
            json!(clean_text(row_obj.get("session_display_name"))),
        );
        out.insert("message_class".to_string(), json!(message_class));
        out.insert(
            "preview_text".to_string(),
            json!(clean_text(row_obj.get("preview_text"))),
        );
        out.insert(
            "notification_text".to_string(),
            json!(clean_text(row_obj.get("notification_text"))),
        );
        out.insert(
            "summary_text".to_string(),
            json!(clean_text(row_obj.get("summary_text"))),
        );
        out.insert(
            "summary_status".to_string(),
            json!(non_empty_or(row_obj.get("summary_status"), "pending")),
        );
        out.insert(
            "narrated_status".to_string(),
            json!(non_empty_or(row_obj.get("narrated_status"), "pending")),
        );
        out.insert(
            "push_status".to_string(),
            json!(non_empty_or(row_obj.get("push_status"), "pending")),
        );
        out.insert("voice".to_string(), json!(clean_text(row_obj.get("voice"))));
        out.insert(
            "created_ts".to_string(),
            json!(row_obj
                .get("created_ts")
                .and_then(Value::as_f64)
                .unwrap_or_else(unix_now_f64)),
        );
        out.insert(
            "updated_ts".to_string(),
            json!(row_obj
                .get("updated_ts")
                .and_then(Value::as_f64)
                .unwrap_or_else(unix_now_f64)),
        );
        out.insert(
            "last_error".to_string(),
            json!(clean_text(row_obj.get("last_error"))),
        );
        cleaned.insert(message_id, Value::Object(out));
    }
    cleaned
}

fn normalize_base_url(raw: Option<&Value>) -> Result<String, String> {
    let value = clean_text(raw);
    let value = if value.is_empty() {
        DEFAULT_TTS_BASE_URL.to_string()
    } else {
        value
    };
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(value.trim_end_matches('/').to_string())
    } else {
        Err("tts_base_url must start with http:// or https://".to_string())
    }
}

fn clean_device_class(raw: Option<&Value>, user_agent: &str) -> String {
    let value = clean_text(raw).to_ascii_lowercase();
    if matches!(value.as_str(), "mobile" | "desktop") {
        return value;
    }
    let ua = user_agent.to_ascii_lowercase();
    if ua.contains("mobile")
        || ua.contains("iphone")
        || ua.contains("android")
        || ua.contains("ipad")
    {
        "mobile".to_string()
    } else {
        "desktop".to_string()
    }
}

fn session_display_name(row: &Value) -> String {
    let value = row
        .get("session_display_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if value.is_empty() {
        "Session".to_string()
    } else {
        value.to_string()
    }
}

fn clean_text(raw: Option<&Value>) -> String {
    raw.and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn compact_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn non_empty_or(raw: Option<&Value>, default: &str) -> String {
    let value = clean_text(raw);
    if value.is_empty() {
        default.to_string()
    } else {
        value
    }
}

fn number_or_null(raw: Option<&Value>) -> Value {
    raw.and_then(Value::as_f64).map_or(Value::Null, Value::from)
}

fn subscription_id(subscription: &Value) -> String {
    let endpoint = subscription
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or("");
    let digest = sha256_hex(endpoint.as_bytes());
    digest[..24].to_string()
}

fn sha256_hex(raw: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(raw);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unix_now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}
