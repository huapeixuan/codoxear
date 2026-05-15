use serde_json::{json, Value};
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CodexMetaInput {
    pub session_id: Option<String>,
    pub owner: Option<String>,
    pub broker_pid: u32,
    pub codex_pid: u32,
    pub cwd: String,
    pub start_ts: f64,
    pub log_path: Option<PathBuf>,
    pub sock_path: PathBuf,
    pub resume_session_id: Option<String>,
    pub transport: Option<String>,
    pub tmux_session: Option<String>,
    pub tmux_window: Option<String>,
    pub spawn_nonce: Option<String>,
}

pub fn codex_sidecar_value(input: &CodexMetaInput) -> Value {
    json!({
        "session_id": input.session_id,
        "backend": "codex",
        "owner": input.owner,
        "supports_web_control": true,
        "broker_pid": input.broker_pid,
        "sessiond_pid": 0,
        "codex_pid": input.codex_pid,
        "cwd": input.cwd,
        "start_ts": input.start_ts,
        "log_path": input.log_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "sock_path": input.sock_path.to_string_lossy().to_string(),
        "agent_backend": "codex",
        "resume_session_id": input.resume_session_id,
        "model_provider": null,
        "preferred_auth_method": null,
        "model": null,
        "reasoning_effort": null,
        "service_tier": null,
        "transport": input.transport,
        "tmux_session": input.tmux_session,
        "tmux_window": input.tmux_window,
        "spawn_nonce": input.spawn_nonce,
    })
}

pub fn pi_sidecar_value(input: &CodexMetaInput, session_path: Option<&Path>) -> Value {
    let mut value = json!({
        "session_id": input.session_id,
        "backend": "pi",
        "transport": "pi-rpc",
        "tmux_session": input.tmux_session,
        "tmux_window": input.tmux_window,
        "owner": input.owner,
        "supports_web_control": true,
        "supports_live_ui": true,
        "ui_protocol_version": 1,
        "broker_pid": input.broker_pid,
        "agent_pid": input.codex_pid,
        "codex_pid": input.codex_pid,
        "cwd": input.cwd,
        "start_ts": input.start_ts,
        "log_path": null,
        "sock_path": input.sock_path.to_string_lossy().to_string(),
        "resume_session_id": input.resume_session_id,
        "spawn_nonce": input.spawn_nonce,
        "agent_backend": "pi",
    });
    if let Some(session_path) = session_path {
        value["session_path"] = json!(session_path.to_string_lossy().to_string());
    }
    value
}

pub fn write_sidecar_atomic(path: &Path, value: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    {
        let mut file = File::create(&tmp)?;
        file.write_all(serde_json::to_string(value)?.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn input(sock_path: PathBuf) -> CodexMetaInput {
        CodexMetaInput {
            session_id: Some("sid".to_string()),
            owner: Some("web".to_string()),
            broker_pid: 10,
            codex_pid: 11,
            cwd: "/repo".to_string(),
            start_ts: 1.25,
            log_path: None,
            sock_path,
            resume_session_id: None,
            transport: None,
            tmux_session: None,
            tmux_window: None,
            spawn_nonce: Some("nonce".to_string()),
        }
    }

    #[test]
    fn codex_sidecar_contains_contract_keys() {
        let value = codex_sidecar_value(&input(PathBuf::from("/tmp/sid.sock")));
        for key in [
            "session_id",
            "backend",
            "owner",
            "supports_web_control",
            "broker_pid",
            "sessiond_pid",
            "codex_pid",
            "cwd",
            "start_ts",
            "log_path",
            "sock_path",
            "agent_backend",
            "resume_session_id",
            "model_provider",
            "preferred_auth_method",
            "model",
            "reasoning_effort",
            "service_tier",
            "transport",
            "tmux_session",
            "tmux_window",
            "spawn_nonce",
        ] {
            assert!(value.get(key).is_some(), "missing key {key}");
        }
        assert_eq!(value["agent_backend"], "codex");
    }

    #[test]
    fn pi_sidecar_keeps_codex_pid_and_session_path_compatibility() {
        let value = pi_sidecar_value(
            &input(PathBuf::from("/tmp/pi.sock")),
            Some(Path::new("/tmp/pi.jsonl")),
        );
        assert_eq!(value["backend"], "pi");
        assert_eq!(value["transport"], "pi-rpc");
        assert_eq!(value["agent_backend"], "pi");
        assert_eq!(value["codex_pid"], 11);
        assert_eq!(value["session_path"], "/tmp/pi.jsonl");
    }

    #[test]
    fn writer_uses_0600_mode() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sid.json");
        write_sidecar_atomic(&path, &json!({"ok": true})).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
