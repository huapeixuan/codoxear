use codoxear_backend_rs::session_create_support::{
    find_codex_resume_candidate_in, find_pi_resume_session_file_in,
};
use serde_json::json;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_jsonl(path: &Path, rows: &[serde_json::Value]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut raw = String::new();
    for row in rows {
        raw.push_str(&serde_json::to_string(row).unwrap());
        raw.push('\n');
    }
    fs::write(path, raw).unwrap();
}

#[test]
fn codex_resume_candidate_requires_matching_id_and_cwd_and_reads_preview() {
    let home = TempDir::new().unwrap();
    let cwd = home.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let sessions = home.path().join(".codex/sessions/2026/05/15");
    let wanted = sessions.join("rollout-2026-05-15T00-00-00-wanted.jsonl");
    let other = sessions.join("rollout-2026-05-15T00-00-01-other.jsonl");
    write_jsonl(
        &wanted,
        &[
            json!({"type":"session_meta","payload":{"id":"resume-a","cwd":cwd.to_string_lossy(),"source":"cli"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for repo\nignore"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Recovered codex thread with enough detail to seed an alias."}]}}),
        ],
    );
    write_jsonl(
        &other,
        &[
            json!({"type":"session_meta","payload":{"id":"resume-a","cwd":"/different","source":"cli"}}),
        ],
    );

    let candidate =
        find_codex_resume_candidate_in(&home.path().join(".codex/sessions"), &cwd, "resume-a")
            .expect("matching codex resume candidate");

    assert_eq!(candidate.session_id, "resume-a");
    assert_eq!(candidate.log_path, wanted);
    assert_eq!(
        candidate.first_user_message.as_deref(),
        Some("Recovered codex thread with enough detail to seed an alias.")
    );
    assert!(
        find_codex_resume_candidate_in(&home.path().join(".codex/sessions"), &cwd, "missing")
            .is_none()
    );
}

#[test]
fn pi_resume_file_requires_id_and_cwd_to_match_same_header() {
    let home = TempDir::new().unwrap();
    let cwd = home.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let sessions = home.path().join(".pi/agent/sessions/--repo--");
    let wanted = sessions.join("wanted.jsonl");
    let wrong_id_same_cwd = sessions.join("wrong-id.jsonl");
    write_jsonl(
        &wanted,
        &[
            json!({"type":"session","id":"pi-resume-a","cwd":cwd.to_string_lossy(),"timestamp":"2026-05-15T00:00:00Z"}),
        ],
    );
    write_jsonl(
        &wrong_id_same_cwd,
        &[
            json!({"type":"session","id":"pi-other","cwd":cwd.to_string_lossy(),"timestamp":"2026-05-15T00:00:01Z"}),
        ],
    );

    assert_eq!(
        find_pi_resume_session_file_in(
            &home.path().join(".pi/agent/sessions"),
            &cwd,
            "pi-resume-a"
        ),
        Some(wanted)
    );
    assert!(find_pi_resume_session_file_in(
        &home.path().join(".pi/agent/sessions"),
        &cwd,
        "missing"
    )
    .is_none());
}

#[test]
fn codex_resume_candidate_ignores_subagent_logs_and_reads_legacy_message_preview() {
    let home = TempDir::new().unwrap();
    let cwd = home.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let sessions = home.path().join(".codex/sessions/2026/05/15");
    let subagent = sessions.join("rollout-subagent.jsonl");
    let wanted = sessions.join("rollout-legacy-message.jsonl");
    write_jsonl(
        &subagent,
        &[
            json!({"type":"session_meta","payload":{"id":"resume-b","cwd":cwd.to_string_lossy(),"source":{"subagent":"worker"}}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"wrong"}]}}),
        ],
    );
    write_jsonl(
        &wanted,
        &[
            json!({"type":"session_meta","payload":{"id":"resume-b","cwd":cwd.to_string_lossy(),"source":"cli"}}),
            json!({"type":"message","role":"user","content":[{"type":"input_text","text":"Legacy message preview from resumed session."}]}),
        ],
    );

    let candidate =
        find_codex_resume_candidate_in(&home.path().join(".codex/sessions"), &cwd, "resume-b")
            .expect("matching non-subagent codex resume candidate");

    assert_eq!(candidate.log_path, wanted);
    assert_eq!(
        candidate.first_user_message.as_deref(),
        Some("Legacy message preview from resumed session.")
    );
}
