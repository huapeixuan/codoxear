use crate::app_state::AppState;
use crate::session_create::CreateSessionRequest;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sha2::Digest;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TMUX_SESSION_NAME: &str = "codoxear";
const SPAWN_META_WAIT_SECONDS: f64 = 3.0;

pub(crate) fn spawn_python_broker(
    state: &AppState,
    request: &CreateSessionRequest,
    cwd_path: &Path,
    spawn_nonce: &str,
    argv: Vec<String>,
    envs: Vec<(String, String)>,
) -> Result<Value, (StatusCode, String)> {
    if request.create_in_tmux {
        let tmux_window = safe_filename(
            &format!(
                "{}-{}",
                cwd_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("session"),
                &spawn_nonce[..6]
            ),
            "session",
        );
        let shell_cmd = build_tmux_shell_command(&argv, &envs);
        run_tmux_launch(&tmux_window, &shell_cmd)?;
        let meta = wait_for_spawned_broker_meta(&state.config.app_dir, spawn_nonce)?;
        let mut out = spawn_result_from_meta(&meta)?;
        out["tmux_session"] = json!(TMUX_SESSION_NAME);
        out["tmux_window"] = json!(tmux_window);
        out["ok"] = json!(true);
        return Ok(out);
    }

    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    command.current_dir(repo_root());
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.env("CODEX_WEB_SPAWN_NONCE", spawn_nonce);
    for (key, value) in envs {
        command.env(key, value);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("spawn failed: {err}"),
        )
    })?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    let mut out = wait_for_spawned_broker_meta(&state.config.app_dir, spawn_nonce)
        .and_then(|meta| spawn_result_from_meta(&meta))?;
    out["ok"] = json!(true);
    Ok(out)
}

pub(crate) fn parse_args(value: Option<&Value>) -> Result<Option<Vec<String>>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err("args must be a list of strings".to_string());
                };
                if !text.is_empty() {
                    out.push(text.to_string());
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err("args must be a list of strings".to_string()),
    }
}

pub(crate) fn parse_optional_bool(value: Option<&Value>, field: &str) -> Result<bool, String> {
    match value {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{field} must be a boolean")),
    }
}

pub(crate) fn normalize_agent_backend(
    value: Option<&Value>,
    default: &str,
) -> Result<String, String> {
    let raw = match value {
        Some(Value::String(text)) => text.trim().to_ascii_lowercase(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase(),
    };
    let out = if raw.is_empty() {
        default.to_string()
    } else {
        raw
    };
    if matches!(out.as_str(), "codex" | "pi") {
        Ok(out)
    } else {
        Err("agent_backend must be one of codex, pi".to_string())
    }
}

pub(crate) fn clean_optional_text_value(value: Option<&Value>) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(clean_optional_text(text)),
        Some(_) => Ok(None),
    }
}

pub(crate) fn clean_optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn clean_optional_resume_session_id(
    value: Option<&Value>,
) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(clean_optional_text(text)),
        Some(_) => Err("resume_session_id must be a string".to_string()),
    }
}

pub(crate) fn normalize_requested_model(value: Option<&Value>) -> Result<Option<String>, String> {
    let Some(text) = clean_optional_text_value(value)? else {
        return Ok(None);
    };
    Ok((!text.eq_ignore_ascii_case("default")).then_some(text))
}

pub(crate) fn normalize_reasoning_effort(
    value: Option<&Value>,
    allowed: &[&str],
) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            let out = text.trim().to_ascii_lowercase();
            if out.is_empty() {
                return Ok(None);
            }
            if allowed.iter().any(|candidate| *candidate == out) {
                Ok(Some(out))
            } else {
                Err(format!(
                    "reasoning_effort must be one of {}",
                    allowed.join(", ")
                ))
            }
        }
        Some(_) => Err("reasoning_effort must be a string".to_string()),
    }
}

pub(crate) fn normalize_preferred_auth_method(
    value: Option<&Value>,
) -> Result<Option<String>, String> {
    let Some(method) = clean_optional_text_value(value)? else {
        return Ok(None);
    };
    if matches!(method.as_str(), "chatgpt" | "apikey") {
        Ok(Some(method))
    } else {
        Err("preferred_auth_method must be one of chatgpt, apikey".to_string())
    }
}

pub(crate) fn normalize_service_tier(value: Option<&Value>) -> Result<Option<String>, String> {
    let Some(tier) = clean_optional_text_value(value)? else {
        return Ok(None);
    };
    if matches!(tier.as_str(), "fast" | "flex") {
        Ok(Some(tier))
    } else {
        Err("service_tier must be one of fast, flex".to_string())
    }
}

pub(crate) fn resolve_dir_target(
    raw: &str,
    field_name: &str,
) -> Result<PathBuf, (StatusCode, String)> {
    if raw.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, format!("{field_name} required")));
    }
    let path = expand_user_path(raw);
    let resolved = progressive_canonicalize(&path);
    if resolved.exists() && !resolved.is_dir() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} is not a directory: {}", resolved.display()),
        ));
    }
    Ok(resolved)
}

fn expand_user_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    let home = std::env::var("HOME").unwrap_or_default();
    let expanded = if trimmed == "~" {
        home
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("${HOME}") {
        format!("{home}{rest}")
    } else {
        trimmed.replace("$HOME", &home)
    };
    PathBuf::from(expanded)
}

fn progressive_canonicalize(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    let mut missing_tail: Vec<OsString> = Vec::new();
    let mut cursor = path;
    loop {
        if let Ok(canonical_prefix) = fs::canonicalize(cursor) {
            let mut out = canonical_prefix;
            for part in missing_tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (cursor.parent(), cursor.file_name()) {
            (Some(parent), Some(name)) => {
                missing_tail.push(name.to_os_string());
                cursor = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

pub(crate) fn create_git_worktree(
    source_cwd: &Path,
    branch: &str,
) -> Result<PathBuf, (StatusCode, String)> {
    let branch = clean_worktree_branch(branch)?;
    let repo_root = git_repo_root(source_cwd)?;
    let target = default_worktree_path(source_cwd, &branch);
    if target.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("derived worktree path already exists: {}", target.display()),
        ));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    }
    let output = Command::new("git")
        .args(["worktree", "add", "-b", &branch, &target.to_string_lossy()])
        .current_dir(repo_root)
        .output()
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err((
            StatusCode::BAD_REQUEST,
            if stderr.is_empty() { stdout } else { stderr },
        ));
    }
    Ok(progressive_canonicalize(&target))
}

fn clean_worktree_branch(branch: &str) -> Result<String, (StatusCode, String)> {
    let out = branch.trim();
    if out.is_empty() {
        Err((
            StatusCode::BAD_REQUEST,
            "worktree_branch required".to_string(),
        ))
    } else {
        Ok(out.to_string())
    }
}

fn git_repo_root(cwd: &Path) -> Result<PathBuf, (StatusCode, String)> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    if !output.status.success() {
        return Err((
            StatusCode::BAD_REQUEST,
            "cwd is not inside a git worktree".to_string(),
        ));
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        Err((
            StatusCode::BAD_REQUEST,
            "cwd is not inside a git worktree".to_string(),
        ))
    } else {
        Ok(PathBuf::from(root))
    }
}

fn default_worktree_path(source_cwd: &Path, branch: &str) -> PathBuf {
    let slug = worktree_path_slug(branch);
    source_cwd
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            "{}-{slug}",
            source_cwd
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("worktree")
        ))
        .to_path_buf()
}

pub(crate) fn worktree_path_slug(branch: &str) -> String {
    let mut slug = String::new();
    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            slug.push(ch);
        } else {
            slug.push('-');
        }
    }
    let trimmed = slug.trim_matches(['.', '-']);
    if trimmed.is_empty() {
        "worktree".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn base_spawn_env(backend: &str, spawn_nonce: &str) -> Vec<(String, String)> {
    vec![
        ("CODEX_WEB_OWNER".to_string(), "web".to_string()),
        ("CODEX_WEB_AGENT_BACKEND".to_string(), backend.to_string()),
        ("CODEX_WEB_SPAWN_NONCE".to_string(), spawn_nonce.to_string()),
    ]
}

pub(crate) fn python_exe() -> String {
    std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string())
}

pub(crate) fn codex_home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".codex"))
}

pub(crate) fn pi_home() -> PathBuf {
    std::env::var("PI_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".pi"))
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

pub(crate) fn pi_new_session_file_for_cwd(cwd: &Path) -> PathBuf {
    let slug = cwd.to_string_lossy().trim_matches('/').replace('/', "-");
    let dir = pi_home()
        .join("agent")
        .join("sessions")
        .join(format!("--{slug}--"));
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    dir.join(format!("{}.jsonl", millis))
}

pub(crate) fn find_pi_resume_session_file(cwd: &Path, resume_id: &str) -> Option<PathBuf> {
    let dir = pi_home().join("agent").join("sessions");
    let mut stack = vec![dir];
    while let Some(path) = stack.pop() {
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            if pi_session_file_matches(&path, cwd, resume_id) {
                return Some(path);
            }
        }
    }
    None
}

fn pi_session_file_matches(path: &Path, cwd: &Path, resume_id: &str) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    raw.lines().take(20).any(|line| {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        value
            .get("sessionId")
            .or_else(|| value.get("session_id"))
            .and_then(Value::as_str)
            == Some(resume_id)
            || value
                .pointer("/session/cwd")
                .and_then(Value::as_str)
                .is_some_and(|raw_cwd| raw_cwd == cwd.to_string_lossy())
    })
}

pub(crate) fn codex_trust_override_for_path(path: &Path) -> String {
    format!(
        "projects={{ {} = {{ trust_level = \"trusted\" }} }}",
        serde_json::to_string(&path.to_string_lossy()).unwrap_or_else(|_| "\".\"".to_string())
    )
}

fn wait_for_spawned_broker_meta(
    app_dir: &Path,
    spawn_nonce: &str,
) -> Result<Value, (StatusCode, String)> {
    let deadline = Instant::now() + Duration::from_secs_f64(SPAWN_META_WAIT_SECONDS);
    let socks = app_dir.join("socks");
    while Instant::now() <= deadline {
        if let Ok(entries) = fs::read_dir(&socks) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let Ok(raw) = fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(meta) = serde_json::from_str::<Value>(&raw) else {
                    continue;
                };
                if meta.get("spawn_nonce").and_then(Value::as_str) == Some(spawn_nonce)
                    && meta.get("broker_pid").and_then(Value::as_i64).is_some()
                {
                    return Ok(meta);
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("tmux launch did not publish broker metadata within {SPAWN_META_WAIT_SECONDS:.1}s"),
    ))
}

fn spawn_result_from_meta(meta: &Value) -> Result<Value, (StatusCode, String)> {
    let broker_pid = meta
        .get("broker_pid")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "spawn metadata is missing broker_pid".to_string(),
            )
        })?;
    let mut out = serde_json::Map::new();
    out.insert("broker_pid".to_string(), json!(broker_pid));
    if let Some(sock_path) = meta.get("sock_path").and_then(Value::as_str) {
        if let Some(stem) = Path::new(sock_path)
            .file_stem()
            .and_then(|stem| stem.to_str())
        {
            out.insert("session_id".to_string(), json!(stem));
        }
    }
    if let Some(backend) = meta.get("backend").and_then(Value::as_str) {
        out.insert("backend".to_string(), json!(backend));
    }
    Ok(Value::Object(out))
}

fn run_tmux_launch(tmux_window: &str, shell_cmd: &str) -> Result<(), (StatusCode, String)> {
    let has_session = Command::new("tmux")
        .args(["has-session", "-t", TMUX_SESSION_NAME])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let mut args = if has_session {
        vec![
            "new-window".to_string(),
            "-d".to_string(),
            "-P".to_string(),
            "-F".to_string(),
            "#{pane_id}".to_string(),
            "-t".to_string(),
            format!("{TMUX_SESSION_NAME}:"),
            "-n".to_string(),
            tmux_window.to_string(),
        ]
    } else {
        vec![
            "new-session".to_string(),
            "-d".to_string(),
            "-P".to_string(),
            "-F".to_string(),
            "#{pane_id}".to_string(),
            "-s".to_string(),
            TMUX_SESSION_NAME.to_string(),
            "-n".to_string(),
            tmux_window.to_string(),
        ]
    };
    args.push(shell_cmd.to_string());
    let output = Command::new("tmux")
        .args(args)
        .output()
        .map_err(internal_error)?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("tmux launch failed: {detail}"),
        ))
    }
}

fn build_tmux_shell_command(argv: &[String], envs: &[(String, String)]) -> String {
    let mut parts = vec![
        "cd".to_string(),
        shell_quote(&repo_root().to_string_lossy()),
    ];
    parts.push("&&".to_string());
    parts.push("exec".to_string());
    parts.push("env".to_string());
    for (key, value) in envs {
        parts.push(shell_quote(&format!("{key}={value}")));
    }
    for arg in argv {
        parts.push(shell_quote(arg));
    }
    parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '='))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn safe_filename(value: &str, default: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches(['.', '-']);
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn spawn_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let digest = sha2::Sha256::digest(format!("{pid}:{nanos}").as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn internal_error(err: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
