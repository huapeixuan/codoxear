use codoxear_backend_rs::broker_client::{
    broker_commands, broker_keys, broker_request, broker_send, broker_shutdown, broker_state,
    broker_ui_response, broker_ui_state, BrokerError,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn spawn_one_shot_server<F>(sock_path: PathBuf, handler: F) -> thread::JoinHandle<()>
where
    F: FnOnce(String, std::os::unix::net::UnixStream) + Send + 'static,
{
    let listener = UnixListener::bind(&sock_path).expect("bind unix listener");
    thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept unix connection");
        let mut request = String::new();
        let reader_stream = stream.try_clone().expect("clone stream");
        BufReader::new(reader_stream)
            .read_line(&mut request)
            .expect("read request line");
        handler(request, stream);
    })
}

fn temp_sock(dir: &TempDir) -> PathBuf {
    dir.path().join("broker.sock")
}

#[test]
fn successful_state_call_returns_typed_state() {
    let dir = TempDir::new().unwrap();
    let sock_path = temp_sock(&dir);
    let server = spawn_one_shot_server(sock_path.clone(), |request, mut stream| {
        assert_eq!(request.trim_end(), r#"{"cmd":"state"}"#);
        stream
            .write_all(
                br#"{"busy":true,"queue_len":3,"token":"abc"}
"#,
            )
            .unwrap();
    });

    let state = broker_state(&sock_path, Duration::from_secs(1)).unwrap();

    assert!(state.busy);
    assert_eq!(state.queue_len, 3);
    assert_eq!(state.token, json!("abc"));
    server.join().unwrap();
}

#[test]
fn ui_state_and_commands_use_read_only_cmd_literals() {
    let dir = TempDir::new().unwrap();
    let ui_sock = dir.path().join("ui.sock");
    let ui_server = spawn_one_shot_server(ui_sock.clone(), |request, mut stream| {
        assert_eq!(request.trim_end(), r#"{"cmd":"ui_state"}"#);
        stream
            .write_all(
                br#"{"ok":true,"view":"x"}
"#,
            )
            .unwrap();
    });
    let ui_state = broker_ui_state(&ui_sock, Duration::from_secs(1)).unwrap();
    assert_eq!(ui_state.raw, json!({"ok": true, "view": "x"}));
    ui_server.join().unwrap();

    let commands_sock = dir.path().join("commands.sock");
    let commands_server = spawn_one_shot_server(commands_sock.clone(), |request, mut stream| {
        assert_eq!(request.trim_end(), r#"{"cmd":"commands"}"#);
        stream
            .write_all(
                br#"{"ok":true,"commands":["a"]}
"#,
            )
            .unwrap();
    });
    let commands = broker_commands(&commands_sock, Duration::from_secs(1)).unwrap();
    assert_eq!(commands.raw, json!({"ok": true, "commands": ["a"]}));
    commands_server.join().unwrap();
}

#[test]
fn mutation_wrappers_emit_python_compatible_json() {
    let dir = TempDir::new().unwrap();

    let send_sock = dir.path().join("send.sock");
    let send_server = spawn_one_shot_server(send_sock.clone(), |request, mut stream| {
        let value: Value = serde_json::from_str(request.trim_end()).unwrap();
        assert_eq!(
            value,
            json!({"cmd": "send", "text": "hello", "images": [{"data_b64": "abc"}]})
        );
        stream
            .write_all(
                br#"{"ok":true}
"#,
            )
            .unwrap();
    });
    let send = broker_send(
        &send_sock,
        "hello",
        Some(vec![json!({"data_b64": "abc"})]),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(send, json!({"ok": true}));
    send_server.join().unwrap();

    let keys_sock = dir.path().join("keys.sock");
    let keys_server = spawn_one_shot_server(keys_sock.clone(), |request, mut stream| {
        assert_eq!(request.trim_end(), r#"{"cmd":"keys","seq":"\u001b"}"#);
        stream
            .write_all(
                br#"{"ok":true}
"#,
            )
            .unwrap();
    });
    let keys = broker_keys(&keys_sock, "\x1b", Duration::from_secs(1)).unwrap();
    assert_eq!(keys, json!({"ok": true}));
    keys_server.join().unwrap();

    let ui_sock = dir.path().join("ui_response.sock");
    let ui_server = spawn_one_shot_server(ui_sock.clone(), |request, mut stream| {
        let value: Value = serde_json::from_str(request.trim_end()).unwrap();
        assert_eq!(
            value,
            json!({"cmd": "ui_response", "id": "ask-1", "value": "yes", "confirmed": true, "cancelled": false})
        );
        stream
            .write_all(
                br#"{"ok":true}
"#,
            )
            .unwrap();
    });
    let ui = broker_ui_response(
        &ui_sock,
        &json!({"id": "ask-1", "value": "yes", "confirmed": true, "cancelled": false, "ignored": "x"}),
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(ui, json!({"ok": true}));
    ui_server.join().unwrap();

    let shutdown_sock = dir.path().join("shutdown.sock");
    let shutdown_server = spawn_one_shot_server(shutdown_sock.clone(), |request, mut stream| {
        assert_eq!(request.trim_end(), r#"{"cmd":"shutdown"}"#);
        stream
            .write_all(
                br#"{"ok":true}
"#,
            )
            .unwrap();
    });
    let shutdown = broker_shutdown(&shutdown_sock, Duration::from_secs(1)).unwrap();
    assert_eq!(shutdown, json!({"ok": true}));
    shutdown_server.join().unwrap();
}

#[test]
fn connect_refused_for_missing_socket_path() {
    let dir = TempDir::new().unwrap();
    let error = broker_state(&temp_sock(&dir), Duration::from_millis(50)).unwrap_err();
    assert_eq!(error, BrokerError::ConnectRefused);
}

#[test]
fn timeout_when_listener_accepts_but_never_responds() {
    let dir = TempDir::new().unwrap();
    let sock_path = temp_sock(&dir);
    let listener = UnixListener::bind(&sock_path).unwrap();
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(200));
    });

    let error = broker_request(
        &sock_path,
        &json!({"cmd": "state"}),
        Duration::from_millis(50),
    )
    .unwrap_err();

    assert_eq!(error, BrokerError::Timeout);
    server.join().unwrap();
}

#[test]
fn malformed_json_response_returns_malformed_error() {
    let dir = TempDir::new().unwrap();
    let sock_path = temp_sock(&dir);
    let server = spawn_one_shot_server(sock_path.clone(), |_request, mut stream| {
        stream.write_all(b"not-json\n").unwrap();
    });

    let error = broker_state(&sock_path, Duration::from_secs(1)).unwrap_err();

    assert!(matches!(error, BrokerError::Malformed(_)));
    server.join().unwrap();
}

#[test]
fn missing_required_state_fields_returns_malformed_error() {
    let dir = TempDir::new().unwrap();
    let sock_path = temp_sock(&dir);
    let server = spawn_one_shot_server(sock_path.clone(), |_request, mut stream| {
        stream
            .write_all(
                br#"{"busy":true}
"#,
            )
            .unwrap();
    });

    let error = broker_state(&sock_path, Duration::from_secs(1)).unwrap_err();

    assert!(matches!(error, BrokerError::Malformed(_)));
    server.join().unwrap();
}

#[test]
fn empty_response_returns_empty_error() {
    let dir = TempDir::new().unwrap();
    let sock_path = temp_sock(&dir);
    let server = spawn_one_shot_server(sock_path.clone(), |_request, _stream| {});

    let error =
        broker_request(&sock_path, &json!({"cmd": "state"}), Duration::from_secs(1)).unwrap_err();

    assert_eq!(error, BrokerError::Empty);
    server.join().unwrap();
}

#[test]
fn arbitrary_request_helper_round_trips_json_line() {
    let dir = TempDir::new().unwrap();
    let sock_path = temp_sock(&dir);
    let server = spawn_one_shot_server(sock_path.clone(), |request, mut stream| {
        assert_eq!(request.trim_end(), r#"{"hello":"world"}"#);
        stream
            .write_all(
                br#"{"ok":true}
"#,
            )
            .unwrap();
    });

    let value = broker_request(
        &sock_path,
        &json!({"hello": "world"}),
        Duration::from_secs(1),
    )
    .unwrap();

    assert_eq!(value, json!({"ok": true}));
    server.join().unwrap();
}

#[test]
fn stale_socket_file_maps_to_connect_refused() {
    let dir = TempDir::new().unwrap();
    let sock_path = temp_sock(&dir);
    create_and_drop_listener(&sock_path);

    let error = broker_state(&sock_path, Duration::from_millis(50)).unwrap_err();

    assert_eq!(error, BrokerError::ConnectRefused);
}

fn create_and_drop_listener(sock_path: &Path) {
    let _listener = UnixListener::bind(sock_path).unwrap();
}
