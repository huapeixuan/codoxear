use crate::app_state::AppState;
use crate::log_normalizer::codex::read_chat_events_from_tail;
use crate::models::SessionRow;
use crate::state_files::{with_state_file_lock, write_value};
use crate::voice_state::{
    clean_ledger_row, clean_voice_settings, default_voice_settings, now_seconds,
};
use axum::http::StatusCode;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

const VOICE_SCAN_BYTES: usize = 8 * 1024 * 1024;
const PREVIEW_LIMIT: usize = 160;

#[derive(Debug, Clone, PartialEq)]
pub struct ClassifiedAssistantMessage {
    pub message_id: String,
    pub message_class: String,
    pub text: String,
    pub ts: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VoiceScanReport {
    pub sessions_scanned: usize,
    pub messages_seen: usize,
    pub rows_created: usize,
    pub rows_replaced: usize,
}

pub fn voice_scan_once(state: &AppState) -> Result<VoiceScanReport, String> {
    let rows = crate::session_loader::load_session_rows(&state.config)?;
    voice_scan_rows_once(&state.config.app_dir, &rows)
}

pub fn voice_scan_rows_once(
    app_dir: &Path,
    rows: &[SessionRow],
) -> Result<VoiceScanReport, String> {
    let settings = load_voice_settings(app_dir);
    let narration_enabled = settings
        .get("tts_enabled_for_narration")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut report = VoiceScanReport::default();
    for row in rows {
        let Some(log_path) = row
            .log_path
            .as_deref()
            .filter(|path| Path::new(path).exists())
        else {
            continue;
        };
        let messages = assistant_messages_from_log(Path::new(log_path))?;
        if messages.is_empty() {
            continue;
        }
        report.sessions_scanned += 1;
        report.messages_seen += messages.len();
        let session_name = session_display_name(row);
        let update = observe_messages_into_ledger(
            app_dir,
            &row.session_id,
            &session_name,
            &messages,
            narration_enabled,
        )?;
        report.rows_created += update.rows_created;
        report.rows_replaced += update.rows_replaced;
    }
    Ok(report)
}

pub fn assistant_messages_from_log(path: &Path) -> Result<Vec<ClassifiedAssistantMessage>, String> {
    let events = read_chat_events_from_tail(path, 0, VOICE_SCAN_BYTES)?;
    let mut out = Vec::new();
    for event in events {
        if event.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let text = compact_text(event.get("text").and_then(Value::as_str).unwrap_or(""));
        if text.is_empty() {
            continue;
        }
        let message_class = event
            .get("message_class")
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "narration" | "final_response"))
            .unwrap_or("narration")
            .to_string();
        let message_id = event
            .get("message_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                fallback_message_id(
                    &message_class,
                    &text,
                    event.get("ts").and_then(Value::as_f64),
                )
            });
        out.push(ClassifiedAssistantMessage {
            message_id,
            message_class,
            text,
            ts: event.get("ts").and_then(Value::as_f64),
        });
    }
    Ok(out)
}

#[derive(Default)]
struct ObserveUpdate {
    rows_created: usize,
    rows_replaced: usize,
}

pub fn observe_messages_into_ledger(
    app_dir: &Path,
    session_id: &str,
    session_display_name: &str,
    messages: &[ClassifiedAssistantMessage],
    narration_enabled: bool,
) -> Result<VoiceScanReport, String> {
    let path = app_dir.join(crate::voice_worker::ledger::DELIVERY_LEDGER_FILE);
    let update = with_state_file_lock(&path, || {
        let mut ledger = read_clean_ledger(&path);
        let mut update = ObserveUpdate::default();
        for message in messages {
            if ledger.contains_key(&message.message_id) {
                continue;
            }
            if message.message_class == "final_response" {
                update.rows_replaced += replace_pending_same_slot(
                    &mut ledger,
                    session_id,
                    "final_response",
                    &message.message_id,
                );
            } else if message.message_class == "narration" {
                update.rows_replaced += replace_pending_same_slot(
                    &mut ledger,
                    session_id,
                    "narration",
                    &message.message_id,
                );
            }
            let now = message.ts.unwrap_or_else(now_seconds);
            let voice_enabled = message.message_class == "final_response" || narration_enabled;
            let row = json!({
                "message_id": message.message_id,
                "session_id": session_id,
                "session_display_name": session_display_name,
                "message_class": message.message_class,
                "preview_text": clip_text(&message.text, PREVIEW_LIMIT),
                "notification_text": "",
                "summary_text": "",
                "summary_status": if voice_enabled { "pending" } else { "skipped" },
                "narrated_status": if voice_enabled { "pending" } else { "skipped" },
                "push_status": if message.message_class == "final_response" { "pending" } else { "skipped" },
                "voice": "",
                "created_ts": now,
                "updated_ts": now,
                "last_error": "",
            });
            if let Some(cleaned) = clean_ledger_row(&message.message_id, &row) {
                ledger.insert(message.message_id.clone(), Value::Object(cleaned));
                update.rows_created += 1;
            }
        }
        trim_ledger_map(&mut ledger, crate::voice_worker::ledger::DELIVERY_LEDGER_MAX);
        write_value(&path, &Value::Object(ledger))?;
        Ok(update)
    })
    .map_err(|(_, message)| message)?;
    Ok(VoiceScanReport {
        sessions_scanned: 0,
        messages_seen: messages.len(),
        rows_created: update.rows_created,
        rows_replaced: update.rows_replaced,
    })
}

fn read_clean_ledger(path: &Path) -> Map<String, Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Map::new();
    };
    let Ok(Value::Object(raw)) = serde_json::from_str::<Value>(&text) else {
        return Map::new();
    };
    raw.into_iter()
        .filter_map(|(message_id, row)| {
            clean_ledger_row(&message_id, &row).map(|cleaned| (message_id, Value::Object(cleaned)))
        })
        .collect()
}

fn replace_pending_same_slot(
    ledger: &mut Map<String, Value>,
    session_id: &str,
    message_class: &str,
    new_message_id: &str,
) -> usize {
    let now = now_seconds();
    let mut changed = 0;
    for (message_id, value) in ledger.iter_mut() {
        if message_id == new_message_id {
            continue;
        }
        let Some(row) = value.as_object_mut() else {
            continue;
        };
        if row.get("session_id").and_then(Value::as_str) != Some(session_id)
            || row.get("message_class").and_then(Value::as_str) != Some(message_class)
            || row.get("narrated_status").and_then(Value::as_str) != Some("pending")
        {
            continue;
        }
        row.insert("narrated_status".to_string(), json!("skipped"));
        if row.get("summary_status").and_then(Value::as_str) == Some("pending") {
            row.insert("summary_status".to_string(), json!("skipped"));
        }
        if row.get("push_status").and_then(Value::as_str) == Some("pending") {
            row.insert("push_status".to_string(), json!("skipped"));
        }
        row.insert("last_error".to_string(), json!("replaced by newer message"));
        row.insert("updated_ts".to_string(), json!(now));
        changed += 1;
    }
    changed
}

fn trim_ledger_map(ledger: &mut Map<String, Value>, max_rows: usize) {
    if ledger.len() <= max_rows {
        return;
    }
    let mut rows = ledger
        .iter()
        .map(|(id, row)| (id.clone(), ledger_sort_ts(row)))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (id, _) in rows.into_iter().take(ledger.len() - max_rows) {
        ledger.remove(&id);
    }
}

fn ledger_sort_ts(row: &Value) -> f64 {
    row.get("updated_ts")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
        .or_else(|| row.get("created_ts").and_then(Value::as_f64))
        .unwrap_or(0.0)
}

fn load_voice_settings(app_dir: &Path) -> Map<String, Value> {
    let path = app_dir.join("voice_settings.json");
    let raw = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    raw.as_ref()
        .and_then(clean_voice_settings)
        .unwrap_or_else(default_voice_settings)
}

fn session_display_name(row: &SessionRow) -> String {
    if !row.alias.trim().is_empty() {
        return row.alias.trim().to_string();
    }
    Path::new(&row.cwd)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Session")
        .to_string()
}

fn fallback_message_id(message_class: &str, text: &str, ts: Option<f64>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(message_class.as_bytes());
    hasher.update(b"\0");
    hasher.update(compact_text(text).as_bytes());
    hasher.update(b"\0");
    if let Some(ts) = ts {
        hasher.update(format!("{:.3}", ts).as_bytes());
    }
    format!("{:x}", hasher.finalize())[..24].to_string()
}

fn compact_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip_text(raw: &str, limit: usize) -> String {
    let text = compact_text(raw);
    if text.chars().count() <= limit {
        return text;
    }
    let mut out = text
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    out = out.trim_end().to_string();
    out.push_str("...");
    out
}

#[allow(dead_code)]
fn internal_error(message: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, message)
}
