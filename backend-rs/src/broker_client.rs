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
            Self::ConnectRefused => write!(f, "connect refused"),
            Self::Timeout => write!(f, "broker request timed out"),
            Self::Empty => write!(f, "empty broker response"),
            Self::Malformed(message) => write!(f, "malformed broker response: {message}"),
            Self::Io(message) => write!(f, "broker io error: {message}"),
        }
    }
}

impl std::error::Error for BrokerError {}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BrokerState {
    pub busy: bool,
    pub queue_len: usize,
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BrokerUiState {
    #[serde(flatten)]
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BrokerCommands {
    #[serde(flatten)]
    pub payload: Value,
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

    let mut payload = serde_json::to_vec(request)
        .map_err(|err| BrokerError::Malformed(format!("serialize request: {err}")))?;
    payload.push(b'\n');
    stream.write_all(&payload).map_err(map_io_error)?;
    stream.flush().map_err(map_io_error)?;

    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    let read = reader.read_line(&mut line).map_err(map_io_error)?;
    if read == 0 || line.trim().is_empty() {
        return Err(BrokerError::Empty);
    }
    serde_json::from_str::<Value>(&line).map_err(|err| BrokerError::Malformed(err.to_string()))
}

pub fn broker_state(sock_path: &Path, timeout: Duration) -> Result<BrokerState, BrokerError> {
    let value = broker_request(sock_path, &json!({"cmd": "state"}), timeout)?;
    serde_json::from_value::<BrokerState>(value)
        .map_err(|err| BrokerError::Malformed(err.to_string()))
}

pub fn broker_ui_state(sock_path: &Path, timeout: Duration) -> Result<BrokerUiState, BrokerError> {
    let value = broker_request(sock_path, &json!({"cmd": "ui_state"}), timeout)?;
    Ok(BrokerUiState { payload: value })
}

pub fn broker_commands(sock_path: &Path, timeout: Duration) -> Result<BrokerCommands, BrokerError> {
    let value = broker_request(sock_path, &json!({"cmd": "commands"}), timeout)?;
    Ok(BrokerCommands { payload: value })
}

fn map_connect_error(err: std::io::Error) -> BrokerError {
    match err.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            BrokerError::ConnectRefused
        }
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => BrokerError::Timeout,
        _ => BrokerError::Io(err.to_string()),
    }
}

fn map_io_error(err: std::io::Error) -> BrokerError {
    match err.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => BrokerError::Timeout,
        _ => BrokerError::Io(err.to_string()),
    }
}
