use crate::broker::codex_state::refresh_codex_log_state;
use crate::broker::codex_state::start_codex_log_watcher;
use crate::broker::config::{normalize_backend, BrokerCli};
use crate::broker::meta::{
    codex_sidecar_value, pi_sidecar_value, write_sidecar_atomic, CodexMetaInput,
};
use serde_json::{json, Value};
use std::ffi::CString;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    pub token: Option<Value>,
    pub child_stdin: Option<std::process::ChildStdin>,
    pub pty_master: Option<std::fs::File>,
    pub pending_ui_requests: serde_json::Map<String, Value>,
    pub live_message_offset: u64,
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
        ChildHandle::Process { child } => (child.id(), child.stdin.take(), None),
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
        token: None,
        child_stdin,
        pty_master: pty_master.as_ref().and_then(|f| f.try_clone().ok()),
        pending_ui_requests: serde_json::Map::new(),
        live_message_offset: 0,
    }));
    write_meta(&state, &env)?;

    let stop = Arc::new(AtomicBool::new(false));
    start_socket_server(state.clone(), env.clone(), stop.clone())?;
    start_output_reader(&mut child, state.clone(), stop.clone());
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
    command.args(["--mode", "rpc"]);
    if let Some(session_file) = &cli.session_file {
        command.args([
            "--session".to_string(),
            session_file.to_string_lossy().to_string(),
        ]);
    }
    command.args(&cli.agent_args);
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

fn spawn_codex_pty(cli: &BrokerCli, env: &BrokerEnv) -> Result<ChildHandle, String> {
    let rows = terminal_size().0;
    let cols = terminal_size().1;
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
            &mut winsize,
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
        ChildHandle::Process { child } => {
            if let Some(stdout) = child.stdout.take() {
                let state2 = state.clone();
                let stop2 = stop.clone();
                thread::spawn(move || read_pipe(stdout, state2, stop2));
            }
            if let Some(stderr) = child.stderr.take() {
                thread::spawn(move || read_pipe(stderr, state, stop));
            }
        }
    }
}

fn read_pipe<R: std::io::Read>(reader: R, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>) {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    while !stop.load(Ordering::SeqCst) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => append_tail(&state, &line),
            Err(_) => break,
        }
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

fn dispatch_command(
    req: &Value,
    state: &Arc<Mutex<State>>,
    env: &BrokerEnv,
    stop: &Arc<AtomicBool>,
) -> Value {
    match req.get("cmd").and_then(Value::as_str) {
        Some("state") => {
            let st = state.lock().expect("broker state poisoned");
            json!({"busy": st.busy, "queue_len": 0, "token": st.token})
        }
        Some("tail") => {
            let _ = refresh_codex_log_state(state, env);
            let st = state.lock().expect("broker state poisoned");
            json!({"tail": st.output_tail})
        }
        Some("send") => {
            let Some(text) = req
                .get("text")
                .and_then(Value::as_str)
                .filter(|v| !v.trim().is_empty())
            else {
                return json!({"error": "text required"});
            };
            let mut st = state.lock().expect("broker state poisoned");
            st.busy = true;
            if st.backend == "pi" {
                if let Some(stdin) = &mut st.child_stdin {
                    let _ = writeln!(stdin, "{text}");
                }
            } else if let Some(master) = &mut st.pty_master {
                let enter = req
                    .get("enter_seq")
                    .and_then(Value::as_str)
                    .map(seq_bytes)
                    .unwrap_or_else(|| {
                        seq_bytes(
                            &std::env::var("CODEX_WEB_ENTER_SEQ")
                                .unwrap_or_else(|_| "\r".to_string()),
                        )
                    });
                let _ = master.write_all(BRACKETED_PASTE_START);
                let _ = master.write_all(text.as_bytes());
                let _ = master.write_all(BRACKETED_PASTE_END);
                let _ = master.write_all(&enter);
            }
            json!({"queued": false, "queue_len": 0})
        }
        Some("keys") => {
            let Some(seq) = req
                .get("seq")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
            else {
                return json!({"error": "seq required"});
            };
            let bytes = seq_bytes(seq);
            let mut st = state.lock().expect("broker state poisoned");
            if st.backend == "pi" && bytes != b"\x1b" {
                return json!({"error": format!("unsupported key sequence: {seq}")});
            }
            if let Some(master) = &mut st.pty_master {
                let _ = master.write_all(&bytes);
            }
            json!({"ok": true, "queued": false, "n": bytes.len()})
        }
        Some("live_messages") if env.backend == "pi" => {
            let st = state.lock().expect("broker state poisoned");
            json!({"offset": st.live_message_offset, "events": []})
        }
        Some("ui_state") if env.backend == "pi" => {
            let st = state.lock().expect("broker state poisoned");
            let requests: Vec<Value> = st
                .pending_ui_requests
                .values()
                .filter(|value| value.get("status").and_then(Value::as_str) == Some("pending"))
                .cloned()
                .collect();
            json!({"requests": requests})
        }
        Some("commands") if env.backend == "pi" => json!({"commands": []}),
        Some("ui_response") if env.backend == "pi" => {
            let Some(id) = req
                .get("id")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
            else {
                return json!({"error": "id required"});
            };
            let mut st = state.lock().expect("broker state poisoned");
            let Some(pending) = st.pending_ui_requests.get_mut(id) else {
                return json!({"error": "unknown or expired request"});
            };
            if pending.get("status").and_then(Value::as_str) != Some("pending") {
                return json!({"error": "request already resolved"});
            }
            pending["status"] = json!("resolved");
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

pub fn write_meta(state: &Arc<Mutex<State>>, env: &BrokerEnv) -> Result<(), String> {
    let st = state.lock().expect("broker state poisoned");
    let input = CodexMetaInput {
        session_id: st.session_id.clone(),
        owner: env.owner.clone(),
        broker_pid: st.broker_pid,
        codex_pid: st.child_pid,
        cwd: st.cwd.to_string_lossy().to_string(),
        start_ts: st.start_ts,
        log_path: st.log_path.clone(),
        sock_path: st.sock_path.clone(),
        resume_session_id: st.resume_session_id.clone(),
        transport: env.transport.clone(),
        tmux_session: env.tmux_session.clone(),
        tmux_window: env.tmux_window.clone(),
        spawn_nonce: env.spawn_nonce.clone(),
    };
    let mut value = if env.backend == "pi" {
        pi_sidecar_value(&input, st.session_path.as_deref())
    } else {
        codex_sidecar_value(&input)
    };
    if env.backend == "codex" {
        value["model_provider"] = json!(env.model_provider);
        value["preferred_auth_method"] = json!(env.preferred_auth_method);
        value["model"] = json!(env.model);
        value["reasoning_effort"] = json!(env.reasoning_effort);
        value["service_tier"] = json!(env.service_tier);
    }
    let meta_path = st.sock_path.with_extension("json");
    write_sidecar_atomic(&meta_path, &value).map_err(|err| format!("write sidecar failed: {err}"))
}

fn resume_session_id_from_args(backend: &str, args: &[String]) -> Option<String> {
    if backend == "pi" {
        args.windows(2)
            .find(|pair| {
                pair[0] == "--session" && !pair[1].trim().is_empty() && !pair[1].ends_with(".jsonl")
            })
            .map(|pair| pair[1].clone())
    } else {
        args.windows(2)
            .find(|pair| pair[0] == "resume" && !pair[1].trim().is_empty())
            .map(|pair| pair[1].clone())
    }
}

fn seq_bytes(raw: &str) -> Vec<u8> {
    match raw {
        "\\r" => b"\r".to_vec(),
        "\\n" => b"\n".to_vec(),
        "\\x1b" => b"\x1b".to_vec(),
        _ => raw.as_bytes().to_vec(),
    }
}

fn terminal_size() -> (u16, u16) {
    let mut size = libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let ok = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut size) } == 0;
    if ok && size.ws_row > 0 && size.ws_col > 0 {
        (size.ws_row, size.ws_col)
    } else {
        (40, 120)
    }
}

fn terminate_process_group(pid: u32) {
    if pid == 0 {
        return;
    }
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGTERM);
    }
}

fn broker_token() -> String {
    format!("broker-{}", std::process::id())
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

fn clean_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn home_dir() -> PathBuf {
    clean_env("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn shell_quote(raw: &str) -> String {
    if raw
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '='))
    {
        return raw.to_string();
    }
    format!("'{}'", raw.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
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
            token: None,
            child_stdin: None,
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
}
