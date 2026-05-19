use codoxear_backend_rs::broker::meta::{codex_sidecar_value, pi_sidecar_value, CodexMetaInput};
use codoxear_backend_rs::runtime::RuntimeConfig;
use codoxear_backend_rs::session_loader::load_session_rows;
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::thread;
use tempfile::TempDir;

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn bind_state_broker(sock_path: &Path) -> thread::JoinHandle<()> {
    let listener = UnixListener::bind(sock_path).unwrap();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            assert_eq!(request.trim_end(), r#"{"cmd":"state"}"#);
            stream
                .write_all(
                    br#"{"busy":false,"queue_len":0,"token":null}
"#,
                )
                .unwrap();
        }
    })
}

fn config(dir: &TempDir) -> RuntimeConfig {
    let app_dir = dir.path().join("app");
    fs::create_dir_all(app_dir.join("socks")).unwrap();
    RuntimeConfig { app_dir }
}

fn rust_written_pi_sidecar(app_dir: &Path, cwd: &Path, session_path: &Path) -> PathBuf {
    let sock_path = app_dir.join("socks/pi-rust.sock");
    let input = CodexMetaInput {
        session_id: Some("pi-thread".into()),
        owner: Some("web".into()),
        broker_pid: std::process::id(),
        codex_pid: std::process::id(),
        cwd: cwd.to_string_lossy().to_string(),
        start_ts: 100.0,
        log_path: None,
        sock_path: sock_path.clone(),
        resume_session_id: None,
        transport: None,
        tmux_session: None,
        tmux_window: None,
        spawn_nonce: Some("nonce".into()),
    };
    let value = pi_sidecar_value(&input, Some(session_path));
    write_json(&sock_path.with_extension("json"), &value);
    sock_path
}

#[test]
fn rust_sidecar_is_accepted_by_rust_session_loader() {
    let dir = TempDir::new().unwrap();
    let cfg = config(&dir);
    let cwd = dir.path().join("project");
    fs::create_dir_all(&cwd).unwrap();
    let session_path = dir.path().join("pi-session.jsonl");
    fs::write(&session_path, "").unwrap();
    let sock_path = rust_written_pi_sidecar(&cfg.app_dir, &cwd, &session_path);
    let broker = bind_state_broker(&sock_path);

    let rows = load_session_rows(&cfg).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent_backend, "pi");
    assert_eq!(rows[0].transport.as_deref(), Some("pi-rpc"));
    assert_eq!(
        rows[0].session_path.as_deref(),
        Some(session_path.to_str().unwrap())
    );
    assert!(rows[0].supports_live_ui);
    broker.join().unwrap();
}

#[test]
fn python_style_sidecar_is_accepted_by_rust_session_loader() {
    let dir = TempDir::new().unwrap();
    let cfg = config(&dir);
    let cwd = dir.path().join("project");
    fs::create_dir_all(&cwd).unwrap();
    let session_path = dir.path().join("pi-session.jsonl");
    fs::write(&session_path, "").unwrap();
    let sock_path = cfg.app_dir.join("socks/pi-python.sock");
    write_json(
        &sock_path.with_extension("json"),
        &json!({
            "session_id": "pi-thread",
            "backend": "pi",
            "transport": "pi-rpc",
            "owner": "web",
            "supports_web_control": true,
            "supports_live_ui": true,
            "ui_protocol_version": 1,
            "broker_pid": std::process::id(),
            "agent_pid": std::process::id(),
            "codex_pid": std::process::id(),
            "cwd": cwd,
            "start_ts": 100.0,
            "log_path": null,
            "sock_path": sock_path,
            "session_path": session_path,
        }),
    );
    let broker = bind_state_broker(&sock_path);

    let rows = load_session_rows(&cfg).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent_backend, "pi");
    assert_eq!(
        rows[0].session_path.as_deref(),
        Some(session_path.to_str().unwrap())
    );
    broker.join().unwrap();
}

#[test]
fn rust_codex_sidecar_keeps_python_contract_shape_and_mode() {
    let dir = TempDir::new().unwrap();
    let sock_path = dir.path().join("codex.sock");
    let input = CodexMetaInput {
        session_id: Some("codex-thread".into()),
        owner: Some("web".into()),
        broker_pid: 10,
        codex_pid: 11,
        cwd: dir.path().to_string_lossy().to_string(),
        start_ts: 1.0,
        log_path: None,
        sock_path: sock_path.clone(),
        resume_session_id: None,
        transport: Some("pty".into()),
        tmux_session: None,
        tmux_window: None,
        spawn_nonce: Some("nonce".into()),
    };
    let path = sock_path.with_extension("json");
    codoxear_backend_rs::broker::meta::write_sidecar_atomic(&path, &codex_sidecar_value(&input))
        .unwrap();
    let raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(raw["agent_backend"], "codex");
    assert!(raw.get("log_path").is_some());
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
