use serde_json::{json, Map, Value};
use std::env;
use std::fs;
use std::path::PathBuf;
use toml::Value as TomlValue;

const SUPPORTED_REASONING_EFFORTS: &[&str] = &["xhigh", "high", "medium", "low"];
const SUPPORTED_PI_REASONING_EFFORTS: &[&str] =
    &["off", "minimal", "low", "medium", "high", "xhigh"];

pub fn read_codex_launch_defaults() -> Result<Value, String> {
    let mut configured_model: Option<String> = None;
    let mut configured_effort: Option<String> = None;
    let mut configured_provider = "openai".to_string();
    let mut configured_auth_method = "apikey".to_string();
    let mut configured_service_tier = "flex".to_string();
    let mut configured_providers = vec!["chatgpt".to_string(), "openai-api".to_string()];

    let config_path = codex_config_path();
    if config_path.exists() {
        let data = fs::read_to_string(&config_path)
            .map_err(|err| format!("read {}: {err}", config_path.display()))?;
        let parsed: TomlValue = toml::from_str(&data)
            .map_err(|_| format!("invalid Codex config in {}", config_path.display()))?;
        let table = parsed
            .as_table()
            .ok_or_else(|| format!("invalid Codex config in {}", config_path.display()))?;
        configured_model = table.get("model").and_then(toml_string);
        configured_effort = table
            .get("model_reasoning_effort")
            .and_then(toml_string)
            .and_then(|value| display_reasoning_effort(&value));
        if let Some(method) = table
            .get("preferred_auth_method")
            .and_then(toml_string)
            .and_then(|value| normalize_requested_preferred_auth_method(&value).transpose())
            .transpose()?
        {
            configured_auth_method = method;
        }
        configured_providers = vec!["chatgpt".to_string(), "openai-api".to_string()];
        configured_providers.extend(
            configured_model_providers(table)
                .into_iter()
                .filter(|provider| provider != "openai"),
        );
        let allowed = allowed_model_providers(&configured_providers);
        if let Some(provider) = table
            .get("model_provider")
            .or_else(|| table.get("model_provider_id"))
            .and_then(toml_string)
            .and_then(|value| {
                normalize_requested_model_provider(&value, Some(&allowed)).transpose()
            })
            .transpose()?
        {
            configured_provider = provider;
        }
        if let Some(tier) = table
            .get("service_tier")
            .and_then(toml_string)
            .and_then(|value| normalize_requested_service_tier(&value).transpose())
            .transpose()?
        {
            configured_service_tier = tier;
        }
    }

    let mut defaults = Map::new();
    defaults.insert("model_provider".to_string(), json!(configured_provider));
    defaults.insert(
        "preferred_auth_method".to_string(),
        json!(configured_auth_method),
    );
    defaults.insert(
        "provider_choice".to_string(),
        json!(provider_choice_for_settings(
            defaults.get("model_provider").and_then(Value::as_str),
            defaults
                .get("preferred_auth_method")
                .and_then(Value::as_str)
        )),
    );
    defaults.insert("model".to_string(), option_string_value(&configured_model));
    defaults.insert("model_providers".to_string(), json!(configured_providers));
    defaults.insert("service_tier".to_string(), json!(configured_service_tier));

    if let Some(effort) = configured_effort {
        defaults.insert("reasoning_effort".to_string(), json!(effort));
        return Ok(Value::Object(defaults));
    }

    let cache_path = models_cache_path();
    if !cache_path.exists() {
        defaults.insert("reasoning_effort".to_string(), Value::Null);
        return Ok(Value::Object(defaults));
    }
    let cache = read_json_file(&cache_path)?;
    let models = cache
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("invalid models cache in {}", cache_path.display()))?;
    let rows = models
        .iter()
        .filter_map(Value::as_object)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        defaults.insert("reasoning_effort".to_string(), Value::Null);
        return Ok(Value::Object(defaults));
    }
    if let Some(model) = configured_model.as_deref() {
        for row in &rows {
            let names = [
                row.get("slug")
                    .and_then(Value::as_str)
                    .and_then(clean_optional_text),
                row.get("display_name")
                    .and_then(Value::as_str)
                    .and_then(clean_optional_text),
            ];
            if names
                .iter()
                .any(|candidate| candidate.as_deref() == Some(model))
            {
                defaults.insert(
                    "reasoning_effort".to_string(),
                    option_string_value(
                        &row.get("default_reasoning_level")
                            .and_then(Value::as_str)
                            .and_then(display_reasoning_effort),
                    ),
                );
                return Ok(Value::Object(defaults));
            }
        }
    }
    let ranked = rows.into_iter().min_by(|left, right| {
        let left_priority = left
            .get("priority")
            .and_then(Value::as_i64)
            .unwrap_or(999_999);
        let right_priority = right
            .get("priority")
            .and_then(Value::as_i64)
            .unwrap_or(999_999);
        left_priority.cmp(&right_priority).then_with(|| {
            left.get("slug")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .cmp(
                    right
                        .get("slug")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
        })
    });
    let effort = ranked
        .and_then(|row| row.get("default_reasoning_level"))
        .and_then(Value::as_str)
        .and_then(display_reasoning_effort);
    defaults.insert("reasoning_effort".to_string(), option_string_value(&effort));
    Ok(Value::Object(defaults))
}

pub fn read_pi_launch_defaults() -> Result<Value, String> {
    let mut configured_provider: Option<String> = None;
    let mut configured_model: Option<String> = None;
    let configured_effort = "high".to_string();
    let mut provider_choices: Vec<String> = Vec::new();
    let mut provider_models: Map<String, Value> = Map::new();

    let settings_path = pi_settings_path();
    if settings_path.exists() {
        let data = read_json_file(&settings_path)?;
        let object = data
            .as_object()
            .ok_or_else(|| format!("invalid Pi settings in {}", settings_path.display()))?;
        configured_provider = object
            .get("defaultProvider")
            .and_then(Value::as_str)
            .and_then(clean_optional_text);
        configured_model = object
            .get("defaultModel")
            .and_then(Value::as_str)
            .and_then(clean_optional_text);
    }

    let models_path = pi_models_path();
    if models_path.exists() {
        let data = read_json_file(&models_path)?;
        let object = data
            .as_object()
            .ok_or_else(|| format!("invalid Pi models config in {}", models_path.display()))?;
        if let Some(providers) = object.get("providers").and_then(Value::as_object) {
            for (key, value) in providers {
                let name = key.trim();
                if name.is_empty() || provider_choices.iter().any(|item| item == name) {
                    continue;
                }
                provider_choices.push(name.to_string());
                let mut model_choices = Vec::new();
                if let Some(models) = value.get("models").and_then(Value::as_array) {
                    for row in models {
                        let Some(model_id) = row
                            .as_object()
                            .and_then(|obj| obj.get("id"))
                            .and_then(Value::as_str)
                            .and_then(clean_optional_text)
                        else {
                            continue;
                        };
                        if !model_choices.iter().any(|item| item == &model_id) {
                            model_choices.push(model_id);
                        }
                    }
                }
                provider_models.insert(name.to_string(), json!(model_choices));
            }
        }
    }

    let fallback_provider = provider_choices
        .iter()
        .find(|name| {
            provider_models
                .get(*name)
                .and_then(Value::as_array)
                .is_some_and(|models| !models.is_empty())
        })
        .cloned()
        .or_else(|| provider_choices.first().cloned());
    let selected_provider = match configured_provider.as_deref() {
        Some(provider) if provider_choices.iter().any(|item| item == provider) => {
            Some(provider.to_string())
        }
        _ => fallback_provider,
    };
    let selected_models = selected_provider
        .as_deref()
        .and_then(|provider| provider_models.get(provider))
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let selected_model = match configured_model.as_deref() {
        Some(model) if selected_models.iter().any(|item| item == model) => Some(model.to_string()),
        _ => selected_models.first().cloned(),
    };

    Ok(json!({
        "agent_backend": "pi",
        "model_provider": selected_provider,
        "preferred_auth_method": Value::Null,
        "provider_choice": selected_provider,
        "provider_choices": provider_choices,
        "model": selected_model,
        "models": selected_models,
        "provider_models": provider_models,
        "reasoning_effort": configured_effort,
        "reasoning_efforts": SUPPORTED_PI_REASONING_EFFORTS,
        "service_tier": Value::Null,
        "supports_fast": false,
    }))
}

pub fn read_new_session_defaults() -> Result<Value, String> {
    let mut codex = read_codex_launch_defaults()?;
    if let Value::Object(ref mut object) = codex {
        object.insert("agent_backend".to_string(), json!("codex"));
        let provider_choices = object
            .get("model_providers")
            .cloned()
            .unwrap_or_else(|| json!([]));
        object.insert("provider_choices".to_string(), provider_choices);
        object.insert(
            "reasoning_efforts".to_string(),
            json!(SUPPORTED_REASONING_EFFORTS),
        );
        object.insert("supports_fast".to_string(), json!(true));
    }
    let pi = read_pi_launch_defaults()?;
    Ok(json!({
        "default_backend": normalize_agent_backend(env::var("CODEX_WEB_DEFAULT_AGENT_BACKEND").ok().as_deref(), "codex")?,
        "backends": {
            "codex": codex,
            "pi": pi,
        },
    }))
}

fn read_json_file(path: &std::path::Path) -> Result<Value, String> {
    let raw = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("parse {}: {err}", path.display()))
}

fn clean_optional_text(value: &str) -> Option<String> {
    let out = value.trim();
    (!out.is_empty()).then(|| out.to_string())
}

fn display_reasoning_effort(value: &str) -> Option<String> {
    let lowered = value.trim().to_ascii_lowercase();
    SUPPORTED_REASONING_EFFORTS
        .iter()
        .find(|candidate| **candidate == lowered)
        .map(|candidate| (*candidate).to_string())
}

fn normalize_requested_model_provider(
    value: &str,
    allowed: Option<&Vec<String>>,
) -> Result<Option<String>, String> {
    let Some(provider) = clean_optional_text(value) else {
        return Ok(None);
    };
    if let Some(allowed) = allowed {
        if !allowed.iter().any(|item| item == &provider) {
            return Err(format!(
                "model_provider must be one of {}",
                allowed.join(", ")
            ));
        }
    }
    Ok(Some(provider))
}

fn normalize_requested_service_tier(value: &str) -> Result<Option<String>, String> {
    let Some(tier) = clean_optional_text(value) else {
        return Ok(None);
    };
    if !matches!(tier.as_str(), "fast" | "flex") {
        return Err("service_tier must be one of fast, flex".to_string());
    }
    Ok(Some(tier))
}

fn normalize_requested_preferred_auth_method(value: &str) -> Result<Option<String>, String> {
    let Some(method) = clean_optional_text(value) else {
        return Ok(None);
    };
    if !matches!(method.as_str(), "chatgpt" | "apikey") {
        return Err("preferred_auth_method must be one of chatgpt, apikey".to_string());
    }
    Ok(Some(method))
}

fn configured_model_providers(table: &toml::map::Map<String, TomlValue>) -> Vec<String> {
    let mut providers = vec!["openai".to_string()];
    let Some(raw) = table.get("model_providers").and_then(TomlValue::as_table) else {
        return providers;
    };
    for key in raw.keys() {
        let name = key.trim();
        if !name.is_empty() && !providers.iter().any(|item| item == name) {
            providers.push(name.to_string());
        }
    }
    providers
}

fn allowed_model_providers(configured_providers: &[String]) -> Vec<String> {
    let mut allowed = vec!["openai".to_string()];
    allowed.extend(
        configured_providers
            .iter()
            .filter(|provider| !matches!(provider.as_str(), "chatgpt" | "openai-api"))
            .cloned(),
    );
    allowed
}

fn provider_choice_for_settings(
    model_provider: Option<&str>,
    preferred_auth_method: Option<&str>,
) -> String {
    match (model_provider.unwrap_or("openai"), preferred_auth_method) {
        ("openai", Some("chatgpt")) => "chatgpt".to_string(),
        ("openai", _) => "openai-api".to_string(),
        (provider, _) => provider.to_string(),
    }
}
fn toml_string(value: &TomlValue) -> Option<String> {
    value.as_str().and_then(clean_optional_text)
}
fn option_string_value(value: &Option<String>) -> Value {
    value.as_ref().map_or(Value::Null, |value| json!(value))
}
fn codex_home() -> PathBuf {
    env::var("CODEX_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".codex"))
}
fn pi_home() -> PathBuf {
    env::var("PI_HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".pi"))
}

fn home_dir() -> PathBuf {
    env::var("HOME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn codex_config_path() -> PathBuf {
    codex_home().join("config.toml")
}

fn models_cache_path() -> PathBuf {
    codex_home().join("models_cache.json")
}

fn pi_settings_path() -> PathBuf {
    pi_home().join("agent").join("settings.json")
}

fn pi_models_path() -> PathBuf {
    pi_home().join("agent").join("models.json")
}

fn normalize_agent_backend(raw: Option<&str>, default: &str) -> Result<String, String> {
    let value = raw.unwrap_or(default).trim().to_ascii_lowercase();
    let value = if value.is_empty() {
        default.to_string()
    } else {
        value
    };
    if matches!(value.as_str(), "codex" | "pi") {
        Ok(value)
    } else {
        Err("agent_backend must be one of codex, pi".to_string())
    }
}
