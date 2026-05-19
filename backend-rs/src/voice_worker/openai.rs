use serde_json::{json, Value};
use std::time::Duration;

use isahc::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRequest {
    pub base_url: String,
    pub model: String,
    pub api_key_present: bool,
    pub session_name: String,
    pub source_label: String,
    pub target_words: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsRequest {
    pub base_url: String,
    pub model: String,
    pub voice: String,
    pub api_key_present: bool,
    pub response_format: &'static str,
}

pub trait OpenAiVoiceClient: Send + Sync {
    fn summarize(&self, request: SummaryRequest, text: &str) -> Result<String, String>;
    fn synthesize(&self, request: TtsRequest, text: &str) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Clone)]
pub struct DisabledOpenAiClient;

impl OpenAiVoiceClient for DisabledOpenAiClient {
    fn summarize(&self, _request: SummaryRequest, _text: &str) -> Result<String, String> {
        Err("OpenAI client not configured".to_string())
    }

    fn synthesize(&self, _request: TtsRequest, _text: &str) -> Result<Vec<u8>, String> {
        Err("OpenAI client not configured".to_string())
    }
}

#[derive(Debug, Clone)]
pub struct HttpOpenAiVoiceClient {
    api_key: String,
    timeout: Duration,
}

impl HttpOpenAiVoiceClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            timeout: default_timeout(),
        }
    }

    pub fn with_timeout(api_key: String, timeout: Duration) -> Self {
        Self { api_key, timeout }
    }
}

impl OpenAiVoiceClient for HttpOpenAiVoiceClient {
    fn summarize(&self, request: SummaryRequest, text: &str) -> Result<String, String> {
        if self.api_key.trim().is_empty() || !request.api_key_present {
            return Err("tts_api_key is required".to_string());
        }
        let payload = summary_payload(
            &request.model,
            text,
            request.target_words,
            &request.session_name,
            &request.source_label,
        );
        let body = self.post_json(&request.base_url, "/chat/completions", &payload)?;
        parse_summary_response(&body)
    }

    fn synthesize(&self, request: TtsRequest, text: &str) -> Result<Vec<u8>, String> {
        if self.api_key.trim().is_empty() || !request.api_key_present {
            return Err("tts_api_key is required".to_string());
        }
        let payload = json!({
            "model": request.model,
            "voice": request.voice,
            "input": text,
            "response_format": request.response_format,
        });
        let body = self.post_json(&request.base_url, "/audio/speech", &payload)?;
        if body.is_empty() {
            return Err("audio/speech returned empty body".to_string());
        }
        Ok(body)
    }
}

impl HttpOpenAiVoiceClient {
    fn post_json(&self, base_url: &str, route: &str, payload: &Value) -> Result<Vec<u8>, String> {
        let endpoint = parse_http_url(base_url, route)?;
        let body = serde_json::to_vec(payload).map_err(|err| err.to_string())?;
        let mut response = isahc::Request::post(endpoint.url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, application/octet-stream")
            .timeout(self.timeout)
            .body(body)
            .map_err(|err| format!("build {route}: {err}"))?
            .send()
            .map_err(|err| format!("send {route}: {err}"))?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .map_err(|err| format!("read {route}: {err}"))?;
        if !(200..300).contains(&status) {
            return Err(format!(
                "{route} failed with {}: {}",
                status,
                clip_error(&String::from_utf8_lossy(&body))
            ));
        }
        Ok(body)
    }
}

pub fn summary_payload(
    model: &str,
    text: &str,
    target_words: u16,
    session_name: &str,
    source_label: &str,
) -> Value {
    let (system_content, label) = if target_words <= 15 {
        (
            "You write spoken mobile notifications. Return one plain sentence of about 15 words. Aim for roughly 12 to 18 words, no markdown, no quotes, no prefixes.",
            "Narration updates",
        )
    } else {
        (
            "You write spoken mobile notifications. Return one plain sentence of about 30 words. Aim for roughly 24 to 36 words, no markdown, no quotes, no prefixes.",
            "Final assistant response",
        )
    };
    let label = if source_label.trim().is_empty() {
        label
    } else {
        source_label.trim()
    };
    let session_name = if session_name.trim().is_empty() {
        "Session"
    } else {
        session_name.trim()
    };
    json!({
        "model": model,
        "temperature": 0.2,
        "max_completion_tokens": 90,
        "messages": [
            {"role": "system", "content": system_content},
            {"role": "user", "content": format!("Session name: {session_name}\n{label}:\n{text}")},
        ],
    })
}

pub fn parse_summary_response(body: &[u8]) -> Result<String, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|err| format!("parse summary json: {err}"))?;
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty())
        .ok_or_else(|| "chat completions response missing choices".to_string())?;
    let message = choices[0]
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| "chat completions response missing message".to_string())?;
    let content = message
        .get("content")
        .ok_or_else(|| "chat completions response missing content".to_string())?;
    let summary = if let Some(text) = content.as_str() {
        compact_text(text)
    } else if let Some(parts) = content.as_array() {
        compact_text(
            &parts
                .iter()
                .filter(|item| {
                    item.get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| matches!(kind, "text" | "output_text"))
                })
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<String>(),
        )
    } else {
        return Err("chat completions response missing content".to_string());
    };
    if summary.is_empty() {
        Err("empty summary response".to_string())
    } else {
        Ok(summary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpEndpoint {
    url: String,
}

pub fn openai_endpoint_url(base_url: &str, route: &str) -> Result<String, String> {
    parse_http_url(base_url, route).map(|endpoint| endpoint.url)
}

fn parse_http_url(base_url: &str, route: &str) -> Result<HttpEndpoint, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("tts_base_url must start with http:// or https://".to_string());
    }
    let rest = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    let (authority, _) = rest.split_once('/').unwrap_or((rest, ""));
    if authority.trim().is_empty() {
        return Err("empty OpenAI host".to_string());
    }
    let route = route.trim_start_matches('/');
    if route.is_empty() {
        return Err("empty OpenAI route".to_string());
    }
    let url = format!("{trimmed}/{route}");
    url.parse::<isahc::http::Uri>()
        .map_err(|_| format!("invalid OpenAI base URL: {base_url}"))?;
    Ok(HttpEndpoint { url })
}

fn compact_text(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip_error(raw: &str) -> String {
    let text = compact_text(raw);
    if text.chars().count() <= 400 {
        return text;
    }
    let mut out = text.chars().take(399).collect::<String>();
    out.push_str("...");
    out
}

pub fn default_timeout() -> Duration {
    Duration::from_secs(30)
}
