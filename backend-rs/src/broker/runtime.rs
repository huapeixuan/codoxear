use self::commands::{handle_keys, handle_live_messages, handle_send};
use crate::broker::codex_state::refresh_codex_log_state;
use crate::broker::codex_state::start_codex_log_watcher;
use crate::broker::config::{normalize_backend, BrokerCli};
use crate::broker::pi_live::{PiLiveRuntime, PiLiveState};
use crate::broker::runtime_support::{
    broker_token, clean_env, home_dir, now_secs, resume_session_id_from_args, shell_quote,
    terminate_process_group, write_meta,
};
use serde_json::{json, Value};
use std::ffi::CString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";
#[derive(Debug, Clone)]
pub struct BrokerEnv {
    pub backend: String,
    pub owner: Option<String>,
    pub spawn_nonce: Option<String>,
    pub transport: Option<String>,
    pub tmux_session: Option<String>,
    pub tmux_window: Option<String>,
    pub app_dir: PathBuf,
    pub codex_home: PathBuf,
    pub pi_home: PathBuf,
    pub codex_bin: String,
    pub pi_bin: String,
    pub model_provider: Option<String>,
    pub preferred_auth_method: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
    pub debug: bool,
}

impl BrokerEnv {
    pub fn from_process(backend_hint: Option<&str>) -> Self {
        let backend = normalize_backend(
            std::env::var("CODEX_WEB_AGENT_BACKEND")
                .ok()
                .as_deref()
                .or(backend_hint),
        );
        let home = home_dir();
        let app_dir = clean_env("CODOXEAR_APP_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share/codoxear"));
        let codex_home = clean_env("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        let pi_home = clean_env("PI_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".pi"));
        Self {
            backend,
            owner: clean_env("CODEX_WEB_OWNER"),
            spawn_nonce: clean_env("CODEX_WEB_SPAWN_NONCE"),
            transport: clean_env("CODEX_WEB_TRANSPORT"),
            tmux_session: clean_env("CODEX_WEB_TMUX_SESSION"),
            tmux_window: clean_env("CODEX_WEB_TMUX_WINDOW"),
            app_dir,
            codex_home,
            pi_home,
            codex_bin: clean_env("CODEX_BIN").unwrap_or_else(|| "codex".to_string()),
            pi_bin: clean_env("PI_BIN").unwrap_or_else(|| "pi".to_string()),
            model_provider: clean_env("CODEX_WEB_MODEL_PROVIDER"),
            preferred_auth_method: clean_env("CODEX_WEB_PREFERRED_AUTH_METHOD"),
            model: clean_env("CODEX_WEB_MODEL"),
            reasoning_effort: clean_env("CODEX_WEB_REASONING_EFFORT")
                .map(|v| v.to_ascii_lowercase()),
            service_tier: clean_env("CODEX_WEB_SERVICE_TIER").map(|v| v.to_ascii_lowercase()),
            debug: std::env::var("CODEX_WEB_BROKER_DEBUG").ok().as_deref() == Some("1"),
        }
    }

    pub fn socks_dir(&self) -> PathBuf {
        self.app_dir.join("socks")
    }
}

#[derive(Debug)]
pub struct State {
    pub backend: String,
    pub child_pid: u32,
    pub broker_pid: u32,
    pub cwd: PathBuf,
    pub start_ts: f64,
    pub sock_path: PathBuf,
    pub session_path: Option<PathBuf>,
    pub session_id: Option<String>,
    pub log_path: Option<PathBuf>,
    pub log_off: u64,
    pub resume_session_id: Option<String>,
    pub busy: bool,
    pub output_tail: String,
    pub stdin_eof: bool,
    pub token: Option<Value>,
    pub child_stdin: Option<std::process::ChildStdin>,
    pub pi_rpc: Option<crate::broker::pi_rpc::PiRpcClient>,
    pub last_turn_id: Option<String>,
    pub pty_master: Option<std::fs::File>,
    pub pending_ui_requests: serde_json::Map<String, Value>,
    pub pi_live: PiLiveState,
    pub prompt_sent_at: Option<std::time::Instant>,
}

pub type BrokerStateHandle = Arc<Mutex<State>>;

#[derive(Debug)]
enum ChildHandle {
    Pty { pid: u32, master: std::fs::File },
    Process { child: Child },
}

pub fn run(cli: BrokerCli) -> i32 {
    let backend_hint = cli.session_file.as_ref().map(|_| "pi");
    let env = BrokerEnv::from_process(backend_hint);
    if env.debug {
        eprintln!(
            "codoxear-broker-rs: starting backend={} cwd={}",
            env.backend,
            cli.cwd.display()
        );
    }
    match run_inner(cli, env) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("codoxear-broker-rs: {err}");
            1
        }
    }
}

fn run_inner(cli: BrokerCli, env: BrokerEnv) -> Result<i32, String> {
    fs::create_dir_all(env.socks_dir()).map_err(|err| format!("create socks dir failed: {err}"))?;
    let start_ts = now_secs();
    let token = broker_token();
    let sock_path = env.socks_dir().join(format!("{token}.sock"));
    let resume_session_id = resume_session_id_from_args(&env.backend, &cli.agent_args)
        .or_else(|| clean_env("CODEX_WEB_RESUME_SESSION_ID"));

    let mut child = if env.backend == "pi" {
        spawn_pi_process(&cli, &env)?
    } else {
        spawn_codex_pty(&cli, &env)?
    };
    let (child_pid, child_stdin, pty_master) = match &mut child {
        ChildHandle::Pty { pid, master } => (
            *pid,
            None,
            Some(master.try_clone().map_err(|e| e.to_string())?),
        ),
        ChildHandle::Process { child } => (child.id(), None, None),
    };

    let state = Arc::new(Mutex::new(State {
        backend: env.backend.clone(),
        child_pid,
        broker_pid: std::process::id(),
        cwd: cli.cwd.clone(),
        start_ts,
        sock_path: sock_path.clone(),
        session_path: cli.session_file.clone(),
        session_id: None,
        log_path: None,
        log_off: 0,
        resume_session_id,
        busy: false,
        output_tail: String::new(),
        stdin_eof: false,
        token: None,
        child_stdin,
        pi_rpc: None,
        last_turn_id: None,
        pty_master: pty_master.as_ref().and_then(|f| f.try_clone().ok()),
        pending_ui_requests: serde_json::Map::new(),
        pi_live: PiLiveState::default(),
        prompt_sent_at: None,
    }));
    if env.backend == "pi" {
        if let ChildHandle::Process { child } = &mut child {
            let rpc = crate::broker::pi_rpc::PiRpcClient::from_child(child)?;
            state.lock().map_err(|_| "broker state poisoned")?.pi_rpc = Some(rpc);
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    start_socket_server(state.clone(), env.clone(), stop.clone())?;
    write_meta(&state, &env)?;
    start_output_reader(&mut child, state.clone(), stop.clone());
    if let ChildHandle::Pty { master, .. } = &child {
        if let Ok(stdin_master) = master.try_clone() {
            crate::broker::pty::start_terminal_bridge(stdin_master, state.clone(), stop.clone());
        }
        crate::broker::pty::install_sigwinch_resize(master.as_raw_fd());
    }
    if env.backend == "pi" {
        crate::broker::pty::install_pi_sigint_handler(state.clone());
    }
    if env.backend == "codex" {
        start_codex_log_watcher(state.clone(), env.clone(), stop.clone());
    }

    let code = wait_child(child, stop.clone());
    stop.store(true, Ordering::SeqCst);
    let _ = fs::remove_file(&sock_path);
    Ok(code)
}

fn spawn_pi_process(cli: &BrokerCli, env: &BrokerEnv) -> Result<ChildHandle, String> {
    let mut command = Command::new(&env.pi_bin);
    let args = pi_rpc_args(cli);
    command.args(&args);
    command.current_dir(&cli.cwd);
    command.env("PI_HOME", &env.pi_home);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .map(|child| ChildHandle::Process { child })
        .map_err(|err| format!("spawn pi failed: {err}"))
}

fn pi_rpc_args(cli: &BrokerCli) -> Vec<String> {
    let mut args = vec!["--mode".to_string(), "rpc".to_string()];
    if let Some(session_file) = &cli.session_file {
        args.extend([
            "--session".to_string(),
            session_file.to_string_lossy().to_string(),
        ]);
    }
    args.extend(ensure_pi_ask_user_extension(&cli.agent_args));
    args
}

fn ensure_pi_ask_user_extension(args: &[String]) -> Vec<String> {
    let bridge = repo_pi_ask_user_bridge_path();
    for pair in args.windows(2) {
        if pair[0] == "-e" && pair[1] == bridge {
            return args.to_vec();
        }
    }
    let mut out = vec!["-e".to_string(), bridge];
    out.extend(args.iter().cloned());
    out
}

fn repo_pi_ask_user_bridge_path() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("codoxear")
        .join("pi_extensions")
        .join("ask_user_bridge.ts")
        .to_string_lossy()
        .to_string()
}

fn spawn_codex_pty(cli: &BrokerCli, env: &BrokerEnv) -> Result<ChildHandle, String> {
    let (rows, cols) = crate::broker::pty::terminal_size();
    let argv = codex_exec_argv(cli, env);
    let mut master_fd: libc::c_int = -1;
    let mut winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pid = unsafe {
        libc::forkpty(
            &mut master_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut winsize as *mut libc::winsize,
        )
    };
    if pid < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if pid == 0 {
        child_exec(&argv, &cli.cwd, env, rows, cols);
    }
    unsafe { libc::setpgid(pid, pid) };
    let master = unsafe { std::fs::File::from_raw_fd(master_fd) };
    Ok(ChildHandle::Pty {
        pid: pid as u32,
        master,
    })
}

fn codex_exec_argv(cli: &BrokerCli, env: &BrokerEnv) -> Vec<String> {
    if env.owner.as_deref() == Some("web") {
        let shell = clean_env("SHELL").unwrap_or_else(|| "/bin/zsh".to_string());
        let mut parts = vec![shell, "-l".to_string(), "-i".to_string(), "-c".to_string()];
        let mut cmd = vec![env.codex_bin.clone()];
        cmd.extend(cli.agent_args.iter().cloned());
        parts.push(format!(
            "exec {}",
            cmd.iter()
                .map(|arg| shell_quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        parts
    } else {
        let mut argv = vec![env.codex_bin.clone()];
        argv.extend(cli.agent_args.iter().cloned());
        argv
    }
}

fn child_exec(argv: &[String], cwd: &Path, env: &BrokerEnv, rows: u16, cols: u16) -> ! {
    unsafe { libc::setpgid(0, 0) };
    let _ = std::env::set_current_dir(cwd);
    std::env::set_var(
        "TERM",
        clean_env("TERM").unwrap_or_else(|| "xterm-256color".to_string()),
    );
    std::env::set_var("COLUMNS", cols.to_string());
    std::env::set_var("LINES", rows.to_string());
    std::env::set_var("CODEX_HOME", &env.codex_home);
    if argv.is_empty() {
        unsafe { libc::_exit(127) }
    }
    let cstrings = match argv
        .iter()
        .map(|arg| CString::new(arg.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(_) => unsafe { libc::_exit(127) },
    };
    let mut ptrs: Vec<*const libc::c_char> = cstrings.iter().map(|s| s.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    unsafe {
        libc::execvp(cstrings[0].as_ptr(), ptrs.as_ptr());
        libc::_exit(127);
    }
}

fn start_output_reader(child: &mut ChildHandle, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>) {
    match child {
        ChildHandle::Pty { master, .. } => {
            let Ok(reader_file) = master.try_clone() else {
                return;
            };
            thread::spawn(move || {
                let mut reader = std::io::BufReader::new(reader_file);
                let mut buf = [0u8; 4096];
                loop {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    match std::io::Read::read(&mut reader, &mut buf) {
                        Ok(0) => break,
                        Ok(n) => append_tail(&state, &String::from_utf8_lossy(&buf[..n])),
                        Err(_) => break,
                    }
                }
            });
        }
        ChildHandle::Process { .. } => {}
    }
}

fn append_tail(state: &Arc<Mutex<State>>, text: &str) {
    let mut st = state.lock().expect("broker state poisoned");
    st.output_tail.push_str(text);
    if st.output_tail.len() > 256 * 1024 {
        let keep_from = st.output_tail.len() - 256 * 1024;
        st.output_tail = st.output_tail[keep_from..].to_string();
    }
}

fn start_socket_server(
    state: Arc<Mutex<State>>,
    env: BrokerEnv,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let sock_path = state
        .lock()
        .expect("broker state poisoned")
        .sock_path
        .clone();
    let _ = fs::remove_file(&sock_path);
    let listener =
        UnixListener::bind(&sock_path).map_err(|err| format!("bind socket failed: {err}"))?;
    fs::set_permissions(&sock_path, fs::Permissions::from_mode(0o600))
        .map_err(|err| format!("chmod socket failed: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("set socket nonblocking failed: {err}"))?;
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let state2 = state.clone();
                    let env2 = env.clone();
                    let stop2 = stop.clone();
                    thread::spawn(move || handle_conn(stream, state2, env2, stop2));
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50))
                }
                Err(_) => break,
            }
        }
    });
    Ok(())
}

fn handle_conn(
    mut stream: UnixStream,
    state: Arc<Mutex<State>>,
    env: BrokerEnv,
    stop: Arc<AtomicBool>,
) {
    let cloned = match stream.try_clone() {
        Ok(cloned) => cloned,
        Err(_) => return,
    };
    let mut reader = BufReader::new(cloned);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        return;
    }
    let req: Value = serde_json::from_str(&line).unwrap_or_else(|_| json!({}));
    let resp = dispatch_command(&req, &state, &env, &stop);
    let _ = writeln!(
        stream,
        "{}",
        serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string())
    );
}

pub(super) fn dispatch_command(
    req: &Value,
    state: &Arc<Mutex<State>>,
    env: &BrokerEnv,
    stop: &Arc<AtomicBool>,
) -> Value {
    match req.get("cmd").and_then(Value::as_str) {
        Some("state") => {
            sync_pi_state(state, env);
            let st = state.lock().expect("broker state poisoned");
            json!({"busy": st.busy, "queue_len": 0, "token": st.token})
        }
        Some("tail") => {
            if env.backend == "pi" {
                drain_pi_output(state);
            } else {
                let _ = refresh_codex_log_state(state, env);
            }
            let st = state.lock().expect("broker state poisoned");
            json!({"tail": st.output_tail})
        }
        Some("send") => handle_send(req, state),
        Some("keys") => handle_keys(req, state),
        Some("live_messages") if env.backend == "pi" => handle_live_messages(req, state),
        Some("ui_state") if env.backend == "pi" => {
            drain_pi_output(state);
            let st = state.lock().expect("broker state poisoned");
            let requests: Vec<Value> = st
                .pending_ui_requests
                .values()
                .filter(|value| value.get("status").and_then(Value::as_str) == Some("pending"))
                .cloned()
                .collect();
            json!({"requests": requests})
        }
        Some("commands") if env.backend == "pi" => {
            let rpc = state.lock().expect("broker state poisoned").pi_rpc.clone();
            let commands = rpc
                .as_ref()
                .and_then(|rpc| rpc.get_commands().ok())
                .unwrap_or_default();
            json!({"commands": commands})
        }
        Some("ui_response") if env.backend == "pi" => {
            let Some(id) = req
                .get("id")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
            else {
                return json!({"error": "id required"});
            };
            {
                let mut st = state.lock().expect("broker state poisoned");
                let Some(pending) = st.pending_ui_requests.get_mut(id) else {
                    return json!({"error": "unknown or expired request"});
                };
                if pending.get("status").and_then(Value::as_str) != Some("pending") {
                    return json!({"error": "request already resolved"});
                }
                pending["status"] = json!("resolved");
            }
            let rpc = state.lock().expect("broker state poisoned").pi_rpc.clone();
            let send_result = rpc.as_ref().map(|rpc| rpc.send_ui_response(id, req));
            if let Some(Err(err)) = send_result {
                let mut st = state.lock().expect("broker state poisoned");
                if let Some(pending) = st.pending_ui_requests.get_mut(id) {
                    pending["status"] = json!("pending");
                }
                return json!({"error": err});
            }
            json!({"ok": true})
        }
        Some("shutdown") => {
            let pid = state.lock().expect("broker state poisoned").child_pid;
            stop.store(true, Ordering::SeqCst);
            terminate_process_group(pid);
            json!({"ok": true})
        }
        _ => json!({"error": "unknown cmd"}),
    }
}

fn sync_pi_state(state: &Arc<Mutex<State>>, env: &BrokerEnv) {
    if env.backend != "pi" {
        return;
    }
    let rpc = state.lock().expect("broker state poisoned").pi_rpc.clone();
    let rpc_state = rpc.as_ref().and_then(|rpc| rpc.get_state().ok());
    drain_pi_output(state);
    let Some(rpc_state) = rpc_state else {
        return;
    };
    let mut rewrite_meta = false;
    {
        let mut st = state.lock().expect("broker state poisoned");
        if let Some(mut busy) = rpc_state.get("busy").and_then(Value::as_bool) {
            if st.busy
                && !busy
                && st
                    .prompt_sent_at
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
            {
                busy = true;
            }
            st.busy = busy;
            if !busy {
                st.prompt_sent_at = None;
            }
        }
        if let Some(turn_id) = rpc_state.get("turn_id").and_then(Value::as_str) {
            st.last_turn_id = Some(turn_id.to_string());
        }
        if let Some(session_id) = rpc_state.get("session_id").and_then(Value::as_str) {
            if st.session_id.as_deref() != Some(session_id) {
                st.session_id = Some(session_id.to_string());
                rewrite_meta = true;
            }
        }
    }
    if rewrite_meta {
        let _ = write_meta(state, env);
    }
}

pub(super) fn drain_pi_output(state: &Arc<Mutex<State>>) {
    let rpc = state.lock().expect("broker state poisoned").pi_rpc.clone();
    let Some(rpc) = rpc else {
        return;
    };
    let (events, stderr_lines) = (rpc.drain_events(), rpc.drain_stderr_lines());
    let mut st = state.lock().expect("broker state poisoned");
    for line in stderr_lines {
        st.output_tail.push_str("[stderr] ");
        st.output_tail.push_str(&line);
        st.output_tail.push('\n');
    }
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
        for event in events {
            crate::broker::pi_live::record_event(&mut live, &event);
        }
    }
    st.busy = busy;
    st.last_turn_id = last_turn_id;
    st.pending_ui_requests = pending_ui_requests;
    st.pi_live = pi_live;
    st.output_tail = output_tail;
    if st.output_tail.len() > 64 * 1024 {
        let keep_from = st.output_tail.len() - 64 * 1024;
        st.output_tail = st.output_tail[keep_from..].to_string();
    }
}

fn wait_child(child: ChildHandle, stop: Arc<AtomicBool>) -> i32 {
    match child {
        ChildHandle::Pty { pid, .. } => loop {
            if stop.load(Ordering::SeqCst) {
                terminate_process_group(pid);
            }
            let mut status: libc::c_int = 0;
            let result = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
            if result == pid as libc::pid_t {
                if libc::WIFEXITED(status) {
                    return libc::WEXITSTATUS(status);
                }
                if libc::WIFSIGNALED(status) {
                    return 128 + libc::WTERMSIG(status);
                }
                return 1;
            }
            if result < 0 {
                return 1;
            }
            thread::sleep(Duration::from_millis(100));
        },
        ChildHandle::Process { mut child } => loop {
            if stop.load(Ordering::SeqCst) {
                terminate_process_group(child.id());
                let _ = child.kill();
            }
            match child.try_wait() {
                Ok(Some(status)) => return status.code().unwrap_or(1),
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(_) => return 1,
            }
        },
    }
}

#[path = "runtime/commands.rs"]
mod commands;

#[cfg(test)]
#[path = "runtime/runtime_tests.rs"]
mod runtime_tests;
