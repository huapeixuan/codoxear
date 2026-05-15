use super::*;
use std::os::unix::net::UnixStream;
use tempfile::TempDir;

fn test_env() -> BrokerEnv {
    let dir = TempDir::new().unwrap();
    BrokerEnv {
        backend: "pi".into(),
        owner: Some("web".into()),
        spawn_nonce: None,
        transport: None,
        tmux_session: None,
        tmux_window: None,
        app_dir: dir.path().to_path_buf(),
        codex_home: dir.path().join("codex"),
        pi_home: dir.path().join("pi"),
        codex_bin: "codex".into(),
        pi_bin: "pi".into(),
        model_provider: None,
        preferred_auth_method: None,
        model: None,
        reasoning_effort: None,
        service_tier: None,
        debug: false,
    }
}

fn test_state(dir: &Path, backend: &str) -> Arc<Mutex<State>> {
    Arc::new(Mutex::new(State {
        backend: backend.into(),
        child_pid: 1,
        broker_pid: 2,
        cwd: dir.to_path_buf(),
        start_ts: 1.0,
        sock_path: dir.join("x.sock"),
        session_path: None,
        session_id: None,
        log_path: None,
        log_off: 0,
        resume_session_id: None,
        busy: backend == "codex",
        output_tail: "tail".into(),
        stdin_eof: false,
        token: None,
        child_stdin: None,
        pi_rpc: None,
        last_turn_id: None,
        pty_master: None,
        pending_ui_requests: serde_json::Map::new(),
        live_message_offset: 0,
    }))
}

#[test]
fn env_resolution_matches_codoxear_app_dir_contract() {
    let dir = TempDir::new().unwrap();
    std::env::set_var("CODOXEAR_APP_DIR", dir.path());
    std::env::set_var("CODEX_WEB_AGENT_BACKEND", " pi ");
    std::env::set_var("CODEX_WEB_BROKER_DEBUG", "1");
    let env = BrokerEnv::from_process(None);
    assert_eq!(env.backend, "pi");
    assert_eq!(env.app_dir, dir.path());
    assert!(env.debug);
    std::env::remove_var("CODOXEAR_APP_DIR");
    std::env::remove_var("CODEX_WEB_AGENT_BACKEND");
    std::env::remove_var("CODEX_WEB_BROKER_DEBUG");
}

#[test]
fn socket_dispatch_matches_python_validation_strings() {
    let dir = TempDir::new().unwrap();
    let env = test_env();
    let state = test_state(dir.path(), "pi");
    let stop = Arc::new(AtomicBool::new(false));
    assert_eq!(
        dispatch_command(&json!({"cmd":"send","text":""}), &state, &env, &stop),
        json!({"error":"text required"})
    );
    assert_eq!(
        dispatch_command(
            &json!({"cmd":"ui_response","id":"missing"}),
            &state,
            &env,
            &stop
        ),
        json!({"error":"unknown or expired request"})
    );
    assert_eq!(
        dispatch_command(&json!({"cmd":"wat"}), &state, &env, &stop),
        json!({"error":"unknown cmd"})
    );
}

#[test]
fn socket_server_round_trips_state() {
    let dir = TempDir::new().unwrap();
    let mut env = BrokerEnv::from_process(Some("codex"));
    env.backend = "codex".into();
    let state = test_state(dir.path(), "codex");
    let stop = Arc::new(AtomicBool::new(false));
    start_socket_server(state, env, stop.clone()).unwrap();
    for _ in 0..20 {
        if dir.path().join("x.sock").exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let mut stream = UnixStream::connect(dir.path().join("x.sock")).unwrap();
    writeln!(stream, "{}", json!({"cmd":"state"})).unwrap();
    let mut resp = String::new();
    BufReader::new(stream).read_line(&mut resp).unwrap();
    stop.store(true, Ordering::SeqCst);
    let value: Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(value["busy"], true);
    assert_eq!(value["queue_len"], 0);
}

#[test]
fn pi_rpc_args_injects_ask_user_bridge_once() {
    let cli = BrokerCli {
        cwd: PathBuf::from("/tmp/work"),
        session_file: Some(PathBuf::from("/tmp/session.jsonl")),
        agent_args: vec!["--model".into(), "fake".into()],
    };
    let args = pi_rpc_args(&cli);
    assert_eq!(
        &args[..4],
        &["--mode", "rpc", "--session", "/tmp/session.jsonl"]
    );
    assert!(args
        .windows(2)
        .any(|pair| pair[0] == "-e" && pair[1].ends_with("ask_user_bridge.ts")));
    assert!(args
        .windows(2)
        .any(|pair| pair[0] == "--model" && pair[1] == "fake"));
}

#[test]
fn pi_rpc_args_does_not_duplicate_ask_user_bridge() {
    let bridge = repo_pi_ask_user_bridge_path();
    let cli = BrokerCli {
        cwd: PathBuf::from("/tmp/work"),
        session_file: None,
        agent_args: vec!["-e".into(), bridge.clone(), "--model".into(), "fake".into()],
    };
    let args = pi_rpc_args(&cli);
    assert_eq!(args.iter().filter(|arg| **arg == "-e").count(), 1);
    assert_eq!(args.iter().filter(|arg| **arg == bridge).count(), 1);
}
