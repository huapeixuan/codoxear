use crate::app_state::AppState;
use crate::handlers::files::resolve_session_path;
use crate::routes::json_response;
use crate::session_loader::find_session;
use crate::state_files::{read_array_file, with_state_file_lock, write_array_file};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Component, Path as FsPath, PathBuf};

const FILE_READ_MAX_BYTES: u64 = 2 * 1024 * 1024;
const FILE_HISTORY_MAX: usize = 20;
const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd"];
const TEXTUAL_EXTENSIONS: &[&str] = &[
    "bash", "c", "cc", "cfg", "conf", "cpp", "css", "csv", "diff", "go", "h", "hpp", "htm", "html",
    "ini", "java", "js", "json", "jsonl", "log", "md", "markdown", "mdown", "mkd", "patch", "py",
    "rs", "scss", "sh", "sql", "svg", "toml", "ts", "tsx", "txt", "xml", "yaml", "yml", "zsh",
];
const TEXTUAL_FILENAMES: &[&str] = &["dockerfile", "license", "makefile", "readme"];

struct ClientFileView {
    kind: String,
    size: u64,
    content_type: Option<String>,
    text: Option<String>,
    editable: bool,
    version: Option<String>,
    blocked_reason: Option<String>,
    viewer_max_bytes: Option<u64>,
}

pub(crate) async fn files_read_post(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(raw_path) = payload.get("path").and_then(Value::as_str) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    };
    if raw_path.trim().is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    }
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    let path = match resolve_client_file_path(&state, session_id, raw_path) {
        Ok(path) => path,
        Err((status, message)) => return json_response(status, json!({"error": message})),
    };
    let view = match read_client_file_view(&path) {
        Ok(view) => view,
        Err((status, message)) => return json_response(status, json!({"error": message})),
    };
    if !session_id.is_empty() {
        let _ = files_add(&state, session_id, &path.to_string_lossy());
    }
    json_response(StatusCode::OK, file_view_payload(&path, view, None))
}

pub(crate) async fn files_inspect_post(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(raw_path) = payload.get("path").and_then(Value::as_str) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    };
    if raw_path.trim().is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    }
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    let path = match resolve_client_file_path(&state, session_id, raw_path) {
        Ok(path) => path,
        Err((status, message)) => return json_response(status, json!({"error": message})),
    };
    let view = match read_client_file_view(&path) {
        Ok(view) => view,
        Err((status, message)) => return json_response(status, json!({"error": message})),
    };
    json_response(
        StatusCode::OK,
        json!({"ok": true, "path": path.to_string_lossy(), "kind": view.kind, "content_type": view.content_type, "size": view.size, "reason": view.blocked_reason, "viewer_max_bytes": view.viewer_max_bytes}),
    )
}

pub(crate) async fn session_file_write(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    match session_file_write_impl(&state, &session_id, &payload) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message, extra)) => {
            let mut object = serde_json::Map::new();
            object.insert("error".to_string(), json!(message));
            if let Some(Value::Object(extra)) = extra {
                object.extend(extra);
            }
            json_response(status, Value::Object(object))
        }
    }
}

fn session_file_write_impl(
    state: &AppState,
    session_id: &str,
    payload: &Value,
) -> Result<Value, (StatusCode, String, Option<Value>)> {
    let Some(raw_path) = payload.get("path").and_then(Value::as_str) else {
        return Err((StatusCode::BAD_REQUEST, "path required".to_string(), None));
    };
    let Some(text) = payload.get("text").and_then(Value::as_str) else {
        return Err((
            StatusCode::BAD_REQUEST,
            "text must be a string".to_string(),
            None,
        ));
    };
    let create = payload
        .get("create")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let version = payload.get("version").and_then(Value::as_str).unwrap_or("");
    if !create && version.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "version required".to_string(),
            None,
        ));
    }
    let row = find_session(&state.config, session_id).map_err(|message| {
        if message.contains("unknown session") {
            (StatusCode::NOT_FOUND, "unknown session".to_string(), None)
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, message, None)
        }
    })?;
    let base = canonical_cwd(FsPath::new(&row.cwd));
    let path = if create {
        resolve_under(&base, raw_path)
            .map_err(|message| (StatusCode::BAD_REQUEST, message, None))?
    } else {
        resolve_session_path(&base, raw_path)
            .map_err(|message| (StatusCode::BAD_REQUEST, message, None))?
    };
    let (size, next_version) = if create {
        match write_new_text_file_atomic(&path, text) {
            Ok(result) => result,
            Err((StatusCode::CONFLICT, message)) => {
                let mut extra = serde_json::Map::new();
                extra.insert("conflict".to_string(), json!(true));
                extra.insert("path".to_string(), json!(path.to_string_lossy()));
                if path.is_file() {
                    if let Ok((_, _, current_version)) = read_text_file_for_write(&path) {
                        extra.insert("version".to_string(), json!(current_version));
                    }
                }
                return Err((StatusCode::CONFLICT, message, Some(Value::Object(extra))));
            }
            Err((status, message)) => return Err((status, message, None)),
        }
    } else {
        let (_, _, current_version) =
            read_text_file_for_write(&path).map_err(|(status, message)| (status, message, None))?;
        if current_version != version {
            let mut extra = serde_json::Map::new();
            extra.insert("conflict".to_string(), json!(true));
            extra.insert("path".to_string(), json!(path.to_string_lossy()));
            extra.insert("version".to_string(), json!(current_version));
            return Err((
                StatusCode::CONFLICT,
                "file changed on disk".to_string(),
                Some(Value::Object(extra)),
            ));
        }
        write_text_file_atomic(&path, text).map_err(|(status, message)| (status, message, None))?
    };
    let _ = files_add(state, session_id, &path.to_string_lossy());
    Ok(
        json!({"ok": true, "path": path.to_string_lossy(), "rel": raw_path, "size": size, "version": next_version, "editable": true}),
    )
}

fn resolve_client_file_path(
    state: &AppState,
    session_id: &str,
    raw_path: &str,
) -> Result<PathBuf, (StatusCode, String)> {
    if raw_path.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "empty path".to_string()));
    }
    if raw_path.contains('\0') || raw_path.contains('\n') || raw_path.len() > 1024 {
        return Err((StatusCode::BAD_REQUEST, "invalid path format".to_string()));
    }
    let raw = expand_user(raw_path);
    if raw.is_absolute() {
        return Ok(canonical_cwd(&raw));
    }
    if !session_id.is_empty() {
        let row = find_session(&state.config, session_id).map_err(|message| {
            if message.contains("unknown session") {
                (StatusCode::NOT_FOUND, "unknown session".to_string())
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, message)
            }
        })?;
        return Ok(canonical_cwd(FsPath::new(&row.cwd)).join(raw));
    }
    Ok(std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(raw))
}

fn file_view_payload(path: &FsPath, view: ClientFileView, rel: Option<&str>) -> Value {
    let path_s = path.to_string_lossy().to_string();
    match view.kind.as_str() {
        "image" => {
            json!({"ok": true, "kind": "image", "content_type": view.content_type, "path": path_s, "rel": rel, "size": view.size, "image_url": format!("/api/files/blob?path={}", percent_encode(&path_s))})
        }
        "pdf" => {
            json!({"ok": true, "kind": "pdf", "content_type": view.content_type, "path": path_s, "rel": rel, "size": view.size, "pdf_url": format!("/api/files/blob?path={}", percent_encode(&path_s))})
        }
        "download_only" => {
            json!({"ok": true, "kind": "download_only", "path": path_s, "rel": rel, "size": view.size, "reason": view.blocked_reason, "viewer_max_bytes": view.viewer_max_bytes})
        }
        _ => {
            json!({"ok": true, "kind": view.kind, "path": path_s, "rel": rel, "size": view.size, "text": view.text, "editable": view.editable, "version": view.version})
        }
    }
}

fn read_client_file_view(path: &FsPath) -> Result<ClientFileView, (StatusCode, String)> {
    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "file not found".to_string()));
    }
    if path.is_dir() {
        return Ok(ClientFileView {
            kind: "directory".to_string(),
            size: 0,
            content_type: None,
            text: None,
            editable: false,
            version: None,
            blocked_reason: None,
            viewer_max_bytes: None,
        });
    }
    if !path.is_file() {
        return Err((StatusCode::BAD_REQUEST, "path is not a file".to_string()));
    }
    let metadata = fs::metadata(path).map_err(map_io_error)?;
    let size = metadata.len();
    let raw = fs::read(path).map_err(map_io_error)?;
    let (kind, content_type) = file_kind(path, &raw);
    if kind == "image" || kind == "pdf" {
        return Ok(ClientFileView {
            kind,
            size,
            content_type,
            text: None,
            editable: false,
            version: None,
            blocked_reason: None,
            viewer_max_bytes: None,
        });
    }
    if size > FILE_READ_MAX_BYTES {
        return Ok(ClientFileView {
            kind: "download_only".to_string(),
            size,
            content_type: None,
            text: None,
            editable: false,
            version: None,
            blocked_reason: Some("too_large".to_string()),
            viewer_max_bytes: Some(FILE_READ_MAX_BYTES),
        });
    }
    match decode_text_view(path, &raw) {
        Some((text, editable, version)) => Ok(ClientFileView {
            kind: markdown_kind(path).to_string(),
            size,
            content_type: None,
            text: Some(text),
            editable,
            version: Some(version),
            blocked_reason: None,
            viewer_max_bytes: None,
        }),
        None => Ok(ClientFileView {
            kind: "download_only".to_string(),
            size,
            content_type: None,
            text: None,
            editable: false,
            version: None,
            blocked_reason: Some("binary".to_string()),
            viewer_max_bytes: None,
        }),
    }
}

fn read_text_file_for_write(path: &FsPath) -> Result<(String, u64, String), (StatusCode, String)> {
    let metadata = fs::metadata(path).map_err(map_io_error)?;
    let size = metadata.len();
    if size > FILE_READ_MAX_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("file too large (max {FILE_READ_MAX_BYTES} bytes)"),
        ));
    }
    let raw = fs::read(path).map_err(map_io_error)?;
    if raw.contains(&0) {
        return Err((
            StatusCode::BAD_REQUEST,
            "binary file not supported".to_string(),
        ));
    }
    let text = String::from_utf8(raw.clone()).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "file is not editable as utf-8 text".to_string(),
        )
    })?;
    Ok((text, size, file_version(&raw)))
}

fn write_text_file_atomic(
    path: &FsPath,
    text: &str,
) -> Result<(usize, String), (StatusCode, String)> {
    if path.is_symlink() {
        return Err((
            StatusCode::BAD_REQUEST,
            "symlink file not supported".to_string(),
        ));
    }
    let metadata = fs::metadata(path).map_err(map_io_error)?;
    let data = text.as_bytes();
    if data.len() as u64 > FILE_READ_MAX_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("file too large (max {FILE_READ_MAX_BYTES} bytes)"),
        ));
    }
    let tmp = path.with_file_name(format!(
        ".{}.codoxear-tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    fs::write(&tmp, data).map_err(map_io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            &tmp,
            fs::Permissions::from_mode(metadata.permissions().mode() & 0o777),
        )
        .map_err(map_io_error)?;
    }
    fs::rename(&tmp, path).map_err(map_io_error)?;
    Ok((data.len(), file_version(data)))
}

fn write_new_text_file_atomic(
    path: &FsPath,
    text: &str,
) -> Result<(usize, String), (StatusCode, String)> {
    if path.is_symlink() {
        return Err((
            StatusCode::BAD_REQUEST,
            "symlink file not supported".to_string(),
        ));
    }
    let Some(parent) = path.parent() else {
        return Err((
            StatusCode::NOT_FOUND,
            "parent directory not found".to_string(),
        ));
    };
    if !parent.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            "parent directory not found".to_string(),
        ));
    }
    if !parent.is_dir() {
        return Err((
            StatusCode::BAD_REQUEST,
            "parent path is not a directory".to_string(),
        ));
    }
    if parent.is_symlink() {
        return Err((
            StatusCode::BAD_REQUEST,
            "symlink parent directory not supported".to_string(),
        ));
    }
    if path.exists() {
        return Err((StatusCode::CONFLICT, "file already exists".to_string()));
    }
    let data = text.as_bytes();
    if data.len() as u64 > FILE_READ_MAX_BYTES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("file too large (max {FILE_READ_MAX_BYTES} bytes)"),
        ));
    }
    let tmp = path.with_file_name(format!(
        ".{}.codoxear-tmp-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    fs::write(&tmp, data).map_err(map_io_error)?;
    match fs::hard_link(&tmp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&tmp);
            Ok((data.len(), file_version(data)))
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp);
            Err((StatusCode::CONFLICT, "file already exists".to_string()))
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(map_io_error(err))
        }
    }
}

fn resolve_under(base: &FsPath, rel: &str) -> Result<PathBuf, String> {
    if rel.trim().is_empty() {
        return Err("path required".to_string());
    }
    if rel.contains('\0') {
        return Err("invalid path".to_string());
    }
    let path = FsPath::new(rel);
    if path.is_absolute() {
        return Err("path must be relative".to_string());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("path escapes session cwd".to_string());
    }
    let root = canonical_cwd(base);
    let candidate = root.join(path);
    let parent = candidate.parent().unwrap_or(&root);
    let parent_canon = parent.canonicalize().unwrap_or_else(|_| root.clone());
    if !parent_canon.starts_with(&root) {
        return Err("path escapes session cwd".to_string());
    }
    Ok(candidate)
}

fn files_add(
    state: &AppState,
    session_id: &str,
    path: &str,
) -> Result<Vec<Value>, (StatusCode, String)> {
    if path.trim().is_empty() {
        return Ok(Vec::new());
    }
    let file_path = state.config.app_dir.join("session_files.json");
    with_state_file_lock(&file_path, || {
        let mut files = read_array_file(&file_path, clean_history_items)?;
        let key = format!("sid:{session_id}");
        let legacy = session_id.to_string();
        let mut current = files
            .remove(&key)
            .or_else(|| files.remove(&legacy))
            .unwrap_or_default();
        current.retain(|item| item.as_str() != Some(path));
        current.insert(0, json!(path));
        current.truncate(FILE_HISTORY_MAX);
        files.insert(key, current.clone());
        write_array_file(&file_path, &files)?;
        Ok(current)
    })
}

fn clean_history_items(items: Vec<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    for item in items {
        let Some(path) = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let value = json!(path);
        if !out.contains(&value) {
            out.push(value);
        }
        if out.len() >= FILE_HISTORY_MAX {
            break;
        }
    }
    out
}

fn file_kind(path: &FsPath, raw: &[u8]) -> (String, Option<String>) {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("svg"))
        .unwrap_or(false)
    {
        return (
            "image".to_string(),
            Some("image/svg+xml; charset=utf-8".to_string()),
        );
    }
    if raw.starts_with(b"\x89PNG\r\n\x1a\n") {
        return ("image".to_string(), Some("image/png".to_string()));
    }
    if raw.starts_with(&[0xff, 0xd8, 0xff]) {
        return ("image".to_string(), Some("image/jpeg".to_string()));
    }
    if raw.len() >= 12 && &raw[..4] == b"RIFF" && &raw[8..12] == b"WEBP" {
        return ("image".to_string(), Some("image/webp".to_string()));
    }
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
        || raw.starts_with(b"%PDF-")
    {
        return ("pdf".to_string(), Some("application/pdf".to_string()));
    }
    ("text".to_string(), None)
}

fn decode_text_view(path: &FsPath, raw: &[u8]) -> Option<(String, bool, String)> {
    if raw.contains(&0) {
        return None;
    }
    let (text, editable) = match std::str::from_utf8(raw) {
        Ok(value) => (value.to_string(), true),
        Err(_) if path_looks_textual(path) || looks_like_text_bytes(raw) => {
            (String::from_utf8_lossy(raw).to_string(), false)
        }
        Err(_) => return None,
    };
    Some((text, editable, file_version(raw)))
}

fn path_looks_textual(path: &FsPath) -> bool {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    TEXTUAL_EXTENSIONS.iter().any(|value| *value == ext)
        || path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|name| {
                TEXTUAL_FILENAMES
                    .iter()
                    .any(|value| *value == name.to_ascii_lowercase())
            })
            .unwrap_or(false)
}

fn looks_like_text_bytes(raw: &[u8]) -> bool {
    raw.iter()
        .all(|byte| *byte >= 32 || matches!(*byte, 9 | 10 | 12 | 13 | 27))
}

fn markdown_kind(path: &FsPath) -> &'static str {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if MARKDOWN_EXTENSIONS.iter().any(|value| *value == ext) {
        "markdown"
    } else {
        "text"
    }
}

fn file_version(raw: &[u8]) -> String {
    format!("{:x}", Sha256::digest(raw))
}

fn canonical_cwd(path: &FsPath) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn expand_user(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(raw.trim_start_matches("~/"));
        }
    }
    PathBuf::from(raw)
}

fn percent_encode(raw: &str) -> String {
    raw.bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn map_io_error(err: io::Error) -> (StatusCode, String) {
    match err.kind() {
        io::ErrorKind::NotFound => (StatusCode::NOT_FOUND, "file not found".to_string()),
        io::ErrorKind::PermissionDenied => (StatusCode::FORBIDDEN, "permission denied".to_string()),
        _ => (StatusCode::BAD_REQUEST, err.to_string()),
    }
}
