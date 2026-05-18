use crate::app_state::AppState;
use crate::broker_client::{broker_send, BrokerError};
use crate::log_normalizer::codex::{idle_from_log, last_chat_role_ts_from_log};
use crate::state_files::{read_array_file, with_state_file_lock, write_array_file};
use crate::write_cleaners::clean_queue_items;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;

const QUEUE_IDLE_GRACE_SECONDS: f64 = 10.0;
const LOG_IDLE_SCAN_BYTES: usize = 8 * 1024 * 1024;
const HARNESS_SCAN_BYTES: usize = 8 * 1024 * 1024;
const HARNESS_PROMPT_PREFIX: &str = r#"Unattended-mode instructions (optimize for 8+ hours, minimal turns, minimal repetition, maximal progress)

- Maintain four internal sections:
  1. Deliverables
     - The concrete outputs the agent owes the user by the end of the task.
     - Stable unless the user changes the request.
  2. Completed
     - Verified facts already established while producing the Deliverables.
  3. Next actions
     - Ordered concrete steps from the current state toward the Deliverables.
  4. Parked user decisions
     - Decisions or inputs that only the user can provide.

- Working rules:
  - Keep these sections internal. Surface them only when yielding is necessary.
  - Default to continuing in the same turn.
  - Before each action, reason until the approach, failure modes, and verification path are clear.
  - Exploration should happen through reading, tracing, inspection, and reasoning.
  - Avoid trial and error.
  - Resolve crashes, bugs, and design mistakes yourself unless a true user decision is required.
  - Use the strongest available verification.
  - Do not repeat the same command, edit, or analysis without a concrete new reason.

- Yield only when:
  - all Deliverables are finished and supported by Completed;
  - the only remaining gap is a Parked user decision;
  - or the next step is irreversible or high-risk and needs explicit user confirmation.

- End-of-turn gate (only when yielding is necessary):
  - Run a clean-room adversarial review via a dedicated subagent.
  - Give it: user intent, Deliverables, Completed, remaining Next actions, Parked user decisions, constraints, and changed artifacts.
  - Apply findings before yielding, or surface the exact remaining user decision or risk.
"#;

static QUEUE_IDLE_SINCE: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();
static HARNESS_LAST_INJECTED: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();
static HARNESS_SCOPE_LAST_INJECTED: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();

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
        let state2 = state.clone();
        tokio::spawn(async move { harness_worker_loop(state2).await });
    }
    if let Some(role) = voice_worker_role(
        env_flag_truthy("CODOXEAR_ENABLE_VOICE_SCAN"),
        env_flag_truthy("CODOXEAR_ENABLE_VOICE_WORKER"),
    ) {
        spawn_voice_worker_owner(state, role);
    }
}

pub fn voice_worker_role(scan_enabled: bool, worker_enabled: bool) -> Option<&'static str> {
    match (scan_enabled, worker_enabled) {
        (false, false) => None,
        (true, false) => Some("scan"),
        (false, true) => Some("worker"),
        (true, true) => Some("scan+worker"),
    }
}

fn spawn_voice_worker_owner(state: AppState, role: &'static str) {
    tokio::spawn(async move {
        let guard = match crate::voice_worker::locks::VoiceOwnerLock::acquire(
            &state.config.app_dir,
            role,
        ) {
            Ok(guard) => guard,
            Err(error) => {
                tracing::warn!(error, role, "voice worker owner lock conflict");
                return;
            }
        };
        tracing::info!(path = %guard.path().display(), role, "voice worker owner lock acquired");
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    });
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
    queue_sweep_once_at(state, now_seconds())
}

pub fn queue_sweep_once_at(state: &AppState, now: f64) -> Result<bool, String> {
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
        let key = queue_state_key(&state.config.app_dir, &row.session_id);
        let Some(item) = next_queue_item(state, &row.session_id)? else {
            clear_queue_idle(&key);
            continue;
        };
        if row.busy || row.queue_len > 0 || row.broker_busy {
            clear_queue_idle(&key);
            continue;
        }
        if !queue_log_idle(&row.log_path)? {
            clear_queue_idle(&key);
            continue;
        }
        if !idle_grace_elapsed(&key, now) {
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
            clear_queue_idle(&key);
            return Err(error.to_string());
        }
        pop_queue_item(state, &row.session_id, &item)?;
        clear_queue_idle(&key);
        return Ok(true);
    }
    Ok(false)
}

pub fn harness_sweep_once(state: &AppState) -> Result<bool, String> {
    harness_sweep_once_at(state, now_seconds())
}

pub fn worker_state_reset_for_tests() {
    queue_idle_map().lock().unwrap().clear();
    last_injected_map().lock().unwrap().clear();
    scope_injected_map().lock().unwrap().clear();
}

pub fn harness_sweep_once_at(state: &AppState, now: f64) -> Result<bool, String> {
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
        let cooldown_seconds = config
            .get("cooldown_minutes")
            .and_then(Value::as_f64)
            .unwrap_or(5.0)
            .max(1.0)
            * 60.0;
        let remaining = config
            .get("remaining_injections")
            .and_then(Value::as_i64)
            .unwrap_or(10);
        if remaining <= 0 {
            disable_harness(state, &row.session_id)?;
            continue;
        }
        let session_key = queue_state_key(&state.config.app_dir, &row.session_id);
        if !cooldown_elapsed(last_injected_map(), &session_key, now, cooldown_seconds) {
            continue;
        }
        let Some(log_path) = row
            .log_path
            .as_deref()
            .filter(|path| Path::new(path).exists())
        else {
            continue;
        };
        let scope_key =
            if let Some(thread_id) = row.thread_id.as_deref().filter(|value| !value.is_empty()) {
                format!("thread:{thread_id}")
            } else {
                format!("log:{log_path}")
            };
        if !cooldown_elapsed(scope_injected_map(), &scope_key, now, cooldown_seconds) {
            continue;
        }
        if row.busy
            || row.queue_len > 0
            || row.broker_busy
            || local_queue_len(state, &row.session_id)? > 0
        {
            continue;
        }
        let Some((role, ts)) = last_chat_role_ts_from_log(Path::new(log_path), HARNESS_SCAN_BYTES)?
        else {
            continue;
        };
        if role != "assistant" || (now - ts) < cooldown_seconds {
            continue;
        }
        if !cooldown_elapsed(scope_injected_map(), &scope_key, now, cooldown_seconds) {
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
        set_cooldown(last_injected_map(), session_key, now);
        set_cooldown(scope_injected_map(), scope_key, now);
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
    let base = HARNESS_PROMPT_PREFIX.trim_end();
    let request = request.trim();
    if request.is_empty() {
        format!("{base}\n")
    } else {
        format!("{base}\n\n---\n\nAdditional request from user: {request}\n")
    }
}

fn queue_log_idle(log_path: &Option<String>) -> Result<bool, String> {
    let Some(path) = log_path.as_deref().filter(|path| Path::new(path).exists()) else {
        return Ok(true);
    };
    Ok(idle_from_log(Path::new(path), LOG_IDLE_SCAN_BYTES)?.unwrap_or(true))
}

fn queue_state_key(app_dir: &Path, session_id: &str) -> String {
    format!("{}::{session_id}", app_dir.display())
}

fn queue_idle_map() -> &'static Mutex<HashMap<String, f64>> {
    QUEUE_IDLE_SINCE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn last_injected_map() -> &'static Mutex<HashMap<String, f64>> {
    HARNESS_LAST_INJECTED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scope_injected_map() -> &'static Mutex<HashMap<String, f64>> {
    HARNESS_SCOPE_LAST_INJECTED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clear_queue_idle(key: &str) {
    queue_idle_map().lock().unwrap().remove(key);
}

fn idle_grace_elapsed(key: &str, now: f64) -> bool {
    let grace = seconds_env(
        "CODEX_WEB_QUEUE_IDLE_GRACE_SECONDS",
        QUEUE_IDLE_GRACE_SECONDS,
    );
    let mut map = queue_idle_map().lock().unwrap();
    let Some(since) = map.get(key).copied() else {
        map.insert(key.to_string(), now);
        return false;
    };
    (now - since) >= grace
}

fn cooldown_elapsed(
    map: &'static Mutex<HashMap<String, f64>>,
    key: &str,
    now: f64,
    cooldown_seconds: f64,
) -> bool {
    let map = map.lock().unwrap();
    map.get(key)
        .map(|last| (now - *last) >= cooldown_seconds)
        .unwrap_or(true)
}

fn set_cooldown(map: &'static Mutex<HashMap<String, f64>>, key: String, now: f64) {
    map.lock().unwrap().insert(key, now);
}

fn seconds_env(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

fn map_broker_error(error: BrokerError) -> String {
    match error {
        BrokerError::ConnectRefused | BrokerError::Timeout | BrokerError::Empty => {
            "broker unavailable".to_string()
        }
        other => other.to_string(),
    }
}
