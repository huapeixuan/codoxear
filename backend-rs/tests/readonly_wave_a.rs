use axum::body::Body;
use axum::http::{Request, StatusCode};
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::routes::router;
use codoxear_backend_rs::runtime::{
    load_or_create_hmac_secret, sign_auth_cookie_value, unix_now_seconds, RuntimeConfig,
};
use http_body_util::BodyExt;
use serde_json::json;
use std::fs;
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

fn signed_cookie(home: &TempDir) -> String {
    let app_dir = home.path().join(".local/share/codoxear");
    let secret = load_or_create_hmac_secret(&app_dir).expect("hmac secret");
    let token = sign_auth_cookie_value(&secret, unix_now_seconds() + 3600).expect("sign cookie");
    format!("codoxear_auth={token}")
}

async fn get_json(app: axum::Router, path: &str, cookie: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(path)
                .header("Cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn voice_settings_legacy_and_canonical_return_defaults() {
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);

    let (legacy_status, legacy) = get_json(app.clone(), "/api/settings/voice", &cookie).await;
    let (v1_status, v1) = get_json(app, "/api/v1/settings/voice", &cookie).await;

    assert_eq!(legacy_status, StatusCode::OK);
    assert_eq!(v1_status, StatusCode::OK);
    assert_eq!(legacy, v1);
    assert_eq!(legacy["ok"], true);
    assert_eq!(legacy["audio"]["stream_url"], "/api/audio/live.m3u8");
}

#[tokio::test]
async fn notifications_message_covers_missing_unknown_and_known() {
    let (home, app) = test_app();
    let app_dir = home.path().join(".local/share/codoxear");
    fs::write(
        app_dir.join("voice_delivery_ledger.json"),
        serde_json::to_string(&json!({
            "m1": {
                "session_id": "s1",
                "message_class": "final_response",
                "notification_text": "hello",
                "summary_status": "sent",
                "push_status": "skipped",
                "updated_ts": 5.0
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let cookie = signed_cookie(&home);

    let (missing_status, missing) =
        get_json(app.clone(), "/api/notifications/message", &cookie).await;
    let (unknown_status, unknown) = get_json(
        app.clone(),
        "/api/notifications/message?message_id=nope",
        &cookie,
    )
    .await;
    let (known_status, known) =
        get_json(app, "/api/notifications/message?message_id=m1", &cookie).await;

    assert_eq!(missing_status, StatusCode::BAD_REQUEST);
    assert_eq!(missing, json!({"error": "message_id required"}));
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    assert_eq!(unknown, json!({"error": "unknown message"}));
    assert_eq!(known_status, StatusCode::OK);
    assert_eq!(known["ok"], true);
    assert_eq!(known["notification_text"], "hello");
}

#[tokio::test]
async fn notifications_feed_and_metrics_return_expected_shapes() {
    let (home, app) = test_app();
    let app_dir = home.path().join(".local/share/codoxear");
    fs::write(
        app_dir.join("voice_delivery_ledger.json"),
        serde_json::to_string(&json!({
            "m1": {
                "session_id": "s1",
                "message_class": "final_response",
                "notification_text": "hello",
                "summary_status": "sent",
                "updated_ts": 5.0
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let cookie = signed_cookie(&home);

    let (bad_status, bad) =
        get_json(app.clone(), "/api/notifications/feed?since=bad", &cookie).await;
    let (ok_status, ok) = get_json(app.clone(), "/api/notifications/feed?since=0", &cookie).await;
    let (metrics_status, metrics) = get_json(app, "/api/metrics", &cookie).await;

    assert_eq!(bad_status, StatusCode::BAD_REQUEST);
    assert_eq!(bad, json!({"error": "invalid since"}));
    assert_eq!(ok_status, StatusCode::OK);
    assert_eq!(ok["items"].as_array().unwrap().len(), 1);
    assert_eq!(metrics_status, StatusCode::OK);
    assert!(metrics["metrics"].is_object());
}
