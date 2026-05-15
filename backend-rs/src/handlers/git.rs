use crate::app_state::AppState;
use crate::git_context::run_git_capture;
use crate::routes::{internal_error, json_response};
use crate::session_loader::find_session;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

const GIT_DIFF_TIMEOUT: Duration = Duration::from_secs(4);
const GIT_DIFF_MAX_BYTES: usize = 800 * 1024;
const GIT_CHANGED_FILES_MAX: usize = 400;
const FILE_READ_MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
pub struct DiffQuery {
    path: Option<String>,
    staged: Option<String>,
}

#[derive(Deserialize)]
pub struct FileVersionsQuery {
    path: Option<String>,
}

pub async fn changed_files(
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
    let cwd = FsPath::new(&row.cwd);
    if let Err(message) = require_git_repo(cwd) {
        return json_response(StatusCode::CONFLICT, json!({"error": message}));
    }
    let unstaged = match run_git_lines(cwd, &["diff", "--name-only"], 64 * 1024) {
        Ok(lines) => norm_list(lines),
        Err(message) => return internal_error(message),
    };
    let staged = match run_git_lines(cwd, &["diff", "--name-only", "--cached"], 64 * 1024) {
        Ok(lines) => norm_list(lines),
        Err(message) => return internal_error(message),
    };
    let unstaged_numstat = run_git(cwd, &["diff", "--numstat"], 128 * 1024).unwrap_or_default();
    let staged_numstat =
        run_git(cwd, &["diff", "--numstat", "--cached"], 128 * 1024).unwrap_or_default();
    let mut stats = parse_git_numstat(&unstaged_numstat);
    for (path, vals) in parse_git_numstat(&staged_numstat) {
        merge_numstat(stats.entry(path).or_default(), &vals);
    }
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for path in unstaged.iter().chain(staged.iter()) {
        if seen.insert(path.clone()) {
            files.push(path.clone());
        }
    }
    let entries = files
        .iter()
        .map(|path| {
            let vals = stats.get(path).cloned().unwrap_or_default();
            json!({
                "path": path,
                "additions": vals.get("additions").cloned().unwrap_or(Value::Null),
                "deletions": vals.get("deletions").cloned().unwrap_or(Value::Null),
                "changed": true,
            })
        })
        .collect::<Vec<_>>();
    json_response(
        StatusCode::OK,
        json!({"ok": true, "cwd": row.cwd, "files": files, "entries": entries, "unstaged": unstaged, "staged": staged}),
    )
}

pub async fn diff(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<DiffQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let Some(raw_path) = query.path.as_deref().filter(|value| !value.is_empty()) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    };
    let cwd = FsPath::new(&row.cwd);
    if let Err(message) = require_git_repo(cwd) {
        return json_response(StatusCode::CONFLICT, json!({"error": message}));
    }
    let (_target, _repo_root, rel) = match resolve_git_path(cwd, raw_path) {
        Ok(value) => value,
        Err(message) => return json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    };
    let staged = query.staged.as_deref() == Some("1");
    let mut args = vec!["diff", "-U3"];
    if staged {
        args.push("--cached");
    }
    args.extend(["--", rel.as_str()]);
    match run_git(cwd, &args, GIT_DIFF_MAX_BYTES) {
        Ok(diff) => json_response(
            StatusCode::OK,
            json!({"ok": true, "cwd": row.cwd, "path": rel, "staged": staged, "diff": diff}),
        ),
        Err(message) => internal_error(message),
    }
}

pub async fn file_versions(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<FileVersionsQuery>,
) -> Response {
    let row = match find_session(&state.config, &session_id) {
        Ok(row) => row,
        Err(message) if message.contains("unknown session") => {
            return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown session"}));
        }
        Err(message) => return internal_error(message),
    };
    let Some(raw_path) = query.path.as_deref().filter(|value| !value.is_empty()) else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "path required"}));
    };
    let cwd = FsPath::new(&row.cwd);
    if let Err(message) = require_git_repo(cwd) {
        return json_response(StatusCode::CONFLICT, json!({"error": message}));
    }
    let (target, _repo_root, rel) = match resolve_git_path(cwd, raw_path) {
        Ok(value) => value,
        Err(message) => return json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    };
    let (current_exists, current_text, current_size) = if target.exists() && target.is_file() {
        match read_text_file_strict(&target, FILE_READ_MAX_BYTES) {
            Ok((text, size)) => (true, text, size),
            Err(message) => {
                return json_response(StatusCode::BAD_REQUEST, json!({"error": message}))
            }
        }
    } else {
        (false, String::new(), 0)
    };
    let (base_exists, base_text) =
        match run_git(cwd, &["show", &format!("HEAD:{rel}")], FILE_READ_MAX_BYTES) {
            Ok(text) => (true, text),
            Err(_) => (false, String::new()),
        };
    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "cwd": row.cwd,
            "path": rel,
            "abs_path": target.to_string_lossy(),
            "current_exists": current_exists,
            "current_size": current_size,
            "current_text": current_text,
            "base_exists": base_exists,
            "base_text": base_text,
        }),
    )
}

fn run_git(cwd: &FsPath, args: &[&str], byte_cap: usize) -> Result<String, String> {
    let (code, out, err) = run_git_capture(cwd, args, GIT_DIFF_TIMEOUT, byte_cap);
    if code == 0 {
        Ok(out)
    } else {
        Err(if err.trim().is_empty() {
            format!("git failed with code {code}")
        } else {
            err.trim().to_string()
        })
    }
}

fn run_git_lines(cwd: &FsPath, args: &[&str], byte_cap: usize) -> Result<Vec<String>, String> {
    Ok(run_git(cwd, args, byte_cap)?
        .lines()
        .map(ToString::to_string)
        .collect())
}

fn require_git_repo(cwd: &FsPath) -> Result<(), String> {
    run_git(cwd, &["rev-parse", "--is-inside-work-tree"], 4096).map(|_| ())
}

fn resolve_git_path(cwd: &FsPath, raw_path: &str) -> Result<(PathBuf, PathBuf, String), String> {
    let repo_root =
        PathBuf::from(run_git(cwd, &["rev-parse", "--show-toplevel"], 64 * 1024)?.trim())
            .canonicalize()
            .map_err(|err| err.to_string())?;
    let target = resolve_session_path(cwd, raw_path)?;
    let rel = target
        .strip_prefix(&repo_root)
        .map_err(|_| "path is outside git repo".to_string())?
        .to_string_lossy()
        .to_string();
    Ok((target, repo_root, rel))
}

fn resolve_session_path(cwd: &FsPath, raw_path: &str) -> Result<PathBuf, String> {
    if raw_path.trim().is_empty() {
        return Err("path required".to_string());
    }
    let raw = FsPath::new(raw_path);
    if raw.is_absolute() || raw_path.split('/').any(|part| part == "..") {
        return Err("path escapes session cwd".to_string());
    }
    let base = cwd.canonicalize().map_err(|err| err.to_string())?;
    let target = base.join(raw_path);
    let parent = target.parent().unwrap_or(&base);
    let resolved_parent = if parent.exists() {
        parent.canonicalize().map_err(|err| err.to_string())?
    } else {
        base.clone()
    };
    if !resolved_parent.starts_with(&base) {
        return Err("path escapes session cwd".to_string());
    }
    Ok(resolved_parent.join(target.file_name().unwrap_or_default()))
}

fn norm_list(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .take(GIT_CHANGED_FILES_MAX)
        .collect()
}

fn parse_git_numstat(text: &str) -> HashMap<String, Map<String, Value>> {
    let mut out: HashMap<String, Map<String, Value>> = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let parts = line.splitn(3, '\t').collect::<Vec<_>>();
        if parts.len() != 3 || parts[2].trim().is_empty() {
            continue;
        }
        let mut entry = Map::new();
        entry.insert("additions".to_string(), parse_numstat_value(parts[0]));
        entry.insert("deletions".to_string(), parse_numstat_value(parts[1]));
        merge_numstat(out.entry(parts[2].trim().to_string()).or_default(), &entry);
    }
    out
}

fn merge_numstat(target: &mut Map<String, Value>, source: &Map<String, Value>) {
    for key in ["additions", "deletions"] {
        let merged = match (target.get(key), source.get(key)) {
            (Some(Value::Number(left)), Some(Value::Number(right))) => {
                json!(left.as_i64().unwrap_or(0) + right.as_i64().unwrap_or(0))
            }
            (None, value) => value.cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        };
        target.insert(key.to_string(), merged);
    }
}

fn parse_numstat_value(raw: &str) -> Value {
    raw.parse::<i64>().map_or(Value::Null, |value| json!(value))
}

fn read_text_file_strict(path: &FsPath, max_bytes: usize) -> Result<(String, usize), String> {
    let raw = fs::read(path).map_err(|err| err.to_string())?;
    if raw.len() > max_bytes {
        return Err(format!("file too large (max {max_bytes} bytes)"));
    }
    let text =
        String::from_utf8(raw.clone()).map_err(|_| "binary file not supported".to_string())?;
    Ok((text, raw.len()))
}
