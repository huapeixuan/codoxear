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
            Self::ConnectRefused => write!(formatter, "connect refused"),
            Self::Timeout => write!(formatter, "timeout"),
            Self::Empty => write!(formatter, "empty response"),
            Self::Malformed(message) => write!(formatter, "malformed response: {message}"),
            Self::Io(message) => write!(formatter, "io error: {message}"),
        }
    }
}

impl std::error::Error for BrokerError {}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerState {
    pub busy: bool,
    pub queue_len: usize,
    pub token: Option<Value>,
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
    let raw = serde_json::to_vec(request).map_err(|err| BrokerError::Malformed(err.to_string()))?;
    stream.write_all(&raw).map_err(map_io_or_timeout)?;
    stream.write_all(b"\n").map_err(map_io_or_timeout)?;
    stream.flush().map_err(map_io_or_timeout)?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).map_err(map_io_or_timeout)?;
    if bytes == 0 || line.trim().is_empty() {
        return Err(BrokerError::Empty);
    }
    serde_json::from_str(line.trim_end()).map_err(|err| BrokerError::Malformed(err.to_string()))
}

pub fn broker_state(sock_path: &Path, timeout: Duration) -> Result<BrokerState, BrokerError> {
    let value = broker_request(sock_path, &json!({"cmd": "state"}), timeout)?;
    let object = value
        .as_object()
        .ok_or_else(|| BrokerError::Malformed("state response is not an object".to_string()))?;
    let busy = object
        .get("busy")
        .and_then(Value::as_bool)
        .ok_or_else(|| BrokerError::Malformed("state.busy missing or invalid".to_string()))?;
    let queue_len = object
        .get("queue_len")
        .and_then(Value::as_u64)
        .ok_or_else(|| BrokerError::Malformed("state.queue_len missing or invalid".to_string()))?
        as usize;
    Ok(BrokerState {
        busy,
        queue_len,
        token: object.get("token").cloned(),
    })
}

pub fn broker_ui_state(sock_path: &Path, timeout: Duration) -> Result<BrokerUiState, BrokerError> {
    Ok(BrokerUiState {
        value: broker_request(sock_path, &json!({"cmd": "ui_state"}), timeout)?,
    })
}

pub fn broker_commands(sock_path: &Path, timeout: Duration) -> Result<BrokerCommands, BrokerError> {
    Ok(BrokerCommands {
        value: broker_request(sock_path, &json!({"cmd": "commands"}), timeout)?,
    })
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

fn map_io_or_timeout(err: std::io::Error) -> BrokerError {
    match err.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => BrokerError::Timeout,
        _ => BrokerError::Io(err.to_string()),
    }
}

fn map_io_error(err: std::io::Error) -> BrokerError {
    BrokerError::Io(err.to_string())
}
