use codoxear_backend_rs::broker_client::{broker_state, BrokerError};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn successful_state_call_parses_required_fields() {
    let dir = TempDir::new().unwrap();
    let sock_path = dir.path().join("broker.sock");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&request).unwrap(),
            json!({"cmd": "state"})
        );
        stream
            .write_all(
                br#"{"busy":true,"queue_len":3,"token":"tok"}
"#,
            )
            .unwrap();
    });

    let state = broker_state(&sock_path, Duration::from_secs(1)).unwrap();

    handle.join().unwrap();
    assert!(state.busy);
    assert_eq!(state.queue_len, 3);
    assert_eq!(state.token, Some("tok".to_string()));
}

#[test]
fn missing_socket_returns_connect_refused() {
    let dir = TempDir::new().unwrap();
    let err =
        broker_state(&dir.path().join("missing.sock"), Duration::from_millis(50)).unwrap_err();

    assert_eq!(err, BrokerError::ConnectRefused);
}

#[test]
fn empty_response_returns_empty_error() {
    let dir = TempDir::new().unwrap();
    let sock_path = dir.path().join("broker.sock");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let handle = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
    });

    let err = broker_state(&sock_path, Duration::from_secs(1)).unwrap_err();

    handle.join().unwrap();
    assert_eq!(err, BrokerError::Empty);
}

#[test]
fn malformed_response_returns_malformed_error() {
    let dir = TempDir::new().unwrap();
    let sock_path = dir.path().join("broker.sock");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"not-json\n").unwrap();
    });

    let err = broker_state(&sock_path, Duration::from_secs(1)).unwrap_err();

    handle.join().unwrap();
    assert!(matches!(err, BrokerError::Malformed(_)));
}

#[test]
fn missing_required_state_fields_returns_malformed_error() {
    let dir = TempDir::new().unwrap();
    let sock_path = dir.path().join("broker.sock");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .write_all(
                br#"{"busy":true}
"#,
            )
            .unwrap();
    });

    let err = broker_state(&sock_path, Duration::from_secs(1)).unwrap_err();

    handle.join().unwrap();
    assert!(matches!(err, BrokerError::Malformed(_)));
}
