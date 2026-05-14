use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::routes::router;
use codoxear_backend_rs::runtime::{
    load_or_create_hmac_secret, sign_auth_cookie_value, unix_now_seconds, RuntimeConfig,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::Digest;
use std::fs;
use std::path::Path;
use std::process;
use tempfile::TempDir;
use tower::ServiceExt;

fn test_app() -> (TempDir, axum::Router) {
    let home = TempDir::new().expect("temp home");
    let app_dir = home.path().join(".local/share/codoxear");
    fs::create_dir_all(&app_dir).unwrap();
    let state = AppState {
        config: RuntimeConfig { app_dir },
    };
    (home, router(state))
}

fn app_dir(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".local/share/codoxear")
}

fn signed_cookie(home: &TempDir) -> String {
    let secret = load_or_create_hmac_secret(&app_dir(home)).expect("hmac secret");
    let token = sign_auth_cookie_value(&secret, unix_now_seconds() + 3600).expect("sign cookie");
    format!("codoxear_auth={token}")
}

fn write_session(home: &TempDir, session_id: &str, backend: &str) {
    let socks = app_dir(home).join("socks");
    fs::create_dir_all(&socks).unwrap();
    fs::write(socks.join(format!("{session_id}.sock")), b"").unwrap();
    fs::write(
        socks.join(format!("{session_id}.json")),
        serde_json::to_vec(&json!({
            "session_id": session_id,
            "cwd": home.path().to_string_lossy(),
            "backend": backend,
            "agent_backend": backend,
            "broker_pid": process::id(),
            "codex_pid": process::id(),
            "start_ts": 1000.0,
            "updated_ts": 1001.0,
            "sock_path": socks.join(format!("{session_id}.sock")).to_string_lossy(),
            "owner": "web",
            "transport": if backend == "pi" { "pi-rpc" } else { "pty" },
            "auto_stop_on_idle": backend == "pi",
            "idle_timeout_seconds": if backend == "pi" { 1800 } else { 0 },
        }))
        .unwrap(),
    )
    .unwrap();
}

fn assert_same_path(value: &Value, expected: &Path) {
    let actual = fs::canonicalize(value.as_str().expect("json path")).unwrap();
    let expected = fs::canonicalize(expected).unwrap();
    assert_eq!(actual, expected);
}

async fn post_json(
    app: axum::Router,
    uri: &str,
    cookie: &str,
    value: Value,
) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&body)
        .unwrap_or_else(|_| json!({"raw": String::from_utf8_lossy(&body)}));
    (status, json)
}

#[tokio::test]
async fn cwd_group_edit_persists_python_compatible_json() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app,
        "/api/cwd_groups/edit",
        &cookie,
        json!({
            "cwd": home.path().to_string_lossy(),
            "label": "  My   Project  ",
            "collapsed": true,
            "hidden": false
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["label"], "My Project");
    let raw = fs::read_to_string(app_dir(&home).join("cwd_groups.json")).unwrap();
    assert!(raw.ends_with('\n'));
    let saved: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(saved[body["cwd"].as_str().unwrap()]["collapsed"], true);
}

#[tokio::test]
async fn rename_edit_and_queue_routes_write_expected_state_files() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "pi");
    write_session(&home, "sid-b", "codex");
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/sessions/sid-a/rename",
        &cookie,
        json!({"name": "  Alpha  Session "}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true, "alias": "Alpha Session"}));

    let (status, body) = post_json(
        app.clone(),
        "/api/sessions/sid-a/edit",
        &cookie,
        json!({
            "name": "Alpha",
            "priority_offset": 0.5,
            "snooze_until": 12345,
            "dependency_session_id": "sid-b"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["dependency_session_id"], "sid-b");

    let img = json!({"file_name": "a.png", "mime_type": "image/png", "data_b64": "aGVsbG8="});
    let (status, body) = post_json(
        app.clone(),
        "/api/sessions/sid-a/enqueue",
        &cookie,
        json!({"text": "hello", "images": [img]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"queued": true, "queue_len": 1}));

    let (status, body) = post_json(
        app.clone(),
        "/api/sessions/sid-a/queue/update",
        &cookie,
        json!({"index": 0, "text": "updated"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true, "queue_len": 1}));

    let queues: Value = serde_json::from_str(
        &fs::read_to_string(app_dir(&home).join("session_queues.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(queues["sid-a"][0]["text"], "updated");
    assert_eq!(queues["sid-a"][0]["images"][0]["file_name"], "a.png");

    let (status, body) = post_json(
        app,
        "/api/sessions/sid-a/queue/delete",
        &cookie,
        json!({"index": 0}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true, "queue_len": 0}));
    let queues: Value = serde_json::from_str(
        &fs::read_to_string(app_dir(&home).join("session_queues.json")).unwrap(),
    )
    .unwrap();
    assert!(queues.get("sid-a").is_none());
}

#[tokio::test]
async fn harness_post_rejects_legacy_text_and_persists_normalized_config() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/sessions/sid-a/harness",
        &cookie,
        json!({"text": "old"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, json!({"error": "unknown field: text (use request)"}));

    let (status, body) = post_json(
        app,
        "/api/sessions/sid-a/harness",
        &cookie,
        json!({
            "enabled": true,
            "request": "check",
            "cooldown_minutes": 3,
            "remaining_injections": 2
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["cooldown_minutes"], 3);
    let harness: Value =
        serde_json::from_str(&fs::read_to_string(app_dir(&home).join("harness.json")).unwrap())
            .unwrap();
    assert_eq!(harness["sid-a"]["request"], "check");
}

#[tokio::test]
async fn heartbeat_updates_web_owned_pi_rpc_sidecar_only() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "pi");
    write_session(&home, "sid-b", "codex");
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/sessions/sid-a/heartbeat",
        &cookie,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["session_id"], "sid-a");
    assert_eq!(body["idle_timeout_seconds"], 1800);
    assert!(body["last_web_activity_ts"].as_f64().unwrap() > 0.0);
    let sidecar: Value =
        serde_json::from_str(&fs::read_to_string(app_dir(&home).join("socks/sid-a.json")).unwrap())
            .unwrap();
    assert!(sidecar["last_web_activity_ts"].as_f64().unwrap() > 0.0);

    let (status, body) = post_json(app, "/api/sessions/sid-b/heartbeat", &cookie, json!({})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body,
        json!({"error": "idle auto-stop heartbeat is only supported for web-owned pi-rpc sessions"})
    );
}

#[tokio::test]
async fn delete_hides_session_and_clears_sidecar_state() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let dir = app_dir(&home);
    fs::write(
        dir.join("session_aliases.json"),
        json!({"sid-a": "Alias"}).to_string(),
    )
    .unwrap();
    fs::write(
        dir.join("session_sidebar.json"),
        json!({"sid-a": {"priority_offset": 0.5}}).to_string(),
    )
    .unwrap();
    fs::write(
        dir.join("session_files.json"),
        json!({"sid-a": ["a.txt"]}).to_string(),
    )
    .unwrap();
    fs::write(
        dir.join("session_queues.json"),
        json!({"sid-a": ["hello"]}).to_string(),
    )
    .unwrap();
    fs::write(
        dir.join("harness.json"),
        json!({"sid-a": {"enabled": true}}).to_string(),
    )
    .unwrap();
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(app, "/api/sessions/sid-a/delete", &cookie, json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true}));
    let hidden: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("hidden_sessions.json")).unwrap())
            .unwrap();
    assert!(hidden.as_array().unwrap().contains(&json!("sid-a")));
    for name in [
        "session_aliases.json",
        "session_sidebar.json",
        "session_files.json",
        "session_queues.json",
        "harness.json",
    ] {
        let saved: Value =
            serde_json::from_str(&fs::read_to_string(dir.join(name)).unwrap()).unwrap();
        assert!(saved.get("sid-a").is_none(), "{name} still contains sid-a");
    }
}

#[tokio::test]
async fn send_falls_back_to_queue_when_live_process_has_stale_socket() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app,
        "/api/sessions/sid-a/send",
        &cookie,
        json!({"text": "queued while switching"}),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"queued": true, "queue_len": 1}));
    let queues: Value = serde_json::from_str(
        &fs::read_to_string(app_dir(&home).join("session_queues.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(queues["sid-a"][0], "queued while switching");
}

#[tokio::test]
async fn global_file_post_read_and_inspect_track_session_history() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let file_path = home.path().join("notes.md");
    fs::write(&file_path, "hello file").unwrap();
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/files/read",
        &cookie,
        json!({"path": file_path.to_string_lossy(), "session_id": "sid-a"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["kind"], "markdown");
    assert_eq!(body["text"], "hello file");
    assert_eq!(body["editable"], true);
    assert_same_path(&body["path"], &file_path);

    let history: Value = serde_json::from_str(
        &fs::read_to_string(app_dir(&home).join("session_files.json")).unwrap(),
    )
    .unwrap();
    assert_same_path(&history["sid:sid-a"][0], &file_path);

    let (status, body) = post_json(
        app,
        "/api/files/inspect",
        &cookie,
        json!({"path": file_path.to_string_lossy(), "session_id": "sid-a"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["kind"], "markdown");
    assert_eq!(body["size"], 10);
}

#[tokio::test]
async fn session_file_write_updates_text_with_version_and_records_history() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let target = home.path().join("draft.txt");
    fs::write(&target, "old").unwrap();
    let old_version = format!("{:x}", sha2::Sha256::digest(b"old"));
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/sessions/sid-a/file/write",
        &cookie,
        json!({"path": "draft.txt", "text": "new text", "version": old_version}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["editable"], true);
    assert_eq!(body["rel"], "draft.txt");
    assert_eq!(fs::read_to_string(&target).unwrap(), "new text");

    let history: Value = serde_json::from_str(
        &fs::read_to_string(app_dir(&home).join("session_files.json")).unwrap(),
    )
    .unwrap();
    assert_same_path(&history["sid:sid-a"][0], &target);

    let (status, body) = post_json(
        app,
        "/api/sessions/sid-a/file/write",
        &cookie,
        json!({"path": "draft.txt", "text": "conflict", "version": "stale"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["conflict"], true);
    assert_eq!(body["error"], "file changed on disk");
}
