use codoxear_backend_rs::log_normalizer::{codex, pi};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixture(path: &str) -> PathBuf {
    repo_root().join("tests/fixtures").join(path)
}

#[test]
fn codex_fixture_produces_chat_event_and_idle_state() {
    let path = fixture("rollout/codex_basic.jsonl");
    let rust = codex::messages_from_codex_log(&path, 0, 200, true, None).unwrap();
    let events = serde_json::to_value(rust).unwrap()["events"].clone();
    assert!(events.as_array().is_some_and(|items| !items.is_empty()));
}

#[test]
fn pi_fixture_helpers_extract_expected_fields() {
    let path = fixture("pi/pi_basic.jsonl");
    let objects = read_jsonl(&path);
    assert_eq!(pi::pi_user_text(&objects[1]).as_deref(), Some("Hello Pi"));
    assert!(pi::pi_assistant_text(&objects[4]).is_some());
    assert!(pi::pi_final_turn(&objects[4]));
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
