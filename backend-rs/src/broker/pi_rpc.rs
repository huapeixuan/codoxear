use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct PiRpcClient {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, Sender<Value>>>>,
    events: Arc<Mutex<Vec<Value>>>,
    stderr_lines: Arc<Mutex<Vec<String>>>,
}

impl PiRpcClient {
    pub fn from_child(child: &mut Child) -> Result<Self, String> {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "pi rpc stdin is unavailable".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "pi rpc stdout is unavailable".to_string())?;
        let stderr = child.stderr.take();
        let this = Self {
            stdin: Arc::new(Mutex::new(stdin)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            events: Arc::new(Mutex::new(Vec::new())),
            stderr_lines: Arc::new(Mutex::new(Vec::new())),
        };
        this.start_stdout_reader(stdout);
        if let Some(stderr) = stderr {
            this.start_stderr_reader(stderr);
        }
        Ok(this)
    }

    pub fn prompt(&self, text: &str, images: Option<Value>) -> Result<Value, String> {
        let mut payload = json!({"message": text});
        if let Some(images) = images.filter(|v| v.is_array()) {
            payload["images"] = images;
        }
        self.send_command("prompt", payload)
    }

    pub fn abort(&self, turn_id: Option<&str>) -> Result<Value, String> {
        let payload = turn_id
            .map(|id| json!({"turn_id": id}))
            .unwrap_or_else(|| json!({}));
        self.send_command("abort", payload)
    }

    pub fn get_state(&self) -> Result<Value, String> {
        self.send_command("get_state", json!({}))
    }

    pub fn get_commands(&self) -> Result<Vec<Value>, String> {
        let result = self.send_command("get_commands", json!({}))?;
        Ok(result
            .get("commands")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub fn send_ui_response(&self, id: &str, req: &Value) -> Result<(), String> {
        let mut payload = json!({"type":"extension_ui_response","id": id});
        if req.get("cancelled").and_then(Value::as_bool) == Some(true) {
            payload["cancelled"] = json!(true);
        } else if let Some(confirmed) = req.get("confirmed").and_then(Value::as_bool) {
            payload["confirmed"] = json!(confirmed);
        } else {
            payload["value"] = req.get("value").cloned().unwrap_or(Value::Null);
        }
        self.write_jsonl(&payload)
    }

    pub fn drain_events(&self) -> Vec<Value> {
        let Ok(mut events) = self.events.lock() else {
            return Vec::new();
        };
        std::mem::take(&mut *events)
    }

    pub fn drain_stderr_lines(&self) -> Vec<String> {
        let Ok(mut lines) = self.stderr_lines.lock() else {
            return Vec::new();
        };
        std::mem::take(&mut *lines)
    }

    fn send_command(&self, typ: &str, payload: Value) -> Result<Value, String> {
        let id = format!("pi-rs-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let mut body = json!({"id": id, "type": typ});
        if let (Some(dst), Some(src)) = (body.as_object_mut(), payload.as_object()) {
            for (key, value) in src {
                dst.insert(key.clone(), value.clone());
            }
        }
        let (tx, rx): (Sender<Value>, Receiver<Value>) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| "pending lock poisoned")?
            .insert(id.clone(), tx);
        let write_result = self.write_jsonl(&body);
        if write_result.is_err() {
            let _ = self.pending.lock().map(|mut p| p.remove(&id));
            return write_result.map(|_| Value::Null);
        }
        let response = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| format!("pi rpc {typ} timed out"));
        let _ = self.pending.lock().map(|mut p| p.remove(&id));
        let response = response?;
        let ok = response
            .get("success")
            .and_then(Value::as_bool)
            .or_else(|| response.get("ok").and_then(Value::as_bool))
            .unwrap_or(false);
        if !ok {
            return Err(response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("pi rpc command failed")
                .to_string());
        }
        Ok(response
            .get("data")
            .or_else(|| response.get("result"))
            .cloned()
            .unwrap_or_else(|| json!({})))
    }

    fn write_jsonl(&self, payload: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().map_err(|_| "stdin lock poisoned")?;
        writeln!(
            stdin,
            "{}",
            serde_json::to_string(payload).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())
    }

    fn start_stdout_reader(&self, stdout: std::process::ChildStdout) {
        let pending = self.pending.clone();
        let events = self.events.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value.get("type").and_then(Value::as_str) == Some("response") {
                    if let Some(id) = value.get("id").and_then(Value::as_str) {
                        if let Some(tx) = pending.lock().ok().and_then(|p| p.get(id).cloned()) {
                            let _ = tx.send(value);
                            continue;
                        }
                    }
                }
                if let Ok(mut out) = events.lock() {
                    out.push(value);
                }
            }
        });
    }

    fn start_stderr_reader(&self, stderr: std::process::ChildStderr) {
        let lines = self.stderr_lines.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Ok(mut out) = lines.lock() {
                    out.push(line);
                    if out.len() > 200 {
                        out.remove(0);
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};
    use tempfile::TempDir;

    fn fake_pi(dir: &TempDir) -> std::path::PathBuf {
        let path = dir.path().join("fake-pi-rpc.sh");
        fs::write(&path, r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  typ=$(printf '%s' "$line" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p')
  if [ "$typ" = "prompt" ]; then
    if printf '%s' "$line" | grep -q 'fail-prompt'; then
      printf '{"type":"response","id":"%s","success":false,"error":"prompt failed"}\n' "$id"
      continue
    fi
    printf '{"type":"response","id":"%s","success":true,"data":{"turn_id":"turn-1"}}\n' "$id"
    printf '{"type":"event","event":"message.delta","delta":"hi","turn_id":"turn-1"}\n'
  elif [ "$typ" = "get_state" ]; then
    printf '{"type":"response","id":"%s","success":true,"data":{"busy":true,"turn_id":"turn-1","session_id":"sess-1"}}\n' "$id"
  elif [ "$typ" = "get_commands" ]; then
    printf '{"type":"response","id":"%s","success":true,"data":{"commands":[{"name":"ask"}]}}\n' "$id"
  elif [ "$typ" = "abort" ]; then
    printf '{"type":"response","id":"%s","success":true,"data":{"aborted":true}}\n' "$id"
  fi
done
"#).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[test]
    fn pi_rpc_client_round_trips_commands_and_events() {
        let dir = TempDir::new().unwrap();
        let mut child = Command::new(fake_pi(&dir))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let rpc = PiRpcClient::from_child(&mut child).unwrap();
        assert_eq!(rpc.prompt("hello", None).unwrap()["turn_id"], "turn-1");
        assert_eq!(
            rpc.prompt("fail-prompt", None).unwrap_err(),
            "prompt failed"
        );
        assert_eq!(rpc.get_state().unwrap()["session_id"], "sess-1");
        assert_eq!(rpc.get_commands().unwrap()[0]["name"], "ask");
        assert!(rpc
            .drain_events()
            .iter()
            .any(|event| event["delta"] == "hi"));
        let _ = child.kill();
        let _ = child.wait();
    }
}
