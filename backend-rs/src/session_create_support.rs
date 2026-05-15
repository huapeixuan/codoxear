use crate::app_state::AppState;
use crate::session_create::CreateSessionRequest;
use axum::http::StatusCode;
use serde_json::{json, Map, Value};
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
    if std::env::var("CODOXEAR_FAKE_SPAWN_FOR_TESTS")
        .ok()
        .is_some_and(|value| crate::workers::env_flag_truthy_value(Some(&value)))
    {
        return Ok(json!({
            "ok": true,
            "session_id": format!("fake-{spawn_nonce}"),
            "backend": request.backend,
            "broker_pid": std::process::id(),
        }));
    }
    if request.create_in_tmux {
        let name = cwd_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("session");
        let tmux_window = crate::write_cleaners::clean_safe_filename(
            &format!("{}-{}", name, &spawn_nonce[..6]),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexResumeCandidate {
    pub session_id: String,
    pub log_path: PathBuf,
    pub first_user_message: Option<String>,
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
    find_pi_resume_session_file_in(&pi_home().join("agent").join("sessions"), cwd, resume_id)
}

pub fn find_pi_resume_session_file_in(
    sessions_dir: &Path,
    cwd: &Path,
    resume_id: &str,
) -> Option<PathBuf> {
    let mut stack = vec![sessions_dir.to_path_buf()];
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

pub fn find_codex_resume_candidate_in(
    sessions_dir: &Path,
    cwd: &Path,
    resume_id: &str,
) -> Option<CodexResumeCandidate> {
    let cwd_text = cwd.to_string_lossy();
    let mut ranked = Vec::new();
    collect_codex_resume_logs(sessions_dir, &mut ranked);
    ranked.sort_by(|left, right| right.0.total_cmp(&left.0));
    for (_, path) in ranked {
        let Some(payload) = codex_session_meta_payload(&path) else {
            continue;
        };
        if is_codex_subagent_meta(&payload) {
            continue;
        }
        if payload.get("id").and_then(Value::as_str) != Some(resume_id) {
            continue;
        }
        if payload.get("cwd").and_then(Value::as_str) != Some(cwd_text.as_ref()) {
            continue;
        }
        return Some(CodexResumeCandidate {
            session_id: resume_id.to_string(),
            first_user_message: first_user_message_preview_from_codex_log(&path),
            log_path: path,
        });
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
        let id_matches = value
            .get("id")
            .or_else(|| value.get("sessionId"))
            .or_else(|| value.get("session_id"))
            .and_then(Value::as_str)
            == Some(resume_id);
        let cwd_matches = value
            .get("cwd")
            .or_else(|| value.pointer("/session/cwd"))
            .and_then(Value::as_str)
            .is_some_and(|raw_cwd| raw_cwd == cwd.to_string_lossy());
        id_matches && cwd_matches
    })
}

fn collect_codex_resume_logs(dir: &Path, out: &mut Vec<(f64, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_codex_resume_logs(&path, out);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        if !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("rollout-"))
        {
            continue;
        }
        let mtime = path
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0);
        out.push((mtime, path));
    }
}

fn codex_session_meta_payload(path: &Path) -> Option<Map<String, Value>> {
    let raw = fs::read(path).ok()?;
    for line in raw.split(|byte| *byte == b'\n') {
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let value = serde_json::from_slice::<Value>(line).ok()?;
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        return value.get("payload")?.as_object().cloned();
    }
    None
}

fn is_codex_subagent_meta(payload: &Map<String, Value>) -> bool {
    payload
        .get("source")
        .and_then(Value::as_object)
        .is_some_and(|source| source.contains_key("subagent"))
}

fn first_user_message_preview_from_codex_log(path: &Path) -> Option<String> {
    let raw = fs::read(path).ok()?;
    for line in raw.split(|byte| *byte == b'\n') {
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let value = serde_json::from_slice::<Value>(line).ok()?;
        let Some(message) = codex_user_message_payload(&value) else {
            continue;
        };
        let text = user_message_text(message);
        let text = text.trim();
        if text.is_empty() || is_scaffold_user_text(text) {
            continue;
        }
        return Some(resume_preview_from_text(text));
    }
    None
}

fn codex_user_message_payload(value: &Value) -> Option<&Map<String, Value>> {
    let message = if value.get("type").and_then(Value::as_str) == Some("response_item") {
        value.get("payload")?.as_object()?
    } else if value.get("type").and_then(Value::as_str) == Some("message") {
        value.as_object()?
    } else {
        return None;
    };
    (message
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("message")
        == "message"
        && message.get("role").and_then(Value::as_str) == Some("user"))
    .then_some(message)
}

fn user_message_text(payload: &Map<String, Value>) -> String {
    match payload.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .or_else(|| part.get("content"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn is_scaffold_user_text(text: &str) -> bool {
    text.starts_with("# AGENTS.md instructions for ") || text.starts_with("<environment_context>")
}

fn resume_preview_from_text(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(120).collect()
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
