use crate::app_state::AppState;
use crate::broker_client::{broker_send, BrokerError};
use crate::state_files::{read_array_file, with_state_file_lock, write_array_file};
use crate::write_cleaners::clean_queue_items;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;

pub fn env_flag_truthy(name: &str) -> bool {
    env_flag_truthy_value(std::env::var(name).ok().as_deref())
}

pub fn env_flag_truthy_value(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed != "0" && !trimmed.eq_ignore_ascii_case("false")
}

pub fn spawn_enabled_workers(state: AppState) {
    if env_flag_truthy("CODOXEAR_ENABLE_QUEUE_SWEEP") {
        let state2 = state.clone();
        tokio::spawn(async move { queue_worker_loop(state2).await });
    }
    if env_flag_truthy("CODOXEAR_ENABLE_HARNESS_SWEEP") {
        let state2 = state;
        tokio::spawn(async move { harness_worker_loop(state2).await });
    }
}

async fn queue_worker_loop(state: AppState) {
    let interval = seconds_env("CODEX_WEB_QUEUE_SWEEP_SECONDS", 1.0);
    loop {
        if let Err(error) = queue_sweep_once(&state) {
            tracing::warn!(error, "queue worker sweep failed");
        }
        sleep(Duration::from_secs_f64(interval.max(0.1))).await;
    }
}

async fn harness_worker_loop(state: AppState) {
    let interval = seconds_env("CODEX_WEB_HARNESS_SWEEP_SECONDS", 2.5);
    loop {
        if let Err(error) = harness_sweep_once(&state) {
            tracing::warn!(error, "harness worker sweep failed");
        }
        sleep(Duration::from_secs_f64(interval.max(0.1))).await;
    }
}

pub fn queue_sweep_once(state: &AppState) -> Result<bool, String> {
    let sessions = crate::session_loader::load_session_rows(&state.config)?;
    let known = sessions
        .iter()
        .map(|row| row.session_id.as_str())
        .collect::<HashSet<_>>();
    let path = state.config.app_dir.join("session_queues.json");
    with_state_file_lock(&path, || {
        let mut queues = read_array_file(&path, clean_queue_items)
            .map_err(|(_, message)| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, message))?;
        let before = queues.len();
        queues.retain(|session_id, items| known.contains(session_id.as_str()) && !items.is_empty());
        if queues.len() != before {
            write_array_file(&path, &queues)?;
        }
        Ok(())
    })
    .map_err(|(_, message)| message)?;

    for row in sessions {
        let Some(item) = next_queue_item(state, &row.session_id)? else {
            continue;
        };
        if row.busy || row.queue_len > 0 || row.broker_busy {
            continue;
        }
        let text = queue_item_text(&item).ok_or_else(|| "invalid queue item".to_string())?;
        let images = item
            .get("images")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let sock = state
            .config
            .app_dir
            .join("socks")
            .join(format!("{}.sock", row.session_id));
        let broker = broker_send(
            &sock,
            &text,
            if row.backend == "pi" {
                Some(images)
            } else {
                None
            },
            Duration::from_secs_f64(3.0),
        )
        .map_err(map_broker_error)?;
        if let Some(error) = broker.get("error").and_then(Value::as_str) {
            return Err(error.to_string());
        }
        pop_queue_item(state, &row.session_id, &item)?;
        return Ok(true);
    }
    Ok(false)
}

pub fn harness_sweep_once(state: &AppState) -> Result<bool, String> {
    let harness_path = state.config.app_dir.join("harness.json");
    let harness =
        crate::state_files::read_object_file(&harness_path).map_err(|(_, message)| message)?;
    for row in crate::session_loader::load_session_rows(&state.config)? {
        let Some(config) = harness.get(&row.session_id).and_then(Value::as_object) else {
            continue;
        };
        if config.get("enabled").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        if row.busy
            || row.queue_len > 0
            || row.broker_busy
            || local_queue_len(state, &row.session_id)? > 0
        {
            continue;
        }
        let remaining = config
            .get("remaining_injections")
            .and_then(Value::as_i64)
            .unwrap_or(10);
        if remaining <= 0 {
            disable_harness(state, &row.session_id)?;
            continue;
        }
        let request = config.get("request").and_then(Value::as_str).unwrap_or("");
        let prompt = render_harness_prompt(request);
        let sock = state
            .config
            .app_dir
            .join("socks")
            .join(format!("{}.sock", row.session_id));
        let broker = broker_send(&sock, &prompt, None, Duration::from_secs_f64(3.0))
            .map_err(map_broker_error)?;
        if let Some(error) = broker.get("error").and_then(Value::as_str) {
            return Err(error.to_string());
        }
        decrement_harness_remaining(state, &row.session_id, remaining)?;
        return Ok(true);
    }
    Ok(false)
}

fn next_queue_item(state: &AppState, session_id: &str) -> Result<Option<Value>, String> {
    let path = state.config.app_dir.join("session_queues.json");
    let queues = crate::state_files::read_array_file(&path, clean_queue_items)
        .map_err(|(_, message)| message)?;
    Ok(queues
        .get(session_id)
        .and_then(|items| items.first())
        .cloned())
}

fn pop_queue_item(state: &AppState, session_id: &str, expected: &Value) -> Result<(), String> {
    let path = state.config.app_dir.join("session_queues.json");
    with_state_file_lock(&path, || {
        let mut queues = read_array_file(&path, clean_queue_items)?;
        if let Some(items) = queues.get_mut(session_id) {
            if items.first() == Some(expected) {
                items.remove(0);
            }
            if items.is_empty() {
                queues.remove(session_id);
            }
        }
        write_array_file(&path, &queues)
    })
    .map_err(|(_, message)| message)
}

fn local_queue_len(state: &AppState, session_id: &str) -> Result<usize, String> {
    let path = state.config.app_dir.join("session_queues.json");
    let queues = crate::state_files::read_array_file(&path, clean_queue_items)
        .map_err(|(_, message)| message)?;
    Ok(queues.get(session_id).map(Vec::len).unwrap_or(0))
}

fn disable_harness(state: &AppState, session_id: &str) -> Result<(), String> {
    let path = state.config.app_dir.join("harness.json");
    with_state_file_lock(&path, || {
        let mut harness = crate::state_files::read_object_file(&path)?;
        if let Some(entry) = harness.get_mut(session_id).and_then(Value::as_object_mut) {
            entry.insert("enabled".to_string(), json!(false));
            entry.insert("remaining_injections".to_string(), json!(0));
        }
        crate::state_files::write_object_file(&path, &harness)
    })
    .map_err(|(_, message)| message)
}

fn decrement_harness_remaining(
    state: &AppState,
    session_id: &str,
    remaining: i64,
) -> Result<(), String> {
    let path = state.config.app_dir.join("harness.json");
    with_state_file_lock(&path, || {
        let mut harness = crate::state_files::read_object_file(&path)?;
        if let Some(entry) = harness.get_mut(session_id).and_then(Value::as_object_mut) {
            let next = (remaining - 1).max(0);
            entry.insert("remaining_injections".to_string(), json!(next));
            if next <= 0 {
                entry.insert("enabled".to_string(), json!(false));
            }
        }
        crate::state_files::write_object_file(&path, &harness)
    })
    .map_err(|(_, message)| message)
}

fn queue_item_text(item: &Value) -> Option<String> {
    match item {
        Value::String(value) => Some(value.trim().to_string()).filter(|value| !value.is_empty()),
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        _ => None,
    }
}

fn render_harness_prompt(request: &str) -> String {
    let request = request.trim();
    if request.is_empty() {
        "Continue working autonomously. Inspect the current context, make useful progress, run relevant verification, and report concise results.".to_string()
    } else {
        format!("Continue working autonomously. User request: {request}")
    }
}

fn seconds_env(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

fn map_broker_error(error: BrokerError) -> String {
    match error {
        BrokerError::ConnectRefused | BrokerError::Timeout | BrokerError::Empty => {
            "broker unavailable".to_string()
        }
        other => other.to_string(),
    }
}
