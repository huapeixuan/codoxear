use crate::app_state::AppState;
use crate::routes::{internal_error, json_response};
use crate::session_loader::find_session;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{self, HeaderValue};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs;
use std::io;
use std::path::{Component, Path as FsPath, PathBuf};

const FILE_READ_MAX_BYTES: u64 = 2 * 1024 * 1024;
const FILE_SEARCH_LIMIT: usize = 120;
const FILE_SEARCH_MAX_CANDIDATES: usize = 200_000;
const FILE_LIST_IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".mypy_cache",
    ".pytest_cache",
    ".svn",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "venv",
    ".venv",
];
const MARKDOWN_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd"];
const TEXTUAL_EXTENSIONS: &[&str] = &[
    "bash", "c", "cc", "cfg", "conf", "cpp", "css", "csv", "diff", "go", "h", "hpp", "htm", "html",
    "ini", "java", "js", "json", "jsonl", "log", "md", "markdown", "mdown", "mkd", "patch", "py",
    "rs", "scss", "sh", "sql", "svg", "toml", "ts", "tsx", "txt", "xml", "yaml", "yml", "zsh",
];
const TEXTUAL_FILENAMES: &[&str] = &["dockerfile", "license", "makefile", "readme"];

#[derive(Deserialize)]
pub struct PathQuery {
    path: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    limit: Option<String>,
}

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

pub async fn file_read(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<PathQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let Some(rel) = query.path.as_deref().filter(|value| !value.is_empty()) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    };
    let target = match resolve_session_path(FsPath::new(&row.cwd), rel) {
        Ok(path) => path,
        Err(message) => return json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    };
    if !target.exists() {
        return json_response(StatusCode::NOT_FOUND, json!({"error": "file not found"}));
    }
    if !target.is_file() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "path is not a file"}),
        );
    }
    let view = match read_client_file_view(&target) {
        Ok(view) => view,
        Err(FileError::Permission(message)) => {
            return json_response(StatusCode::FORBIDDEN, json!({"error": message}));
        }
        Err(FileError::BadRequest(message)) => {
            return json_response(StatusCode::BAD_REQUEST, json!({"error": message}));
        }
        Err(FileError::NotFound(message)) => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": message}));
        }
    };
    let path_s = target.to_string_lossy().to_string();
    match view.kind.as_str() {
        "image" => json_response(
            StatusCode::OK,
            json!({"ok": true, "kind": "image", "content_type": view.content_type, "path": path_s, "rel": rel, "size": view.size, "image_url": format!("/api/sessions/{session_id}/file/blob?path={}", percent_encode(rel))}),
        ),
        "pdf" => json_response(
            StatusCode::OK,
            json!({"ok": true, "kind": "pdf", "content_type": view.content_type, "path": path_s, "rel": rel, "size": view.size, "pdf_url": format!("/api/sessions/{session_id}/file/blob?path={}", percent_encode(rel))}),
        ),
        "download_only" => json_response(
            StatusCode::OK,
            json!({"ok": true, "kind": "download_only", "path": path_s, "rel": rel, "size": view.size, "reason": view.blocked_reason, "viewer_max_bytes": view.viewer_max_bytes}),
        ),
        _ => json_response(
            StatusCode::OK,
            json!({"ok": true, "kind": view.kind, "path": path_s, "rel": rel, "size": view.size, "text": view.text, "editable": view.editable, "version": view.version}),
        ),
    }
}

pub async fn file_search(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let Some(raw_query) = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "q required"}));
    };
    let limit = match parse_limit(query.limit.as_deref()) {
        Ok(limit) => limit,
        Err(message) => return json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    };
    match search_session_relative_files(FsPath::new(&row.cwd), raw_query, limit) {
        Ok(payload) => json_response(
            StatusCode::OK,
            json!({"ok": true, "cwd": canonical_cwd(FsPath::new(&row.cwd)).to_string_lossy(), "query": payload["query"].clone(), "mode": payload["mode"].clone(), "matches": payload["matches"].clone(), "scanned": payload["scanned"].clone(), "truncated": payload["truncated"].clone()}),
        ),
        Err(FileError::Permission(message)) => {
            json_response(StatusCode::FORBIDDEN, json!({"error": message}))
        }
        Err(FileError::NotFound(message)) => {
            json_response(StatusCode::NOT_FOUND, json!({"error": message}))
        }
        Err(FileError::BadRequest(message)) => {
            json_response(StatusCode::BAD_REQUEST, json!({"error": message}))
        }
    }
}

pub async fn file_list(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<PathQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let rel = query.path.as_deref().unwrap_or_default();
    match list_session_directory_entries(FsPath::new(&row.cwd), rel) {
        Ok(entries) => json_response(
            StatusCode::OK,
            json!({"ok": true, "cwd": canonical_cwd(FsPath::new(&row.cwd)).to_string_lossy(), "path": rel, "entries": entries}),
        ),
        Err(FileError::Permission(message)) => {
            json_response(StatusCode::FORBIDDEN, json!({"error": message}))
        }
        Err(FileError::NotFound(message)) => {
            json_response(StatusCode::NOT_FOUND, json!({"error": message}))
        }
        Err(FileError::BadRequest(message)) => {
            json_response(StatusCode::BAD_REQUEST, json!({"error": message}))
        }
    }
}

pub async fn file_blob(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<PathQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let Some(rel) = query.path.as_deref().filter(|value| !value.is_empty()) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    };
    let target = match resolve_session_path(FsPath::new(&row.cwd), rel) {
        Ok(path) => path,
        Err(message) => return json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    };
    match inline_blob_response(&target) {
        Ok(response) => response,
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub async fn files_blob(Query(query): Query<PathQuery>) -> Response {
    let Some(raw_path) = query.path.as_deref().filter(|value| !value.is_empty()) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    };
    let path = expand_absolute(raw_path);
    match inline_blob_response(&path) {
        Ok(response) => response,
        Err((status, message)) => json_response(status, json!({"error": message})),
    }
}

pub async fn file_download(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<PathQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let Some(rel) = query.path.as_deref().filter(|value| !value.is_empty()) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    };
    let target = match resolve_session_path(FsPath::new(&row.cwd), rel) {
        Ok(path) => path,
        Err(message) => return json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    };
    match fs::read(&target) {
        Ok(raw) if target.is_file() => bytes_response(
            raw,
            "application/octet-stream",
            Some(download_disposition(&target)),
        ),
        Ok(_) => json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "path is not a file"}),
        ),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            json_response(StatusCode::NOT_FOUND, json!({"error": "file not found"}))
        }
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            json_response(StatusCode::FORBIDDEN, json!({"error": "permission denied"}))
        }
        Err(err) => json_response(StatusCode::BAD_REQUEST, json!({"error": err.to_string()})),
    }
}

pub fn resolve_session_path(base: &FsPath, raw_path: &str) -> Result<PathBuf, String> {
    if raw_path.trim().is_empty() {
        return Err("path required".to_string());
    }
    if raw_path.contains('\0') {
        return Err("invalid path".to_string());
    }
    let raw = FsPath::new(raw_path);
    if raw.is_absolute() {
        return Err("path must be relative".to_string());
    }
    if raw
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("path escapes session cwd".to_string());
    }
    let resolved_base = canonical_cwd(base);
    let candidate = resolved_base.join(raw);
    if let Ok(canon) = candidate.canonicalize() {
        if !canon.starts_with(&resolved_base) {
            return Err("path escapes session cwd".to_string());
        }
        return Ok(canon);
    }
    let parent = candidate.parent().unwrap_or(&resolved_base);
    let parent_canon = parent
        .canonicalize()
        .unwrap_or_else(|_| resolved_base.clone());
    if !parent_canon.starts_with(&resolved_base) {
        return Err("path escapes session cwd".to_string());
    }
    Ok(candidate)
}

fn canonical_cwd(path: &FsPath) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn read_client_file_view(path: &FsPath) -> Result<ClientFileView, FileError> {
    if !path.exists() {
        return Err(FileError::NotFound("file not found".to_string()));
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
        return Err(FileError::BadRequest("path is not a file".to_string()));
    }
    let metadata = fs::metadata(path).map_err(map_io_error)?;
    let size = metadata.len();
    let raw = fs::read(path).map_err(map_io_error)?;
    let prefix = &raw[..raw.len().min(4096)];
    let (kind, content_type) = file_kind(path, prefix);
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
    match decode_text_view_for_client(path, &raw) {
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

fn inline_blob_response(path: &FsPath) -> Result<Response, (StatusCode, String)> {
    if !path.exists() {
        return Err((StatusCode::NOT_FOUND, "file not found".to_string()));
    }
    if !path.is_file() {
        return Err((StatusCode::BAD_REQUEST, "path is not a file".to_string()));
    }
    let raw = fs::read(path).map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    let (kind, ctype) = file_kind(path, &raw);
    let Some(content_type) = ctype.filter(|_| kind == "image" || kind == "pdf") else {
        return Err((
            StatusCode::BAD_REQUEST,
            "file is not previewable inline".to_string(),
        ));
    };
    Ok(bytes_response(
        raw,
        &content_type,
        Some(format!(
            "inline; filename*=UTF-8''{}",
            percent_encode(path.file_name().and_then(|s| s.to_str()).unwrap_or("file"))
        )),
    ))
}

fn bytes_response(raw: Vec<u8>, content_type: &str, disposition: Option<String>) -> Response {
    let len = raw.len();
    let mut response = Response::new(Body::from(raw));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&len.to_string()).unwrap(),
    );
    if let Some(value) = disposition {
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&value).unwrap(),
        );
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::EXPIRES, HeaderValue::from_static("0"));
    response
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

fn decode_text_view_for_client(path: &FsPath, raw: &[u8]) -> Option<(String, bool, String)> {
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
    let version = format!("{:x}", Sha256::digest(raw));
    Some((text, editable, version))
}

fn path_looks_textual(path: &FsPath) -> bool {
    let ext = file_extension(path);
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
    if MARKDOWN_EXTENSIONS
        .iter()
        .any(|value| *value == file_extension(path))
    {
        "markdown"
    } else {
        "text"
    }
}

fn file_extension(path: &FsPath) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn list_session_directory_entries(base: &FsPath, raw_path: &str) -> Result<Value, FileError> {
    let root = canonical_cwd(base);
    if !root.exists() {
        return Err(FileError::NotFound("session cwd not found".to_string()));
    }
    if !root.is_dir() {
        return Err(FileError::BadRequest(
            "session cwd is not a directory".to_string(),
        ));
    }
    let target = resolve_session_relative_child(&root, raw_path)?;
    if !target.exists() {
        return Err(FileError::NotFound("path not found".to_string()));
    }
    if !target.is_dir() {
        return Err(FileError::BadRequest("path is not a directory".to_string()));
    }
    let mut entries = Vec::new();
    let mut children = fs::read_dir(&target)
        .map_err(map_io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_io_error)?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let child_path = child.path();
        let name = child.file_name().to_string_lossy().to_string();
        if child_path.is_dir() && ignored_dir(&name) {
            continue;
        }
        let rel = child_path
            .strip_prefix(&root)
            .unwrap_or(&child_path)
            .to_string_lossy()
            .replace('\\', "/");
        entries.push(json!({"name": name, "path": rel, "kind": if child_path.is_dir() { "dir" } else { "file" }}));
    }
    entries.sort_by_key(entry_sort_key);
    Ok(Value::Array(entries))
}

fn resolve_session_relative_child(base: &FsPath, raw_path: &str) -> Result<PathBuf, FileError> {
    let rel = raw_path.trim();
    if rel.is_empty() {
        return Ok(base.to_path_buf());
    }
    if rel.contains('\0') {
        return Err(FileError::BadRequest("invalid path".to_string()));
    }
    let path = FsPath::new(rel);
    if path.is_absolute() {
        return Err(FileError::BadRequest("path must be relative".to_string()));
    }
    let resolved = resolve_session_path(base, rel).map_err(FileError::BadRequest)?;
    Ok(resolved)
}

fn search_session_relative_files(
    base: &FsPath,
    query: &str,
    limit: usize,
) -> Result<Value, FileError> {
    let root = canonical_cwd(base);
    if !root.exists() {
        return Err(FileError::NotFound("session cwd not found".to_string()));
    }
    if !root.is_dir() {
        return Err(FileError::BadRequest(
            "session cwd is not a directory".to_string(),
        ));
    }
    let raw_query = query.trim();
    if raw_query.is_empty() {
        return Err(FileError::BadRequest("query required".to_string()));
    }
    let clamped = limit.clamp(1, FILE_SEARCH_LIMIT);
    let mut heap: BinaryHeap<Reverse<(i64, String)>> = BinaryHeap::new();
    let mut scanned = 0usize;
    search_walk(&root, &root, raw_query, clamped, &mut scanned, &mut heap)?;
    let mut matches = heap
        .into_iter()
        .map(|Reverse((score, path))| (score, path))
        .collect::<Vec<_>>();
    matches.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(
        json!({"mode": "walk", "query": raw_query, "matches": matches.into_iter().map(|(score, path)| json!({"path": path, "score": score})).collect::<Vec<_>>(), "scanned": scanned, "truncated": false}),
    )
}

fn search_walk(
    root: &FsPath,
    current: &FsPath,
    query: &str,
    limit: usize,
    scanned: &mut usize,
    heap: &mut BinaryHeap<Reverse<(i64, String)>>,
) -> Result<(), FileError> {
    let mut entries = fs::read_dir(current)
        .map_err(map_io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_io_error)?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !ignored_dir(&name) {
                dirs.push(path);
            }
        } else if path.is_file() {
            files.push(path);
        }
    }
    for path in files {
        *scanned += 1;
        if *scanned > FILE_SEARCH_MAX_CANDIDATES {
            return Ok(());
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let score = file_search_score(&rel, query);
        if score < 0 {
            continue;
        }
        let item = Reverse((score, rel));
        if heap.len() < limit {
            heap.push(item);
        } else if heap.peek().map(|top| item > *top).unwrap_or(true) {
            heap.pop();
            heap.push(item);
        }
    }
    for dir in dirs {
        search_walk(root, &dir, query, limit, scanned, heap)?;
    }
    Ok(())
}

fn file_search_score(candidate: &str, query: &str) -> i64 {
    let lower = candidate.to_ascii_lowercase();
    let raw = query.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return 0;
    }
    if lower == raw {
        return 12000;
    }
    let base = FsPath::new(candidate)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if base == raw {
        return 10000;
    }
    let mut total = 0i64;
    for token in raw.split_whitespace() {
        if let Some(idx) = lower.find(token) {
            let prev = if idx > 0 {
                lower.as_bytes()[idx - 1] as char
            } else {
                '\0'
            };
            let boundary_bonus = if idx == 0 || "/._-".contains(prev) {
                24
            } else {
                0
            };
            let base_idx = base.find(token).map(|value| value as i64).unwrap_or(-1);
            total += 240 - (idx as i64 * 2)
                + boundary_bonus
                + if base_idx >= 0 { 44 - base_idx } else { 0 };
            continue;
        }
        let mut pos = None;
        let mut first = None;
        let mut last = None;
        let mut consecutive = 0;
        let mut boundaries = 0;
        for ch in token.chars() {
            let start = pos.map(|value: usize| value + 1).unwrap_or(0);
            let found = lower[start..].find(ch).map(|value| value + start);
            let Some(found) = found else {
                return -1;
            };
            if first.is_none() {
                first = Some(found);
            }
            if last.map(|value| found == value + 1).unwrap_or(false) {
                consecutive += 1;
            }
            if found == 0 || "/._-".contains(lower.as_bytes()[found - 1] as char) {
                boundaries += 1;
            }
            last = Some(found);
            pos = Some(found);
        }
        let first = first.unwrap_or(0) as i64;
        let span = last.unwrap_or(0) as i64 - first + 1;
        total +=
            120 - first - 0.max(span - token.len() as i64) * 4 + consecutive * 10 + boundaries * 8;
    }
    total
}

fn parse_limit(raw: Option<&str>) -> Result<usize, String> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(FILE_SEARCH_LIMIT);
    };
    let limit = value
        .parse::<usize>()
        .map_err(|_| "limit must be an integer".to_string())?;
    (limit >= 1)
        .then_some(limit)
        .ok_or_else(|| "limit must be >= 1".to_string())
}

fn entry_sort_key(value: &Value) -> (i32, String) {
    let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
    let name = value.get("name").and_then(Value::as_str).unwrap_or("");
    (if kind == "dir" { 0 } else { 1 }, name.to_string())
}

fn ignored_dir(name: &str) -> bool {
    FILE_LIST_IGNORED_DIRS.contains(&name)
}

fn map_io_error(err: io::Error) -> FileError {
    match err.kind() {
        io::ErrorKind::NotFound => FileError::NotFound("file not found".to_string()),
        io::ErrorKind::PermissionDenied => FileError::Permission("permission denied".to_string()),
        _ => FileError::BadRequest(err.to_string()),
    }
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn download_disposition(path: &FsPath) -> String {
    format!(
        "attachment; filename*=UTF-8''{}",
        percent_encode(
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("file")
        )
    )
}

fn expand_absolute(raw: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let text = raw.trim().replace("${HOME}", &home);
    let expanded = text.strip_prefix("~/").map_or_else(
        || PathBuf::from(&text),
        |stripped| PathBuf::from(home).join(stripped),
    );
    expanded.canonicalize().unwrap_or(expanded)
}

#[derive(Debug)]
enum FileError {
    NotFound(String),
    Permission(String),
    BadRequest(String),
}
