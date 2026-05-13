use crate::broker_client::broker_state;
use crate::models::SessionRow;
use crate::runtime::RuntimeConfig;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const HARNESS_DEFAULT_IDLE_MINUTES: f64 = 5.0;
const HARNESS_DEFAULT_MAX_INJECTIONS: i64 = 10;
const SIDEBAR_PRIORITY_HALF_LIFE_SECONDS: f64 = 8.0 * 3600.0;
const SESSION_LIST_FALLBACK_GROUP_KEY: &str = "__no_working_directory__";
const SESSION_LIST_GROUP_PAGE_SIZE: usize = 5;
const SESSION_LIST_RECENT_GROUP_LIMIT: usize = 3;

#[derive(Debug, Deserialize)]
struct SessionMeta {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    codex_pid: Option<i64>,
    #[serde(default)]
    broker_pid: Option<i64>,
    #[serde(default)]
    agent_backend: Option<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    transport: Option<String>,
    #[serde(default)]
    supports_live_ui: Option<bool>,
    #[serde(default)]
    ui_protocol_version: Option<i64>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    workspace_cwd: Option<String>,
    #[serde(default)]
    log_path: Option<String>,
    #[serde(default)]
    session_path: Option<String>,
    #[serde(default)]
    start_ts: Option<f64>,
    #[serde(default)]
    updated_ts: Option<f64>,
    #[serde(default)]
    busy: Option<bool>,
    #[serde(default)]
    queue_len: Option<usize>,
    #[serde(default)]
    model_provider: Option<String>,
    #[serde(default)]
    preferred_auth_method: Option<String>,
    #[serde(default)]
    provider_choice: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
    #[serde(default)]
    service_tier: Option<String>,
    #[serde(default)]
    tmux_session: Option<String>,
    #[serde(default)]
    tmux_window: Option<String>,
}

pub fn load_session_rows(config: &RuntimeConfig) -> Result<Vec<SessionRow>, String> {
    let context = SessionStateContext {
        aliases: read_string_map(&config.app_dir.join("session_aliases.json"))?,
        hidden: read_hidden_sessions(&config.app_dir.join("hidden_sessions.json"))?,
        queues: read_array_map(&config.app_dir.join("session_queues.json"))?,
        harness: read_object_map(&config.app_dir.join("harness.json"))?,
        sidebar: read_object_map(&config.app_dir.join("session_sidebar.json"))?,
        files: read_string_array_map(&config.app_dir.join("session_files.json"))?,
    };

    let mut rows = Vec::new();
    let socks_dir = config.app_dir.join("socks");
    if socks_dir.exists() {
        let mut entries = fs::read_dir(&socks_dir)
            .map_err(|err| format!("read {}: {err}", socks_dir.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("read {}: {err}", socks_dir.display()))?;
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let sock_path = entry.path();
            if sock_path.extension().and_then(|ext| ext.to_str()) != Some("sock") {
                continue;
            }
            let Some(session_id) = sock_path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if context.hidden.contains(session_id) {
                continue;
            }
            let meta_path = sock_path.with_extension("json");
            let meta: SessionMeta = read_json_file(&meta_path)?;
            if let Some(row) = session_from_meta(session_id, &sock_path, meta, &context)? {
                rows.push(row);
            }
        }
    }

    rows.sort_by(session_sort_key);
    Ok(rows)
}

pub fn find_session(config: &RuntimeConfig, session_id: &str) -> Result<SessionRow, String> {
    load_session_rows(config)?
        .into_iter()
        .find(|row| row.session_id == session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))
}

pub fn priority_from_elapsed_seconds(elapsed: f64) -> f64 {
    if elapsed <= 0.0 {
        return 1.0;
    }
    let lambda = std::f64::consts::LN_2 / SIDEBAR_PRIORITY_HALF_LIFE_SECONDS;
    clip01((-lambda * elapsed).exp())
}

pub fn clip01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

pub fn session_list_visible_grouped_rows(
    rows: &[SessionRow],
    cwd_groups: &Map<String, Value>,
) -> HashMap<String, Vec<SessionRow>> {
    let hidden_group_keys = cwd_groups
        .iter()
        .filter(|(_, entry)| {
            entry
                .get("hidden")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .map(|(cwd, _)| cwd.clone())
        .collect::<HashSet<_>>();
    let mut grouped: HashMap<String, Vec<SessionRow>> = HashMap::new();
    for row in rows {
        let key = session_list_group_key(row);
        if hidden_group_keys.contains(&key) || existing_workspace_dir(&key).is_none() {
            continue;
        }
        grouped.entry(key).or_default().push(row.clone());
    }
    grouped
}

pub fn frontend_session_list_row(row: &SessionRow) -> Value {
    json!({
        "session_id": row.session_id,
        "thread_id": row.thread_id,
        "title": row.title,
        "alias": row.alias,
        "first_user_message": row.first_user_message,
        "cwd": row.cwd,
        "agent_backend": row.agent_backend,
        "owned": row.owned,
        "busy": row.busy,
        "queue_len": row.queue_len,
        "git_branch": row.git_branch,
        "pr_summary": row.pr_summary,
        "transport": row.transport,
        "blocked": row.blocked,
        "snoozed": row.snoozed,
        "historical": false,
    })
}

pub fn load_sessions_directories_payload(
    rows: &[SessionRow],
    cwd_groups: &Map<String, Value>,
    group_key: Option<&str>,
    offset: usize,
    limit: usize,
    group_offset: usize,
    group_limit: usize,
) -> Value {
    let grouped = session_list_visible_grouped_rows(rows, cwd_groups);
    let mut group_order = grouped.keys().cloned().collect::<Vec<_>>();
    group_order.sort_by(|a, b| session_list_group_sort_key(&grouped, a, b));

    if let Some(key) = group_key {
        let group_rows = grouped.get(key).cloned().unwrap_or_default();
        let stop = offset.saturating_add(limit.max(1));
        let sessions = group_rows[offset.min(group_rows.len())..stop.min(group_rows.len())]
            .iter()
            .map(frontend_session_list_row)
            .collect::<Vec<_>>();
        let remaining = group_rows.len().saturating_sub(stop);
        let mut remaining_by_group = Map::new();
        if remaining > 0 {
            remaining_by_group.insert(key.to_string(), json!(remaining));
        }
        return json!({"sessions": sessions, "remaining_by_group": remaining_by_group});
    }

    let mut selected = group_order
        .iter()
        .take(SESSION_LIST_RECENT_GROUP_LIMIT)
        .cloned()
        .collect::<HashSet<_>>();
    for (key, group_rows) in &grouped {
        if group_rows.iter().any(|row| row.busy) {
            selected.insert(key.clone());
        }
    }

    let mut omitted_group_count = 0usize;
    if group_offset > 0 || group_limit != SESSION_LIST_RECENT_GROUP_LIMIT {
        let group_stop = group_offset.saturating_add(group_limit.max(1));
        selected = group_order
            [group_offset.min(group_order.len())..group_stop.min(group_order.len())]
            .iter()
            .cloned()
            .collect();
        omitted_group_count = group_order.len().saturating_sub(group_stop);
    }

    let mut sessions = Vec::new();
    let mut remaining_by_group = Map::new();
    for key in &group_order {
        if !selected.contains(key) {
            continue;
        }
        let group_rows = grouped.get(key).expect("ordered key exists");
        let page_stop = SESSION_LIST_GROUP_PAGE_SIZE.min(group_rows.len());
        sessions.extend(
            group_rows[..page_stop]
                .iter()
                .map(frontend_session_list_row),
        );
        let remaining = group_rows.len().saturating_sub(page_stop);
        if remaining > 0 {
            remaining_by_group.insert(key.clone(), json!(remaining));
        }
    }
    if group_offset == 0 && group_limit == SESSION_LIST_RECENT_GROUP_LIMIT {
        omitted_group_count = group_order.len().saturating_sub(selected.len());
    }
    json!({
        "sessions": sessions,
        "remaining_by_group": remaining_by_group,
        "omitted_group_count": omitted_group_count,
    })
}

pub fn load_sessions_recent_payload(
    rows: &[SessionRow],
    cwd_groups: &Map<String, Value>,
    offset: usize,
    limit: usize,
) -> Value {
    let grouped = session_list_visible_grouped_rows(rows, cwd_groups);
    let mut group_order = grouped.keys().cloned().collect::<Vec<_>>();
    group_order.sort_by(|a, b| session_list_group_sort_key(&grouped, a, b));
    let mut ordered_rows = Vec::new();
    for key in &group_order {
        if let Some(group_rows) = grouped.get(key) {
            ordered_rows.extend(group_rows.clone());
        }
    }
    let mut indexed = ordered_rows.into_iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by(|(left_index, left), (right_index, right)| {
        right
            .busy
            .cmp(&left.busy)
            .then_with(|| sort_f64_desc(left.updated_ts, right.updated_ts))
            .then_with(|| sort_f64_desc(left.start_ts, right.start_ts))
            .then_with(|| left_index.cmp(right_index))
    });
    let recent_rows = indexed.into_iter().map(|(_, row)| row).collect::<Vec<_>>();
    let stop = offset.saturating_add(limit.max(1));
    let sessions = recent_rows[offset.min(recent_rows.len())..stop.min(recent_rows.len())]
        .iter()
        .map(frontend_session_list_row)
        .collect::<Vec<_>>();
    let remaining = recent_rows.len().saturating_sub(stop);
    json!({"sessions": sessions, "remaining": remaining})
}

struct SessionStateContext {
    aliases: HashMap<String, String>,
    hidden: HashSet<String>,
    queues: HashMap<String, Vec<Value>>,
    harness: HashMap<String, Value>,
    sidebar: HashMap<String, Value>,
    files: HashMap<String, Vec<String>>,
}

fn session_from_meta(
    session_id: &str,
    sock_path: &Path,
    meta: SessionMeta,
    context: &SessionStateContext,
) -> Result<Option<SessionRow>, String> {
    let has_pid_metadata = meta.broker_pid.is_some() || meta.codex_pid.is_some();
    let broker_pid = meta.broker_pid.unwrap_or(0);
    let codex_pid = meta.codex_pid.unwrap_or(0);
    if has_pid_metadata && !pid_alive(broker_pid) && !pid_alive(codex_pid) {
        return Ok(None);
    }
    let cwd = required_string(meta.cwd, "cwd", session_id)?;
    let start_ts = meta.start_ts.unwrap_or_else(epoch_now);
    let updated_ts = meta.updated_ts.unwrap_or(start_ts);
    let agent_backend = normalize_backend(meta.agent_backend.or(meta.backend));
    let queue_values = context.queues.get(session_id).cloned().unwrap_or_default();
    let queue_items = queue_values
        .iter()
        .filter_map(queue_item_display_text)
        .collect::<Vec<_>>();
    let sidecar_queue_len = meta.queue_len.unwrap_or(queue_items.len());
    let sidecar_busy = meta.busy.unwrap_or(false);
    let (busy, broker_busy, queue_len, token) = match broker_state(
        sock_path,
        Duration::from_millis(1500),
    ) {
        Ok(state) => (state.busy, state.busy, state.queue_len, state.token),
        Err(err) => {
            tracing::warn!(session_id, error = %err, "broker state unavailable; using sidecar fallback");
            (sidecar_busy, false, sidecar_queue_len, Value::Null)
        }
    };
    let sidebar_entry = context.sidebar.get(session_id);
    let priority_offset = object_number(sidebar_entry, "priority_offset")
        .unwrap_or(0.0)
        .clamp(-1.0, 1.0);
    let snooze_until = object_number(sidebar_entry, "snooze_until");
    let dependency_session_id = object_string(sidebar_entry, "dependency_session_id");
    let blocked = dependency_session_id.is_some();
    let snoozed = snooze_until
        .map(|value| value > epoch_now())
        .unwrap_or(false);
    let time_priority = priority_from_elapsed_seconds((epoch_now() - updated_ts).max(0.0));
    let base_priority = clip01(time_priority + priority_offset);
    let final_priority = if blocked || snoozed {
        0.0
    } else {
        base_priority
    };
    let harness_entry = context.harness.get(session_id);
    Ok(Some(SessionRow {
        session_id: session_id.to_string(),
        thread_id: clean_optional(meta.session_id).or_else(|| Some(session_id.to_string())),
        title: None,
        alias: context.aliases.get(session_id).cloned().unwrap_or_default(),
        first_user_message: None,
        agent_backend: agent_backend.clone(),
        backend: agent_backend,
        owner: meta.owner.clone(),
        owned: meta.owner.as_deref() == Some("web"),
        transport: clean_optional(meta.transport),
        supports_live_ui: meta.supports_live_ui.unwrap_or(false),
        ui_protocol_version: meta.ui_protocol_version,
        cwd,
        workspace_cwd: clean_optional(meta.workspace_cwd),
        log_path: clean_optional(meta.log_path),
        session_path: clean_optional(meta.session_path),
        start_ts,
        updated_ts,
        broker_pid,
        codex_pid,
        busy,
        broker_busy,
        queue_len,
        queue_items,
        token,
        harness_enabled: object_bool(harness_entry, "enabled").unwrap_or(false),
        harness_cooldown_minutes: object_number(harness_entry, "cooldown_minutes")
            .unwrap_or(HARNESS_DEFAULT_IDLE_MINUTES),
        harness_remaining_injections: object_i64(harness_entry, "remaining_injections")
            .unwrap_or(HARNESS_DEFAULT_MAX_INJECTIONS),
        harness_request: object_string(harness_entry, "request").unwrap_or_default(),
        files: context.files.get(session_id).cloned().unwrap_or_default(),
        priority_offset,
        snooze_until,
        dependency_session_id,
        final_priority,
        base_priority,
        time_priority,
        blocked,
        snoozed,
        git_branch: None,
        pr_summary: Value::Null,
        todo_snapshot: json!({"available": false, "error": false, "items": []}),
        model_provider: clean_optional(meta.model_provider),
        preferred_auth_method: clean_optional(meta.preferred_auth_method),
        provider_choice: clean_optional(meta.provider_choice),
        model: clean_optional(meta.model),
        reasoning_effort: clean_optional(meta.reasoning_effort),
        service_tier: clean_optional(meta.service_tier),
        tmux_session: clean_optional(meta.tmux_session),
        tmux_window: clean_optional(meta.tmux_window),
    }))
}

fn session_sort_key(left: &SessionRow, right: &SessionRow) -> std::cmp::Ordering {
    sort_f64_desc(left.final_priority, right.final_priority)
        .then_with(|| sort_f64_desc(left.updated_ts, right.updated_ts))
        .then_with(|| sort_f64_desc(left.start_ts, right.start_ts))
        .then_with(|| left.session_id.cmp(&right.session_id))
}

fn session_list_group_key(row: &SessionRow) -> String {
    crate::runtime::normalize_cwd_group_key(&row.cwd)
        .unwrap_or_else(|| SESSION_LIST_FALLBACK_GROUP_KEY.to_string())
}

fn session_list_group_sort_key(
    grouped: &HashMap<String, Vec<SessionRow>>,
    left: &str,
    right: &str,
) -> std::cmp::Ordering {
    let left_rows = grouped.get(left).map(Vec::as_slice).unwrap_or(&[]);
    let right_rows = grouped.get(right).map(Vec::as_slice).unwrap_or(&[]);
    let left_busy = left_rows.iter().any(|row| row.busy);
    let right_busy = right_rows.iter().any(|row| row.busy);
    right_busy
        .cmp(&left_busy)
        .then_with(|| sort_f64_desc(latest_updated(left_rows), latest_updated(right_rows)))
}

fn latest_updated(rows: &[SessionRow]) -> f64 {
    rows.iter()
        .map(|row| row.updated_ts)
        .fold(0.0_f64, f64::max)
}

fn sort_f64_desc(left: f64, right: f64) -> std::cmp::Ordering {
    right
        .partial_cmp(&left)
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn existing_workspace_dir(cwd: &str) -> Option<String> {
    if cwd == SESSION_LIST_FALLBACK_GROUP_KEY {
        return Some(cwd.to_string());
    }
    let normalized = crate::runtime::normalize_cwd_group_key(cwd)?;
    fs::metadata(&normalized)
        .ok()
        .filter(|metadata| metadata.is_dir())
        .map(|_| normalized)
}

fn read_json_file<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let raw = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("parse {}: {err}", path.display()))
}

fn read_optional_value(path: &Path) -> Result<Option<Value>, String> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|err| format!("parse {}: {err}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("read {}: {err}", path.display())),
    }
}

fn read_string_map(path: &Path) -> Result<HashMap<String, String>, String> {
    let Some(Value::Object(object)) = read_optional_value(path)? else {
        return Ok(HashMap::new());
    };
    Ok(object
        .into_iter()
        .filter_map(|(key, value)| clean_optional_value(&value).map(|value| (key, value)))
        .collect())
}

fn read_object_map(path: &Path) -> Result<HashMap<String, Value>, String> {
    let Some(Value::Object(object)) = read_optional_value(path)? else {
        return Ok(HashMap::new());
    };
    Ok(object.into_iter().collect())
}

fn read_array_map(path: &Path) -> Result<HashMap<String, Vec<Value>>, String> {
    let Some(Value::Object(object)) = read_optional_value(path)? else {
        return Ok(HashMap::new());
    };
    Ok(object
        .into_iter()
        .filter_map(|(key, value)| value.as_array().cloned().map(|items| (key, items)))
        .collect())
}

fn queue_item_display_text(item: &Value) -> Option<Value> {
    let text = match item {
        Value::String(value) => value.trim().to_string(),
        Value::Object(object) => object.get("text")?.as_str()?.trim().to_string(),
        _ => return None,
    };
    if text.is_empty() {
        return None;
    }
    let image_count = item
        .get("images")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter(|item| item.is_object()).count())
        .unwrap_or(0);
    if image_count == 0 {
        return Some(Value::String(text));
    }
    let suffix = if image_count == 1 { "image" } else { "images" };
    Some(Value::String(format!("{text} [{image_count} {suffix}]")))
}

fn read_string_array_map(path: &Path) -> Result<HashMap<String, Vec<String>>, String> {
    let Some(Value::Object(object)) = read_optional_value(path)? else {
        return Ok(HashMap::new());
    };
    Ok(object
        .into_iter()
        .filter_map(|(key, value)| {
            value.as_array().map(|items| {
                (
                    key,
                    items
                        .iter()
                        .filter_map(clean_optional_value)
                        .collect::<Vec<_>>(),
                )
            })
        })
        .collect())
}

fn read_hidden_sessions(path: &Path) -> Result<HashSet<String>, String> {
    let Some(value) = read_optional_value(path)? else {
        return Ok(HashSet::new());
    };
    match value {
        Value::Array(items) => Ok(items.iter().filter_map(clean_optional_value).collect()),
        Value::Object(object) => Ok(object
            .into_iter()
            .filter_map(|(key, value)| value.as_bool().unwrap_or(true).then_some(key))
            .collect()),
        _ => Ok(HashSet::new()),
    }
}

fn object_number(object: Option<&Value>, key: &str) -> Option<f64> {
    object
        .and_then(Value::as_object)
        .and_then(|entry| entry.get(key))
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn object_i64(object: Option<&Value>, key: &str) -> Option<i64> {
    object
        .and_then(Value::as_object)
        .and_then(|entry| entry.get(key))
        .and_then(Value::as_i64)
}

fn object_bool(object: Option<&Value>, key: &str) -> Option<bool> {
    object
        .and_then(Value::as_object)
        .and_then(|entry| entry.get(key))
        .and_then(Value::as_bool)
}

fn object_string(object: Option<&Value>, key: &str) -> Option<String> {
    object
        .and_then(Value::as_object)
        .and_then(|entry| entry.get(key))
        .and_then(clean_optional_value)
}

fn required_string(value: Option<String>, label: &str, session_id: &str) -> Result<String, String> {
    clean_optional(value)
        .ok_or_else(|| format!("missing {label} in metadata for session {session_id}"))
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn clean_optional_value(value: &Value) -> Option<String> {
    value.as_str().and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn normalize_backend(value: Option<String>) -> String {
    match clean_optional(value)
        .unwrap_or_else(|| "codex".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "pi" => "pi".to_string(),
        _ => "codex".to_string(),
    }
}

fn epoch_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

fn pid_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new("/proc").join(pid.to_string()).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}
