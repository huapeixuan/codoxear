use crate::app_state::AppState;
use crate::routes::json_response;
use crate::runtime::tmux_available;
use crate::session_create_support::{
    base_spawn_env, clean_optional_resume_session_id, clean_optional_text,
    clean_optional_text_value, codex_home, codex_trust_override_for_path, create_git_worktree,
    find_pi_resume_session_file, internal_error, normalize_agent_backend,
    normalize_preferred_auth_method, normalize_reasoning_effort, normalize_requested_model,
    normalize_service_tier, parse_args, parse_optional_bool, pi_home, pi_new_session_file_for_cwd,
    python_exe, repo_root, resolve_dir_target, spawn_nonce, spawn_python_broker,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const SUPPORTED_REASONING_EFFORTS: &[&str] = &["xhigh", "high", "medium", "low"];
const SUPPORTED_PI_REASONING_EFFORTS: &[&str] =
    &["off", "minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSessionRequest {
    pub cwd: String,
    pub backend: String,
    pub args: Option<Vec<String>>,
    pub resume_session_id: Option<String>,
    pub worktree_branch: Option<String>,
    pub model_provider: Option<String>,
    pub preferred_auth_method: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
    pub create_in_tmux: bool,
}

pub(crate) async fn session_create(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    let request = match parse_create_session_request(&payload) {
        Ok(request) => request,
        Err(message) => {
            let mut out = serde_json::Map::new();
            if message == "cwd required" {
                out.insert("field".to_string(), json!("cwd"));
            }
            out.insert("error".to_string(), json!(message));
            return json_response(StatusCode::BAD_REQUEST, Value::Object(out));
        }
    };
    match spawn_web_session(&state, &request) {
        Ok(value) => json_response(StatusCode::OK, value),
        Err((status, message)) => {
            let mut out = serde_json::Map::new();
            if message.starts_with("cwd ") {
                out.insert("field".to_string(), json!("cwd"));
            }
            out.insert("error".to_string(), json!(message));
            json_response(status, Value::Object(out))
        }
    }
}

pub fn parse_create_session_request(value: &Value) -> Result<CreateSessionRequest, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "invalid json body (expected object)".to_string())?;
    let cwd = object
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| "cwd required".to_string())?
        .to_string();
    let backend = normalize_agent_backend(
        object.get("backend"),
        normalize_agent_backend(object.get("agent_backend"), "codex")?.as_str(),
    )?;
    let args = parse_args(object.get("args"))?;
    let resume_session_id = clean_optional_resume_session_id(object.get("resume_session_id"))?;
    let create_in_tmux = parse_optional_bool(object.get("create_in_tmux"), "create_in_tmux")?;
    let model = normalize_requested_model(object.get("model"))?;

    if backend == "pi" {
        return Ok(CreateSessionRequest {
            cwd,
            backend,
            args,
            resume_session_id,
            worktree_branch: None,
            model_provider: clean_optional_text_value(object.get("model_provider"))?,
            preferred_auth_method: None,
            model,
            reasoning_effort: normalize_reasoning_effort(
                object.get("reasoning_effort"),
                SUPPORTED_PI_REASONING_EFFORTS,
            )?,
            service_tier: None,
            create_in_tmux,
        });
    }

    let worktree_branch = match object.get("worktree_branch") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => clean_optional_text(value),
        Some(_) => return Err("worktree_branch must be a string".to_string()),
    };

    Ok(CreateSessionRequest {
        cwd,
        backend,
        args,
        resume_session_id,
        worktree_branch,
        model_provider: clean_optional_text_value(object.get("model_provider"))?,
        preferred_auth_method: normalize_preferred_auth_method(
            object.get("preferred_auth_method"),
        )?,
        model,
        reasoning_effort: normalize_reasoning_effort(
            object.get("reasoning_effort"),
            SUPPORTED_REASONING_EFFORTS,
        )?,
        service_tier: normalize_service_tier(object.get("service_tier"))?,
        create_in_tmux,
    })
}

fn spawn_web_session(
    state: &AppState,
    request: &CreateSessionRequest,
) -> Result<Value, (StatusCode, String)> {
    let cwd_path = resolve_dir_target(&request.cwd, "cwd")?;
    if !cwd_path.exists() {
        fs::create_dir_all(&cwd_path).map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                format!("cwd could not be created: {}: {err}", cwd_path.display()),
            )
        })?;
    }
    if !cwd_path.is_dir() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("cwd is not a directory: {}", cwd_path.display()),
        ));
    }
    if request.create_in_tmux && !tmux_available() {
        return Err((
            StatusCode::BAD_REQUEST,
            "tmux is unavailable on this host".to_string(),
        ));
    }
    let spawn_nonce = spawn_nonce();
    if request.backend == "pi" {
        return spawn_pi_session(state, request, &cwd_path, &spawn_nonce);
    }
    spawn_codex_session(state, request, &cwd_path, &spawn_nonce)
}

fn spawn_pi_session(
    state: &AppState,
    request: &CreateSessionRequest,
    cwd_path: &Path,
    spawn_nonce: &str,
) -> Result<Value, (StatusCode, String)> {
    let session_file = if let Some(resume_id) = request.resume_session_id.as_deref() {
        let path = find_pi_resume_session_file(cwd_path, resume_id).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                format!("resume session not found for cwd: {resume_id}"),
            )
        })?;
        path
    } else {
        pi_new_session_file_for_cwd(cwd_path)
    };
    if let Some(parent) = session_file.parent() {
        fs::create_dir_all(parent).map_err(internal_error)?;
    }
    let extension = repo_root().join("codoxear/pi_extensions/ask_user_bridge.ts");
    let argv = vec![
        python_exe(),
        "-m".to_string(),
        "codoxear.pi_broker".to_string(),
        "--cwd".to_string(),
        cwd_path.to_string_lossy().to_string(),
        "--session-file".to_string(),
        session_file.to_string_lossy().to_string(),
        "--".to_string(),
        "-e".to_string(),
        extension.to_string_lossy().to_string(),
    ];
    let mut envs = base_spawn_env("pi", spawn_nonce);
    envs.push((
        "PI_HOME".to_string(),
        pi_home().to_string_lossy().to_string(),
    ));
    spawn_python_broker(state, request, cwd_path, spawn_nonce, argv, envs)
}

fn spawn_codex_session(
    state: &AppState,
    request: &CreateSessionRequest,
    cwd_path: &Path,
    spawn_nonce: &str,
) -> Result<Value, (StatusCode, String)> {
    if request.resume_session_id.is_some() && request.worktree_branch.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "worktree_branch cannot be used when resuming a session".to_string(),
        ));
    }
    let spawn_cwd = if let Some(branch) = request.worktree_branch.as_deref() {
        create_git_worktree(cwd_path, branch)?
    } else {
        cwd_path.to_path_buf()
    };
    if let Some(resume_id) = request.resume_session_id.as_deref() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("resume session not found for cwd: {resume_id}"),
        ));
    }

    let mut argv = vec![
        python_exe(),
        "-m".to_string(),
        "codoxear.broker".to_string(),
        "--cwd".to_string(),
        spawn_cwd.to_string_lossy().to_string(),
        "--".to_string(),
        "-c".to_string(),
        codex_trust_override_for_path(&spawn_cwd),
        "--dangerously-bypass-approvals-and-sandbox".to_string(),
    ];
    if let Some(model) = &request.model {
        argv.extend(["--model".to_string(), model.clone()]);
    }
    if let Some(reasoning) = &request.reasoning_effort {
        argv.extend([
            "-c".to_string(),
            format!("model_reasoning_effort=\"{reasoning}\""),
        ]);
    }
    if let Some(provider) = &request.model_provider {
        argv.extend(["-c".to_string(), format!("model_provider=\"{provider}\"")]);
    }
    if let Some(method) = &request.preferred_auth_method {
        argv.extend([
            "-c".to_string(),
            format!("preferred_auth_method=\"{method}\""),
        ]);
    }
    if let Some(tier) = &request.service_tier {
        argv.extend(["-c".to_string(), format!("service_tier=\"{tier}\"")]);
    }
    argv.extend(request.args.clone().unwrap_or_default());
    let mut envs = base_spawn_env("codex", spawn_nonce);
    envs.push((
        "CODEX_HOME".to_string(),
        codex_home().to_string_lossy().to_string(),
    ));
    if let Some(provider) = &request.model_provider {
        envs.push(("CODEX_WEB_MODEL_PROVIDER".to_string(), provider.clone()));
    }
    if let Some(method) = &request.preferred_auth_method {
        envs.push((
            "CODEX_WEB_PREFERRED_AUTH_METHOD".to_string(),
            method.clone(),
        ));
    }
    if let Some(model) = &request.model {
        envs.push(("CODEX_WEB_MODEL".to_string(), model.clone()));
    }
    if let Some(reasoning) = &request.reasoning_effort {
        envs.push(("CODEX_WEB_REASONING_EFFORT".to_string(), reasoning.clone()));
    }
    if let Some(tier) = &request.service_tier {
        envs.push(("CODEX_WEB_SERVICE_TIER".to_string(), tier.clone()));
    }
    spawn_python_broker(state, request, &spawn_cwd, spawn_nonce, argv, envs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codex_create_request_matches_python_validation() {
        let parsed = parse_create_session_request(&json!({
            "cwd": "/tmp/project",
            "backend": "codex",
            "args": ["--foo", ""],
            "resume_session_id": "  ",
            "worktree_branch": " feature/x ",
            "model_provider": "openai",
            "preferred_auth_method": "chatgpt",
            "model": "default",
            "reasoning_effort": "HIGH",
            "service_tier": "flex",
            "create_in_tmux": false
        }))
        .unwrap();
        assert_eq!(parsed.args, Some(vec!["--foo".to_string()]));
        assert_eq!(parsed.resume_session_id, None);
        assert_eq!(parsed.worktree_branch, Some("feature/x".to_string()));
        assert_eq!(parsed.model, None);
        assert_eq!(parsed.reasoning_effort, Some("high".to_string()));
        assert_eq!(parsed.preferred_auth_method, Some("chatgpt".to_string()));
    }

    #[test]
    fn parse_pi_create_ignores_worktree_and_rejects_bad_reasoning() {
        let parsed = parse_create_session_request(&json!({
            "cwd": "/tmp/project",
            "backend": "pi",
            "worktree_branch": "ignored",
            "reasoning_effort": "minimal"
        }))
        .unwrap();
        assert_eq!(parsed.worktree_branch, None);
        assert_eq!(parsed.reasoning_effort, Some("minimal".to_string()));
        assert_eq!(
            parse_create_session_request(
                &json!({"cwd": "/tmp", "backend": "pi", "reasoning_effort": "turbo"})
            )
            .unwrap_err(),
            "reasoning_effort must be one of off, minimal, low, medium, high, xhigh"
        );
    }

    #[test]
    fn worktree_slug_matches_python_shape() {
        assert_eq!(
            crate::session_create_support::worktree_path_slug("feature/api v2"),
            "feature-api-v2"
        );
        assert_eq!(
            crate::session_create_support::worktree_path_slug("!!!"),
            "worktree"
        );
    }
}
