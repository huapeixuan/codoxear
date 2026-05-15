use crate::app_state::AppState;
use crate::routes::{internal_error, json_response};
use crate::runtime::read_cwd_groups;
use crate::session_loader::{
    load_session_rows, load_sessions_directories_payload, load_sessions_recent_payload,
};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const RECENT_PAGE_SIZE: usize = 12;
const RECENT_GROUP_LIMIT: usize = 6;

#[derive(Deserialize)]
pub struct SessionsQuery {
    view: Option<String>,
    group_key: Option<String>,
    offset: Option<String>,
    limit: Option<String>,
    group_offset: Option<String>,
    group_limit: Option<String>,
}

#[derive(Deserialize)]
pub struct ResumeCandidatesQuery {
    cwd: Option<String>,
    backend: Option<String>,
    agent_backend: Option<String>,
}

pub async fn sessions(
    State(state): State<AppState>,
    Query(query): Query<SessionsQuery>,
) -> Response {
    match sessions_payload(&state, &query) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(SessionListError::BadRequest(payload)) => {
            json_response(StatusCode::BAD_REQUEST, payload)
        }
        Err(SessionListError::Internal(message)) => internal_error(message),
    }
}

pub async fn session_resume_candidates(Query(query): Query<ResumeCandidatesQuery>) -> Response {
    match resume_candidates_payload(&query) {
        Ok(payload) => json_response(StatusCode::OK, payload),
        Err(payload) => json_response(StatusCode::BAD_REQUEST, payload),
    }
}

fn sessions_payload(state: &AppState, query: &SessionsQuery) -> Result<Value, SessionListError> {
    let view = query
        .view
        .as_deref()
        .unwrap_or("directories")
        .trim()
        .to_ascii_lowercase();
    if !matches!(view.as_str(), "directories" | "recent") {
        return Err(SessionListError::BadRequest(
            json!({"error": "unsupported sessions view", "view": view}),
        ));
    }

    let offset = parse_usize(query.offset.as_deref(), 0);
    let limit = parse_usize(query.limit.as_deref(), RECENT_PAGE_SIZE).clamp(1, 50);
    let group_offset = parse_usize(query.group_offset.as_deref(), 0);
    let group_limit = parse_usize(query.group_limit.as_deref(), RECENT_GROUP_LIMIT).clamp(1, 20);

    if view == "recent"
        && (query.group_key.is_some() || group_offset > 0 || query.group_limit.is_some())
    {
        return Err(SessionListError::BadRequest(json!({
            "error": "group pagination is not supported for recent view"
        })));
    }

    let rows = load_session_rows(&state.config).map_err(SessionListError::Internal)?;
    let cwd_groups = read_cwd_groups(&state.config.app_dir.join("cwd_groups.json"))
        .map_err(SessionListError::Internal)?;
    let payload = if view == "recent" {
        load_sessions_recent_payload(&rows, &cwd_groups, offset, limit)
    } else {
        load_sessions_directories_payload(
            &rows,
            &cwd_groups,
            query.group_key.as_deref(),
            offset,
            limit,
            group_offset,
            group_limit,
        )
    };
    Ok(payload)
}

fn resume_candidates_payload(query: &ResumeCandidatesQuery) -> Result<Value, Value> {
    if let Some(raw) = query.agent_backend.as_deref() {
        normalize_agent_backend(raw, "codex").map_err(|message| json!({"error": message}))?;
    }
    let cwd_raw = query.cwd.as_deref().unwrap_or_default();
    let cwd =
        resolve_dir_target(cwd_raw).map_err(|message| json!({"error": message, "field": "cwd"}))?;
    normalize_requested_backend(query.backend.as_deref())
        .map_err(|message| json!({"error": message, "field": "backend"}))?;
    let info = describe_session_cwd(&cwd);
    let mut payload = Map::new();
    payload.insert("ok".to_string(), Value::Bool(true));
    if let Value::Object(object) = info {
        payload.extend(object);
    }
    payload.insert("sessions".to_string(), Value::Array(Vec::new()));
    Ok(Value::Object(payload))
}

fn describe_session_cwd(cwd: &Path) -> Value {
    let exists = cwd.exists();
    let repo_root = exists.then(|| git_repo_root(cwd)).flatten();
    let git_branch = if exists {
        crate::git_context::current_git_branch(cwd).unwrap_or_default()
    } else {
        String::new()
    };
    json!({
        "cwd": cwd.to_string_lossy(),
        "exists": exists,
        "will_create": !exists,
        "git_repo": repo_root.is_some(),
        "git_root": repo_root.map(|path| path.to_string_lossy().to_string()).unwrap_or_default(),
        "git_branch": git_branch,
    })
}

fn git_repo_root(cwd: &Path) -> Option<PathBuf> {
    let (code, out, _) = crate::git_context::run_git_capture(
        cwd,
        &["rev-parse", "--show-toplevel"],
        std::time::Duration::from_secs(2),
        64 * 1024,
    );
    (code == 0)
        .then(|| out.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn resolve_dir_target(raw: &str) -> Result<PathBuf, String> {
    if raw.trim().is_empty() {
        return Err("cwd required".to_string());
    }
    let expanded = expand_user(raw.trim());
    let path = if expanded.exists() {
        expanded
            .canonicalize()
            .map_err(|err| format!("resolve cwd: {err}"))?
    } else if let Some(parent) = expanded.parent().filter(|parent| parent.exists()) {
        parent
            .canonicalize()
            .map_err(|err| format!("resolve cwd: {err}"))?
            .join(expanded.file_name().unwrap_or_default())
    } else {
        expanded
    };
    if path.exists() && !path.is_dir() {
        return Err(format!("cwd is not a directory: {}", path.display()));
    }
    Ok(path)
}

fn expand_user(raw: &str) -> PathBuf {
    if raw == "~" || raw.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(raw.trim_start_matches("~/"));
        }
    }
    PathBuf::from(raw)
}

fn normalize_agent_backend(raw: &str, default: &str) -> Result<String, String> {
    let backend = if raw.trim().is_empty() {
        default.to_string()
    } else {
        raw.trim().to_ascii_lowercase()
    };
    if matches!(backend.as_str(), "codex" | "pi") {
        Ok(backend)
    } else {
        Err("agent_backend must be one of codex, pi".to_string())
    }
}

fn normalize_requested_backend(raw: Option<&str>) -> Result<String, String> {
    let backend = raw.unwrap_or("codex").trim().to_ascii_lowercase();
    if backend.is_empty() {
        return Ok("codex".to_string());
    }
    if matches!(backend.as_str(), "codex" | "pi") {
        Ok(backend)
    } else {
        Err("backend must be one of codex, pi".to_string())
    }
}

fn parse_usize(raw: Option<&str>, default: usize) -> usize {
    raw.and_then(|value| value.parse::<isize>().ok())
        .map(|value| value.max(0) as usize)
        .unwrap_or(default)
}

enum SessionListError {
    BadRequest(Value),
    Internal(String),
}
