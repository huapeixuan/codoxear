use crate::app_state::AppState;
use crate::models::SessionRow;
use crate::routes::{internal_error, json_response};
use crate::session_loader::find_session;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};
use std::process::{Command, Stdio};

pub(crate) async fn takeover_open(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let descriptor = takeover_descriptor(&row);
    if descriptor.get("eligible").and_then(Value::as_bool) != Some(true) {
        return json_response(StatusCode::OK, descriptor);
    }
    json_response(StatusCode::OK, open_takeover_terminal(descriptor))
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
    let Some(tmux_session) = row
        .tmux_session
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return json!({"ok": true, "eligible": false, "reason": "tmux metadata is unavailable"});
    };
    let Some(tmux_window) = row.tmux_window.as_deref().filter(|value| !value.is_empty()) else {
        return json!({"ok": true, "eligible": false, "reason": "tmux metadata is unavailable"});
    };
    let attach_command = build_tmux_attach_command(tmux_session, tmux_window);
    json!({
        "ok": true,
        "eligible": true,
        "backend": row.backend,
        "transport": row.transport,
        "tmux_session": tmux_session,
        "tmux_window": tmux_window,
        "attach_command": attach_command,
    })
}

fn open_takeover_terminal(descriptor: Value) -> Value {
    let attach_command = descriptor
        .get("attach_command")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if attach_command.is_empty() {
        return json!({"ok": true, "opened": false, "reason": "attach command is unavailable"});
    }
    let Some(argv) = detect_terminal_open_argv(&attach_command) else {
        return json!({
            "ok": true,
            "opened": false,
            "reason": "no supported GUI terminal launcher found",
            "attach_command": attach_command,
        });
    };
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    let _ = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    json!({"ok": true, "opened": true, "attach_command": attach_command})
}

fn detect_terminal_open_argv(attach_command: &str) -> Option<Vec<String>> {
    #[cfg(target_os = "macos")]
    {
        Some(vec![
            "osascript".to_string(),
            "-e".to_string(),
            format!(
                "tell application \"Terminal\"\n  activate\n  do script {}\nend tell\n",
                serde_json::to_string(attach_command).unwrap_or_else(|_| "\"tmux\"".to_string())
            ),
        ])
    }
    #[cfg(not(target_os = "macos"))]
    {
        let candidates = [
            vec!["x-terminal-emulator", "-e", "sh", "-lc", attach_command],
            vec!["gnome-terminal", "--", "sh", "-lc", attach_command],
            vec!["kitty", "sh", "-lc", attach_command],
            vec!["wezterm", "start", "--", "sh", "-lc", attach_command],
            vec!["ghostty", "-e", "sh", "-lc", attach_command],
            vec!["konsole", "-e", "sh", "-lc", attach_command],
        ];
        candidates
            .into_iter()
            .find(|argv| command_exists(argv[0]))
            .map(|argv| argv.into_iter().map(str::to_string).collect())
    }
}

fn build_tmux_attach_command(session_name: &str, window_name: &str) -> String {
    format!(
        "tmux attach -t {} \\; select-window -t {}",
        shell_quote(session_name),
        shell_quote(&format!("{session_name}:{window_name}"))
    )
}

fn shell_quote(raw: &str) -> String {
    if raw.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'=')
    }) {
        return raw.to_string();
    }
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

#[cfg(not(target_os = "macos"))]
fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|path| path.join(name))
                .find(|candidate| candidate.is_file())
        })
        .is_some()
}
