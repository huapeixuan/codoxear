use crate::app_state::AppState;
use crate::broker_client::{broker_commands, broker_ui_state};
use crate::git_context::resolve_repo_context;
use crate::models::SessionRow;
use crate::routes::{internal_error, json_response};
use crate::session_loader::find_session;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

pub async fn diagnostics(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) => json_response(StatusCode::OK, diagnostics_payload(&row)),
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

pub async fn queue(State(state): State<AppState>, Path(session_id): Path<String>) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) => json_response(
            StatusCode::OK,
            json!({"ok": true, "queue": row.queue_items}),
        ),
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

pub async fn harness_get(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) => json_response(
            StatusCode::OK,
            json!({
                "enabled": row.harness_enabled,
                "ok": true,
                "cooldown_minutes": row.harness_cooldown_minutes,
                "remaining_injections": row.harness_remaining_injections,
                "request": row.harness_request,
            }),
        ),
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

pub async fn workspace(State(state): State<AppState>, Path(session_id): Path<String>) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) => json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "session_id": row.session_id,
                "diagnostics": diagnostics_payload(&row),
                "queue": {"items": row.queue_items},
            }),
        ),
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

pub async fn details(State(state): State<AppState>, Path(session_id): Path<String>) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) => json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "session": session_detail_payload(&row),
            }),
        ),
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

pub async fn ui_state(State(state): State<AppState>, Path(session_id): Path<String>) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) if row.backend == "pi" => {
            let Some(sock_path) = row.sock_path(&state.config.app_dir) else {
                return json_response(
                    StatusCode::BAD_GATEWAY,
                    json!({"error": "broker unavailable"}),
                );
            };
            match broker_ui_state(&sock_path, Duration::from_millis(1500)) {
                Ok(payload) => json_response(StatusCode::OK, payload.raw),
                Err(_) => json_response(
                    StatusCode::BAD_GATEWAY,
                    json!({"error": "broker unavailable"}),
                ),
            }
        }
        Ok(_) => json_response(
            StatusCode::BAD_GATEWAY,
            json!({"error": "ui interactions are only supported for pi sessions"}),
        ),
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

pub async fn commands(State(state): State<AppState>, Path(session_id): Path<String>) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) if row.backend == "pi" => {
            let Some(sock_path) = row.sock_path(&state.config.app_dir) else {
                return json_response(
                    StatusCode::BAD_GATEWAY,
                    json!({"error": "broker unavailable"}),
                );
            };
            match broker_commands(&sock_path, Duration::from_secs(2)) {
                Ok(payload) => json_response(StatusCode::OK, payload.raw),
                Err(_) => json_response(
                    StatusCode::BAD_GATEWAY,
                    json!({"error": "broker unavailable"}),
                ),
            }
        }
        Ok(_) => json_response(
            StatusCode::BAD_GATEWAY,
            json!({"error": "command listing is only supported for pi sessions"}),
        ),
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

pub async fn takeover(State(state): State<AppState>, Path(session_id): Path<String>) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) => json_response(StatusCode::OK, takeover_descriptor(&row)),
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

#[derive(Deserialize)]
pub struct RepoQuery {
    refresh: Option<String>,
}

pub async fn repo(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<RepoQuery>,
) -> Response {
    match find_session(&state.config, &session_id) {
        Ok(row) => {
            let refresh = query.refresh.as_deref() == Some("1");
            let context = resolve_repo_context(std::path::Path::new(&row.cwd), refresh);
            json_response(StatusCode::OK, context.to_detail_dict())
        }
        Err(message) if message.contains("unknown session") => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        }
        Err(message) => internal_error(message),
    }
}

fn diagnostics_payload(row: &SessionRow) -> Value {
    json!({
        "session_id": row.session_id,
        "thread_id": row.thread_id,
        "agent_backend": row.agent_backend,
        "backend": row.backend,
        "owned": row.owned,
        "transport": row.transport,
        "cwd": row.cwd,
        "start_ts": row.start_ts,
        "updated_ts": row.updated_ts,
        "log_path": row.log_path,
        "session_file_path": row.log_path,
        "broker_pid": row.broker_pid,
        "codex_pid": row.codex_pid,
        "busy": row.busy,
        "broker_busy": row.broker_busy,
        "queue_len": row.queue_len,
        "token": if row.token.is_null() { Value::Null } else { row.token.clone() },
        "model_provider": row.model_provider,
        "preferred_auth_method": row.preferred_auth_method,
        "provider_choice": row.provider_choice,
        "model": row.model,
        "reasoning_effort": row.reasoning_effort,
        "service_tier": row.service_tier,
        "tmux_session": row.tmux_session,
        "tmux_window": row.tmux_window,
        "git_branch": row.git_branch,
        "pr_summary": row.pr_summary,
        "time_priority": row.time_priority,
        "base_priority": row.base_priority,
        "final_priority": row.final_priority,
        "priority_offset": row.priority_offset,
        "snooze_until": row.snooze_until,
        "dependency_session_id": row.dependency_session_id,
        "todo_snapshot": row.todo_snapshot,
    })
}

fn session_detail_payload(row: &SessionRow) -> Value {
    let mut payload = crate::session_loader::frontend_session_list_row(row);
    if let Value::Object(ref mut object) = payload {
        object.insert("can_takeover_in_tmux".to_string(), Value::Bool(false));
        object.insert(
            "takeover_reason_unavailable".to_string(),
            Value::String("takeover is only available for web-owned Pi sessions".to_string()),
        );
    }
    payload
}

fn takeover_descriptor(row: &SessionRow) -> Value {
    if row.backend != "pi" || !row.owned {
        return json!({
            "ok": true,
            "eligible": false,
            "reason": "takeover is only available for web-owned Pi sessions",
        });
    }
    if !row
        .transport
        .as_deref()
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case("pi-rpc")
    {
        return json!({
            "ok": true,
            "eligible": false,
            "reason": "takeover requires a live Pi pi-rpc session",
        });
    }
    if row.tmux_session.is_none() || row.tmux_window.is_none() {
        return json!({
            "ok": true,
            "eligible": false,
            "reason": "tmux target is unavailable for this session",
        });
    }
    json!({
        "ok": true,
        "eligible": true,
        "session_id": row.session_id,
        "tmux_session": row.tmux_session,
        "tmux_window": row.tmux_window,
    })
}

trait SessionRowExt {
    fn sock_path(&self, app_dir: &std::path::Path) -> Option<std::path::PathBuf>;
}

impl SessionRowExt for SessionRow {
    fn sock_path(&self, app_dir: &std::path::Path) -> Option<std::path::PathBuf> {
        Some(
            app_dir
                .join("socks")
                .join(format!("{}.sock", self.session_id)),
        )
    }
}
