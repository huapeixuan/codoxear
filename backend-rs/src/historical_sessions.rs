use crate::app_state::AppState;
use crate::post_handlers::enqueue_impl;
use crate::session_create::{spawn_web_session, CreateSessionRequest};
use crate::session_create_support::pi_home;
use axum::http::StatusCode;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

pub(crate) fn parse_historical_session_id(session_id: &str) -> Option<(&str, &str)> {
    let rest = session_id.trim().strip_prefix("history:")?;
    let (backend, resume_id) = rest.split_once(':')?;
    if !matches!(backend, "codex" | "pi") || resume_id.trim().is_empty() {
        return None;
    }
    Some((backend, resume_id))
}

pub(crate) fn send_historical_pi(
    state: &AppState,
    resume_id: &str,
    text: &str,
    images: Vec<Value>,
) -> Result<Value, (StatusCode, String)> {
    let cwd = historical_pi_cwd(resume_id).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "historical session is missing resume metadata".to_string(),
        )
    })?;
    let request = CreateSessionRequest {
        cwd,
        backend: "pi".to_string(),
        args: None,
        resume_session_id: Some(resume_id.to_string()),
        worktree_branch: None,
        model_provider: None,
        preferred_auth_method: None,
        model: None,
        reasoning_effort: None,
        service_tier: None,
        create_in_tmux: false,
    };
    let spawned = spawn_web_session(state, &request)?;
    let live_id = spawned
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "spawned session did not return a session id".to_string(),
            )
        })?;
    let mut out = enqueue_impl(state, live_id, text, images).or_else(|err| {
        if std::env::var("CODOXEAR_FAKE_SPAWN_FOR_TESTS")
            .ok()
            .is_some_and(|value| crate::workers::env_flag_truthy_value(Some(&value)))
        {
            enqueue_historical_fake(state, live_id, text)
        } else {
            Err(err)
        }
    })?;
    out["session_id"] = json!(live_id);
    out["backend"] = json!("pi");
    Ok(out)
}

fn enqueue_historical_fake(
    state: &AppState,
    live_id: &str,
    text: &str,
) -> Result<Value, (StatusCode, String)> {
    let path = state.config.app_dir.join("session_queues.json");
    let len = crate::state_files::with_state_file_lock(&path, || {
        let mut queues =
            crate::state_files::read_array_file(&path, crate::write_cleaners::clean_queue_items)?;
        let q = queues.entry(live_id.to_string()).or_default();
        q.push(json!(text));
        let len = q.len();
        crate::state_files::write_array_file(&path, &queues)?;
        Ok(len)
    })?;
    Ok(json!({"queued": true, "queue_len": len}))
}

fn historical_pi_cwd(resume_id: &str) -> Option<String> {
    let mut stack = vec![pi_home().join("agent/sessions")];
    while let Some(path) = stack.pop() {
        let Ok(entries) = fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                if let Some(cwd) = pi_session_cwd_from_file(&path, resume_id) {
                    return Some(cwd);
                }
            }
        }
    }
    None
}

fn pi_session_cwd_from_file(path: &PathBuf, resume_id: &str) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    raw.lines()
        .take(20)
        .find_map(|line| pi_session_cwd_from_line(line, resume_id))
}

fn pi_session_cwd_from_line(line: &str, resume_id: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(line).ok()?;
    let id = value
        .get("id")
        .or_else(|| value.get("sessionId"))
        .or_else(|| value.get("session_id"))
        .and_then(Value::as_str)?;
    if id != resume_id {
        return None;
    }
    value
        .get("cwd")
        .or_else(|| value.pointer("/session/cwd"))
        .and_then(Value::as_str)
        .map(str::to_string)
}
