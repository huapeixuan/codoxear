use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use base64::Engine;
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::routes::router;
use codoxear_backend_rs::runtime::{
    load_or_create_hmac_secret, sign_auth_cookie_value, unix_now_seconds, RuntimeConfig,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sha2::Digest;
use std::fs;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
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

fn patch_session_sidecar(home: &TempDir, session_id: &str, patch: Value) {
    let path = app_dir(home)
        .join("socks")
        .join(format!("{session_id}.json"));
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (key, patch_value) in patch.as_object().unwrap() {
        value[key] = patch_value.clone();
    }
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn queue_len_on_disk(app_dir: &Path, session_id: &str) -> usize {
    let value: Value =
        serde_json::from_slice(&fs::read(app_dir.join("session_queues.json")).unwrap()).unwrap();
    value
        .get(session_id)
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn spawn_broker_server<F>(
    sock_path: PathBuf,
    request_count: usize,
    handler: F,
) -> thread::JoinHandle<()>
where
    F: Fn(Value) -> Value + Send + Sync + 'static,
{
    let _ = fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path).expect("bind broker socket");
    let handler = std::sync::Arc::new(handler);
    thread::spawn(move || {
        for _ in 0..request_count {
            let (stream, _) = listener.accept().expect("accept broker request");
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(line.trim_end()).unwrap();
            let response = handler(request);
            let mut stream = stream;
            stream
                .write_all(serde_json::to_string(&response).unwrap().as_bytes())
                .unwrap();
            stream.write_all(b"\n").unwrap();
        }
    })
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

async fn post_json_no_cookie(app: axum::Router, uri: &str, value: Value) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
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
        json!({
            "sid-a": {"priority_offset": 0.5},
            "sid-b": {"priority_offset": 0.2, "dependency_session_id": "sid-a"}
        })
        .to_string(),
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
    let sidebar: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("session_sidebar.json")).unwrap())
            .unwrap();
    assert!(sidebar["sid-b"].get("dependency_session_id").is_none());
}

#[tokio::test]
async fn heartbeat_preserves_sidecar_private_file_mode() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "pi");
    let sidecar_path = app_dir(&home).join("socks/sid-a.json");
    #[cfg(unix)]
    fs::set_permissions(&sidecar_path, fs::Permissions::from_mode(0o600)).unwrap();
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(app, "/api/sessions/sid-a/heartbeat", &cookie, json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&sidecar_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let sidecar: Value = serde_json::from_str(&fs::read_to_string(sidecar_path).unwrap()).unwrap();
    assert_eq!(sidecar["session_id"], "sid-a");
    assert!(sidecar["last_web_activity_ts"].as_f64().unwrap() > 0.0);
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

#[tokio::test]
async fn voice_settings_and_subscription_writes_persist_python_shape() {
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);
    let dir = app_dir(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/settings/voice",
        &cookie,
        json!({
            "tts_enabled_for_narration": true,
            "tts_enabled_for_final_response": false,
            "tts_base_url": "https://example.com/v1/",
            "tts_api_key": "  key  ",
            "summarization_model": "sum",
            "tts_model": "tts"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["tts_base_url"], "https://example.com/v1");
    let saved: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("voice_settings.json")).unwrap())
            .unwrap();
    assert_eq!(saved["tts_api_key"], "key");

    let subscription =
        json!({"endpoint": "https://push.example/sub", "keys": {"p256dh": "p", "auth": "a"}});
    let (status, body) = post_json(
        app.clone(),
        "/api/notifications/subscription",
        &cookie,
        json!({
            "subscription": subscription,
            "user_agent": "Mozilla iPhone",
            "device_label": "Phone",
            "device_class": ""
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(
        body["subscriptions"][0]["endpoint"],
        "https://push.example/sub"
    );
    assert_eq!(body["subscriptions"][0]["device_class"], "mobile");

    let saved: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("push_subscriptions.json")).unwrap())
            .unwrap();
    assert_eq!(
        saved[0]["subscription"]["endpoint"],
        "https://push.example/sub"
    );
    assert_eq!(saved[0]["notifications_enabled"], true);

    let (status, body) = post_json(
        app,
        "/api/notifications/subscription/toggle",
        &cookie,
        json!({"endpoint": "https://push.example/sub", "enabled": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["subscriptions"][0]["notifications_enabled"], false);
}

#[tokio::test]
async fn audio_listener_heartbeat_validates_payload_and_tracks_memory_state() {
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/audio/listener",
        &cookie,
        json!({"client_id": "browser-1", "enabled": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true, "active_listener_count": 1}));

    let (status, body) = post_json(
        app,
        "/api/audio/listener",
        &cookie,
        json!({"client_id": "browser-1", "enabled": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ok": true, "active_listener_count": 0}));
}

#[tokio::test]
async fn voice_debug_endpoints_are_feature_disabled_without_side_effects() {
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);
    let dir = app_dir(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/notifications/test_push",
        &cookie,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["ok"], false);
    assert_eq!(body["phase"], "phase5");

    let (status, body) = post_json(app, "/api/audio/test_announcement", &cookie, json!({})).await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(body["ok"], false);
    assert_eq!(body["phase"], "phase5");
    assert!(!dir.join("push_ledger.json").exists());
    assert!(!dir.join("audio_announcement_queue.json").exists());
}

#[tokio::test]
async fn hooks_notify_is_public_no_auth_noop() {
    let (_home, app) = test_app();

    let (status, body) = post_json_no_cookie(app, "/api/hooks/notify", json!({"x": 1})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({"ignored": true}));
}

#[tokio::test]
async fn takeover_open_returns_descriptor_without_side_effect_when_not_eligible() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let cookie = signed_cookie(&home);

    let (status, body) =
        post_json(app, "/api/sessions/sid-a/takeover/open", &cookie, json!({})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["eligible"], false);
    assert_eq!(
        body["reason"],
        "takeover is only available for web-owned Pi sessions"
    );
}

#[tokio::test]
async fn inject_file_rejects_pi_sessions_before_decoding_upload() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "pi");
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app,
        "/api/sessions/sid-a/inject_file",
        &cookie,
        json!({"data_b64": "not-base64", "filename": "x.txt", "attachment_index": 1}),
    )
    .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["backend"], "pi");
    assert_eq!(body["operation"], "attachment_injection");
}

#[test]
fn worker_flag_parser_matches_python_truthy_semantics() {
    use codoxear_backend_rs::workers::env_flag_truthy_value;

    assert!(!env_flag_truthy_value(None));
    assert!(!env_flag_truthy_value(Some("")));
    assert!(!env_flag_truthy_value(Some("0")));
    assert!(!env_flag_truthy_value(Some("false")));
    assert!(!env_flag_truthy_value(Some(" FALSE ")));
    assert!(env_flag_truthy_value(Some("1")));
    assert!(env_flag_truthy_value(Some("true")));
    assert!(env_flag_truthy_value(Some("yes")));
}

#[tokio::test]
async fn inject_file_accepts_python_default_body_above_axum_json_default_limit() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let socks = app_dir(&home).join("socks");
    let sock_path = socks.join("sid-a.sock");
    let server = spawn_broker_server(sock_path, 2, |request| {
        match request["cmd"].as_str().unwrap() {
            "state" => json!({"busy": false, "queue_len": 0}),
            "keys" => {
                assert!(request["seq"].as_str().unwrap().contains("Attachment 1:"));
                json!({"ok": true})
            }
            other => panic!("unexpected broker cmd {other}"),
        }
    });
    let cookie = signed_cookie(&home);
    let raw = vec![b'a'; 3 * 1024 * 1024];
    let data_b64 = base64::engine::general_purpose::STANDARD.encode(raw);

    let (status, body) = post_json(
        app,
        "/api/sessions/sid-a/inject_file",
        &cookie,
        json!({"data_b64": data_b64, "filename": "big.txt", "attachment_index": 1}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let out = std::path::PathBuf::from(body["path"].as_str().unwrap());
    assert_eq!(fs::metadata(out).unwrap().len(), 3 * 1024 * 1024);
    server.join().unwrap();
}

#[tokio::test]
async fn inject_file_rejects_decoded_payload_above_python_default_limit() {
    let (home, app) = test_app();
    write_session(&home, "sid-a", "codex");
    let cookie = signed_cookie(&home);
    let data_b64 =
        base64::engine::general_purpose::STANDARD.encode(vec![0_u8; (16 * 1024 * 1024) + 1]);

    let (status, body) = post_json(
        app,
        "/api/sessions/sid-a/inject_file",
        &cookie,
        json!({"data_b64": data_b64, "filename": "too-big.bin", "attachment_index": 1}),
    )
    .await;

    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(body["error"].as_str().unwrap().contains("16777216"));
}

#[test]
fn queue_worker_requires_idle_grace_before_popping_queue_item() {
    use codoxear_backend_rs::workers::{queue_sweep_once_at, worker_state_reset_for_tests};
    worker_state_reset_for_tests();
    let (home, _app) = test_app();
    write_session(&home, "sid-a", "codex");
    let app_dir = app_dir(&home);
    let log_path = app_dir.join("sid-a.jsonl");
    fs::write(
        &log_path,
        r#"{"type":"response_item","timestamp":"2026-05-14T00:00:00Z","payload":{"type":"message","role":"assistant","end_turn":true,"content":[{"type":"output_text","text":"done"}]}}
"#,
    )
    .unwrap();
    patch_session_sidecar(
        &home,
        "sid-a",
        json!({"log_path": log_path.to_string_lossy()}),
    );
    fs::write(
        app_dir.join("session_queues.json"),
        serde_json::to_vec(&json!({"sid-a": [{"text": "next"}]})).unwrap(),
    )
    .unwrap();
    let state = AppState {
        config: RuntimeConfig {
            app_dir: app_dir.clone(),
        },
    };
    let server = spawn_broker_server(app_dir.join("socks/sid-a.sock"), 4, |request| match request
        ["cmd"]
        .as_str()
        .unwrap()
    {
        "state" => json!({"busy": false, "queue_len": 0}),
        "send" => json!({"ok": true}),
        other => panic!("unexpected broker cmd {other}"),
    });

    assert!(!queue_sweep_once_at(&state, 100.0).unwrap());
    assert_eq!(queue_len_on_disk(&app_dir, "sid-a"), 1);
    assert!(!queue_sweep_once_at(&state, 105.0).unwrap());
    assert_eq!(queue_len_on_disk(&app_dir, "sid-a"), 1);
    assert!(queue_sweep_once_at(&state, 111.0).unwrap());
    assert_eq!(queue_len_on_disk(&app_dir, "sid-a"), 0);
    server.join().unwrap();
}

#[test]
fn queue_worker_prunes_missing_session_without_broker_side_effects() {
    use codoxear_backend_rs::workers::{queue_sweep_once_at, worker_state_reset_for_tests};
    worker_state_reset_for_tests();
    let (home, _app) = test_app();
    let app_dir = app_dir(&home);
    fs::write(
        app_dir.join("session_queues.json"),
        serde_json::to_vec(&json!({"missing": [{"text": "drop me"}]})).unwrap(),
    )
    .unwrap();
    let state = AppState {
        config: RuntimeConfig {
            app_dir: app_dir.clone(),
        },
    };

    assert!(!queue_sweep_once_at(&state, 100.0).unwrap());
    let queues: Value =
        serde_json::from_slice(&fs::read(app_dir.join("session_queues.json")).unwrap()).unwrap();
    assert_eq!(queues, json!({}));
}

#[test]
fn harness_worker_requires_assistant_tail_and_cooldown_before_injecting() {
    use codoxear_backend_rs::log_normalizer::codex::last_chat_role_ts_from_log;
    use codoxear_backend_rs::workers::{harness_sweep_once_at, worker_state_reset_for_tests};
    worker_state_reset_for_tests();
    let (home, _app) = test_app();
    write_session(&home, "sid-a", "codex");
    let app_dir = app_dir(&home);
    let log_path = app_dir.join("sid-a.jsonl");
    fs::write(
        &log_path,
        r#"{"type":"response_item","timestamp":"2026-05-14T00:00:00Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}
"#,
    )
    .unwrap();
    patch_session_sidecar(
        &home,
        "sid-a",
        json!({"log_path": log_path.to_string_lossy(), "session_id": "thread-a"}),
    );
    fs::write(
        app_dir.join("harness.json"),
        serde_json::to_vec(&json!({"sid-a": {"enabled": true, "cooldown_minutes": 1, "remaining_injections": 2, "request": "ship it"}})).unwrap(),
    )
    .unwrap();
    let state = AppState {
        config: RuntimeConfig {
            app_dir: app_dir.clone(),
        },
    };

    assert!(!harness_sweep_once_at(&state, 100.0).unwrap());
    fs::write(
        &log_path,
        r#"{"type":"response_item","timestamp":"1970-01-01T00:01:00.500Z","payload":{"type":"message","role":"assistant","end_turn":true,"content":[{"type":"output_text","text":"done"}]}}
"#,
    )
    .unwrap();
    let last = last_chat_role_ts_from_log(&log_path, 256 * 1024)
        .unwrap()
        .expect("fractional assistant timestamp");
    assert_eq!(last.0, "assistant");
    assert!((last.1 - 60.5).abs() < 0.001);
    assert!(!harness_sweep_once_at(&state, 100.0).unwrap());
    let server = spawn_broker_server(app_dir.join("socks/sid-a.sock"), 2, |request| match request
        ["cmd"]
        .as_str()
        .unwrap()
    {
        "state" => json!({"busy": false, "queue_len": 0}),
        "send" => {
            let text = request["text"].as_str().unwrap();
            assert!(text.starts_with("Unattended-mode instructions"));
            assert!(text.contains("Additional request from user: ship it"));
            json!({"ok": true})
        }
        other => panic!("unexpected broker cmd {other}"),
    });
    assert!(harness_sweep_once_at(&state, 121.0).unwrap());
    assert!(!harness_sweep_once_at(&state, 150.0).unwrap());
    let harness: Value =
        serde_json::from_slice(&fs::read(app_dir.join("harness.json")).unwrap()).unwrap();
    assert_eq!(harness["sid-a"]["remaining_injections"], 1);
    server.join().unwrap();
}

#[test]
fn harness_worker_disables_zero_remaining_without_broker_side_effects() {
    use codoxear_backend_rs::workers::{harness_sweep_once_at, worker_state_reset_for_tests};
    worker_state_reset_for_tests();
    let (home, _app) = test_app();
    write_session(&home, "sid-a", "codex");
    let app_dir = app_dir(&home);
    fs::write(
        app_dir.join("harness.json"),
        serde_json::to_vec(&json!({"sid-a": {"enabled": true, "cooldown_minutes": 1, "remaining_injections": 0, "request": ""}})).unwrap(),
    )
    .unwrap();
    let state = AppState {
        config: RuntimeConfig {
            app_dir: app_dir.clone(),
        },
    };

    assert!(!harness_sweep_once_at(&state, 100.0).unwrap());
    let harness: Value =
        serde_json::from_slice(&fs::read(app_dir.join("harness.json")).unwrap()).unwrap();
    assert_eq!(harness["sid-a"]["enabled"], false);
    assert_eq!(harness["sid-a"]["remaining_injections"], 0);
}

#[tokio::test]
async fn session_create_parser_validation_and_deferred_spawn_response() {
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);

    let (status, body) =
        post_json(app.clone(), "/api/sessions", &cookie, json!({"cwd": "   "})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "cwd required");
    assert_eq!(body["field"], "cwd");

    let cwd = home.path().join("new-session");
    let (status, body) = post_json(
        app.clone(),
        "/api/sessions",
        &cookie,
        json!({
            "cwd": cwd.to_string_lossy(),
            "backend": "codex",
            "args": ["--foo", ""],
            "resume_session_id": " resume-1 ",
            "worktree_branch": " feature/x ",
            "model": "default",
            "reasoning_effort": "HIGH",
            "service_tier": "fast",
            "create_in_tmux": false
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"],
        "worktree_branch cannot be used when resuming a session"
    );

    let (status, body) = post_json(
        app,
        "/api/sessions",
        &cookie,
        json!({"cwd": cwd.to_string_lossy(), "backend": "pi", "resume_session_id": "resume-1", "preferred_auth_method": "apikey", "service_tier": "flex", "worktree_branch": "ignored", "reasoning_effort": "minimal"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "resume session not found for cwd: resume-1");
}
