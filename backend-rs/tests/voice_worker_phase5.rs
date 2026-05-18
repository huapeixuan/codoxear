use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::routes::router;
use codoxear_backend_rs::runtime::{
    load_or_create_hmac_secret, sign_auth_cookie_value, unix_now_seconds, RuntimeConfig,
};
use codoxear_backend_rs::voice_worker::hls::{
    empty_playlist, render_playlist, safe_segment_path, HlsSegment, HLS_MAX_SEGMENTS,
};
use codoxear_backend_rs::voice_worker::ledger::{
    new_pending_final_response, write_ledger_trimmed, DELIVERY_LEDGER_FILE,
};
use codoxear_backend_rs::voice_worker::locks::VoiceOwnerLock;
use codoxear_backend_rs::voice_worker::vapid::{
    default_vapid_subject_from_env, load_or_create_public_key, normalize_vapid_subject,
    public_key_from_pem, VAPID_PRIVATE_KEY_FILE,
};
use codoxear_backend_rs::voice_worker::webpush::{
    should_drop_subscription, WebPushPayload, WebPushSendError, WEB_PUSH_TTL_SECONDS,
};
use codoxear_backend_rs::workers::voice_worker_role;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use tower::ServiceExt;

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
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

#[test]
fn voice_worker_role_combines_scan_and_delivery_under_one_owner() {
    assert_eq!(voice_worker_role(false, false), None);
    assert_eq!(voice_worker_role(true, false), Some("scan"));
    assert_eq!(voice_worker_role(false, true), Some("worker"));
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
