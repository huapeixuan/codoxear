use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRequest {
    pub base_url: String,
    pub model: String,
    pub api_key_present: bool,
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
        let payload = summary_payload(&request.model, text, request.target_words);
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
        let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
            .map_err(|err| format!("connect {}:{}: {err}", endpoint.host, endpoint.port))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|err| format!("set read timeout: {err}"))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|err| format!("set write timeout: {err}"))?;
        let request = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nAccept: application/json, application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            endpoint.path,
            endpoint.host_header,
            self.api_key,
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&body))
            .map_err(|err| format!("write {route}: {err}"))?;
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .map_err(|err| format!("read {route}: {err}"))?;
        let response = parse_http_response(&raw)?;
        if !(200..300).contains(&response.status) {
            return Err(format!(
                "{route} failed with {}: {}",
                response.status,
                clip_error(&String::from_utf8_lossy(&response.body))
            ));
        }
        Ok(response.body)
    }
}

pub fn summary_payload(model: &str, text: &str, target_words: u16) -> Value {
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
    json!({
        "model": model,
        "temperature": 0.2,
        "max_completion_tokens": 90,
        "messages": [
            {"role": "system", "content": system_content},
            {"role": "user", "content": format!("Session name: Session\n{label}:\n{text}")},
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
    host: String,
    host_header: String,
    port: u16,
    path: String,
}

fn parse_http_url(base_url: &str, route: &str) -> Result<HttpEndpoint, String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("http://")
        .ok_or_else(|| "Rust OpenAI client currently supports http:// endpoints in tests; use trait mocks for https://".to_string())?;
    let (authority, base_path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = if let Some((host, raw_port)) = authority.rsplit_once(':') {
        let port = raw_port
            .parse::<u16>()
            .map_err(|_| format!("invalid port in {base_url}"))?;
        (host.to_string(), port)
    } else {
        (authority.to_string(), 80)
    };
    if host.trim().is_empty() {
        return Err("empty OpenAI host".to_string());
    }
    let path = format!(
        "/{}{}",
        base_path.trim_matches('/'),
        route
            .strip_prefix('/')
            .map(|value| format!("/{value}"))
            .unwrap_or_else(|| format!("/{route}"))
    );
    Ok(HttpEndpoint {
        host: host.clone(),
        host_header: if port == 80 {
            host
        } else {
            format!("{host}:{port}")
        },
        port,
        path: path.replace("//", "/"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn parse_http_response(raw: &[u8]) -> Result<HttpResponse, String> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "malformed HTTP response".to_string())?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = raw[split + 4..].to_vec();
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "malformed HTTP status".to_string())?;
    Ok(HttpResponse { status, body })
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
