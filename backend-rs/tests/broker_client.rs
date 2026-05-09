use codoxear_backend_rs::broker_client::{
    broker_commands, broker_state, broker_ui_state, BrokerError,
};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn spawn_one_response(sock_path: &Path, response: &'static str) -> thread::JoinHandle<Value> {
    let listener = UnixListener::bind(sock_path).expect("bind unix listener");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept client");
        let mut request = String::new();
        BufReader::new(stream.try_clone().expect("clone stream"))
            .read_line(&mut request)
            .expect("read request");
        stream
            .write_all(response.as_bytes())
            .expect("write response");
        serde_json::from_str(request.trim()).expect("request json")
    })
}

#[test]
fn broker_state_success_reads_one_json_line() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("state.sock");
    let handle = spawn_one_response(&sock, "{\"busy\":true,\"queue_len\":2,\"token\":\"abc\"}\n");

    let state = broker_state(&sock, Duration::from_millis(500)).unwrap();

    assert!(state.busy);
    assert_eq!(state.queue_len, 2);
    assert_eq!(state.token.as_deref(), Some("abc"));
    assert_eq!(handle.join().unwrap(), serde_json::json!({"cmd": "state"}));
}

#[test]
fn broker_ui_state_uses_ui_state_command() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("ui.sock");
    let handle = spawn_one_response(&sock, "{\"ok\":true,\"sidebar\":{}}\n");

    let state = broker_ui_state(&sock, Duration::from_millis(500)).unwrap();

    assert_eq!(state.value["ok"], true);
    assert_eq!(
        handle.join().unwrap(),
        serde_json::json!({"cmd": "ui_state"})
    );
}

#[test]
fn broker_commands_uses_commands_command() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("commands.sock");
    let handle = spawn_one_response(&sock, "{\"commands\":[\"/help\"]}\n");

    let commands = broker_commands(&sock, Duration::from_millis(500)).unwrap();

    assert_eq!(commands.value["commands"][0], "/help");
    assert_eq!(
        handle.join().unwrap(),
        serde_json::json!({"cmd": "commands"})
    );
}

#[test]
fn broker_state_connect_refused_for_missing_socket() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("missing.sock");

    let err = broker_state(&sock, Duration::from_millis(50)).unwrap_err();

    assert_eq!(err, BrokerError::ConnectRefused);
}

#[test]
fn broker_state_timeout_when_listener_does_not_respond() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("timeout.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let handle = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(200));
    });

    let err = broker_state(&sock, Duration::from_millis(25)).unwrap_err();

    assert_eq!(err, BrokerError::Timeout);
    handle.join().unwrap();
}

#[test]
fn broker_state_rejects_malformed_json() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("malformed.sock");
    let handle = spawn_one_response(&sock, "not-json\n");

    let err = broker_state(&sock, Duration::from_millis(500)).unwrap_err();

    assert!(matches!(err, BrokerError::Malformed(_)));
    handle.join().unwrap();
}

#[test]
fn broker_state_rejects_missing_required_fields() {
    let dir = TempDir::new().unwrap();
    let sock = dir.path().join("missing-fields.sock");
    let handle = spawn_one_response(&sock, "{\"busy\":true}\n");

    let err = broker_state(&sock, Duration::from_millis(500)).unwrap_err();

    assert_eq!(err, BrokerError::Malformed("missing queue_len".to_string()));
    handle.join().unwrap();
}
