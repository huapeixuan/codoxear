use super::*;
use std::io::{BufRead, BufReader, Write};
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
        pi_live: PiLiveState::default(),
    }))
}

fn attach_rpc(state: &Arc<Mutex<State>>, script: String) -> std::process::Child {
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    state.lock().unwrap().pi_rpc =
        Some(crate::broker::pi_rpc::PiRpcClient::from_child(&mut child).unwrap());
    child
}

fn socket_json(state: &Arc<Mutex<State>>, env: &BrokerEnv, req: Value) -> Value {
    dispatch_command(&req, state, env, &Arc::new(AtomicBool::new(false)))
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
fn pi_live_messages_coalesce_streams_and_honor_offsets() {
    let dir = TempDir::new().unwrap();
    let env = test_env();
    let state = test_state(dir.path(), "pi");
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut st = state.lock().unwrap();
        let start_ts = st.start_ts;
        let mut busy = st.busy;
        let mut last_turn_id = st.last_turn_id.clone();
        let mut pending_ui_requests = std::mem::take(&mut st.pending_ui_requests);
        let mut pi_live = std::mem::take(&mut st.pi_live);
        let mut output_tail = std::mem::take(&mut st.output_tail);
        {
            let mut live = PiLiveRuntime {
                start_ts,
                busy: &mut busy,
                last_turn_id: &mut last_turn_id,
                pending_ui_requests: &mut pending_ui_requests,
                live: &mut pi_live,
                output_tail: &mut output_tail,
            };
            crate::broker::pi_live::record_event(
                &mut live,
                &json!({"type":"message.delta","delta":"he","turn_id":"turn-1"}),
            );
            crate::broker::pi_live::record_event(
                &mut live,
                &json!({"type":"message.delta","delta":"llo","turn_id":"turn-1"}),
            );
        }
        st.busy = busy;
        st.last_turn_id = last_turn_id;
        st.pending_ui_requests = pending_ui_requests;
        st.pi_live = pi_live;
        st.output_tail = output_tail;
    }
    let first = dispatch_command(
        &json!({"cmd":"live_messages","offset":0}),
        &state,
        &env,
        &stop,
    );
    assert_eq!(first["offset"], 2);
    assert_eq!(first["events"].as_array().unwrap().len(), 1);
    assert_eq!(first["events"][0]["text"], "hello");

    let second = dispatch_command(
        &json!({"cmd":"live_messages","offset":2}),
        &state,
        &env,
        &stop,
    );
    assert_eq!(second, json!({"offset":2,"events":[]}));
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

#[test]
fn pi_send_propagates_prompt_error_and_clears_busy() {
    let dir = TempDir::new().unwrap();
    let env = test_env();
    let state = test_state(dir.path(), "pi");
    let mut child = attach_rpc(&state, r#"while IFS= read -r line; do id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'); printf '{"type":"response","id":"%s","success":false,"error":"prompt failed"}\n' "$id"; done"#.into());
    assert_eq!(
        socket_json(&state, &env, json!({"cmd":"send","text":"hello"})),
        json!({"error":"prompt failed"})
    );
    assert!(!state.lock().unwrap().busy);
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn pi_escape_keys_calls_abort_rpc() {
    let dir = TempDir::new().unwrap();
    let env = test_env();
    let state = test_state(dir.path(), "pi");
    state.lock().unwrap().last_turn_id = Some("turn-7".into());
    let stop = Arc::new(AtomicBool::new(false));
    let marker = dir.path().join("abort-seen");
    let script = format!(
        r#"while IFS= read -r line; do id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'); case "$line" in *'"type":"abort"'*) echo yes > '{}'; printf '{{"type":"response","id":"%s","success":true,"data":{{"aborted":true}}}}\n' "$id";; esac; done"#,
        marker.display()
    );
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    state.lock().unwrap().pi_rpc =
        Some(crate::broker::pi_rpc::PiRpcClient::from_child(&mut child).unwrap());
    assert_eq!(
        dispatch_command(&json!({"cmd":"keys","seq":"\\x1b"}), &state, &env, &stop),
        json!({"ok":true,"queued":false,"n":1})
    );
    assert!(marker.exists());
    assert_eq!(state.lock().unwrap().last_turn_id, None);
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn pi_live_messages_coalesce_stream_deltas() {
    let dir = TempDir::new().unwrap();
    let env = test_env();
    let state = test_state(dir.path(), "pi");
    let stop = Arc::new(AtomicBool::new(false));
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg("printf '%s\n' '{\"type\":\"message.delta\",\"turn_id\":\"turn-1\",\"delta\":\"Hel\"}' '{\"type\":\"message.delta\",\"turn_id\":\"turn-1\",\"delta\":\"lo\"}' '{\"type\":\"turn.completed\",\"turn_id\":\"turn-1\"}'; sleep 2")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    state.lock().unwrap().pi_rpc =
        Some(crate::broker::pi_rpc::PiRpcClient::from_child(&mut child).unwrap());
    std::thread::sleep(std::time::Duration::from_millis(100));
    let resp = dispatch_command(
        &json!({"cmd":"live_messages","offset":0}),
        &state,
        &env,
        &stop,
    );
    assert_eq!(resp["offset"], 3);
    let events = resp["events"].as_array().unwrap();
    assert!(events
        .iter()
        .any(|event| event["text"] == "Hello" && event["completed"] == true));
    let _ = child.kill();
    let _ = child.wait();
}
