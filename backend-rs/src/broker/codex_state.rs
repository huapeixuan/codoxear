use crate::broker::runtime::{write_meta, BrokerEnv, BrokerStateHandle};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const SESSION_ID_RE_LEN: usize = 36;

pub fn start_codex_log_watcher(state: BrokerStateHandle, env: BrokerEnv, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let (pid, current_log, codex_home) = {
                let st = state.lock().expect("broker state poisoned");
                (st.child_pid, st.log_path.clone(), env.codex_home.clone())
            };
            if current_log.is_none() {
                if let Some(path) = discover_codex_log_for_pid(pid, &codex_home) {
                    let _ = register_codex_log(&state, &env, path);
                }
            } else {
                let _ = refresh_codex_log_state(&state, &env);
            }
            thread::sleep(Duration::from_millis(250));
        }
    });
}

fn discover_codex_log_for_pid(pid: u32, codex_home: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<_> = discover_open_logs(pid)
        .unwrap_or_else(|_| HashSet::new())
        .into_iter()
        .collect();
    if candidates.is_empty() {
        candidates.extend(scan_codex_session_logs(codex_home));
    }
    choose_codex_log(candidates, codex_home)
}

#[cfg(target_os = "linux")]
fn discover_open_logs(pid: u32) -> Result<HashSet<PathBuf>, String> {
    Ok(crate::broker::log_discovery::proc_open_jsonl_logs(
        Path::new("/proc"),
        pid,
        unsafe { libc::geteuid() },
    ))
}

#[cfg(target_os = "macos")]
fn discover_open_logs(pid: u32) -> Result<HashSet<PathBuf>, String> {
    let pids = crate::broker::log_discovery::macos_descendants(pid, |parent| {
        std::process::Command::new("pgrep")
            .args(["-P", &parent.to_string()])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).to_string())
            .map_err(|err| err.to_string())
    })?;
    crate::broker::log_discovery::macos_open_jsonl_logs(&pids, |pid| {
        std::process::Command::new("lsof")
            .args(["-p", &pid.to_string(), "-F", "n"])
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).to_string())
            .map_err(|err| err.to_string())
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn discover_open_logs(_pid: u32) -> Result<HashSet<PathBuf>, String> {
    Ok(HashSet::new())
}

pub fn choose_codex_log(mut candidates: Vec<PathBuf>, codex_home: &Path) -> Option<PathBuf> {
    let sessions = codex_home.join("sessions");
    candidates.retain(|path| {
        path.file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
    });
    candidates.retain(|path| path.starts_with(&sessions));
    candidates.retain(|path| !is_subagent_log(path));
    candidates.sort_by_key(|path| fs::metadata(path).and_then(|m| m.modified()).ok());
    candidates.pop()
}

fn scan_codex_session_logs(codex_home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_rollout_logs(&codex_home.join("sessions"), &mut out);
    out
}

fn collect_rollout_logs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rollout_logs(&path, out);
        } else if path
            .file_name()
            .and_then(|v| v.to_str())
            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
        {
            out.push(path);
        }
    }
}

pub fn register_codex_log(
    state: &BrokerStateHandle,
    env: &BrokerEnv,
    path: PathBuf,
) -> Result<(), String> {
    let Some(session_id) =
        read_session_id_from_log(&path).or_else(|| session_id_from_filename(&path))
    else {
        return Ok(());
    };
    {
        let mut st = state.lock().expect("broker state poisoned");
        st.session_id = Some(session_id);
        st.log_path = Some(path.clone());
        st.log_off = 0;
    }
    write_meta(state, env)
}

pub fn refresh_codex_log_state(state: &BrokerStateHandle, env: &BrokerEnv) -> Result<(), String> {
    let (path, off) = {
        let st = state.lock().expect("broker state poisoned");
        let Some(path) = st.log_path.clone() else {
            return Ok(());
        };
        (path, st.log_off)
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let new_off = raw.len() as u64;
    if new_off == off {
        return Ok(());
    }
    let mut token = None;
    let mut busy = None;
    for line in raw.lines() {
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(update) = token_update_from_obj(&obj) {
            token = Some(update);
        }
        if let Some(next_busy) = busy_from_obj(&obj) {
            busy = Some(next_busy);
        }
    }
    {
        let mut st = state.lock().expect("broker state poisoned");
        st.log_off = new_off;
        if let Some(token) = token {
            st.token = Some(token);
        }
        if let Some(busy) = busy {
            st.busy = busy;
        }
    }
    write_meta(state, env)
}

fn token_update_from_obj(obj: &Value) -> Option<Value> {
    let payload = obj.get("payload")?.as_object()?;
    if obj.get("type").and_then(Value::as_str) != Some("event_msg")
        || payload.get("type").and_then(Value::as_str) != Some("token_count")
    {
        return None;
    }
    let info = payload.get("info")?.as_object()?;
    let ctx = info.get("model_context_window")?.as_i64()?;
    let last = info.get("last_token_usage")?.as_object()?;
    let total = last.get("total_tokens")?.as_i64()?;
    Some(json!({
        "context_window": ctx,
        "tokens_in_context": total,
        "tokens_remaining": (ctx - total).max(0),
        "percent_remaining": context_percent_remaining(total, ctx),
        "baseline_tokens": crate::log_normalizer::pi::CONTEXT_WINDOW_BASELINE_TOKENS,
        "as_of": obj.get("timestamp").and_then(Value::as_str),
    }))
}

fn busy_from_obj(obj: &Value) -> Option<bool> {
    match obj.get("type").and_then(Value::as_str) {
        Some("event_msg") => match obj.pointer("/payload/type").and_then(Value::as_str) {
            Some("user_message")
            | Some("agent_message")
            | Some("agent_reasoning")
            | Some("token_count") => Some(true),
            Some("turn_aborted")
            | Some("thread_rolled_back")
            | Some("task_complete")
            | Some("turn_complete") => Some(false),
            _ => None,
        },
        Some("response_item") => match obj.pointer("/payload/type").and_then(Value::as_str) {
            Some("message")
                if obj.pointer("/payload/role").and_then(Value::as_str) == Some("assistant") =>
            {
                Some(obj.pointer("/payload/end_turn").and_then(Value::as_bool) != Some(true))
            }
            Some("reasoning")
            | Some("function_call")
            | Some("custom_tool_call")
            | Some("web_search_call")
            | Some("local_shell_call") => Some(true),
            Some("function_call_output") | Some("custom_tool_call_output") => Some(true),
            _ => None,
        },
        _ => None,
    }
}

fn context_percent_remaining(tokens_in_context: i64, context_window: i64) -> i64 {
    let baseline = crate::log_normalizer::pi::CONTEXT_WINDOW_BASELINE_TOKENS;
    if context_window <= baseline {
        return 0;
    }
    let effective = context_window - baseline;
    let used = (tokens_in_context - baseline).max(0);
    (((effective - used).max(0) as f64 / effective as f64) * 100.0).round() as i64
}

fn is_subagent_log(path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    raw.lines().take(20).any(|line| {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return false;
        };
        value.pointer("/payload/source/subagent").is_some()
    })
}

fn read_session_id_from_log(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    for line in raw.lines().take(20) {
        let value = serde_json::from_str::<Value>(line).ok()?;
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        if let Some(id) = value
            .pointer("/payload/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            return Some(id.to_string());
        }
    }
    None
}

fn session_id_from_filename(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    for start in 0..name.len().saturating_sub(SESSION_ID_RE_LEN - 1) {
        let end = start + SESSION_ID_RE_LEN;
        let candidate = &name[start..end];
        if is_uuid_like(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

fn is_uuid_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == SESSION_ID_RE_LEN
        && [8, 13, 18, 23].iter().all(|idx| bytes[*idx] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, byte)| [8, 13, 18, 23].contains(&idx) || byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::runtime::State;
    use std::sync::Mutex;
    use tempfile::TempDir;

    fn test_env(dir: &Path) -> BrokerEnv {
        BrokerEnv {
            backend: "codex".into(),
            owner: Some("web".into()),
            spawn_nonce: None,
            transport: None,
            tmux_session: None,
            tmux_window: None,
            app_dir: dir.to_path_buf(),
            codex_home: dir.join("codex-home"),
            pi_home: dir.join("pi"),
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

    fn test_state(dir: &Path) -> BrokerStateHandle {
        Arc::new(Mutex::new(State {
            backend: "codex".into(),
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
            busy: false,
            output_tail: String::new(),
            token: None,
            child_stdin: None,
            pty_master: None,
            pending_ui_requests: serde_json::Map::new(),
            live_message_offset: 0,
        }))
    }

    #[test]
    fn registers_codex_log_and_updates_token_state() {
        let dir = TempDir::new().unwrap();
        let codex_home = dir.path().join("codex-home");
        let sessions = codex_home.join("sessions/2026/05/15");
        fs::create_dir_all(&sessions).unwrap();
        let sid = "12345678-1234-1234-1234-123456789abc";
        let log = sessions.join(format!("rollout-test-{sid}.jsonl"));
        fs::write(&log, format!("{}\n{}\n", json!({"type":"session_meta","payload":{"id":sid,"cwd":dir.path()}}), json!({"type":"event_msg","timestamp":"2026-05-15T00:00:00Z","payload":{"type":"token_count","info":{"model_context_window":200000,"last_token_usage":{"total_tokens":100000}}}}))).unwrap();
        let mut env = test_env(dir.path());
        env.codex_home = codex_home;
        let state = test_state(dir.path());

        register_codex_log(&state, &env, log.clone()).unwrap();
        refresh_codex_log_state(&state, &env).unwrap();

        let st = state.lock().unwrap();
        assert_eq!(st.session_id.as_deref(), Some(sid));
        assert_eq!(st.log_path.as_ref(), Some(&log));
        assert!(st.token.is_some());
    }

    #[test]
    fn ignores_subagent_log_when_choosing_codex_log() {
        let dir = TempDir::new().unwrap();
        let codex_home = dir.path().join("codex-home");
        let sessions = codex_home.join("sessions/2026/05/15");
        fs::create_dir_all(&sessions).unwrap();
        let sub = sessions.join("rollout-sub-12345678-1234-1234-1234-123456789abc.jsonl");
        let main = sessions.join("rollout-main-aaaaaaaa-1234-1234-1234-aaaaaaaaaaaa.jsonl");
        fs::write(
            &sub,
            serde_json::to_string(
                &json!({"type":"session_meta","payload":{"id":"sub","source":{"subagent":"x"}}}),
            )
            .unwrap(),
        )
        .unwrap();
        fs::write(
            &main,
            serde_json::to_string(&json!({"type":"session_meta","payload":{"id":"main"}})).unwrap(),
        )
        .unwrap();
        let chosen = choose_codex_log(vec![sub, main.clone()], &codex_home).unwrap();
        assert_eq!(chosen, main);
    }
}
