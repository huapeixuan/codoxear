use crate::state_files::write_value;
use crate::voice_worker::hls::empty_playlist;
use crate::voice_worker::ledger::DELIVERY_LEDGER_FILE;
use crate::voice_worker::openai::{SummaryRequest, TtsRequest};
use crate::voice_worker::state::QueuedVoiceTask;
use crate::voice_worker::webpush::WebPushTarget;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

pub fn reset_hls_playlist(app_dir: &Path) -> Result<(), String> {
    let audio_dir = app_dir.join("audio");
    let segments_dir = audio_dir.join("segments");
    fs::create_dir_all(&segments_dir)
        .map_err(|err| format!("create {}: {err}", segments_dir.display()))?;
    if let Ok(entries) = fs::read_dir(&segments_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("ts") {
                let _ = fs::remove_file(path);
            }
        }
    }
    fs::write(audio_dir.join("live.m3u8"), empty_playlist(1))
        .map_err(|err| format!("write live.m3u8: {err}"))?;
    Ok(())
}

pub fn collect_task_ids(tasks: &[QueuedVoiceTask]) -> std::collections::BTreeSet<&str> {
    tasks
        .iter()
        .flat_map(|task| {
            task.source_message_ids
                .iter()
                .chain(std::iter::once(&task.message_id))
        })
        .map(String::as_str)
        .collect()
}

pub fn prune_listeners(listeners: &mut std::collections::HashMap<String, f64>, now_ts: f64) {
    listeners.retain(|_, seen_at| (now_ts - *seen_at) <= super::state::LISTENER_TTL_SECONDS);
}

pub fn read_ledger_object(path: &Path) -> Map<String, Value> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

pub fn delivery_ledger_path(app_dir: &Path) -> std::path::PathBuf {
    app_dir.join(DELIVERY_LEDGER_FILE)
}

pub fn load_worker_settings(app_dir: &Path) -> Value {
    let settings = fs::read_to_string(app_dir.join("voice_settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| crate::voice_state::clean_voice_settings(&value))
        .unwrap_or_else(crate::voice_state::default_voice_settings);
    Value::Object(settings)
}

pub fn should_deliver_row(row: &Value) -> bool {
    row.get("narrated_status").and_then(Value::as_str) == Some("pending")
        || row.get("push_status").and_then(Value::as_str) == Some("pending")
}

pub fn ledger_sort_ts(row: &Value) -> f64 {
    row.get("created_ts")
        .and_then(Value::as_f64)
        .or_else(|| row.get("updated_ts").and_then(Value::as_f64))
        .unwrap_or(0.0)
}

pub fn compact_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn clip_text(raw: &str, limit: usize) -> String {
    let text = compact_text(raw);
    if text.chars().count() <= limit {
        return text;
    }
    let mut out = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    out.push_str("...");
    out
}

pub fn summary_request(
    settings: &Value,
    session_name: &str,
    source_label: &str,
    target_words: u16,
) -> SummaryRequest {
    SummaryRequest {
        base_url: settings
            .get("tts_base_url")
            .and_then(Value::as_str)
            .unwrap_or("https://api.openai.com/v1")
            .to_string(),
        model: settings
            .get("summarization_model")
            .and_then(Value::as_str)
            .unwrap_or("gpt-4.1-mini")
            .to_string(),
        api_key_present: !settings
            .get("tts_api_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .is_empty(),
        session_name: session_name.to_string(),
        source_label: source_label.to_string(),
        target_words,
    }
}

pub fn tts_request(settings: &Value, voice: &str) -> TtsRequest {
    TtsRequest {
        base_url: settings
            .get("tts_base_url")
            .and_then(Value::as_str)
            .unwrap_or("https://api.openai.com/v1")
            .to_string(),
        model: settings
            .get("tts_model")
            .and_then(Value::as_str)
            .unwrap_or("gpt-4o-mini-tts")
            .to_string(),
        voice: voice.to_string(),
        api_key_present: !settings
            .get("tts_api_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .is_empty(),
        response_format: "aac",
    }
}

pub fn voice_for_session(session_id: &str, session_name: &str) -> String {
    const VOICES: &[&str] = &[
        "alloy", "ash", "ballad", "cedar", "coral", "echo", "fable", "marin", "nova", "onyx",
        "sage", "shimmer", "verse",
    ];
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(session_name.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    VOICES[(u64::from_be_bytes(bytes) as usize) % VOICES.len()].to_string()
}

pub fn read_subscription_map(app_dir: &Path) -> Map<String, Value> {
    let mut records = Map::new();
    for item in crate::voice_state::read_subscriptions(app_dir) {
        let Some(object) = item.as_object() else {
            continue;
        };
        let Some(id) = object.get("id").and_then(Value::as_str) else {
            continue;
        };
        records.insert(id.to_string(), Value::Object(object.clone()));
    }
    records
}

pub fn read_mobile_push_targets(app_dir: &Path) -> Vec<WebPushTarget> {
    crate::voice_state::read_subscriptions(app_dir)
        .into_iter()
        .filter(|record| {
            record
                .get("notifications_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && record.get("device_class").and_then(Value::as_str) == Some("mobile")
        })
        .filter_map(|record| {
            let id = record.get("id")?.as_str()?.to_string();
            let subscription = record.get("subscription")?.as_object()?;
            let keys = subscription.get("keys")?.as_object()?;
            Some(WebPushTarget {
                id,
                endpoint: subscription.get("endpoint")?.as_str()?.to_string(),
                p256dh: keys.get("p256dh")?.as_str()?.to_string(),
                auth: keys.get("auth")?.as_str()?.to_string(),
            })
        })
        .collect()
}

pub fn write_subscription_map(app_dir: &Path, records: Map<String, Value>) -> Result<(), String> {
    let path = app_dir.join("push_subscriptions.json");
    let mut items = records
        .into_values()
        .filter_map(|value| {
            crate::voice_state::clean_subscription_record(&value).map(Value::Object)
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        right
            .get("updated_ts")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .partial_cmp(
                &left
                    .get("updated_ts")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
            )
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    write_value(&path, &Value::Array(items)).map_err(|(_, message)| message)
}
