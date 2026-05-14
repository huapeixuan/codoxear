use crate::app_state::AppState;
use crate::launch_defaults::{read_codex_launch_defaults, read_pi_launch_defaults};
use crate::routes::json_response;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

const SUPPORTED_REASONING_EFFORTS: &[&str] = &["xhigh", "high", "medium", "low"];
const SUPPORTED_PI_REASONING_EFFORTS: &[&str] =
    &["off", "minimal", "low", "medium", "high", "xhigh"];

#[derive(Debug, PartialEq)]
pub struct CreateSessionRequest {
    pub cwd: String,
    pub backend: String,
    pub args: Option<Vec<String>>,
    pub resume_session_id: Option<String>,
    pub worktree_branch: Option<String>,
    pub model_provider: Option<String>,
    pub auth_method: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
    pub create_in_tmux: bool,
}

pub(crate) async fn session_create(
    State(_state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    match parse_create_session_request(&payload) {
        Ok(request) => json_response(
            StatusCode::NOT_IMPLEMENTED,
            json!({
                "error": "session create is not implemented in Rust Phase 3 yet",
                "ok": false,
                "backend": request.backend,
                "phase": "phase3",
            }),
        ),
        Err(message) => {
            let mut body = json!({"error": message});
            if body["error"] == "cwd required" {
                body["field"] = json!("cwd");
            }
            json_response(StatusCode::BAD_REQUEST, body)
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
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "cwd required".to_string())?
        .to_string();
    let backend = normalize_agent_backend(
        object.get("backend"),
        normalize_agent_backend(object.get("agent_backend"), "codex".to_string())?,
    )?;
    let args = clean_args(object.get("args"))?;
    let resume_session_id = clean_optional_resume_session_id(object.get("resume_session_id"))?;

    if backend == "pi" {
        let create_in_tmux = clean_create_in_tmux(object.get("create_in_tmux"))?;
        let allowed = pi_provider_choices();
        return Ok(CreateSessionRequest {
            cwd,
            backend,
            args,
            resume_session_id,
            worktree_branch: None,
            model_provider: normalize_model_provider(
                object.get("model_provider"),
                allowed.as_ref(),
            )?,
            auth_method: None,
            model: normalize_model(object.get("model")),
            reasoning_effort: normalize_reasoning_effort(
                object.get("reasoning_effort"),
                SUPPORTED_PI_REASONING_EFFORTS,
            )?,
            service_tier: None,
            create_in_tmux,
        });
    }

    let allowed = codex_allowed_providers();
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
        model_provider: normalize_model_provider(object.get("model_provider"), Some(&allowed))?,
        auth_method: normalize_auth_method(object.get("preferred_auth_method"))?,
        model: normalize_model(object.get("model")),
        reasoning_effort: normalize_reasoning_effort(
            object.get("reasoning_effort"),
            SUPPORTED_REASONING_EFFORTS,
        )?,
        service_tier: normalize_service_tier(object.get("service_tier"))?,
        create_in_tmux: clean_create_in_tmux(object.get("create_in_tmux"))?,
    })
}

fn clean_args(value: Option<&Value>) -> Result<Option<Vec<String>>, String> {
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

fn clean_optional_resume_session_id(value: Option<&Value>) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(clean_optional_text(value)),
        Some(_) => Err("resume_session_id must be a string".to_string()),
    }
}

fn clean_create_in_tmux(value: Option<&Value>) -> Result<bool, String> {
    match value {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err("create_in_tmux must be a boolean".to_string()),
    }
}

fn normalize_agent_backend(value: Option<&Value>, default: String) -> Result<String, String> {
    match value {
        None | Some(Value::Null) => Ok(default),
        Some(Value::String(value)) => {
            let out = value.trim().to_ascii_lowercase();
            if out.is_empty() {
                return Ok(default);
            }
            if !matches!(out.as_str(), "codex" | "pi") {
                return Err("backend must be one of codex, pi".to_string());
            }
            Ok(out)
        }
        Some(_) => Err("backend must be a string".to_string()),
    }
}

fn normalize_model(value: Option<&Value>) -> Option<String> {
    let out = value
        .and_then(Value::as_str)
        .and_then(clean_optional_text)?;
    (!out.eq_ignore_ascii_case("default")).then_some(out)
}

fn normalize_reasoning_effort(
    value: Option<&Value>,
    allowed: &[&str],
) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let out = value.trim().to_ascii_lowercase();
            if out.is_empty() {
                return Ok(None);
            }
            if !allowed.iter().any(|candidate| *candidate == out) {
                return Err(format!(
                    "reasoning_effort must be one of {}",
                    allowed.join(", ")
                ));
            }
            Ok(Some(out))
        }
        Some(_) => Err("reasoning_effort must be a string".to_string()),
    }
}

fn normalize_model_provider(
    value: Option<&Value>,
    allowed: Option<&Vec<String>>,
) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let Some(provider) = clean_optional_text(value) else {
                return Ok(None);
            };
            if let Some(allowed) = allowed {
                if !allowed.iter().any(|candidate| candidate == &provider) {
                    return Err(format!(
                        "model_provider must be one of {}",
                        allowed.join(", ")
                    ));
                }
            }
            Ok(Some(provider))
        }
        Some(_) => Err("model_provider must be a string".to_string()),
    }
}

fn normalize_auth_method(value: Option<&Value>) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let Some(method) = clean_optional_text(value) else {
                return Ok(None);
            };
            if !matches!(method.as_str(), "chatgpt" | "apikey") {
                return Err("preferred_auth_method must be one of chatgpt, apikey".to_string());
            }
            Ok(Some(method))
        }
        Some(_) => Err("preferred_auth_method must be a string".to_string()),
    }
}

fn normalize_service_tier(value: Option<&Value>) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let Some(tier) = clean_optional_text(value) else {
                return Ok(None);
            };
            if !matches!(tier.as_str(), "fast" | "flex") {
                return Err("service_tier must be one of fast, flex".to_string());
            }
            Ok(Some(tier))
        }
        Some(_) => Err("service_tier must be a string".to_string()),
    }
}

fn clean_optional_text(value: &str) -> Option<String> {
    let out = value.trim();
    (!out.is_empty()).then(|| out.to_string())
}

fn codex_allowed_providers() -> Vec<String> {
    let mut allowed = vec!["openai".to_string()];
    if let Ok(defaults) = read_codex_launch_defaults() {
        if let Some(items) = defaults.get("model_providers").and_then(Value::as_array) {
            for item in items {
                let Some(provider) = item.as_str().and_then(clean_optional_text) else {
                    continue;
                };
                if !matches!(provider.as_str(), "chatgpt" | "openai-api")
                    && !allowed.iter().any(|candidate| candidate == &provider)
                {
                    allowed.push(provider);
                }
            }
        }
    }
    allowed
}

fn pi_provider_choices() -> Option<Vec<String>> {
    let defaults = read_pi_launch_defaults().ok()?;
    let providers = defaults.get("provider_choices")?.as_array()?;
    let out = providers
        .iter()
        .filter_map(|item| item.as_str().and_then(clean_optional_text))
        .collect::<Vec<_>>();
    (!out.is_empty()).then_some(out)
}

pub fn resolve_create_cwd(raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw).expand_tilde();
    if path.exists() {
        if !path.is_dir() {
            return Err(format!("cwd is not a directory: {}", path.display()));
        }
        return fs::canonicalize(&path).map_err(|err| format!("cwd: {err}"));
    }
    fs::create_dir_all(&path)
        .map_err(|err| format!("cwd could not be created: {}: {err}", path.display()))?;
    fs::canonicalize(&path).map_err(|err| format!("cwd: {err}"))
}

trait ExpandTilde {
    fn expand_tilde(self) -> PathBuf;
}

impl ExpandTilde for PathBuf {
    fn expand_tilde(self) -> PathBuf {
        let Some(raw) = self.to_str() else {
            return self;
        };
        if raw == "~" {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home);
            }
        }
        if let Some(rest) = raw.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home).join(rest);
            }
        }
        self
    }
}
