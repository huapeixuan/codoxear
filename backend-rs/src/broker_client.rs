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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrokerError::ConnectRefused => write!(f, "connect refused"),
            BrokerError::Timeout => write!(f, "timeout"),
            BrokerError::Empty => write!(f, "empty response"),
            BrokerError::Malformed(message) => write!(f, "malformed response: {message}"),
            BrokerError::Io(message) => write!(f, "io error: {message}"),
        }
    }
}

impl std::error::Error for BrokerError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerState {
    pub busy: bool,
    pub queue_len: usize,
    pub token: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerUiState {
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerCommands {
    pub value: Value,
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

    let mut raw = serde_json::to_vec(request)
        .map_err(|err| BrokerError::Malformed(format!("encode request: {err}")))?;
    raw.push(b'\n');
    stream.write_all(&raw).map_err(map_io_error)?;
    stream.flush().map_err(map_io_error)?;

    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    let bytes = reader.read_line(&mut line).map_err(map_io_error)?;
    if bytes == 0 || line.trim().is_empty() {
        return Err(BrokerError::Empty);
    }
    serde_json::from_str(&line).map_err(|err| BrokerError::Malformed(err.to_string()))
}

pub fn broker_state(sock_path: &Path, timeout: Duration) -> Result<BrokerState, BrokerError> {
    let value = broker_request(sock_path, &json!({"cmd": "state"}), timeout)?;
    parse_broker_state(value)
}

pub fn broker_ui_state(sock_path: &Path, timeout: Duration) -> Result<BrokerUiState, BrokerError> {
    broker_request(sock_path, &json!({"cmd": "ui_state"}), timeout)
        .map(|value| BrokerUiState { value })
}

pub fn broker_commands(sock_path: &Path, timeout: Duration) -> Result<BrokerCommands, BrokerError> {
    broker_request(sock_path, &json!({"cmd": "commands"}), timeout)
        .map(|value| BrokerCommands { value })
}

fn parse_broker_state(value: Value) -> Result<BrokerState, BrokerError> {
    #[derive(Deserialize)]
    struct RawState {
        busy: Option<bool>,
        queue_len: Option<usize>,
        #[serde(default)]
        token: Option<String>,
    }

    let state: RawState =
        serde_json::from_value(value).map_err(|err| BrokerError::Malformed(err.to_string()))?;
    let busy = state
        .busy
        .ok_or_else(|| BrokerError::Malformed("missing busy".to_string()))?;
    let queue_len = state
        .queue_len
        .ok_or_else(|| BrokerError::Malformed("missing queue_len".to_string()))?;
    Ok(BrokerState {
        busy,
        queue_len,
        token: state.token,
    })
}

fn map_connect_error(err: std::io::Error) -> BrokerError {
    match err.kind() {
        std::io::ErrorKind::NotFound
        | std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::ConnectionReset => BrokerError::ConnectRefused,
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => BrokerError::Timeout,
        _ => BrokerError::Io(err.to_string()),
    }
}

fn map_io_error(err: std::io::Error) -> BrokerError {
    match err.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => BrokerError::Timeout,
        std::io::ErrorKind::ConnectionRefused
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::BrokenPipe => BrokerError::ConnectRefused,
        _ => BrokerError::Io(err.to_string()),
    }
}
