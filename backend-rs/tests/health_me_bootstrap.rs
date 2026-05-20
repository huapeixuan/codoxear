use axum::body::Body;
use axum::http::{Request, StatusCode};
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::routes::{router, router_with_url_prefix_and_static_dir};
use codoxear_backend_rs::runtime::{
    load_or_create_hmac_secret, sign_auth_cookie_value, unix_now_seconds, RuntimeConfig,
};
use http_body_util::BodyExt;
use std::fs;
use tempfile::TempDir;
use tower::ServiceExt;

fn test_app() -> (TempDir, axum::Router) {
    let home = TempDir::new().expect("temp home");
    let app_dir = home.path().join(".local/share/codoxear");
    let state = AppState {
        config: RuntimeConfig { app_dir },
        fake_spawn_for_tests: false,
        fake_spawn_session_id_for_tests: None,
    };
    (home, router(state))
}

async fn response_bytes(response: axum::response::Response) -> Vec<u8> {
    response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes()
        .to_vec()
}

async fn response_text(response: axum::response::Response) -> String {
    String::from_utf8(response_bytes(response).await).expect("utf8 response")
}

fn signed_cookie(home: &TempDir) -> String {
    let app_dir = home.path().join(".local/share/codoxear");
    let secret = load_or_create_hmac_secret(&app_dir).expect("hmac secret");
    let token = sign_auth_cookie_value(&secret, unix_now_seconds() + 3600).expect("sign cookie");
    format!("codoxear_auth={token}")
}

#[tokio::test]
async fn health_legacy_is_public() {
    let (_home, app) = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_bytes(response).await,
        br#"{"ok":true,"service":"codoxear-backend-rs"}"#
    );
}

#[tokio::test]
async fn health_canonical_matches_legacy() {
    let (_home, app) = test_app();
    let legacy = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let canonical = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response_bytes(canonical).await,
        response_bytes(legacy).await
    );
}

#[tokio::test]
async fn url_prefix_mounts_ui_canonical_api_and_legacy_alias() {
    let home = TempDir::new().expect("temp home");
    let app_dir = home.path().join(".local/share/codoxear");
    let state = AppState {
        config: RuntimeConfig { app_dir },
        fake_spawn_for_tests: false,
        fake_spawn_session_id_for_tests: None,
    };
    let static_dir = home.path().join("static");
    fs::create_dir_all(static_dir.join("dist/assets")).unwrap();
    fs::write(
        static_dir.join("dist/index.html"),
        concat!(
            r#"<!doctype html><html><head><title>Codoxear</title>"#,
            r#"<script type="module" src="./assets/index-test.js"></script>"#,
            r#"</head><body><div id="app"></div></body></html>"#
        ),
    )
    .unwrap();
    let app = router_with_url_prefix_and_static_dir(state, "/codoxear", static_dir).unwrap();

    let redirect = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/codoxear")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(redirect.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        redirect
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/codoxear/")
    );

    let prefixed_root = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/codoxear/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prefixed_root.status(), StatusCode::OK, "prefixed root");
    let prefixed_root_body = response_text(prefixed_root).await;
    assert!(prefixed_root_body.contains("<html"));
    assert!(!prefixed_root_body.contains("./src/main.tsx"));

    for path in [
        "/codoxear/api/v1/health",
        "/codoxear/api/health",
        "/api/v1/health",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response_text(response).await,
            r#"{"ok":true,"service":"codoxear-backend-rs"}"#
        );
    }
}

#[tokio::test]
async fn me_without_cookie_is_unauthorized() {
    let (_home, app) = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_bytes(response).await,
        br#"{"error":"unauthorized"}"#
    );
}

#[tokio::test]
async fn me_with_signed_cookie_is_ok() {
    let (home, app) = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/me")
                .header("Cookie", signed_cookie(&home))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_bytes(response).await, br#"{"ok":true}"#);
}

#[tokio::test]
async fn sessions_bootstrap_with_signed_cookie_has_phase1_keys() {
    let (home, app) = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/sessions/bootstrap")
                .header("Cookie", signed_cookie(&home))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&response_bytes(response).await).unwrap();
    assert_eq!(body["recent_cwds"], serde_json::json!([]));
    assert_eq!(body["cwd_groups"], serde_json::json!({}));
    assert!(body.get("new_session_defaults").is_some());
    assert!(body
        .get("tmux_available")
        .and_then(|value| value.as_bool())
        .is_some());
}

#[tokio::test]
async fn ref_style_bootstrap_path_is_not_registered() {
    let (_home, app) = test_app();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/bootstrap")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
