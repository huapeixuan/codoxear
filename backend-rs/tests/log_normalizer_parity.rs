use codoxear_backend_rs::log_normalizer::{codex, pi};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn python_json(script: &str) -> Value {
    let python = std::env::var("CODOXEAR_CONTRACT_PYTHON")
        .ok()
        .or_else(|| {
            let candidate = PathBuf::from("/tmp/codoxear-venv/bin/python");
            candidate
                .exists()
                .then(|| candidate.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "python3".to_string());
    let output = Command::new(python)
        .arg("-c")
        .arg(script)
        .current_dir(repo_root())
        .output()
        .expect("run python parity helper");
    assert!(
        output.status.success(),
        "python failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("python emitted JSON")
}

fn fixture(path: &str) -> PathBuf {
    repo_root().join("tests/fixtures").join(path)
}

#[test]
fn codex_chat_events_match_python_fixture() {
    let path = fixture("rollout/codex_basic.jsonl");
    let rust = codex::messages_from_codex_log(&path, 0, 200, true, None).unwrap();
    let py = python_json(&format!(
        r#"
import json
from pathlib import Path
from codoxear import rollout_log
path = Path({path:?})
events = rollout_log._read_chat_events_from_tail(path, min_events=1, max_scan_bytes=1024*1024)
print(json.dumps({{"events": events}}, separators=(",", ":")))
"#,
        path = path.to_string_lossy()
    ));

    let rust_events = serde_json::to_value(rust).unwrap()["events"].clone();
    assert_eq!(
        scrub_message_ids(rust_events),
        scrub_message_ids(py["events"].clone())
    );
}

#[test]
fn codex_idle_and_token_match_python_fixture() {
    let path = fixture("rollout/codex_token.jsonl");
    let rust_idle = codex::idle_from_log(&path, 1024 * 1024).unwrap();
    let rust_token = codex::token_snapshot_from_log(&path, 1024 * 1024).unwrap();
    let py = python_json(&format!(
        r#"
import json
from pathlib import Path
from codoxear import rollout_log
path = Path({path:?})
print(json.dumps({{
  "idle": rollout_log._compute_idle_from_log(path, max_scan_bytes=1024*1024),
  "token": rollout_log._find_latest_token_update(path, max_scan_bytes=1024*1024),
}}, separators=(",", ":")))
"#,
        path = path.to_string_lossy()
    ));

    assert_eq!(json!(rust_idle), py["idle"]);
    if py["token"].is_object() {
        assert_eq!(rust_token, Some(py["token"].clone()));
    }
}

#[test]
fn pi_message_helpers_match_python_fixture() {
    let path = fixture("pi/pi_basic.jsonl");
    let objects = read_jsonl(&path);
    let rust = json!({
        "user": pi::pi_user_text(&objects[1]),
        "assistant": pi::pi_assistant_text(&objects[4]),
        "final": pi::pi_final_turn(&objects[4]),
        "tool_count": pi::pi_assistant_tool_use_count(&objects[2]),
        "thinking_count": pi::pi_assistant_thinking_count(&objects[2]),
        "role": pi::pi_message_role(&objects[2]),
    });
    let py = python_json(&format!(
        r#"
import json
from pathlib import Path
from codoxear import pi_log
objs = [json.loads(line) for line in Path({path:?}).read_text().splitlines() if line.strip()]
print(json.dumps({{
  "user": pi_log.pi_user_text(objs[1]),
  "assistant": pi_log.pi_assistant_text(objs[4]),
  "final": pi_log.pi_assistant_is_final_turn_end(objs[4]),
  "tool_count": pi_log.pi_assistant_tool_use_count(objs[2]),
  "thinking_count": pi_log.pi_assistant_thinking_count(objs[2]),
  "role": pi_log.pi_message_role(objs[2]),
}}, separators=(",", ":")))
"#,
        path = path.to_string_lossy()
    ));

    assert_eq!(rust, py);
}

#[test]
fn pi_run_settings_and_token_match_python_fixture() {
    let log_path = fixture("pi/pi_basic.jsonl");
    let settings_path = fixture("pi/pi_settings.jsonl");
    let models_path = fixture("pi/models.json");
    let objects = read_jsonl(&log_path);
    let rust = json!({
        "settings": pi::pi_run_settings(&settings_path, 8 * 1024 * 1024).unwrap(),
        "token": pi::pi_token_update(&objects[4], Some(&models_path)),
    });
    let py = python_json(&format!(
        r#"
import json
from pathlib import Path
from codoxear import pi_log
log_path = Path({log_path:?})
settings_path = Path({settings_path:?})
models_path = Path({models_path:?})
objs = [json.loads(line) for line in log_path.read_text().splitlines() if line.strip()]
print(json.dumps({{
  "settings": list(pi_log.read_pi_run_settings(settings_path, max_scan_bytes=8*1024*1024)),
  "token": pi_log.pi_token_update(objs[4], models_path=models_path),
}}, separators=(",", ":")))
"#,
        log_path = log_path.to_string_lossy(),
        settings_path = settings_path.to_string_lossy(),
        models_path = models_path.to_string_lossy()
    ));

    assert_eq!(rust, py);
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn scrub_message_ids(mut value: Value) -> Value {
    match &mut value {
        Value::Array(items) => {
            for item in items {
                if let Value::Object(object) = item {
                    object.remove("message_id");
                }
            }
        }
        Value::Object(object) => {
            object.remove("message_id");
        }
        _ => {}
    }
    value
}
