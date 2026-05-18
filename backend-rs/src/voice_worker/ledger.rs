use crate::state_files::{with_state_file_lock, write_value};
use crate::voice_state::{clean_ledger_row, now_seconds, value_f64_public};
use axum::http::StatusCode;
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::fs;
use std::path::Path;

pub const DELIVERY_LEDGER_FILE: &str = "voice_delivery_ledger.json";
pub const DELIVERY_LEDGER_MAX: usize = 4000;

pub fn read_ledger(app_dir: &Path) -> Map<String, Value> {
    let path = app_dir.join(DELIVERY_LEDGER_FILE);
    let Ok(text) = fs::read_to_string(path) else {
        return Map::new();
    };
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(&text) else {
        return Map::new();
    };
    let mut cleaned = Map::new();
    for (message_id, row) in object {
        if let Some(record) = clean_ledger_row(&message_id, &row) {
            cleaned.insert(message_id, Value::Object(record));
        }
    }
    cleaned
}

pub fn write_ledger_trimmed(
    app_dir: &Path,
    rows: Vec<Value>,
    max_rows: usize,
) -> Result<(), (StatusCode, String)> {
    let path = app_dir.join(DELIVERY_LEDGER_FILE);
    with_state_file_lock(&path, || {
        let mut rows = rows
            .into_iter()
            .filter_map(|value| {
                let id = value
                    .get("message_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)?;
                clean_ledger_row(&id, &value).map(|row| (id, Value::Object(row)))
            })
            .collect::<Vec<_>>();
        rows.sort_by(|(_, left), (_, right)| {
            ledger_sort_ts(left)
                .partial_cmp(&ledger_sort_ts(right))
                .unwrap_or(Ordering::Equal)
        });
        if rows.len() > max_rows {
            rows.drain(0..rows.len() - max_rows);
        }
        let object = rows.into_iter().collect::<Map<String, Value>>();
        write_value(&path, &Value::Object(object))
    })
}

pub fn new_pending_final_response(
    message_id: &str,
    session_id: &str,
    session_display_name: &str,
    preview_text: &str,
) -> Value {
    let now = now_seconds();
    json!({
        "message_id": message_id,
        "session_id": session_id,
        "session_display_name": session_display_name,
        "message_class": "final_response",
        "preview_text": preview_text,
        "notification_text": "",
        "summary_text": "",
        "summary_status": "pending",
        "narrated_status": "pending",
        "push_status": "pending",
        "voice": "",
        "created_ts": now,
        "updated_ts": now,
        "last_error": "",
    })
}

fn ledger_sort_ts(row: &Value) -> f64 {
    let updated = value_f64_public(row.get("updated_ts").unwrap_or(&Value::Null));
    if updated > 0.0 {
        updated
    } else {
        value_f64_public(row.get("created_ts").unwrap_or(&Value::Null))
    }
}
