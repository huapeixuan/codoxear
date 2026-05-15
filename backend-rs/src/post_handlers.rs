use crate::app_state::AppState;
use crate::broker_client::{broker_keys, broker_send, broker_ui_response, BrokerError};
use crate::lifecycle_post::heartbeat_impl;
use crate::models::SessionRow;
use crate::routes::{internal_error, json_response};
use crate::runtime::{
    clean_alias_public, cookie_path, cookie_secure, cookie_ttl_seconds, cwd_group_entry_public,
    load_or_create_hmac_secret, normalize_cwd_group_key, sign_auth_cookie_value, unix_now_seconds,
};
use crate::session_loader::{find_session, load_session_rows};
use crate::state_files::{
    read_array_file, read_object_file, read_string_file, with_state_file_lock, write_array_file,
    write_object_file, write_string_file,
};
use crate::write_cleaners::{
    clean_dependency_session_id, clean_harness_cooldown, clean_harness_remaining,
    clean_positive_number, clean_priority_offset, clean_queue_items, clean_snooze_until,
    legacy_ui_response_text, normalize_pi_image_inputs,
};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

pub(crate) async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    let Some(password) = payload.get("password").and_then(Value::as_str) else {
        return json_response(StatusCode::FORBIDDEN, json!({"error": "bad password"}));
    };
    if !password_matches(password) {
        return json_response(StatusCode::FORBIDDEN, json!({"error": "bad password"}));
    }
    let secret = match load_or_create_hmac_secret(&state.config.app_dir) {
        Ok(secret) => secret,
        Err(message) => return internal_error(message),
    };
    let token = match sign_auth_cookie_value(&secret, unix_now_seconds() + cookie_ttl_seconds()) {
        Ok(token) => token,
        Err(message) => return internal_error(message),
    };
    let forwarded_proto = headers
        .get("X-Forwarded-Proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = match cookie_path() {
        Ok(path) => path,
        Err(message) => return internal_error(message),
    };
    let mut attrs = vec![
        format!("codoxear_auth={token}"),
        format!("Path={path}"),
        "HttpOnly".to_string(),
        "SameSite=Strict".to_string(),
        format!("Max-Age={}", cookie_ttl_seconds()),
    ];
    if cookie_secure() || forwarded_proto == "https" {
        attrs.push("Secure".to_string());
    }
    json_response_with_cookie(StatusCode::OK, json!({"ok": true}), &attrs.join("; "))
}

pub(crate) async fn logout() -> Response {
    let path = match cookie_path() {
        Ok(path) => path,
        Err(message) => return internal_error(message),
    };
    json_response_with_cookie(
        StatusCode::OK,
        json!({"ok": true}),
        &format!(
            "codoxear_auth={}; Path={path}; Max-Age=0; HttpOnly; SameSite=Strict",
            "delet".to_string() + "ed"
        ),
    )
}

pub(crate) async fn hooks_notify() -> Response {
    json_response(StatusCode::OK, json!({"ignored": true}))
}

pub(crate) async fn cwd_group_edit(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    match cwd_group_edit_impl(&state, &payload) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn session_rename(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(name) = payload.get("name").and_then(Value::as_str) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "name required"}));
    };
    match rename_session_impl(&state, &session_id, name) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn session_edit(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(name) = payload.get("name").and_then(Value::as_str) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "name required"}));
    };
    match edit_session_impl(&state, &session_id, name, &payload) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn session_enqueue(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(text) = payload.get("text").and_then(Value::as_str) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "text required"}));
    };
    let images = match normalize_pi_image_inputs(payload.get("images").unwrap_or(&Value::Null)) {
        Ok(images) => images,
        Err(message) => return json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    };
    match enqueue_impl(&state, &session_id, text, images) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn queue_delete(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(index) = payload.get("index").and_then(Value::as_i64) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "index required"}));
    };
    match queue_delete_impl(&state, &session_id, index) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn queue_update(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(index) = payload.get("index").and_then(Value::as_i64) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "index required"}));
    };
    let Some(text) = payload.get("text").and_then(Value::as_str) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "text required"}));
    };
    match queue_update_impl(&state, &session_id, index, text) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn session_harness(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    match harness_set_impl(&state, &session_id, &payload) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn session_interrupt(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    match interrupt_impl(&state, &session_id) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn session_send(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(text) = payload.get("text").and_then(Value::as_str) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "text required"}));
    };
    let images = match normalize_pi_image_inputs(payload.get("images").unwrap_or(&Value::Null)) {
        Ok(images) => images,
        Err(message) => return json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    };
    match send_impl(&state, &session_id, text, images) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn session_ui_response(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    match ui_response_impl(&state, &session_id, &payload) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

fn password_matches(password: &str) -> bool {
    let expected = std::env::var("CODEX_WEB_PASSWORD").unwrap_or_default();
    if expected.trim().is_empty() {
        return false;
    }
    let actual_hash = Sha256::digest(password.as_bytes());
    let expected_hash = Sha256::digest(expected.trim().as_bytes());
    actual_hash.as_slice() == expected_hash.as_slice()
}

fn json_response_with_cookie(status: StatusCode, value: Value, cookie: &str) -> Response {
    let mut response = json_response(status, value);
    if let Ok(cookie) = HeaderValue::from_str(cookie) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

fn cwd_group_edit_impl(state: &AppState, payload: &Value) -> Result<Value, (StatusCode, String)> {
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or((StatusCode::BAD_REQUEST, "empty cwd".to_string()))?;
    let normalized =
        normalize_cwd_group_key(cwd).ok_or((StatusCode::BAD_REQUEST, "empty cwd".to_string()))?;
    if let Some(value) = payload.get("label") {
        if !value.is_null() && !value.is_string() {
            return Err((
                StatusCode::BAD_REQUEST,
                "label must be a string".to_string(),
            ));
        }
    }
    if let Some(value) = payload.get("collapsed") {
        if !value.is_null() && !value.is_boolean() {
            return Err((
                StatusCode::BAD_REQUEST,
                "collapsed must be a boolean".to_string(),
            ));
        }
    }
    if let Some(value) = payload.get("hidden") {
        if !value.is_null() && !value.is_boolean() {
            return Err((
                StatusCode::BAD_REQUEST,
                "hidden must be a boolean".to_string(),
            ));
        }
    }
    let known = known_cwd_keys(state)?;
    let path = state.config.app_dir.join("cwd_groups.json");
    let existing_groups = read_object_file(&path)?;
    let existing = existing_groups.get(&normalized).cloned().unwrap_or_else(|| {
        json!({"label": "", "collapsed": false, "hidden": false, "hidden_after_live_start_ts": null})
    });
    let label = payload
        .get("label")
        .and_then(Value::as_str)
        .map(clean_alias_public)
        .unwrap_or_else(|| {
            existing
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        });
    let collapsed = payload
        .get("collapsed")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            existing
                .get("collapsed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
    let hidden = payload
        .get("hidden")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            existing
                .get("hidden")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        });
    let hidden_ts = if hidden {
        payload
            .get("hidden_after_live_start_ts")
            .and_then(clean_positive_number)
            .or_else(|| {
                existing
                    .get("hidden_after_live_start_ts")
                    .and_then(clean_positive_number)
            })
    } else {
        None
    };
    if !known.contains_key(&normalized) && label.is_empty() && !collapsed && !hidden {
        return Ok(json!({"ok": true, "cwd": normalized, "label": "", "collapsed": false}));
    }
    if !known.contains_key(&normalized) {
        return Err((
            StatusCode::BAD_REQUEST,
            "cwd is not a known session working directory".to_string(),
        ));
    }
    let entry = cwd_group_entry_public(&label, collapsed, hidden, hidden_ts);
    let mut groups = existing_groups;
    if label.is_empty() && !collapsed && !hidden {
        groups.remove(&normalized);
    } else {
        groups.insert(normalized.clone(), entry.clone());
    }
    with_state_file_lock(&path, || write_object_file(&path, &groups))?;
    let mut out = Map::new();
    out.insert("ok".to_string(), json!(true));
    out.insert("cwd".to_string(), json!(normalized));
    if let Value::Object(entry) = entry {
        out.extend(entry);
    }
    Ok(Value::Object(out))
}

fn rename_session_impl(
    state: &AppState,
    session_id: &str,
    name: &str,
) -> Result<Value, (StatusCode, String)> {
    let _ = session_or_404(state, session_id)?;
    let alias = clean_alias_public(name);
    let path = state.config.app_dir.join("session_aliases.json");
    with_state_file_lock(&path, || {
        let mut aliases = read_string_file(&path, clean_alias_public)?;
        if alias.is_empty() {
            aliases.remove(session_id);
        } else {
            aliases.insert(session_id.to_string(), alias.clone());
        }
        write_string_file(&path, &aliases)
    })?;
    Ok(json!({"ok": true, "alias": alias}))
}

fn edit_session_impl(
    state: &AppState,
    session_id: &str,
    name: &str,
    payload: &Value,
) -> Result<Value, (StatusCode, String)> {
    let _ = session_or_404(state, session_id)?;
    let alias = clean_alias_public(name);
    let priority_offset = clean_priority_offset(payload.get("priority_offset"))?;
    let snooze_until = clean_snooze_until(payload.get("snooze_until"))?;
    let dependency_session_id = clean_dependency_session_id(payload.get("dependency_session_id"))?;
    if dependency_session_id.as_deref() == Some(session_id) {
        return Err((
            StatusCode::BAD_REQUEST,
            "session cannot depend on itself".to_string(),
        ));
    }
    if let Some(dep) = dependency_session_id.as_deref() {
        let _ = session_or_404(state, dep).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "dependency session not found".to_string(),
            )
        })?;
    }
    let alias_path = state.config.app_dir.join("session_aliases.json");
    with_state_file_lock(&alias_path, || {
        let mut aliases = read_string_file(&alias_path, clean_alias_public)?;
        if alias.is_empty() {
            aliases.remove(session_id);
        } else {
            aliases.insert(session_id.to_string(), alias.clone());
        }
        write_string_file(&alias_path, &aliases)
    })?;

    let sidebar_path = state.config.app_dir.join("session_sidebar.json");
    with_state_file_lock(&sidebar_path, || {
        let mut sidebar = read_object_file(&sidebar_path)?;
        let mut entry = Map::new();
        entry.insert("priority_offset".to_string(), json!(priority_offset));
        if let Some(value) = snooze_until {
            entry.insert("snooze_until".to_string(), json!(value));
        }
        if let Some(value) = dependency_session_id.clone() {
            entry.insert("dependency_session_id".to_string(), json!(value));
        }
        sidebar.insert(session_id.to_string(), Value::Object(entry));
        write_object_file(&sidebar_path, &sidebar)
    })?;
    Ok(
        json!({"ok": true, "alias": alias, "priority_offset": priority_offset, "snooze_until": snooze_until, "dependency_session_id": dependency_session_id}),
    )
}

fn enqueue_impl(
    state: &AppState,
    session_id: &str,
    text: &str,
    images: Vec<Value>,
) -> Result<Value, (StatusCode, String)> {
    let _ = session_or_404(state, session_id)?;
    if text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "text required".to_string()));
    }
    let path = state.config.app_dir.join("session_queues.json");
    let len = with_state_file_lock(&path, || {
        let mut queues = read_array_file(&path, clean_queue_items)?;
        let q = queues.entry(session_id.to_string()).or_default();
        q.push(if images.is_empty() {
            json!(text)
        } else {
            json!({"text": text, "images": images})
        });
        let len = q.len();
        write_array_file(&path, &queues)?;
        Ok(len)
    })?;
    Ok(json!({"queued": true, "queue_len": len}))
}

fn queue_delete_impl(
    state: &AppState,
    session_id: &str,
    index: i64,
) -> Result<Value, (StatusCode, String)> {
    let _ = session_or_404(state, session_id)?;
    let path = state.config.app_dir.join("session_queues.json");
    let len = with_state_file_lock(&path, || {
        let mut queues = read_array_file(&path, clean_queue_items)?;
        let q = queues.entry(session_id.to_string()).or_default();
        if index < 0 || index as usize >= q.len() {
            return Err((StatusCode::BAD_GATEWAY, "index out of range".to_string()));
        }
        q.remove(index as usize);
        let len = q.len();
        if q.is_empty() {
            queues.remove(session_id);
        }
        write_array_file(&path, &queues)?;
        Ok(len)
    })?;
    Ok(json!({"ok": true, "queue_len": len}))
}

fn queue_update_impl(
    state: &AppState,
    session_id: &str,
    index: i64,
    text: &str,
) -> Result<Value, (StatusCode, String)> {
    let _ = session_or_404(state, session_id)?;
    if text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "text required".to_string()));
    }
    let path = state.config.app_dir.join("session_queues.json");
    let len = with_state_file_lock(&path, || {
        let mut queues = read_array_file(&path, clean_queue_items)?;
        let q = queues.entry(session_id.to_string()).or_default();
        if index < 0 || index as usize >= q.len() {
            return Err((StatusCode::BAD_GATEWAY, "index out of range".to_string()));
        }
        let images = q[index as usize]
            .get("images")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        q[index as usize] = if images.is_empty() {
            json!(text)
        } else {
            json!({"text": text, "images": images})
        };
        let len = q.len();
        write_array_file(&path, &queues)?;
        Ok(len)
    })?;
    Ok(json!({"ok": true, "queue_len": len}))
}

fn harness_set_impl(
    state: &AppState,
    session_id: &str,
    payload: &Value,
) -> Result<Value, (StatusCode, String)> {
    let _ = session_or_404(state, session_id)?;
    if payload.get("text").is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "unknown field: text (use request)".to_string(),
        ));
    }
    let path = state.config.app_dir.join("harness.json");
    let (enabled, request, cooldown, remaining) = with_state_file_lock(&path, || {
        let mut harness = read_object_file(&path)?;
        let mut current = harness
            .get(session_id)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(value) = payload.get("enabled") {
            current.insert(
                "enabled".to_string(),
                json!(value.as_bool().unwrap_or(false)),
            );
        }
        if let Some(value) = payload.get("request") {
            let Some(request) = value.as_str() else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "request must be a string".to_string(),
                ));
            };
            current.insert("request".to_string(), json!(request));
        }
        if let Some(value) = payload.get("cooldown_minutes") {
            current.insert(
                "cooldown_minutes".to_string(),
                json!(clean_harness_cooldown(value)?),
            );
        }
        if let Some(value) = payload.get("remaining_injections") {
            current.insert(
                "remaining_injections".to_string(),
                json!(clean_harness_remaining(value)?),
            );
        }
        let enabled = current
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let request = current
            .get("request")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let cooldown =
            clean_harness_cooldown(current.get("cooldown_minutes").unwrap_or(&Value::Null))?;
        let remaining =
            clean_harness_remaining(current.get("remaining_injections").unwrap_or(&Value::Null))?;
        let entry = json!({"enabled": enabled, "request": request, "cooldown_minutes": cooldown, "remaining_injections": remaining});
        harness.insert(session_id.to_string(), entry);
        write_object_file(&path, &harness)?;
        Ok((enabled, request, cooldown, remaining))
    })?;
    Ok(
        json!({"ok": true, "enabled": enabled, "request": request, "cooldown_minutes": cooldown, "remaining_injections": remaining}),
    )
}

fn interrupt_impl(state: &AppState, session_id: &str) -> Result<Value, (StatusCode, String)> {
    let row = session_or_404(state, session_id)?;
    let sock = row_sock_path(&state.config.app_dir, &row);
    let broker =
        broker_keys(&sock, "\\x1b", Duration::from_secs_f64(2.0)).map_err(map_broker_error)?;
    if let Some(err) = broker
        .get("error")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Err((StatusCode::BAD_GATEWAY, err.to_string()));
    }
    Ok(json!({"ok": true, "broker": broker}))
}

fn send_impl(
    state: &AppState,
    session_id: &str,
    text: &str,
    images: Vec<Value>,
) -> Result<Value, (StatusCode, String)> {
    let row = session_or_404(state, session_id)?;
    if text.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "text required".to_string()));
    }
    if supports_idle_auto_stop(&row) {
        heartbeat_impl(state, session_id)?;
    }
    let sock = row_sock_path(&state.config.app_dir, &row);
    let response = match broker_send(
        &sock,
        text,
        if row.backend == "pi" {
            Some(images.clone())
        } else {
            None
        },
        Duration::from_secs_f64(3.0),
    ) {
        Ok(response) => response,
        Err(_error) if broker_process_alive(&row) => {
            return enqueue_impl(state, session_id, text, images);
        }
        Err(error) => return Err(map_broker_error(error)),
    };
    if let Some(err) = response
        .get("error")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Err((StatusCode::BAD_GATEWAY, err.to_string()));
    }
    if response.get("queue_len").and_then(Value::as_u64).is_none() {
        return Err((
            StatusCode::BAD_GATEWAY,
            "invalid broker send response".to_string(),
        ));
    }
    Ok(response)
}

fn ui_response_impl(
    state: &AppState,
    session_id: &str,
    payload: &Value,
) -> Result<Value, (StatusCode, String)> {
    let row = session_or_404(state, session_id)?;
    if row.backend != "pi" {
        return Err((
            StatusCode::BAD_GATEWAY,
            "ui interactions are only supported for pi sessions".to_string(),
        ));
    }
    let sock = row_sock_path(&state.config.app_dir, &row);
    let response = broker_ui_response(&sock, payload, Duration::from_secs_f64(3.0))
        .map_err(map_broker_error)?;
    if response.get("error").and_then(Value::as_str) == Some("unknown cmd") {
        if payload.get("cancelled").and_then(Value::as_bool) == Some(true) {
            let _ = broker_keys(&sock, "\\x1b", Duration::from_secs_f64(2.0))
                .map_err(map_broker_error)?;
            return Ok(json!({"ok": true, "legacy_fallback": true}));
        }
        let Some(text) = legacy_ui_response_text(payload) else {
            return Err((
                StatusCode::BAD_GATEWAY,
                "ui response value required".to_string(),
            ));
        };
        let _ = broker_send(&sock, &text, None, Duration::from_secs_f64(3.0))
            .map_err(map_broker_error)?;
        return Ok(json!({"ok": true, "legacy_fallback": true}));
    }
    if let Some(err) = response
        .get("error")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Err((StatusCode::BAD_GATEWAY, err.to_string()));
    }
    Ok(json!({"ok": true}))
}

fn session_or_404(state: &AppState, session_id: &str) -> Result<SessionRow, (StatusCode, String)> {
    find_session(&state.config, session_id).map_err(|message| {
        if message.contains("unknown session") {
            (StatusCode::NOT_FOUND, "unknown session".to_string())
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, message)
        }
    })
}

fn row_sock_path(app_dir: &FsPath, row: &SessionRow) -> PathBuf {
    app_dir
        .join("socks")
        .join(format!("{}.sock", row.session_id))
}

fn known_cwd_keys(state: &AppState) -> Result<HashMap<String, ()>, (StatusCode, String)> {
    let mut out = HashMap::new();
    for row in load_session_rows(&state.config)
        .map_err(|message| (StatusCode::INTERNAL_SERVER_ERROR, message))?
    {
        if let Some(key) = normalize_cwd_group_key(&row.cwd) {
            out.insert(key, ());
        }
    }
    for cwd in read_object_file(&state.config.app_dir.join("cwd_groups.json"))?.keys() {
        if let Some(key) = normalize_cwd_group_key(cwd) {
            out.insert(key, ());
        }
    }
    Ok(out)
}

fn broker_process_alive(row: &SessionRow) -> bool {
    pid_alive(row.broker_pid) || pid_alive(row.codex_pid)
}

fn supports_idle_auto_stop(row: &SessionRow) -> bool {
    row.auto_stop_on_idle
        && row.owned
        && row.backend == "pi"
        && row.transport.as_deref().map(str::trim) == Some("pi-rpc")
        && row.idle_timeout_seconds.unwrap_or(0) > 0
}

#[cfg(unix)]
fn pid_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn pid_alive(pid: i64) -> bool {
    pid > 0
}

fn map_broker_error(error: BrokerError) -> (StatusCode, String) {
    (
        StatusCode::BAD_GATEWAY,
        match error {
            BrokerError::ConnectRefused | BrokerError::Timeout | BrokerError::Empty => {
                "broker unavailable".to_string()
            }
            other => other.to_string(),
        },
    )
}
