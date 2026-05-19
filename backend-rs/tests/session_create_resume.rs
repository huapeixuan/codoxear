use codoxear_backend_rs::session_create::selected_broker_argv;
use codoxear_backend_rs::session_create_support::{
    build_tmux_shell_command, find_codex_resume_candidate_in, find_pi_resume_session_file_in,
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
fn selected_broker_argv_uses_rust_bin_for_codex_and_pi_when_set() {
    let cwd = Path::new("/repo");
    assert_eq!(
        selected_broker_argv("codex", cwd, None, Some(" /tmp/codoxear-broker-rs ")),
        vec!["/tmp/codoxear-broker-rs", "--cwd", "/repo", "--"]
    );
    assert_eq!(
        selected_broker_argv(
            "pi",
            cwd,
            Some(Path::new("/tmp/pi.jsonl")),
            Some("/tmp/codoxear-broker-rs"),
        ),
        vec![
            "/tmp/codoxear-broker-rs",
            "--cwd",
            "/repo",
            "--session-file",
            "/tmp/pi.jsonl",
            "--",
        ]
    );
}

#[test]
fn selected_broker_argv_defaults_to_rust_broker_when_override_blank() {
    let pi_argv = selected_broker_argv(
        "pi",
        Path::new("/repo"),
        Some(Path::new("/tmp/pi.jsonl")),
        Some("  "),
    );
    assert_eq!(pi_argv[0], "codoxear-broker-rs");
    assert!(pi_argv.iter().any(|arg| arg == "--session-file"));
    assert_eq!(pi_argv[pi_argv.len() - 1], "--");

    let codex_argv = selected_broker_argv("codex", Path::new("/repo"), None, None);
    assert_eq!(
        codex_argv,
        vec!["codoxear-broker-rs", "--cwd", "/repo", "--"]
    );
}

#[test]
fn tmux_shell_command_uses_selected_rust_broker_binary() {
    let mut argv = selected_broker_argv(
        "pi",
        Path::new("/repo with spaces"),
        Some(Path::new("/tmp/pi session.jsonl")),
        Some("/opt/bin/codoxear-broker-rs"),
    );
    argv.extend(["-e".to_string(), "/bridge/ask_user_bridge.ts".to_string()]);
    let shell = build_tmux_shell_command(
        &argv,
        &[
            ("CODEX_WEB_AGENT_BACKEND".to_string(), "pi".to_string()),
            ("CODEX_WEB_TRANSPORT".to_string(), "tmux".to_string()),
        ],
    );

    assert!(shell.contains("/opt/bin/codoxear-broker-rs"));
    assert!(shell.contains("--session-file"));
    assert!(shell.contains("ask_user_bridge.ts"));
    assert!(shell.contains("CODEX_WEB_AGENT_BACKEND=pi"));
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
