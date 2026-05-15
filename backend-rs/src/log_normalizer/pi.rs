use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

pub const CONTEXT_WINDOW_BASELINE_TOKENS: i64 = 12_000;

pub fn pi_session_header(path: &Path) -> Result<Option<Value>, String> {
    let Some(obj) = read_jsonl_first_object(path)? else {
        return Ok(None);
    };
    Ok((obj.get("type").and_then(Value::as_str) == Some("session")).then_some(obj))
}

pub fn pi_session_id(path: &Path) -> Result<Option<String>, String> {
    Ok(pi_session_header(path)?.and_then(|obj| clean_string(obj.get("id"))))
}

pub fn pi_log_cwd(path: &Path) -> Result<Option<String>, String> {
    Ok(pi_session_header(path)?.and_then(|obj| clean_string(obj.get("cwd"))))
}

pub fn pi_user_text(obj: &Value) -> Option<String> {
    let message = obj.get("message")?.as_object()?;
    (message.get("role")?.as_str()? == "user").then_some(())?;
    let parts = text_parts(message.get("content")?);
    (!parts.is_empty()).then(|| parts.join(""))
}

pub fn pi_assistant_content_parts(obj: &Value) -> Vec<&Value> {
    let Some(message) = obj.get("message").and_then(Value::as_object) else {
        return Vec::new();
    };
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Vec::new();
    }
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter(|item| item.is_object()).collect())
        .unwrap_or_default()
}

pub fn pi_assistant_text(obj: &Value) -> Option<String> {
    let out = pi_assistant_content_parts(obj)
        .into_iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| clean_string(part.get("text")))
        .collect::<Vec<_>>();
    (!out.is_empty()).then(|| out.join(""))
}

pub fn pi_final_turn(obj: &Value) -> bool {
    let Some(message) = obj.get("message").and_then(Value::as_object) else {
        return false;
    };
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return false;
    }
    if pi_assistant_text(obj).is_none() {
        return false;
    }

    if pi_assistant_tool_use_count(obj) == 0
        && pi_assistant_thinking_count(obj) == 0
        && message.get("stopReason").and_then(Value::as_str) != Some("toolUse")
    {
        return true;
    }

    let stop_reason = message.get("stopReason").and_then(Value::as_str);
    if stop_reason.is_some_and(|reason| !reason.is_empty() && reason != "toolUse") {
        return true;
    }

    pi_assistant_content_parts(obj)
        .into_iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("textSignature").and_then(Value::as_str))
        .filter_map(|raw| serde_json::from_str::<Value>(raw).ok())
        .any(|sig| sig.get("phase").and_then(Value::as_str) == Some("final_answer"))
}

pub fn pi_assistant_tool_use_count(obj: &Value) -> usize {
    pi_assistant_content_parts(obj)
        .into_iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("toolCall"))
        .count()
}

pub fn pi_assistant_thinking_count(obj: &Value) -> usize {
    pi_assistant_content_parts(obj)
        .into_iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("thinking"))
        .count()
}

pub fn pi_message_role(obj: &Value) -> Option<String> {
    clean_string(obj.get("message")?.get("role"))
}

pub fn pi_token_update(obj: &Value, models_path: Option<&Path>) -> Option<Value> {
    let message = obj.get("message")?.as_object()?;
    if message.get("role")?.as_str()? != "assistant" {
        return None;
    }
    let total_tokens = message.get("usage")?.get("totalTokens")?.as_i64()?;
    let context_window = pi_model_context_window(
        message.get("provider").and_then(Value::as_str),
        message.get("model").and_then(Value::as_str),
        models_path,
    )?;
    let as_of = obj
        .get("timestamp")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    Some(json!({
        "context_window": context_window,
        "tokens_in_context": total_tokens,
        "tokens_remaining": (context_window - total_tokens).max(0),
        "percent_remaining": context_percent_remaining(total_tokens, context_window),
        "baseline_tokens": CONTEXT_WINDOW_BASELINE_TOKENS,
        "as_of": as_of,
    }))
}

pub fn pi_run_settings(
    path: &Path,
    max_scan_bytes: usize,
) -> Result<crate::log_normalizer::RunSettings, String> {
    let mut provider = None;
    let mut model = None;
    let mut thinking_level = None;

    if let Some(header) = pi_session_header(path)? {
        provider = clean_string(header.get("provider"));
        model = clean_string(header.get("modelId"));
        thinking_level = clean_string(header.get("thinkingLevel"));
    }

    let size = fs::metadata(path)
        .map(|meta| meta.len() as usize)
        .unwrap_or_default();
    let start = size.saturating_sub(max_scan_bytes);
    let file = fs::File::open(path).map_err(|err| format!("open {}: {err}", path.display()))?;
    let mut offset = 0usize;
    for raw in BufReader::new(file).split(b'\n') {
        let raw = raw.map_err(|err| format!("read {}: {err}", path.display()))?;
        offset += raw.len() + 1;
        if offset < start || raw.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let Ok(obj) = serde_json::from_slice::<Value>(&raw) else {
            continue;
        };
        match obj.get("type").and_then(Value::as_str) {
            Some("model_change") => {
                if let Some(value) = clean_string(obj.get("provider")) {
                    provider = Some(value);
                }
                if let Some(value) = clean_string(obj.get("modelId")) {
                    model = Some(value);
                }
            }
            Some("thinking_level_change") => {
                if let Some(value) = clean_string(obj.get("thinkingLevel")) {
                    thinking_level = Some(value);
                }
            }
            _ => {}
        }
    }
    Ok((provider, model, thinking_level))
}

pub fn messages_from_pi_log(
    path: &Path,
    offset: usize,
    limit: usize,
    _init: bool,
    before: Option<usize>,
) -> Result<Value, String> {
    let objects = read_jsonl_objects(path)?;
    let events = crate::log_normalizer::codex::extract_chat_events(&objects)?.events;
    let end = before.unwrap_or(events.len()).min(events.len());
    let start = offset.min(end);
    let stop = start.saturating_add(limit).min(end);
    Ok(json!({
        "events": events[start..stop].to_vec(),
        "has_older": stop < end,
        "offset": start,
        "limit": limit,
    }))
}

fn pi_model_context_window(
    provider: Option<&str>,
    model: Option<&str>,
    models_path: Option<&Path>,
) -> Option<i64> {
    let provider = provider?.trim();
    let model = model?.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    let path = models_path
        .map(Path::to_path_buf)
        .unwrap_or_else(default_pi_models_path);
    let data = fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<Value>(&data).ok()?;
    let providers = value.get("providers")?.as_object()?;
    let rows = providers.get(provider)?.get("models")?.as_array()?;
    rows.iter().find_map(|row| {
        (row.get("id").and_then(Value::as_str) == Some(model))
            .then(|| row.get("contextWindow").and_then(Value::as_i64))
            .flatten()
            .filter(|value| *value > 0)
    })
}

fn default_pi_models_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".pi/agent/models.json")
}

fn context_percent_remaining(tokens_in_context: i64, context_window: i64) -> i64 {
    if context_window <= CONTEXT_WINDOW_BASELINE_TOKENS {
        return 0;
    }
    let effective = context_window - CONTEXT_WINDOW_BASELINE_TOKENS;
    let used = (tokens_in_context - CONTEXT_WINDOW_BASELINE_TOKENS).max(0);
    let remaining = (effective - used).max(0);
    ((remaining as f64 / effective as f64) * 100.0).round() as i64
}

fn text_parts(content: &Value) -> Vec<String> {
    if let Some(text) = content.as_str().and_then(clean_str) {
        return vec![text];
    }
    content
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|part| clean_string(part.get("text")))
                .collect()
        })
        .unwrap_or_default()
}

fn read_jsonl_first_object(path: &Path) -> Result<Option<Value>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("open {}: {err}", path.display())),
    };
    for raw in BufReader::new(file).split(b'\n') {
        let raw = raw.map_err(|err| format!("read {}: {err}", path.display()))?;
        if raw.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&raw) else {
            continue;
        };
        return Ok(value.is_object().then_some(value));
    }
    Ok(None)
}

fn read_jsonl_objects(path: &Path) -> Result<Vec<Value>, String> {
    let raw = fs::read(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    Ok(raw
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(|byte| byte.is_ascii_whitespace()))
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .filter(Value::is_object)
        .collect())
}

fn clean_string(value: Option<&Value>) -> Option<String> {
    value?.as_str().and_then(clean_str)
}

fn clean_str(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
