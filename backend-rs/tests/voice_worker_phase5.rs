use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::models::SessionRow;
use codoxear_backend_rs::routes::router;
use codoxear_backend_rs::runtime::{
    load_or_create_hmac_secret, sign_auth_cookie_value, unix_now_seconds, RuntimeConfig,
};
use codoxear_backend_rs::voice_worker::hls::{
    empty_playlist, parse_duration, render_playlist, safe_segment_path, HlsMediaRunner, HlsSegment,
    MergedHlsStream, HLS_MAX_SEGMENTS,
};
use codoxear_backend_rs::voice_worker::ledger::{
    new_pending_final_response, write_ledger_trimmed, DELIVERY_LEDGER_FILE,
};
use codoxear_backend_rs::voice_worker::locks::VoiceOwnerLock;
use codoxear_backend_rs::voice_worker::openai::{
    openai_endpoint_url, parse_summary_response, summary_payload, OpenAiVoiceClient,
    SummaryRequest, TtsRequest,
};
use codoxear_backend_rs::voice_worker::scan::{
    assistant_messages_from_log, observe_messages_into_ledger, voice_scan_rows_once,
    ClassifiedAssistantMessage,
};
use codoxear_backend_rs::voice_worker::state::{
    reset_runtime_registry_for_tests, runtime_for_app_dir, QueuedVoiceTask,
};
use codoxear_backend_rs::voice_worker::vapid::{
    default_vapid_subject_from_env, load_or_create_public_key, normalize_vapid_subject,
    public_key_from_pem, tailscale_https_subject_with_runner, VAPID_PRIVATE_KEY_FILE,
};
use codoxear_backend_rs::voice_worker::webpush::{
    build_webpush_message, should_drop_subscription, WebPushPayload, WebPushSendError,
    WebPushSender, WebPushTarget, WEB_PUSH_TTL_SECONDS,
};
use codoxear_backend_rs::workers::voice_worker_role;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tower::ServiceExt;

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

#[derive(Clone)]
struct FakeOpenAi {
    summary: Arc<Mutex<Result<String, String>>>,
    speech: Arc<Mutex<Result<Vec<u8>, String>>>,
    summary_calls: Arc<Mutex<Vec<(SummaryRequest, String)>>>,
    speech_calls: Arc<Mutex<Vec<(TtsRequest, String)>>>,
}

impl Default for FakeOpenAi {
    fn default() -> Self {
        Self::success()
    }
}

impl FakeOpenAi {
    fn success() -> Self {
        Self {
            summary: Arc::new(Mutex::new(Ok("short summary".to_string()))),
            speech: Arc::new(Mutex::new(Ok(b"fake-aac".to_vec()))),
            summary_calls: Arc::new(Mutex::new(Vec::new())),
            speech_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_speech_error(message: &str) -> Self {
        let client = Self::success();
        *client.speech.lock().unwrap() = Err(message.to_string());
        client
    }

    fn with_summary_error(message: &str) -> Self {
        let client = Self::success();
        *client.summary.lock().unwrap() = Err(message.to_string());
        client
    }
}

impl OpenAiVoiceClient for FakeOpenAi {
    fn summarize(&self, request: SummaryRequest, text: &str) -> Result<String, String> {
        self.summary_calls
            .lock()
            .unwrap()
            .push((request, text.to_string()));
        self.summary.lock().unwrap().clone()
    }

    fn synthesize(&self, request: TtsRequest, text: &str) -> Result<Vec<u8>, String> {
        self.speech_calls
            .lock()
            .unwrap()
            .push((request, text.to_string()));
        self.speech.lock().unwrap().clone()
    }
}

type FakePushCall = (WebPushTarget, Value, u32, String);

#[derive(Clone, Default)]
struct FakePushSender {
    results: Arc<Mutex<Vec<Result<(), WebPushSendError>>>>,
    calls: Arc<Mutex<Vec<FakePushCall>>>,
}

impl FakePushSender {
    fn new(results: Vec<Result<(), WebPushSendError>>) -> Self {
        Self {
            results: Arc::new(Mutex::new(results)),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl WebPushSender for FakePushSender {
    fn send_json(
        &self,
        target: &WebPushTarget,
        payload: &Value,
        ttl_seconds: u32,
        vapid_subject: &str,
    ) -> Result<(), WebPushSendError> {
        self.calls.lock().unwrap().push((
            target.clone(),
            payload.clone(),
            ttl_seconds,
            vapid_subject.to_string(),
        ));
        self.results.lock().unwrap().pop().unwrap_or(Ok(()))
    }
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn test_app() -> (TempDir, axum::Router) {
    reset_runtime_registry_for_tests();
    let home = TempDir::new().expect("temp home");
    let app_dir = home.path().join(".local/share/codoxear");
    fs::create_dir_all(&app_dir).unwrap();
    let state = AppState {
        config: RuntimeConfig { app_dir },
        fake_spawn_for_tests: false,
        fake_spawn_session_id_for_tests: None,
    };
    (home, router(state))
}

fn app_dir(home: &TempDir) -> PathBuf {
    home.path().join(".local/share/codoxear")
}

fn signed_cookie(home: &TempDir) -> String {
    let secret = load_or_create_hmac_secret(&app_dir(home)).unwrap();
    let token = sign_auth_cookie_value(&secret, unix_now_seconds() + 3600).expect("sign cookie");
    format!("codoxear_auth={token}")
}

async fn post_json(
    app: axum::Router,
    uri: &str,
    cookie: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let response = app
        .oneshot(
            builder
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    (status, value)
}

async fn get(app: axum::Router, uri: &str, cookie: Option<&str>) -> (StatusCode, String, Vec<u8>) {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let response = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, content_type, body)
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
        .join("voice")
        .join(name)
}

fn repo_fixture_path(parts: &[&str]) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    for part in parts {
        path.push(part);
    }
    path
}

fn write_voice_settings(app_dir: &Path, extra: Value) {
    let mut settings = serde_json::Map::new();
    settings.insert("tts_enabled_for_narration".to_string(), json!(false));
    settings.insert("tts_enabled_for_final_response".to_string(), json!(true));
    settings.insert("tts_base_url".to_string(), json!("http://127.0.0.1:1/v1"));
    settings.insert("tts_api_key".to_string(), json!("test-key"));
    settings.insert("summarization_model".to_string(), json!("sum-model"));
    settings.insert("tts_model".to_string(), json!("tts-model"));
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            settings.insert(key.clone(), value.clone());
        }
    }
    fs::write(
        app_dir.join("voice_settings.json"),
        serde_json::to_string(&Value::Object(settings)).unwrap(),
    )
    .unwrap();
}

fn write_ledger(app_dir: &Path, rows: Value) {
    fs::write(
        app_dir.join(DELIVERY_LEDGER_FILE),
        serde_json::to_string_pretty(&rows).unwrap() + "\n",
    )
    .unwrap();
}

fn write_mobile_subscriptions(app_dir: &Path, endpoints: &[&str]) {
    let rows = endpoints
        .iter()
        .enumerate()
        .map(|(idx, endpoint)| {
            json!({
                "id": format!("sub-{idx}"),
                "subscription": {
                    "endpoint": endpoint,
                    "keys": {"p256dh": "p256", "auth": "auth"}
                },
                "notifications_enabled": true,
                "created_ts": idx as f64 + 1.0,
                "updated_ts": idx as f64 + 1.0,
                "last_success_ts": null,
                "last_failure_ts": null,
                "last_error": "",
                "user_agent": "iPhone",
                "device_label": "phone",
                "device_class": "mobile",
            })
        })
        .collect::<Vec<_>>();
    fs::write(
        app_dir.join("push_subscriptions.json"),
        serde_json::to_string_pretty(&rows).unwrap() + "\n",
    )
    .unwrap();
}

#[derive(Default)]
struct FakeHlsRunner;

impl HlsMediaRunner for FakeHlsRunner {
    fn append_aac_as_segments(&self, _input: &Path, output_pattern: &Path) -> Result<(), String> {
        let parent = output_pattern.parent().unwrap();
        let pattern = output_pattern.file_name().unwrap().to_string_lossy();
        let first = pattern.replace("%03d", "000");
        let second = pattern.replace("%03d", "001");
        fs::write(parent.join(first), b"ts-1").unwrap();
        fs::write(parent.join(second), b"ts-2").unwrap();
        Ok(())
    }

    fn append_silence_segment(&self, output: &Path) -> Result<(), String> {
        fs::write(output, b"silence").unwrap();
        Ok(())
    }

    fn segment_duration(&self, segment: &Path) -> Result<f64, String> {
        let name = segment.file_name().unwrap().to_string_lossy();
        if name.contains("001") {
            Ok(7.25)
        } else {
            Ok(6.0)
        }
    }
}

#[test]
fn voice_worker_role_combines_scan_and_delivery_under_one_owner() {
    assert_eq!(voice_worker_role(false, false), None);
    assert_eq!(voice_worker_role(true, false), Some("scan"));
    assert_eq!(voice_worker_role(false, true), Some("worker-drain-only"));
    assert_eq!(voice_worker_role(true, true), Some("scan+worker"));
}

#[test]
fn voice_owner_lock_uses_create_new_and_cleans_up_on_drop() {
    let dir = TempDir::new().unwrap();
    let first = VoiceOwnerLock::acquire(dir.path(), "worker").unwrap();
    let lock_path = dir.path().join("voice_worker.lock");
    let body = fs::read_to_string(&lock_path).unwrap();
    assert!(body.contains("role=worker"));
    assert!(body.contains("process=codoxear-backend-rs"));

    let err = VoiceOwnerLock::acquire(dir.path(), "scan").unwrap_err();
    assert!(err.contains("voice owner lock already exists"));

    drop(first);
    assert!(!lock_path.exists());
    let second = VoiceOwnerLock::acquire(dir.path(), "scan").unwrap();
    assert_eq!(second.path(), lock_path.as_path());
}

#[test]
fn vapid_public_key_matches_python_for_existing_pem_fixture() {
    let dir = TempDir::new().unwrap();
    let pem = dir.path().join(VAPID_PRIVATE_KEY_FILE);
    fs::copy(fixture_path(VAPID_PRIVATE_KEY_FILE), &pem).unwrap();

    assert_eq!(
        public_key_from_pem(&pem).unwrap(),
        "BHUSo99p_eF5I6SkXJCwU4GZpK5bJ5lkiiueG8IQzzk1oMdi40D0lGCfigqkd5tM7LsruGX0aDHi5YqVFiClvxE"
    );
}

#[test]
fn rust_created_vapid_pem_is_reloaded_with_stable_public_key() {
    let dir = TempDir::new().unwrap();
    let first = load_or_create_public_key(dir.path()).unwrap();
    let pem = dir.path().join(VAPID_PRIVATE_KEY_FILE);
    assert!(pem.exists());
    assert_eq!(load_or_create_public_key(dir.path()).unwrap(), first);
}

#[test]
fn vapid_subject_normalization_matches_python_rules_without_tailscale_probe() {
    assert_eq!(
        normalize_vapid_subject(" https://example.test/ ").unwrap(),
        "https://example.test"
    );
    assert_eq!(
        normalize_vapid_subject("mailto:ops@example.test").unwrap(),
        "mailto:ops@example.test"
    );
    assert!(normalize_vapid_subject("example.test")
        .unwrap_err()
        .contains("vapid subject"));

    let _guard = EnvGuard::set("CODEX_WEB_PUSH_VAPID_SUBJECT", "https://push.example/");
    assert_eq!(default_vapid_subject_from_env(), "https://push.example");
}

#[test]
fn vapid_subject_uses_tailscale_dns_fallback_when_env_is_unset() {
    let _guard = EnvGuard::set("CODEX_WEB_PUSH_VAPID_SUBJECT", "");
    let subject = tailscale_https_subject_with_runner(|timeout| {
        assert_eq!(timeout, Duration::from_secs(5));
        Ok(r#"{"Self":{"DNSName":"phone.tail123.ts.net."}}"#.to_string())
    });
    assert_eq!(subject.as_deref(), Some("https://phone.tail123.ts.net"));
    assert!(tailscale_https_subject_with_runner(|_| Ok(r#"{"Self":{}}"#.to_string())).is_none());
}

#[test]
fn ledger_write_trims_to_newest_rows_using_actual_file_name() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();

    write_ledger_trimmed(
        app_dir,
        vec![
            json!({"message_id": "old", "session_id": "s", "message_class": "final_response", "notification_text": "Old", "updated_ts": 1.0}),
            json!({"message_id": "mid", "session_id": "s", "message_class": "final_response", "notification_text": "Mid", "updated_ts": 2.0}),
            json!({"message_id": "new", "session_id": "s", "message_class": "final_response", "notification_text": "New", "updated_ts": 3.0}),
        ],
        2,
    )
    .unwrap();

    let value: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).expect("ledger file"))
            .unwrap();
    let object = value.as_object().unwrap();
    assert!(!object.contains_key("old"));
    assert!(object.contains_key("mid"));
    assert!(object.contains_key("new"));
    assert!(!app_dir.join("push_ledger.json").exists());
}

#[test]
fn pending_final_response_row_preserves_python_status_semantics() {
    let row = new_pending_final_response("m1", "s1", "Session", "hello world");
    assert_eq!(row["message_class"], "final_response");
    assert_eq!(row["summary_status"], "pending");
    assert_eq!(row["narrated_status"], "pending");
    assert_eq!(row["push_status"], "pending");
}

#[test]
fn rust_scan_observe_writes_python_compatible_ledger_and_replaces_same_slot() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    fs::write(
        app_dir.join("voice_settings.json"),
        serde_json::to_string(&json!({"tts_enabled_for_narration": false})).unwrap(),
    )
    .unwrap();

    let first = ClassifiedAssistantMessage {
        message_id: "m-old".to_string(),
        message_class: "final_response".to_string(),
        text: "older final answer body".to_string(),
        ts: Some(10.0),
    };
    let second = ClassifiedAssistantMessage {
        message_id: "m-new".to_string(),
        message_class: "final_response".to_string(),
        text: "new final answer body".to_string(),
        ts: Some(11.0),
    };
    let report = observe_messages_into_ledger(app_dir, "sid", "Repo", &[first], false).unwrap();
    assert_eq!(report.rows_created, 1);
    let report = observe_messages_into_ledger(app_dir, "sid", "Repo", &[second], false).unwrap();
    assert_eq!(report.rows_created, 1);
    assert_eq!(report.rows_replaced, 1);

    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["m-old"]["narrated_status"], "skipped");
    assert_eq!(ledger["m-old"]["last_error"], "replaced by newer message");
    assert_eq!(ledger["m-new"]["summary_status"], "pending");
    assert_eq!(ledger["m-new"]["push_status"], "pending");
    assert!(fs::read_to_string(app_dir.join(DELIVERY_LEDGER_FILE))
        .unwrap()
        .ends_with('\n'));
}

#[test]
fn narration_scan_obeys_disabled_narration_setting() {
    let dir = TempDir::new().unwrap();
    let msg = ClassifiedAssistantMessage {
        message_id: "n1".to_string(),
        message_class: "narration".to_string(),
        text: "working update".to_string(),
        ts: Some(12.0),
    };
    observe_messages_into_ledger(dir.path(), "sid", "Repo", &[msg], false).unwrap();
    let ledger: Value =
        serde_json::from_slice(&fs::read(dir.path().join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["n1"]["summary_status"], "skipped");
    assert_eq!(ledger["n1"]["narrated_status"], "skipped");
    assert_eq!(ledger["n1"]["push_status"], "skipped");
}

#[test]
fn scan_reads_codex_and_pi_log_fixtures_and_dedupes_on_restart() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    fs::write(
        app_dir.join("voice_settings.json"),
        serde_json::to_string(&json!({"tts_enabled_for_narration": true})).unwrap(),
    )
    .unwrap();
    let codex_log = repo_fixture_path(&["tests", "fixtures", "rollout", "codex_basic.jsonl"]);
    let pi_log = app_dir.join("pi-with-narration.jsonl");
    let mut pi_fixture = fs::read_to_string(repo_fixture_path(&[
        "tests",
        "fixtures",
        "pi",
        "pi_basic.jsonl",
    ]))
    .unwrap();
    pi_fixture.push_str("\n{\"type\":\"message\",\"timestamp\":\"2026-05-09T11:00:04Z\",\"message\":{\"role\":\"assistant\",\"stopReason\":\"toolUse\",\"content\":[{\"type\":\"text\",\"text\":\"Working update\"},{\"type\":\"toolCall\",\"name\":\"Read\"}]}}\n");
    fs::write(&pi_log, pi_fixture).unwrap();

    let codex_messages = assistant_messages_from_log(&codex_log).unwrap();
    assert_eq!(codex_messages.len(), 1);
    assert_eq!(codex_messages[0].message_class, "final_response");
    let pi_messages = assistant_messages_from_log(&pi_log).unwrap();
    assert!(pi_messages
        .iter()
        .any(|msg| msg.message_class == "narration"));
    assert!(pi_messages
        .iter()
        .any(|msg| msg.message_class == "final_response"));

    let rows = vec![
        SessionRow {
            session_id: "codex-session".to_string(),
            alias: "Codex Fixture".to_string(),
            cwd: "/tmp/codex".to_string(),
            log_path: Some(codex_log.to_string_lossy().into_owned()),
            ..SessionRow::default()
        },
        SessionRow {
            session_id: "pi-session".to_string(),
            alias: "Pi Fixture".to_string(),
            cwd: "/tmp/pi".to_string(),
            log_path: Some(pi_log.to_string_lossy().into_owned()),
            ..SessionRow::default()
        },
    ];
    let first = voice_scan_rows_once(app_dir, &rows).unwrap();
    assert_eq!(first.sessions_scanned, 2);
    assert_eq!(first.rows_created, codex_messages.len() + pi_messages.len());
    let second = voice_scan_rows_once(app_dir, &rows).unwrap();
    assert_eq!(second.rows_created, 0);
    assert_eq!(second.rows_replaced, 0);
}

#[test]
fn scan_ignores_malformed_log_lines_and_replaces_existing_pending_final_response() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    let log = app_dir.join("malformed-codex.jsonl");
    fs::write(
        &log,
        [
            "not-json",
            r#"{"type":"response_item","timestamp":"2026-05-09T10:00:02Z","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"newer final answer"}]}}"#,
        ]
        .join("\n"),
    )
    .unwrap();
    let old = ClassifiedAssistantMessage {
        message_id: "old-final".to_string(),
        message_class: "final_response".to_string(),
        text: "older final answer".to_string(),
        ts: Some(1.0),
    };
    observe_messages_into_ledger(app_dir, "sid", "Repo", &[old], false).unwrap();
    let rows = vec![SessionRow {
        session_id: "sid".to_string(),
        alias: "Repo".to_string(),
        cwd: "/tmp/repo".to_string(),
        log_path: Some(log.to_string_lossy().into_owned()),
        ..SessionRow::default()
    }];
    let report = voice_scan_rows_once(app_dir, &rows).unwrap();
    assert_eq!(report.rows_created, 1);
    assert_eq!(report.rows_replaced, 1);
    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(
        ledger["old-final"]["last_error"],
        "replaced by newer message"
    );
}

#[test]
fn listener_runtime_tracks_ttl_drop_and_updates_voice_snapshot() {
    reset_runtime_registry_for_tests();
    let dir = TempDir::new().unwrap();
    let runtime = runtime_for_app_dir(dir.path());
    fs::write(
        dir.path().join(DELIVERY_LEDGER_FILE),
        serde_json::to_string(&json!({
            "q1": {
                "message_id":"q1", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"final_response", "preview_text":"body", "notification_text":"",
                "summary_text":"", "summary_status":"pending", "narrated_status":"pending",
                "push_status":"pending", "voice":"", "created_ts":1.0, "updated_ts":1.0,
                "last_error":""
            }
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        runtime
            .listener_heartbeat("c1", true, 100.0)
            .unwrap()
            .active_listener_count,
        1
    );
    runtime.enqueue_for_tests(QueuedVoiceTask {
        message_id: "q1".to_string(),
        source_message_ids: vec!["q1".to_string()],
    });
    assert_eq!(runtime.snapshot(101.0).queue_depth, 1);
    let update = runtime.listener_heartbeat("c1", false, 102.0).unwrap();
    assert!(update.last_listener_dropped);
    assert_eq!(update.active_listener_count, 0);
    assert_eq!(runtime.snapshot(102.0).queue_depth, 0);
    let ledger: Value =
        serde_json::from_slice(&fs::read(dir.path().join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["q1"]["narrated_status"], "skipped");
    assert_eq!(ledger["q1"]["last_error"], "no active listener");

    runtime.listener_heartbeat("c2", true, 200.0).unwrap();
    assert_eq!(runtime.snapshot(200.0).active_listener_count, 1);
    assert_eq!(runtime.snapshot(246.0).active_listener_count, 0);
}

#[tokio::test(flavor = "current_thread")]
async fn listener_route_updates_settings_snapshot_and_test_push_requires_worker_flag() {
    reset_runtime_registry_for_tests();
    let _worker_guard = EnvGuard::set("CODOXEAR_ENABLE_VOICE_WORKER", "1");
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);

    let (status, body) = post_json(
        app.clone(),
        "/api/audio/listener",
        Some(&cookie),
        json!({"client_id":"c1", "enabled": true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active_listener_count"], 1);

    let (settings_status, _, settings_body) =
        get(app.clone(), "/api/settings/voice", Some(&cookie)).await;
    assert_eq!(settings_status, StatusCode::OK);
    let settings: Value = serde_json::from_slice(&settings_body).unwrap();
    assert_eq!(settings["audio"]["active_listener_count"], 1);

    let (disabled_status, disabled_body) = post_json(
        app,
        "/api/notifications/test_push",
        Some(&cookie),
        json!({}),
    )
    .await;
    assert_eq!(disabled_status, StatusCode::BAD_REQUEST);
    assert!(disabled_body["error"]
        .as_str()
        .unwrap()
        .contains("no enabled mobile"));
}

#[test]
fn openai_summary_payload_and_response_parsing_match_python_shapes() {
    let payload = summary_payload(
        "gpt-test",
        "Long body",
        30,
        "Repo",
        "Final assistant response",
    );
    assert_eq!(payload["model"], "gpt-test");
    assert!(payload["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("about 30 words"));
    assert_eq!(
        parse_summary_response(br#"{"choices":[{"message":{"content":"  hello   world "}}]}"#)
            .unwrap(),
        "hello world"
    );
    assert_eq!(
        parse_summary_response(
            br#"{"choices":[{"message":{"content":[{"type":"text","text":"hello "},{"type":"output_text","text":"world"}]}}]}"#,
        )
        .unwrap(),
        "hello world"
    );
    assert!(parse_summary_response(br#"{"choices":[]}"#)
        .unwrap_err()
        .contains("choices"));
}

#[test]
fn openai_endpoint_builder_accepts_default_https_base_url() {
    assert_eq!(
        openai_endpoint_url("https://api.openai.com/v1", "/chat/completions").unwrap(),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        openai_endpoint_url("https://api.openai.com/v1/", "/chat/completions").unwrap(),
        "https://api.openai.com/v1/chat/completions"
    );
    assert_eq!(
        openai_endpoint_url("http://127.0.0.1:8080/v1/", "/audio/speech").unwrap(),
        "http://127.0.0.1:8080/v1/audio/speech"
    );
    assert!(
        openai_endpoint_url("ftp://api.openai.com/v1", "/audio/speech")
            .unwrap_err()
            .contains("tts_base_url")
    );
}

#[test]
fn hls_stream_appends_fake_segments_silence_and_rolls_cleanup() {
    let dir = TempDir::new().unwrap();
    let mut stream = MergedHlsStream::new(dir.path().join("audio")).unwrap();
    let duration = stream
        .append_audio(&FakeHlsRunner, "message-abcdef1234567890", b"aac")
        .unwrap();
    assert_eq!(duration, 13.25);
    let playlist = fs::read_to_string(dir.path().join("audio/live.m3u8")).unwrap();
    assert!(playlist.contains("000001-message-abcd.ts"));
    assert!(playlist.contains("000002-message-abcd.ts"));
    assert_eq!(stream.snapshot().segment_count, 2);
    assert!(!stream.append_silence(&FakeHlsRunner, false, 1.0).unwrap());
    assert!(stream.append_silence(&FakeHlsRunner, true, 2.0).unwrap());
    assert!(parse_duration("N/A").unwrap_err().contains("N/A"));

    for idx in 0..25 {
        stream
            .append_silence(&FakeHlsRunner, true, 100.0 + idx as f64)
            .unwrap();
    }
    let snapshot = stream.snapshot();
    assert_eq!(snapshot.segment_count, HLS_MAX_SEGMENTS);
    let segment_count = fs::read_dir(dir.path().join("audio/segments"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .and_then(|value| value.to_str())
                == Some("ts")
        })
        .count();
    assert_eq!(segment_count, HLS_MAX_SEGMENTS);
}

#[test]
fn webpush_message_builds_encrypted_payload_with_ttl_and_vapid_headers() {
    let dir = TempDir::new().unwrap();
    let pem = dir.path().join(VAPID_PRIVATE_KEY_FILE);
    fs::copy(fixture_path(VAPID_PRIVATE_KEY_FILE), &pem).unwrap();
    let target = WebPushTarget {
        id: "sub1".to_string(),
        endpoint: "http://127.0.0.1:9/push".to_string(),
        p256dh: "BH1HTeKM7-NwaLGHEqxeu2IamQaVVLkcsFHPIHmsCnqxcBHPQBprF41bEMOr3O1hUQ2jU1opNEm1F_lZV_sxMP8".to_string(),
        auth: "sBXU5_tIYz-5w7G2B25BEw".to_string(),
    };
    let message = build_webpush_message(
        &pem,
        &target,
        &WebPushPayload::final_response_default("s", "Repo", "m", 1000).to_json_value(),
        WEB_PUSH_TTL_SECONDS,
        "https://localhost",
    )
    .unwrap();
    assert_eq!(message.ttl, WEB_PUSH_TTL_SECONDS);
    assert!(message.payload.is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn enabled_test_announcement_enqueues_ledger_row() {
    reset_runtime_registry_for_tests();
    let _guard = EnvGuard::set("CODOXEAR_ENABLE_VOICE_WORKER", "1");
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);
    let app_dir = app_dir(&home);
    fs::write(
        app_dir.join("voice_settings.json"),
        serde_json::to_string(&json!({
            "tts_enabled_for_final_response": true,
            "tts_base_url": "http://127.0.0.1:1/v1",
            "tts_api_key": "test-key"
        }))
        .unwrap(),
    )
    .unwrap();
    let (listener_status, _) = post_json(
        app.clone(),
        "/api/audio/listener",
        Some(&cookie),
        json!({"client_id":"c1", "enabled": true}),
    )
    .await;
    assert_eq!(listener_status, StatusCode::OK);
    let (status, body) = post_json(
        app,
        "/api/audio/test_announcement",
        Some(&cookie),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["message_id"].as_str().unwrap().starts_with("test-"));
    assert_eq!(body["queue_depth"], 1);
    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(
        ledger[body["message_id"].as_str().unwrap()]["narrated_status"],
        "pending"
    );
}

#[test]
fn worker_step_processes_final_response_push_tts_and_hls_with_playing_gate() {
    reset_runtime_registry_for_tests();
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    write_voice_settings(app_dir, json!({"tts_enabled_for_final_response": true}));
    write_mobile_subscriptions(app_dir, &["https://push.example.test/ok"]);
    write_ledger(
        app_dir,
        json!({
            "m1": {
                "message_id":"m1", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"final_response", "preview_text":"Longer final answer body",
                "notification_text":"", "summary_text":"", "summary_status":"pending",
                "narrated_status":"pending", "push_status":"pending", "voice":"",
                "created_ts":1.0, "updated_ts":1.0, "last_error":""
            },
            "m2": {
                "message_id":"m2", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"final_response", "preview_text":"Second final answer body",
                "notification_text":"", "summary_text":"", "summary_status":"pending",
                "narrated_status":"pending", "push_status":"pending", "voice":"",
                "created_ts":2.0, "updated_ts":2.0, "last_error":""
            }
        }),
    );
    let runtime = runtime_for_app_dir(app_dir);
    runtime.listener_heartbeat("c1", true, 10.0).unwrap();
    let client = FakeOpenAi::success();
    let push = FakePushSender::new(vec![Ok(()), Ok(())]);

    let first = runtime
        .worker_step(&client, &FakeHlsRunner, &push, 10.0)
        .unwrap()
        .unwrap();
    assert_eq!(first.action, "prepared");
    let appended = runtime
        .worker_step(&client, &FakeHlsRunner, &push, 10.1)
        .unwrap()
        .unwrap();
    assert_eq!(appended.action, "appended");
    assert!(runtime
        .worker_step(&client, &FakeHlsRunner, &push, 11.0)
        .unwrap()
        .is_none());
    let second = runtime
        .worker_step(&client, &FakeHlsRunner, &push, 24.0)
        .unwrap()
        .unwrap();
    assert_eq!(second.message_id, "m2");

    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["m1"]["summary_status"], "sent");
    assert_eq!(ledger["m1"]["summary_text"], "short summary");
    assert_eq!(ledger["m1"]["notification_text"], "short summary");
    assert_eq!(ledger["m1"]["push_status"], "sent");
    assert_eq!(ledger["m1"]["narrated_status"], "sent");
    assert_eq!(
        client.summary_calls.lock().unwrap()[0].0.source_label,
        "Final assistant response"
    );
    assert_eq!(
        client.speech_calls.lock().unwrap()[0].1,
        "Turn summary from Repo. short summary"
    );
    let push_call = &push.calls.lock().unwrap()[0];
    assert_eq!(push_call.2, WEB_PUSH_TTL_SECONDS);
    assert_eq!(push_call.1["notification_text"], "回复完成");
    assert!(fs::read_to_string(app_dir.join("audio/live.m3u8"))
        .unwrap()
        .contains("segments/000001-m1.ts"));
}

#[test]
fn worker_step_processes_narration_and_continues_after_tts_error() {
    reset_runtime_registry_for_tests();
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    write_voice_settings(
        app_dir,
        json!({"tts_enabled_for_narration": true, "tts_enabled_for_final_response": false}),
    );
    write_ledger(
        app_dir,
        json!({
            "n1": {
                "message_id":"n1", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"narration", "preview_text":"Long and verbose narration body",
                "notification_text":"", "summary_text":"", "summary_status":"pending",
                "narrated_status":"pending", "push_status":"skipped", "voice":"",
                "created_ts":1.0, "updated_ts":1.0, "last_error":""
            },
            "n2": {
                "message_id":"n2", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"narration", "preview_text":"Next narration body",
                "notification_text":"", "summary_text":"", "summary_status":"pending",
                "narrated_status":"pending", "push_status":"skipped", "voice":"",
                "created_ts":2.0, "updated_ts":2.0, "last_error":""
            }
        }),
    );
    let runtime = runtime_for_app_dir(app_dir);
    runtime.listener_heartbeat("c1", true, 10.0).unwrap();
    let failing_client = FakeOpenAi::with_speech_error("audio/speech returned empty body");
    let push = FakePushSender::default();
    let first = runtime
        .worker_step(&failing_client, &FakeHlsRunner, &push, 10.0)
        .unwrap()
        .unwrap();
    assert_eq!(first.action, "error");
    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["n1"]["narrated_status"], "error");
    assert_eq!(ledger["n1"]["summary_status"], "error");
    assert!(ledger["n1"]["last_error"]
        .as_str()
        .unwrap()
        .contains("empty body"));

    let ok_client = FakeOpenAi::success();
    let second = runtime
        .worker_step(&ok_client, &FakeHlsRunner, &push, 11.0)
        .unwrap()
        .unwrap();
    assert_eq!(second.action, "prepared");
    assert_eq!(
        ok_client.summary_calls.lock().unwrap()[0].0.target_words,
        15
    );
    assert_eq!(
        ok_client.speech_calls.lock().unwrap()[0].1,
        "From Repo. short summary"
    );
}

#[test]
fn final_response_push_partial_failure_all_failure_and_stale_drop_are_recorded() {
    reset_runtime_registry_for_tests();
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    write_voice_settings(app_dir, json!({"tts_enabled_for_final_response": false}));
    write_mobile_subscriptions(
        app_dir,
        &[
            "https://push.example.test/ok",
            "https://push.example.test/fail",
            "https://removed.invalid/stale",
        ],
    );
    write_ledger(
        app_dir,
        json!({
            "m1": {
                "message_id":"m1", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"final_response", "preview_text":"Final body", "notification_text":"",
                "summary_text":"", "summary_status":"pending", "narrated_status":"pending",
                "push_status":"pending", "voice":"", "created_ts":1.0, "updated_ts":1.0,
                "last_error":""
            }
        }),
    );
    let runtime = runtime_for_app_dir(app_dir);
    runtime.listener_heartbeat("c1", true, 10.0).unwrap();
    let push = FakePushSender::new(vec![
        Err(WebPushSendError::StaleSubscription(410)),
        Err(WebPushSendError::Transport("timeout".to_string())),
        Ok(()),
    ]);
    let report = runtime
        .worker_step(&FakeOpenAi::success(), &FakeHlsRunner, &push, 10.0)
        .unwrap()
        .unwrap();
    assert_eq!(report.action, "final-push");
    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["m1"]["push_status"], "sent");
    assert_eq!(ledger["m1"]["narrated_status"], "skipped");
    let subscriptions: Value =
        serde_json::from_slice(&fs::read(app_dir.join("push_subscriptions.json")).unwrap())
            .unwrap();
    assert_eq!(subscriptions.as_array().unwrap().len(), 2);
    assert_eq!(push.calls.lock().unwrap().len(), 3);
    assert!(push
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(_, payload, ttl, _)| {
            *ttl == 300 && payload["notification_text"] == "回复完成"
        }));

    reset_runtime_registry_for_tests();
    write_mobile_subscriptions(app_dir, &["https://push.example.test/fail"]);
    write_ledger(
        app_dir,
        json!({
            "m2": {
                "message_id":"m2", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"final_response", "preview_text":"Final body", "notification_text":"",
                "summary_text":"", "summary_status":"pending", "narrated_status":"pending",
                "push_status":"pending", "voice":"", "created_ts":2.0, "updated_ts":2.0,
                "last_error":""
            }
        }),
    );
    let runtime = runtime_for_app_dir(app_dir);
    runtime.listener_heartbeat("c1", true, 20.0).unwrap();
    let push = FakePushSender::new(vec![Err(WebPushSendError::Transport(
        "offline".to_string(),
    ))]);
    runtime
        .worker_step(&FakeOpenAi::success(), &FakeHlsRunner, &push, 20.0)
        .unwrap();
    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["m2"]["push_status"], "error");
}

#[test]
fn final_response_summary_error_sends_push_but_does_not_tts_or_hls() {
    reset_runtime_registry_for_tests();
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    write_voice_settings(app_dir, json!({"tts_enabled_for_final_response": true}));
    write_mobile_subscriptions(app_dir, &["https://push.example.test/ok"]);
    write_ledger(
        app_dir,
        json!({
            "m1": {
                "message_id":"m1", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"final_response", "preview_text":"Final body", "notification_text":"",
                "summary_text":"", "summary_status":"pending", "narrated_status":"pending",
                "push_status":"pending", "voice":"", "created_ts":1.0, "updated_ts":1.0,
                "last_error":""
            }
        }),
    );
    let runtime = runtime_for_app_dir(app_dir);
    runtime.listener_heartbeat("c1", true, 10.0).unwrap();
    let client = FakeOpenAi::with_summary_error("/chat/completions failed with 500");
    let push = FakePushSender::new(vec![Ok(())]);
    let report = runtime
        .worker_step(&client, &FakeHlsRunner, &push, 10.0)
        .unwrap()
        .unwrap();
    assert_eq!(report.action, "summary-error");
    assert!(runtime
        .worker_step(&client, &FakeHlsRunner, &push, 10.1)
        .unwrap()
        .is_none());
    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["m1"]["summary_status"], "error");
    assert_eq!(ledger["m1"]["narrated_status"], "error");
    assert_eq!(ledger["m1"]["push_status"], "sent");
    assert!(client.speech_calls.lock().unwrap().is_empty());
    assert_eq!(push.calls.lock().unwrap().len(), 1);
    assert!(!app_dir.join("audio/live.m3u8").exists());
}

#[test]
fn final_response_summary_error_row_is_not_requeued_for_tts_after_restart() {
    reset_runtime_registry_for_tests();
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    write_voice_settings(app_dir, json!({"tts_enabled_for_final_response": true}));
    write_ledger(
        app_dir,
        json!({
            "m1": {
                "message_id":"m1", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"final_response", "preview_text":"Final body", "notification_text":"Final body",
                "summary_text":"", "summary_status":"error", "narrated_status":"pending",
                "push_status":"sent", "voice":"", "created_ts":1.0, "updated_ts":1.0,
                "last_error":"/chat/completions failed with 500"
            }
        }),
    );
    let runtime = runtime_for_app_dir(app_dir);
    runtime.listener_heartbeat("c1", true, 10.0).unwrap();
    assert_eq!(runtime.enqueue_pending_ledger_for_delivery().unwrap(), 0);
    assert!(runtime
        .worker_step(
            &FakeOpenAi::success(),
            &FakeHlsRunner,
            &FakePushSender::default(),
            10.0
        )
        .unwrap()
        .is_none());

    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger["m1"]["summary_status"], "error");
    assert_eq!(ledger["m1"]["narrated_status"], "pending");
    assert!(!app_dir.join("audio/live.m3u8").exists());
}

#[test]
fn successful_test_announcement_tts_appends_hls_side_effect() {
    reset_runtime_registry_for_tests();
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path();
    write_voice_settings(app_dir, json!({"tts_enabled_for_final_response": true}));
    let runtime = runtime_for_app_dir(app_dir);
    runtime
        .listener_heartbeat("c1", true, unix_now_seconds() as f64)
        .unwrap();
    let (message_id, queue_depth) = runtime
        .enqueue_test_announcement("alloy".to_string())
        .unwrap();
    assert_eq!(queue_depth, 1);
    let client = FakeOpenAi::success();
    let prepared = runtime
        .worker_step(&client, &FakeHlsRunner, &FakePushSender::default(), 10.0)
        .unwrap()
        .unwrap();
    assert_eq!(prepared.action, "prepared");
    let appended = runtime
        .worker_step(&client, &FakeHlsRunner, &FakePushSender::default(), 10.1)
        .unwrap()
        .unwrap();
    assert_eq!(appended.action, "appended");
    let ledger: Value =
        serde_json::from_slice(&fs::read(app_dir.join(DELIVERY_LEDGER_FILE)).unwrap()).unwrap();
    assert_eq!(ledger[&message_id]["narrated_status"], "sent");
    assert!(fs::read_to_string(app_dir.join("audio/live.m3u8"))
        .unwrap()
        .contains(&format!("segments/000001-{}.ts", &message_id[..12])));
}

#[test]
fn hls_playlist_rendering_and_segment_guard_match_contract() {
    assert_eq!(
        empty_playlist(1),
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:12\n#EXT-X-MEDIA-SEQUENCE:1\n"
    );
    let dir = TempDir::new().unwrap();
    let segments = vec![HlsSegment {
        seq: 7,
        name: "000007-msg.ts".to_string(),
        duration: 6.0,
        path: dir.path().join("000007-msg.ts"),
    }];
    let playlist = render_playlist(7, &segments);
    assert!(playlist.starts_with("#EXTM3U\n#EXT-X-VERSION:3"));
    assert!(playlist.contains("#EXT-X-MEDIA-SEQUENCE:7"));
    assert!(playlist.contains("#EXTINF:6.000,\nsegments/000007-msg.ts"));
    assert_eq!(HLS_MAX_SEGMENTS, 18);

    assert!(safe_segment_path(dir.path(), "000001-ok.ts").is_some());
    assert!(safe_segment_path(dir.path(), "../secret.ts").is_none());
    assert!(safe_segment_path(dir.path(), "bad.aac").is_none());
}

#[test]
fn webpush_payload_ttl_and_stale_drop_match_python_semantics() {
    let payload = WebPushPayload::final_response_default("s1", "Session", "m1", 1234);
    assert_eq!(payload.notification_text, "回复完成");
    assert_eq!(payload.to_json_value()["notification_text"], "回复完成");
    assert_eq!(WEB_PUSH_TTL_SECONDS, 300);
    assert!(should_drop_subscription("https://gone.invalid/sub", None));
    assert!(should_drop_subscription(
        "https://push.example/sub",
        Some(&WebPushSendError::StaleSubscription(410))
    ));
    assert!(!should_drop_subscription(
        "https://push.example/sub",
        Some(&WebPushSendError::Transport("timeout".to_string()))
    ));
}

#[tokio::test]
async fn hls_routes_are_authenticated_path_safe_and_use_existing_artifacts() {
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);
    let app_dir = app_dir(&home);
    let segments = app_dir.join("audio/segments");
    fs::create_dir_all(&segments).unwrap();
    fs::write(
        app_dir.join("audio/live.m3u8"),
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:12\n#EXT-X-MEDIA-SEQUENCE:7\n#EXTINF:6.000,\nsegments/voice-000.ts\n",
    )
    .unwrap();
    fs::write(segments.join("voice-000.ts"), b"fake-ts").unwrap();

    let (unauth, _, _) = get(app.clone(), "/api/audio/live.m3u8", None).await;
    assert_eq!(unauth, StatusCode::UNAUTHORIZED);

    let (playlist_status, playlist_type, playlist_body) =
        get(app.clone(), "/api/audio/live.m3u8", Some(&cookie)).await;
    assert_eq!(playlist_status, StatusCode::OK);
    assert_eq!(playlist_type, "application/vnd.apple.mpegurl");
    assert!(String::from_utf8_lossy(&playlist_body).contains("segments/voice-000.ts"));

    let (segment_status, segment_type, segment_body) = get(
        app.clone(),
        "/api/audio/segments/voice-000.ts",
        Some(&cookie),
    )
    .await;
    assert_eq!(segment_status, StatusCode::OK);
    assert_eq!(segment_type, "video/mp2t");
    assert_eq!(segment_body, b"fake-ts");

    let (traversal, _, _) = get(
        app.clone(),
        "/api/audio/segments/../voice-000.ts",
        Some(&cookie),
    )
    .await;
    assert_eq!(traversal, StatusCode::NOT_FOUND);

    let (wrong_ext, _, _) = get(app, "/api/audio/segments/voice-000.aac", Some(&cookie)).await;
    assert_eq!(wrong_ext, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "current_thread")]
async fn voice_routes_preserve_v1_alias_auth_snapshots_and_disabled_debug_no_side_effects() {
    reset_runtime_registry_for_tests();
    let _worker_guard = EnvGuard::set("CODOXEAR_ENABLE_VOICE_WORKER", "");
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);
    let app_dir = app_dir(&home);
    fs::create_dir_all(app_dir.join("audio/segments")).unwrap();
    fs::write(
        app_dir.join("audio/live.m3u8"),
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n#EXT-X-MEDIA-SEQUENCE:1\n#EXTINF:1.000,\nsegments/alias.ts\n",
    )
    .unwrap();
    fs::write(app_dir.join("audio/segments/alias.ts"), b"ts").unwrap();
    write_mobile_subscriptions(&app_dir, &["https://push.example.test/ok"]);
    write_ledger(
        &app_dir,
        json!({
            "m1": {
                "message_id":"m1", "session_id":"sid", "session_display_name":"Repo",
                "message_class":"final_response", "preview_text":"Final body",
                "notification_text":"Summary", "summary_text":"Summary", "summary_status":"sent",
                "narrated_status":"skipped", "push_status":"sent", "voice":"alloy",
                "created_ts":1.0, "updated_ts":2.0, "last_error":""
            }
        }),
    );

    let (settings_status, _, settings_body) =
        get(app.clone(), "/api/v1/settings/voice", Some(&cookie)).await;
    assert_eq!(settings_status, StatusCode::OK);
    let settings: Value = serde_json::from_slice(&settings_body).unwrap();
    assert_eq!(settings["audio"]["stream_url"], "/api/audio/live.m3u8");
    assert_eq!(settings["notifications"]["enabled_devices"], 1);

    let (subs_status, _, subs_body) = get(
        app.clone(),
        "/api/v1/notifications/subscription",
        Some(&cookie),
    )
    .await;
    assert_eq!(subs_status, StatusCode::OK);
    let subs: Value = serde_json::from_slice(&subs_body).unwrap();
    assert_eq!(subs["subscriptions"].as_array().unwrap().len(), 1);

    let (message_status, _, message_body) = get(
        app.clone(),
        "/api/v1/notifications/message?message_id=m1",
        Some(&cookie),
    )
    .await;
    assert_eq!(message_status, StatusCode::OK);
    let message: Value = serde_json::from_slice(&message_body).unwrap();
    assert_eq!(message["notification_text"], "Summary");
    let (feed_status, _, feed_body) = get(
        app.clone(),
        "/api/v1/notifications/feed?since=0",
        Some(&cookie),
    )
    .await;
    assert_eq!(feed_status, StatusCode::OK);
    let feed: Value = serde_json::from_slice(&feed_body).unwrap();
    assert_eq!(feed["items"][0]["message_id"], "m1");

    let (playlist_status, _, _) = get(app.clone(), "/api/v1/audio/live.m3u8", Some(&cookie)).await;
    assert_eq!(playlist_status, StatusCode::OK);
    let (segment_status, _, _) = get(
        app.clone(),
        "/api/v1/audio/segments/alias.ts",
        Some(&cookie),
    )
    .await;
    assert_eq!(segment_status, StatusCode::OK);
    let (missing_segment_status, _, _) = get(
        app.clone(),
        "/api/v1/audio/segments/missing.ts",
        Some(&cookie),
    )
    .await;
    assert_eq!(missing_segment_status, StatusCode::NOT_FOUND);

    let (unauth_debug, _) = post_json(
        app.clone(),
        "/api/v1/notifications/test_push",
        None,
        json!({}),
    )
    .await;
    assert_eq!(unauth_debug, StatusCode::UNAUTHORIZED);
    let (disabled_push_status, disabled_push) = post_json(
        app.clone(),
        "/api/v1/notifications/test_push",
        Some(&cookie),
        json!({}),
    )
    .await;
    assert_eq!(disabled_push_status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(disabled_push["phase"], "phase5");
    let (disabled_audio_status, _) = post_json(
        app,
        "/api/v1/audio/test_announcement",
        Some(&cookie),
        json!({}),
    )
    .await;
    assert_eq!(disabled_audio_status, StatusCode::NOT_IMPLEMENTED);
    assert!(!app_dir.join("audio_announcement_queue.json").exists());
}
