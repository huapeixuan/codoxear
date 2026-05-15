use crate::app_state::AppState;
use crate::broker_client::broker_shutdown;
use crate::models::SessionRow;
use crate::routes::json_response;
use crate::session_loader::find_session;
use crate::state_files::{
    read_object_file, read_value, with_state_file_lock, write_object_file, write_value,
};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) async fn session_heartbeat(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    match heartbeat_impl(&state, &session_id) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) async fn session_delete(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    match delete_session_impl(&state, &session_id) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub(crate) fn heartbeat_impl(
    state: &AppState,
    session_id: &str,
) -> Result<Value, (StatusCode, String)> {
    let row = session_or_404(state, session_id)?;
    if !row.auto_stop_on_idle
        || !row.owned
        || row.backend != "pi"
        || row.transport.as_deref().map(str::trim) != Some("pi-rpc")
        || row.idle_timeout_seconds.unwrap_or(0) <= 0
    {
        return Err((
            StatusCode::CONFLICT,
            "idle auto-stop heartbeat is only supported for web-owned pi-rpc sessions".to_string(),
        ));
    }
    let ts = now_f64();
    let path = state
        .config
        .app_dir
        .join("socks")
        .join(format!("{session_id}.json"));
    with_state_file_lock(&path, || {
        let mut meta = read_object_file(&path)?;
        meta.insert("last_web_activity_ts".to_string(), json!(ts));
        meta.insert(
            "idle_timeout_seconds".to_string(),
            json!(row.idle_timeout_seconds.unwrap_or(0)),
        );
        meta.insert("auto_stop_on_idle".to_string(), json!(true));
        write_sidecar_object_file(&path, &meta)
    })?;
    Ok(
        json!({"ok": true, "session_id": row.session_id, "idle_timeout_seconds": row.idle_timeout_seconds.unwrap_or(0), "last_web_activity_ts": ts}),
    )
}

fn delete_session_impl(state: &AppState, session_id: &str) -> Result<Value, (StatusCode, String)> {
    if parse_historical_session_id(session_id).is_some() {
        hide_session_id(&state.config.app_dir, session_id)?;
        clear_session_state(&state.config.app_dir, session_id)?;
        return Ok(json!({"ok": true}));
    }
    let row = session_or_404(state, session_id)?;
    let sock = row_sock_path(&state.config.app_dir, &row);
    let shutdown_ok = broker_shutdown(&sock, Duration::from_secs_f64(1.0))
        .ok()
        .and_then(|value| value.get("ok").and_then(Value::as_bool))
        .unwrap_or(false);
    if !shutdown_ok {
        terminate_pid(row.broker_pid);
        terminate_pid(row.codex_pid);
    }
    let _ = fs::remove_file(&sock);
    let _ = fs::remove_file(sock.with_extension("json"));
    hide_session_id(&state.config.app_dir, session_id)?;
    clear_session_state(&state.config.app_dir, session_id)?;
    Ok(json!({"ok": true}))
}

fn hide_session_id(app_dir: &FsPath, session_id: &str) -> Result<(), (StatusCode, String)> {
    let hidden_path = app_dir.join("hidden_sessions.json");
    with_state_file_lock(&hidden_path, || {
        let mut hidden = match read_value(&hidden_path)? {
            Some(Value::Array(items)) => items,
            _ => Vec::new(),
        };
        if !hidden.iter().any(|item| item.as_str() == Some(session_id)) {
            hidden.push(json!(session_id));
        }
        hidden.sort_by_key(|a| a.to_string());
        write_value(&hidden_path, &Value::Array(hidden))
    })
}

fn parse_historical_session_id(session_id: &str) -> Option<(&str, &str)> {
    let rest = session_id.trim().strip_prefix("history:")?;
    let (backend, resume_id) = rest.split_once(':')?;
    if !matches!(backend, "codex" | "pi") || resume_id.trim().is_empty() {
        return None;
    }
    Some((backend, resume_id))
}

fn clear_session_state(app_dir: &FsPath, session_id: &str) -> Result<(), (StatusCode, String)> {
    for file in [
        "session_aliases.json",
        "session_files.json",
        "session_queues.json",
        "harness.json",
    ] {
        let path = app_dir.join(file);
        with_state_file_lock(&path, || {
            let mut object = read_object_file(&path)?;
            object.remove(session_id);
            write_object_file(&path, &object)
        })?;
    }
    let sidebar_path = app_dir.join("session_sidebar.json");
    with_state_file_lock(&sidebar_path, || {
        let mut sidebar = read_object_file(&sidebar_path)?;
        sidebar.remove(session_id);
        for entry in sidebar.values_mut() {
            let Some(object) = entry.as_object_mut() else {
                continue;
            };
            if object.get("dependency_session_id").and_then(Value::as_str) == Some(session_id) {
                object.remove("dependency_session_id");
            }
        }
        write_object_file(&sidebar_path, &sidebar)
    })?;
    Ok(())
}

fn write_sidecar_object_file(
    path: &FsPath,
    object: &serde_json::Map<String, Value>,
) -> Result<(), (StatusCode, String)> {
    let parent = path.parent().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("missing parent for {}", path.display()),
    ))?;
    fs::create_dir_all(parent).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create {}: {err}", parent.display()),
        )
    })?;
    let tmp = path.with_extension("json.tmp");
    let raw = serde_json::to_string_pretty(&Value::Object(object.clone()))
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        + "\n";
    let mut file = fs::File::create(&tmp).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write {}: {err}", tmp.display()),
        )
    })?;
    file.write_all(raw.as_bytes()).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write {}: {err}", tmp.display()),
        )
    })?;
    file.sync_all().map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("sync {}: {err}", tmp.display()),
        )
    })?;
    drop(file);
    set_private_mode(&tmp)?;
    fs::rename(&tmp, path).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("rename {}: {err}", path.display()),
        )
    })?;
    set_private_mode(path)?;
    if let Ok(parent_file) = fs::File::open(parent) {
        let _ = parent_file.sync_all();
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_mode(path: &FsPath) -> Result<(), (StatusCode, String)> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("chmod {}: {err}", path.display()),
        )
    })
}

#[cfg(not(unix))]
fn set_private_mode(_path: &FsPath) -> Result<(), (StatusCode, String)> {
    Ok(())
}

#[cfg(unix)]
fn terminate_pid(pid: i64) {
    if pid <= 0 || pid as u32 == std::process::id() {
        return;
    }
    unsafe {
        let _ = libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn terminate_pid(_pid: i64) {}

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

fn now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}
