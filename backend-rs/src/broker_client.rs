use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrokerError {
    ConnectRefused,
    Timeout,
    Empty,
    Malformed(String),
    Io(String),
}

impl fmt::Display for BrokerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrokerError::ConnectRefused => write!(formatter, "broker socket connect refused"),
            BrokerError::Timeout => write!(formatter, "broker socket timeout"),
            BrokerError::Empty => write!(formatter, "broker socket returned empty response"),
            BrokerError::Malformed(message) => {
                write!(formatter, "malformed broker response: {message}")
            }
            BrokerError::Io(message) => write!(formatter, "broker socket io error: {message}"),
        }
    }
}

impl std::error::Error for BrokerError {}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BrokerState {
    pub busy: bool,
    pub queue_len: usize,
    #[serde(default)]
    pub token: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerUiState {
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerCommands {
    pub raw: Value,
}

pub fn broker_request(
    sock_path: &Path,
    request: &Value,
    timeout: Duration,
) -> Result<Value, BrokerError> {
    let mut stream = UnixStream::connect(sock_path).map_err(map_connect_error)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(map_io_error)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(map_io_error)?;

    let payload = serde_json::to_vec(request)
        .map_err(|err| BrokerError::Malformed(format!("request serialization failed: {err}")))?;
    stream.write_all(&payload).map_err(map_io_error)?;
    stream.write_all(b"\n").map_err(map_io_error)?;
    stream.flush().map_err(map_io_error)?;

    let mut line = String::new();
    let bytes_read = BufReader::new(stream)
        .read_line(&mut line)
        .map_err(map_io_error)?;
    if bytes_read == 0 || line.trim().is_empty() {
        return Err(BrokerError::Empty);
    }
    serde_json::from_str(line.trim_end())
        .map_err(|err| BrokerError::Malformed(format!("invalid JSON: {err}")))
}

pub fn broker_state(sock_path: &Path, timeout: Duration) -> Result<BrokerState, BrokerError> {
    let value = broker_request(sock_path, &json!({ "cmd": "state" }), timeout)?;
    serde_json::from_value(value).map_err(|err| BrokerError::Malformed(err.to_string()))
}

pub fn broker_ui_state(sock_path: &Path, timeout: Duration) -> Result<BrokerUiState, BrokerError> {
    let raw = broker_request(sock_path, &json!({ "cmd": "ui_state" }), timeout)?;
    Ok(BrokerUiState { raw })
}

pub fn broker_commands(sock_path: &Path, timeout: Duration) -> Result<BrokerCommands, BrokerError> {
    let raw = broker_request(sock_path, &json!({ "cmd": "commands" }), timeout)?;
    Ok(BrokerCommands { raw })
}

pub fn broker_send(
    sock_path: &Path,
    text: &str,
    images: Option<Vec<Value>>,
    timeout: Duration,
) -> Result<Value, BrokerError> {
    let mut request = serde_json::Map::new();
    request.insert("cmd".to_string(), json!("send"));
    request.insert("text".to_string(), json!(text));
    if let Some(images) = images.filter(|images| !images.is_empty()) {
        request.insert("images".to_string(), Value::Array(images));
    }
    broker_request(sock_path, &Value::Object(request), timeout)
}

pub fn broker_keys(sock_path: &Path, seq: &str, timeout: Duration) -> Result<Value, BrokerError> {
    broker_request(sock_path, &json!({ "cmd": "keys", "seq": seq }), timeout)
}

pub fn broker_ui_response(
    sock_path: &Path,
    payload: &Value,
    timeout: Duration,
) -> Result<Value, BrokerError> {
    let mut request = serde_json::Map::new();
    request.insert("cmd".to_string(), json!("ui_response"));
    if let Some(object) = payload.as_object() {
        for key in ["id", "value", "confirmed", "cancelled"] {
            if let Some(value) = object.get(key) {
                request.insert(key.to_string(), value.clone());
            }
        }
    }
    broker_request(sock_path, &Value::Object(request), timeout)
}

pub fn broker_shutdown(sock_path: &Path, timeout: Duration) -> Result<Value, BrokerError> {
    broker_request(sock_path, &json!({ "cmd": "shutdown" }), timeout)
}

fn map_connect_error(error: std::io::Error) -> BrokerError {
    match error.kind() {
        std::io::ErrorKind::NotFound
        | std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::AddrNotAvailable => BrokerError::ConnectRefused,
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => BrokerError::Timeout,
        _ => BrokerError::Io(error.to_string()),
    }
}

fn map_io_error(error: std::io::Error) -> BrokerError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => BrokerError::Timeout,
        _ => BrokerError::Io(error.to_string()),
    }
}
