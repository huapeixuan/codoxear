use axum::body::Body;
use axum::http::{header, Request, StatusCode};
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

fn test_app() -> (TempDir, axum::Router, String) {
    let home = TempDir::new().expect("temp home");
    let app_dir = home.path().join(".local/share/codoxear");
    let state = AppState {
        config: RuntimeConfig {
            app_dir: app_dir.clone(),
        },
    };
    let secret = load_or_create_hmac_secret(&app_dir).expect("hmac secret");
    let token = sign_auth_cookie_value(&secret, unix_now_seconds() + 3600).expect("sign cookie");
    (home, router(state), format!("codoxear_auth={token}"))
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
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8",
        "{path}"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn voice_and_subscription_routes_return_snapshots_on_v1_and_legacy_paths() {
    let (home, app, cookie) = test_app();
    let app_dir = home.path().join(".local/share/codoxear");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("voice_settings.json"),
        json!({"tts_enabled_for_narration": true, "tts_model": "custom"}).to_string(),
    )
    .unwrap();
    fs::write(
        app_dir.join("push_subscriptions.json"),
        json!([{
            "subscription": {"endpoint": "https://push.example/a", "keys": {"p256dh": "p", "auth": "a"}},
            "notifications_enabled": true,
            "updated_ts": 1.0,
            "user_agent": "Mobile"
        }])
        .to_string(),
    )
    .unwrap();

    for path in ["/api/settings/voice", "/api/v1/settings/voice"] {
        let (status, body) = get_json(app.clone(), path, &cookie).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], json!(true));
        assert_eq!(body["tts_enabled_for_narration"], json!(true));
        assert_eq!(body["tts_model"], json!("custom"));
    }
    for path in [
        "/api/notifications/subscription",
        "/api/v1/notifications/subscription",
    ] {
        let (status, body) = get_json(app.clone(), path, &cookie).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], json!(true));
        assert_eq!(body["subscriptions"].as_array().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn notification_message_and_feed_routes_cover_error_and_success_paths() {
    let (home, app, cookie) = test_app();
    let app_dir = home.path().join(".local/share/codoxear");
    fs::create_dir_all(&app_dir).unwrap();
    fs::write(
        app_dir.join("voice_delivery_ledger.json"),
        json!({
            "msg-1": {
                "session_id": "sess-a",
                "session_display_name": "Alpha",
                "message_class": "final_response",
                "notification_text": "done",
                "summary_status": "sent",
                "push_status": "sent",
                "updated_ts": 10.0
            }
        })
        .to_string(),
    )
    .unwrap();

    let (status, body) = get_json(app.clone(), "/api/notifications/message", &cookie).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, json!({"error": "message_id required"}));

    let (status, body) = get_json(
        app.clone(),
        "/api/notifications/message?message_id=missing",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({"error": "unknown message"}));

    let (status, body) = get_json(
        app.clone(),
        "/api/v1/notifications/message?message_id=msg-1",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["message_id"], json!("msg-1"));

    let (status, body) = get_json(app.clone(), "/api/notifications/feed?since=nope", &cookie).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body, json!({"error": "invalid since"}));

    let (status, body) = get_json(app, "/api/v1/notifications/feed?since=0", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], json!(true));
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn metrics_route_returns_empty_metrics_snapshot() {
    let (_home, app, cookie) = test_app();

    for path in ["/api/metrics", "/api/v1/metrics"] {
        let (status, body) = get_json(app.clone(), path, &cookie).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({"metrics": {}}));
    }
}
