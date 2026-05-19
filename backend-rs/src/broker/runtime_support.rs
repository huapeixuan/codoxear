use crate::broker::meta::{
    codex_sidecar_value, pi_sidecar_value, write_sidecar_atomic, CodexMetaInput,
};
use crate::broker::runtime::{BrokerEnv, State};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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

pub fn resume_session_id_from_args(backend: &str, args: &[String]) -> Option<String> {
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

pub fn seq_bytes(raw: &str) -> Vec<u8> {
    match raw {
        "\\r" => b"\r".to_vec(),
        "\\n" => b"\n".to_vec(),
        "\\x1b" => b"\x1b".to_vec(),
        _ => raw.as_bytes().to_vec(),
    }
}

pub fn terminate_process_group(pid: u32) {
    if pid == 0 {
        return;
    }
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGTERM);
    }
}

pub fn broker_token() -> String {
    format!("broker-{}", std::process::id())
}

pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn clean_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn home_dir() -> PathBuf {
    clean_env("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn shell_quote(raw: &str) -> String {
    if raw
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | ':' | '='))
    {
        return raw.to_string();
    }
    format!("'{}'", raw.replace('\'', "'\\''"))
}
