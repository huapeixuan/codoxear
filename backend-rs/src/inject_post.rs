use crate::app_state::AppState;
use crate::broker_client::{broker_keys, BrokerError};
use crate::models::SessionRow;
use crate::routes::json_response;
use crate::session_loader::find_session;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use base64::Engine;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ATTACH_UPLOAD_MAX_BYTES: usize = 10 * 1024 * 1024;

pub(crate) async fn session_inject_file(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    match inject_impl(&state, &session_id, &payload) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, value)) => json_response(status, value),
    }
}

fn inject_impl(
    state: &AppState,
    session_id: &str,
    payload: &Value,
) -> Result<Value, (StatusCode, Value)> {
    let row = session_or_404(state, session_id)?;
    if row.backend == "pi" {
        return Err((
            StatusCode::CONFLICT,
            json!({
                "error": "attachment injection is not supported for Pi sessions",
                "backend": "pi",
                "operation": "attachment_injection",
            }),
        ));
    }
    let Some(filename) = payload
        .get("filename")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": "filename required"}),
        ));
    };
    let Some(index) = payload.get("attachment_index").and_then(Value::as_i64) else {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": "attachment_index must be an integer"}),
        ));
    };
    if index <= 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": "attachment_index must be >= 1"}),
        ));
    }
    let Some(data_b64) = payload
        .get("data_b64")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            json!({"error": "data_b64 required"}),
        ));
    };
    let raw = base64::engine::general_purpose::STANDARD
        .decode(data_b64.as_bytes())
        .map_err(|_| (StatusCode::BAD_REQUEST, json!({"error": "invalid base64"})))?;
    let out_path = stage_uploaded_file(&state.config.app_dir, session_id, filename, &raw)
        .map_err(|(status, message)| (status, json!({"error": message})))?;
    let inject_text = format!("Attachment {index}: {}\n", out_path.display());
    let seq = format!("\x1b[200~{inject_text}\x1b[201~");
    let sock = state
        .config
        .app_dir
        .join("socks")
        .join(format!("{}.sock", row.session_id));
    let broker = broker_keys(&sock, &seq, Duration::from_secs_f64(2.0)).map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            json!({"error": map_broker_error(error)}),
        )
    })?;
    if let Some(err) = broker
        .get("error")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        return Err((StatusCode::BAD_GATEWAY, json!({"error": err})));
    }
    Ok(
        json!({"ok": true, "path": out_path.to_string_lossy(), "inject_text": inject_text, "broker": broker}),
    )
}

fn session_or_404(state: &AppState, session_id: &str) -> Result<SessionRow, (StatusCode, Value)> {
    find_session(&state.config, session_id).map_err(|message| {
        if message.contains("unknown session") {
            (StatusCode::NOT_FOUND, json!({"error": "unknown session"}))
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": message}))
        }
    })
}

fn stage_uploaded_file(
    app_dir: &FsPath,
    session_id: &str,
    filename: &str,
    raw: &[u8],
) -> Result<PathBuf, (StatusCode, String)> {
    if raw.len() > ATTACH_UPLOAD_MAX_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("file too large (max {ATTACH_UPLOAD_MAX_BYTES} bytes)"),
        ));
    }
    let safe_name = safe_filename(filename);
    let subdir = app_dir.join("uploads").join(session_id);
    fs::create_dir_all(&subdir)
        .map_err(|err| (StatusCode::BAD_REQUEST, format!("create upload dir: {err}")))?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let out_path = subdir.join(format!("{now_ms}_{safe_name}"));
    let mut file = fs::File::create(&out_path)
        .map_err(|err| (StatusCode::BAD_REQUEST, format!("write upload: {err}")))?;
    file.write_all(raw)
        .map_err(|err| (StatusCode::BAD_REQUEST, format!("write upload: {err}")))?;
    file.sync_all()
        .map_err(|err| (StatusCode::BAD_REQUEST, format!("sync upload: {err}")))?;
    drop(file);
    set_private_mode(&out_path)?;
    Ok(out_path)
}

fn safe_filename(name: &str) -> String {
    let base = FsPath::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let cleaned = base
        .chars()
        .filter(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' '))
        .collect::<String>()
        .trim()
        .replace(' ', "_");
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned.chars().take(96).collect()
    }
}

#[cfg(unix)]
fn set_private_mode(path: &FsPath) -> Result<(), (StatusCode, String)> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|err| (StatusCode::BAD_REQUEST, format!("chmod upload: {err}")))
}

#[cfg(not(unix))]
fn set_private_mode(_path: &FsPath) -> Result<(), (StatusCode, String)> {
    Ok(())
}

fn map_broker_error(error: BrokerError) -> String {
    match error {
        BrokerError::ConnectRefused | BrokerError::Timeout | BrokerError::Empty => {
            "broker unavailable".to_string()
        }
        other => other.to_string(),
    }
}
