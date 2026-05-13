use codoxear_backend_rs::models::SessionRow;
use codoxear_backend_rs::runtime::RuntimeConfig;
use codoxear_backend_rs::session_loader::{
    clip01, find_session, load_session_rows, load_sessions_directories_payload,
    load_sessions_recent_payload, priority_from_elapsed_seconds,
};
use serde_json::{json, Map, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;
use tempfile::TempDir;

fn config(dir: &TempDir) -> RuntimeConfig {
    let app_dir = dir.path().join("app");
    fs::create_dir_all(app_dir.join("socks")).unwrap();
    RuntimeConfig { app_dir }
}

fn write_json(path: PathBuf, value: Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn bind_state_broker(sock_path: PathBuf, response: &'static [u8]) -> thread::JoinHandle<()> {
    let listener = UnixListener::bind(sock_path).unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut request)
            .unwrap();
        assert_eq!(request.trim_end(), r#"{"cmd":"state"}"#);
        stream.write_all(response).unwrap();
    })
}

fn write_session_meta(config: &RuntimeConfig, id: &str, cwd: &str, updated_ts: f64) -> PathBuf {
    let sock_path = config.app_dir.join("socks").join(format!("{id}.sock"));
    let listener = UnixListener::bind(&sock_path).unwrap();
    drop(listener);
    write_json(
        sock_path.with_extension("json"),
        json!({
            "session_id": format!("thread-{id}"),
            "agent_backend": "codex",
            "owner": "web",
            "cwd": cwd,
            "start_ts": updated_ts - 10.0,
            "updated_ts": updated_ts,
            "busy": false,
            "queue_len": 1,
        }),
    );
    sock_path
}

#[test]
fn load_session_rows_merges_state_files_and_live_broker_state() {
    let dir = TempDir::new().unwrap();
    let config = config(&dir);
    let cwd = dir.path().join("project");
    fs::create_dir(&cwd).unwrap();
    let sock_path = write_session_meta(&config, "s1", cwd.to_str().unwrap(), 100.0);
    fs::remove_file(&sock_path).unwrap();
    let broker = bind_state_broker(
        sock_path.clone(),
        br#"{"busy":true,"queue_len":4,"token":{"used":1}}
"#,
    );
    write_json(
        config.app_dir.join("session_aliases.json"),
        json!({"s1": "Alias"}),
    );
    write_json(
        config.app_dir.join("session_queues.json"),
        json!({"s1": [{"id": "q1"}, {"id": "q2"}]}),
    );
    write_json(
        config.app_dir.join("harness.json"),
        json!({"s1": {"enabled": true, "cooldown_minutes": 9, "remaining_injections": 2, "request": "keep going"}}),
    );
    write_json(
        config.app_dir.join("session_sidebar.json"),
        json!({"s1": {"priority_offset": 0.2}}),
    );
    write_json(
        config.app_dir.join("session_files.json"),
        json!({"s1": ["README.md"]}),
    );

    let rows = load_session_rows(&config).unwrap();

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.session_id, "s1");
    assert_eq!(row.alias, "Alias");
    assert!(row.busy);
    assert!(row.broker_busy);
    assert_eq!(row.queue_len, 4);
    assert_eq!(row.token, json!({"used": 1}));
    assert!(row.harness_enabled);
    assert_eq!(row.files, vec!["README.md"]);
    broker.join().unwrap();
}

#[test]
fn load_session_rows_falls_back_to_sidecar_when_broker_is_down() {
    let dir = TempDir::new().unwrap();
    let config = config(&dir);
    let cwd = dir.path().join("project");
    fs::create_dir(&cwd).unwrap();
    write_session_meta(&config, "s1", cwd.to_str().unwrap(), 100.0);

    let rows = load_session_rows(&config).unwrap();

    assert_eq!(rows.len(), 1);
    assert!(!rows[0].broker_busy);
    assert!(!rows[0].busy);
    assert_eq!(rows[0].queue_len, 1);
}

#[test]
fn load_session_rows_skips_hidden_sessions() {
    let dir = TempDir::new().unwrap();
    let config = config(&dir);
    let cwd = dir.path().join("project");
    fs::create_dir(&cwd).unwrap();
    write_session_meta(&config, "s1", cwd.to_str().unwrap(), 100.0);
    write_json(config.app_dir.join("hidden_sessions.json"), json!(["s1"]));

    assert!(load_session_rows(&config).unwrap().is_empty());
}

#[test]
fn find_session_returns_unknown_session_error() {
    let dir = TempDir::new().unwrap();
    let config = config(&dir);

    let error = find_session(&config, "missing").unwrap_err();

    assert!(error.contains("unknown session: missing"));
}

#[test]
fn priority_helpers_match_python_boundaries() {
    assert_eq!(clip01(-1.0), 0.0);
    assert_eq!(clip01(2.0), 1.0);
    assert_eq!(clip01(0.25), 0.25);
    assert_eq!(priority_from_elapsed_seconds(0.0), 1.0);
    let half_life = 8.0 * 3600.0;
    assert!((priority_from_elapsed_seconds(half_life) - 0.5).abs() < 1e-9);
    assert!(priority_from_elapsed_seconds(half_life * 100.0) < 0.001);
}

#[test]
fn directories_payload_groups_and_filters_hidden_cwds() {
    let dir = TempDir::new().unwrap();
    let cwd_a = dir.path().join("a");
    let cwd_b = dir.path().join("b");
    fs::create_dir(&cwd_a).unwrap();
    fs::create_dir(&cwd_b).unwrap();
    let rows = vec![
        row("s1", cwd_a.to_str().unwrap(), true, 30.0),
        row("s2", cwd_a.to_str().unwrap(), false, 20.0),
        row("s3", cwd_b.to_str().unwrap(), false, 10.0),
    ];
    let mut cwd_groups = Map::new();
    cwd_groups.insert(
        codoxear_backend_rs::runtime::normalize_cwd_group_key(cwd_b.to_str().unwrap()).unwrap(),
        json!({"hidden": true}),
    );

    let payload = load_sessions_directories_payload(&rows, &cwd_groups, None, 0, 5, 0, 3);

    assert_eq!(payload["sessions"].as_array().unwrap().len(), 2);
    assert_eq!(payload["sessions"][0]["session_id"], "s1");
    assert_eq!(payload["remaining_by_group"], json!({}));
}

#[test]
fn recent_payload_orders_busy_then_updated() {
    let dir = TempDir::new().unwrap();
    let cwd = dir.path().join("project");
    fs::create_dir(&cwd).unwrap();
    let rows = vec![
        row("old", cwd.to_str().unwrap(), false, 10.0),
        row("busy", cwd.to_str().unwrap(), true, 5.0),
        row("new", cwd.to_str().unwrap(), false, 20.0),
    ];

    let payload = load_sessions_recent_payload(&rows, &Map::new(), 0, 20);
    let sessions = payload["sessions"].as_array().unwrap();

    assert_eq!(sessions[0]["session_id"], "busy");
    assert_eq!(sessions[1]["session_id"], "new");
    assert_eq!(sessions[2]["session_id"], "old");
    assert_eq!(payload["remaining"], 0);
}

fn row(id: &str, cwd: &str, busy: bool, updated_ts: f64) -> SessionRow {
    SessionRow {
        session_id: id.to_string(),
        thread_id: Some(format!("thread-{id}")),
        title: None,
        alias: String::new(),
        first_user_message: None,
        agent_backend: "codex".to_string(),
        backend: "codex".to_string(),
        owner: Some("web".to_string()),
        owned: true,
        transport: None,
        supports_live_ui: false,
        ui_protocol_version: None,
        cwd: cwd.to_string(),
        workspace_cwd: None,
        log_path: None,
        session_path: None,
        start_ts: updated_ts - 1.0,
        updated_ts,
        broker_pid: 0,
        codex_pid: 0,
        busy,
        broker_busy: busy,
        queue_len: 0,
        queue_items: Vec::new(),
        token: Value::Null,
        harness_enabled: false,
        harness_cooldown_minutes: 5.0,
        harness_remaining_injections: 3,
        harness_request: String::new(),
        files: Vec::new(),
        priority_offset: 0.0,
        snooze_until: None,
        dependency_session_id: None,
        final_priority: 1.0,
        base_priority: 1.0,
        time_priority: 1.0,
        blocked: false,
        snoozed: false,
        git_branch: None,
        pr_summary: Value::Null,
        todo_snapshot: json!({"available": false, "error": false, "items": []}),
        model_provider: None,
        preferred_auth_method: None,
        provider_choice: None,
        model: None,
        reasoning_effort: None,
        service_tier: None,
        tmux_session: None,
        tmux_window: None,
    }
}
