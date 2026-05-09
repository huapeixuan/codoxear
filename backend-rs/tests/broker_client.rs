use codoxear_backend_rs::broker_client::{broker_request, broker_state, BrokerError, BrokerState};
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn spawn_one_shot_server(sock_path: &std::path::Path, response: &'static [u8]) {
    let listener = UnixListener::bind(sock_path).unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        reader.read_line(&mut request).unwrap();
        stream.write_all(response).unwrap();
    });
}

#[test]
fn broker_state_successfully_reads_json_line() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("broker.sock");
    spawn_one_shot_server(
        &sock,
        br#"{"busy":true,"queue_len":3,"token":"abc"}
"#,
    );

    let state = broker_state(&sock, Duration::from_secs(1)).unwrap();

    assert_eq!(
        state,
        BrokerState {
            busy: true,
            queue_len: 3,
            token: Some(json!("abc"))
        }
    );
}

#[test]
fn broker_request_reports_connect_refused_for_missing_socket() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("missing.sock");

    let err = broker_state(&sock, Duration::from_millis(50)).unwrap_err();

    assert_eq!(err, BrokerError::ConnectRefused);
}

#[test]
fn broker_request_reports_malformed_json() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("broker.sock");
    spawn_one_shot_server(&sock, b"not-json\n");

    let err = broker_request(&sock, &json!({"cmd": "state"}), Duration::from_secs(1)).unwrap_err();

    assert!(matches!(err, BrokerError::Malformed(_)));
}

#[test]
fn broker_state_requires_busy_and_queue_len_fields() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("broker.sock");
    spawn_one_shot_server(
        &sock,
        br#"{"busy":true}
"#,
    );

    let err = broker_state(&sock, Duration::from_secs(1)).unwrap_err();

    assert!(matches!(err, BrokerError::Malformed(_)));
}

#[test]
fn broker_request_reports_timeout() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("broker.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(200));
    });

    let err = broker_state(&sock, Duration::from_millis(25)).unwrap_err();

    assert_eq!(err, BrokerError::Timeout);
}
