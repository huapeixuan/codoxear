use crate::app_state::AppState;
use crate::broker_client::broker_ui_state;
use crate::log_normalizer::{codex, pi};
use crate::routes::{internal_error, json_response};
use crate::session_loader::find_session;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path as FsPath;
use std::time::Duration;

#[derive(Deserialize)]
pub struct MessagesQuery {
    offset: Option<String>,
    limit: Option<String>,
    before: Option<String>,
    init: Option<String>,
}

#[derive(Deserialize)]
pub struct LiveQuery {
    offset: Option<String>,
    live_offset: Option<String>,
    requests_version: Option<String>,
}

pub async fn messages(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<MessagesQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let offset = parse_nonnegative(query.offset.as_deref(), "offset", 0);
    let before = parse_nonnegative(query.before.as_deref(), "before", 0);
    let limit = parse_limit(query.limit.as_deref());
    let init = query.init.as_deref() == Some("1");
    match messages_payload(&row, offset, limit, before, init) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(message) => json_response(StatusCode::BAD_GATEWAY, json!({"error": message})),
    }
}

pub async fn tail(State(state): State<AppState>, Path(session_id): Path<String>) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let tail = row
        .log_path
        .as_deref()
        .map(FsPath::new)
        .and_then(|path| read_tail(path, 64 * 1024).ok())
        .unwrap_or_default();
    json_response(StatusCode::OK, json!({"tail": tail}))
}

pub async fn live(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<LiveQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let offset = parse_nonnegative(query.offset.as_deref(), "offset", 0);
    let live_offset = parse_nonnegative(query.live_offset.as_deref(), "live_offset", offset);
    let client_requests_version = query.requests_version.as_deref();
    match messages_payload(&row, offset, 80, 0, false) {
        Ok(payload) => json_response(
            StatusCode::OK,
            live_payload(
                &row,
                &state.config.app_dir,
                payload,
                offset,
                live_offset,
                client_requests_version,
            ),
        ),
        Err(message) => json_response(StatusCode::BAD_GATEWAY, json!({"error": message})),
    }
}

fn live_payload(
    row: &crate::models::SessionRow,
    app_dir: &FsPath,
    page: Value,
    offset: usize,
    live_offset: usize,
    client_requests_version: Option<&str>,
) -> Value {
    let mut requests = Vec::new();
    if row.backend == "pi" {
        let sock_path = default_sock_path(row, app_dir);
        if let Ok(ui) = broker_ui_state(&sock_path, Duration::from_millis(1500)) {
            requests = ui
                .raw
                .get("requests")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| item.is_object())
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
        }
    }
    let requests_version = ui_requests_version(&requests);
    let merged_events = page.get("events").cloned().unwrap_or_else(|| json!([]));
    let mut payload = json!({
        "ok": true,
        "session_id": row.session_id,
        "offset": page.get("offset").cloned().unwrap_or_else(|| json!(offset)),
        "live_offset": live_offset,
        "busy": page.get("busy").cloned().unwrap_or_else(|| json!(row.busy)),
        "events": merged_events,
        "requests_version": requests_version,
    });
    if client_requests_version != payload.get("requests_version").and_then(Value::as_str) {
        payload["requests"] = Value::Array(requests);
    }
    payload
}

fn messages_payload(
    row: &crate::models::SessionRow,
    offset: usize,
    limit: usize,
    before: usize,
    init: bool,
) -> Result<Value, String> {
    let state_token = state_for_row(row);
    let queue_len = row.queue_len;
    let log_path_opt = if row.backend == "pi" {
        row.session_path.as_deref().or(row.log_path.as_deref())
    } else {
        row.log_path.as_deref()
    };
    let Some(log_path) = log_path_opt else {
        return Ok(pending_log_payload(row, queue_len, state_token));
    };
    let path = FsPath::new(log_path);
    if !path.exists() {
        return Ok(pending_log_payload(row, queue_len, state_token));
    }
    if row.backend == "pi" {
        let page =
            pi::messages_from_pi_log(path, offset, limit, init, (before > 0).then_some(before))?;
        return Ok(common_payload_from_value(row, page, queue_len, state_token));
    }
    let page =
        codex::messages_from_codex_log(path, offset, limit, init, (before > 0).then_some(before))?;
    Ok(json!({
        "thread_id": row.thread_id,
        "log_path": log_path,
        "offset": page.offset,
        "events": page.events,
        "meta_delta": {"thinking": 0, "tool": 0, "system": 0},
        "turn_start": false,
        "turn_end": false,
        "turn_aborted": false,
        "diag": {"tool_names": [], "last_tool": null, "meta_refresh_ms": 0.0},
        "busy": row.busy || !page.idle.unwrap_or(true),
        "queue_len": queue_len,
        "token": state_token.or(page.token),
        "has_older": page.has_older,
        "next_before": 0,
    }))
}

fn common_payload_from_value(
    row: &crate::models::SessionRow,
    page: Value,
    queue_len: usize,
    token: Option<Value>,
) -> Value {
    json!({
        "thread_id": row.thread_id,
        "log_path": row.log_path,
        "offset": page.get("offset").cloned().unwrap_or_else(|| json!(0)),
        "events": page.get("events").cloned().unwrap_or_else(|| json!([])),
        "meta_delta": {"thinking": 0, "tool": 0, "system": 0},
        "turn_start": false,
        "turn_end": false,
        "turn_aborted": false,
        "diag": {"tool_names": [], "last_tool": null},
        "busy": row.busy,
        "queue_len": queue_len,
        "token": token,
        "has_older": page.get("has_older").cloned().unwrap_or_else(|| json!(false)),
        "next_before": 0,
    })
}

fn pending_log_payload(
    row: &crate::models::SessionRow,
    queue_len: usize,
    token: Option<Value>,
) -> Value {
    json!({
        "thread_id": row.thread_id,
        "log_path": null,
        "offset": 0,
        "events": [],
        "meta_delta": {"thinking": 0, "tool": 0, "system": 0},
        "turn_start": false,
        "turn_end": false,
        "turn_aborted": false,
        "diag": {"pending_log": true},
        "busy": false,
        "queue_len": queue_len,
        "token": token,
        "has_older": false,
        "next_before": 0,
    })
}

fn state_for_row(row: &crate::models::SessionRow) -> Option<Value> {
    row.token.is_object().then(|| row.token.clone())
}

fn default_sock_path(row: &crate::models::SessionRow, app_dir: &FsPath) -> std::path::PathBuf {
    app_dir
        .join("socks")
        .join(format!("{}.sock", row.session_id))
}

fn ui_requests_version(requests: &[Value]) -> String {
    let sorted: Vec<_> = requests.iter().map(SortedJson::from).collect();
    let canonical = serde_json::to_vec(&sorted).unwrap_or_default();
    let digest = Sha256::digest(canonical);
    URL_SAFE_NO_PAD.encode(&digest[..12])
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum SortedJson<'a> {
    Null,
    Bool(bool),
    Number(&'a serde_json::Number),
    String(&'a str),
    Array(Vec<SortedJson<'a>>),
    Object(BTreeMap<&'a str, SortedJson<'a>>),
}

impl<'a> From<&'a Value> for SortedJson<'a> {
    fn from(value: &'a Value) -> Self {
        match value {
            Value::Null => SortedJson::Null,
            Value::Bool(value) => SortedJson::Bool(*value),
            Value::Number(value) => SortedJson::Number(value),
            Value::String(value) => SortedJson::String(value),
            Value::Array(items) => SortedJson::Array(items.iter().map(SortedJson::from).collect()),
            Value::Object(object) => SortedJson::Object(
                object
                    .iter()
                    .map(|(key, value)| (key.as_str(), SortedJson::from(value)))
                    .collect(),
            ),
        }
    }
}

fn parse_nonnegative(raw: Option<&str>, _field: &str, default: usize) -> usize {
    raw.and_then(|value| value.parse::<isize>().ok())
        .map(|value| value.max(0) as usize)
        .unwrap_or(default)
}

fn parse_limit(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.parse::<isize>().ok())
        .map(|value| value.clamp(20, 200) as usize)
        .unwrap_or(80)
}

fn read_tail(path: &FsPath, max_bytes: usize) -> Result<String, String> {
    let raw = fs::read(path).map_err(|err| err.to_string())?;
    let start = raw.len().saturating_sub(max_bytes);
    Ok(String::from_utf8_lossy(&raw[start..]).to_string())
}

#[cfg(test)]
mod tests {
    use super::ui_requests_version;
    use serde_json::json;

    #[test]
    fn ui_requests_version_uses_python_sorted_key_canonical_json() {
        let requests = vec![json!({
            "z": 1,
            "a": {"b": 2, "a": 1},
            "method": "update_plan",
            "text": "嗨"
        })];

        assert_eq!(ui_requests_version(&requests), "Kt4bMxFAZ8m3UacQ");
    }
}
