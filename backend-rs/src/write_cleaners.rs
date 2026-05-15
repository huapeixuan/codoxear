use axum::http::StatusCode;
use serde_json::{json, Value};
use std::path::Path;

pub fn normalize_pi_image_inputs(value: &Value) -> Result<Vec<Value>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let Some(items) = value.as_array() else {
        return Err("images must be a list".to_string());
    };
    let mut out = Vec::new();
    for item in items {
        let Some(obj) = item.as_object() else {
            return Err("images must contain objects".to_string());
        };
        let file_name = obj
            .get("file_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("image file_name required".to_string())?;
        let mime_type = obj
            .get("mime_type")
            .and_then(Value::as_str)
            .filter(|s| s.starts_with("image/"))
            .ok_or("image mime_type must start with image/".to_string())?;
        let data_b64 = obj
            .get("data_b64")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("image data_b64 required".to_string())?;
        out.push(json!({"file_name": file_name, "mime_type": mime_type, "data_b64": data_b64}));
    }
    Ok(out)
}

pub fn clean_queue_items(items: Vec<Value>) -> Vec<Value> {
    items
        .into_iter()
        .filter_map(|item| {
            let text = match &item {
                Value::String(s) => s.trim().to_string(),
                Value::Object(o) => o.get("text")?.as_str()?.trim().to_string(),
                _ => return None,
            };
            if text.is_empty() {
                return None;
            }
            let images = item
                .get("images")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            Some(if images.is_empty() {
                json!(text)
            } else {
                json!({"text": text, "images": images})
            })
        })
        .collect()
}

pub fn clean_priority_offset(value: Option<&Value>) -> Result<f64, (StatusCode, String)> {
    let Some(value) = value else {
        return Ok(0.0);
    };
    if value.is_null() {
        return Ok(0.0);
    }
    let Some(out) = value.as_f64() else {
        return Err((
            StatusCode::BAD_REQUEST,
            "priority_offset must be a number".to_string(),
        ));
    };
    if !out.is_finite() {
        return Err((
            StatusCode::BAD_REQUEST,
            "priority_offset must be finite".to_string(),
        ));
    }
    if !(-1.0..=1.0).contains(&out) {
        return Err((
            StatusCode::BAD_REQUEST,
            "priority_offset must be within [-1, 1]".to_string(),
        ));
    }
    Ok(out)
}

pub fn clean_snooze_until(value: Option<&Value>) -> Result<Option<f64>, (StatusCode, String)> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() || value == "" || value == 0 {
        return Ok(None);
    }
    let Some(out) = value.as_f64() else {
        return Err((
            StatusCode::BAD_REQUEST,
            "snooze_until must be a unix timestamp or null".to_string(),
        ));
    };
    if !out.is_finite() {
        return Err((
            StatusCode::BAD_REQUEST,
            "snooze_until must be finite".to_string(),
        ));
    }
    if out <= 0.0 {
        return Ok(None);
    }
    Ok(Some(out))
}

pub fn clean_dependency_session_id(
    value: Option<&Value>,
) -> Result<Option<String>, (StatusCode, String)> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(raw) = value.as_str() else {
        return Err((
            StatusCode::BAD_REQUEST,
            "dependency_session_id must be a string or null".to_string(),
        ));
    };
    let trimmed = raw.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
}

pub fn clean_harness_cooldown(value: &Value) -> Result<i64, (StatusCode, String)> {
    if value.is_null() {
        return Ok(5);
    }
    let Some(out) = value.as_i64() else {
        return Err((
            StatusCode::BAD_REQUEST,
            "harness cooldown_minutes must be an integer".to_string(),
        ));
    };
    if out < 1 {
        return Err((
            StatusCode::BAD_REQUEST,
            "harness cooldown_minutes must be at least 1".to_string(),
        ));
    }
    Ok(out)
}

pub fn clean_harness_remaining(value: &Value) -> Result<i64, (StatusCode, String)> {
    if value.is_null() {
        return Ok(10);
    }
    let Some(out) = value.as_i64() else {
        return Err((
            StatusCode::BAD_REQUEST,
            "harness remaining_injections must be an integer".to_string(),
        ));
    };
    if out < 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "harness remaining_injections must be at least 0".to_string(),
        ));
    }
    Ok(out)
}

pub fn clean_positive_number(value: &Value) -> Option<f64> {
    let out = value.as_f64()?;
    (out.is_finite() && out > 0.0).then_some(out)
}

pub fn clean_hidden_after_live_start_ts(value: &Value) -> Option<f64> {
    if value.is_null() || value.is_boolean() {
        return None;
    }
    clean_positive_number(value)
}

pub fn clean_safe_filename(name: &str, default: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(default);
    let cleaned = base
        .chars()
        .filter(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' '))
        .collect::<String>()
        .trim()
        .replace(' ', "_");
    let cleaned = cleaned.trim_matches(['.', '-']).to_string();
    if cleaned.is_empty() {
        default.to_string()
    } else {
        cleaned.chars().take(96).collect()
    }
}

pub fn attachment_inject_text(index: i64, path: &Path) -> Result<String, String> {
    if index <= 0 {
        return Err("attachment_index must be >= 1".to_string());
    }
    Ok(format!("Attachment {index}: {}\n", path.display()))
}

pub fn legacy_ui_response_text(payload: &Value) -> Option<String> {
    if payload.get("cancelled").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    if let Some(confirmed) = payload.get("confirmed").and_then(Value::as_bool) {
        return Some(if confirmed { "yes" } else { "no" }.to_string());
    }
    if let Some(value) = payload.get("value") {
        if let Some(text) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return Some(text.to_string());
        }
        if let Some(items) = value.as_array() {
            let parts = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            if !parts.is_empty() {
                return Some(parts.join(", "));
            }
        }
    }
    None
}
