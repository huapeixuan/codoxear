use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::get;
use axum::Router;
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::handlers::health_meta;
use codoxear_backend_rs::runtime::RuntimeConfig;
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

fn test_app() -> (TempDir, axum::Router) {
    let home = TempDir::new().expect("temp home");
    let app_dir = home.path().join(".local/share/codoxear");
    let state = AppState {
        config: RuntimeConfig { app_dir },
    };
    let app = Router::new()
        .route("/health", get(health_meta::health))
        .route("/me", get(health_meta::me))
        .route("/sessions/bootstrap", get(health_meta::sessions_bootstrap))
        .with_state(state);
    (home, app)
}

async fn assert_json_200(app: axum::Router, path: &str) -> Vec<u8> {
    let response = app
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8",
        "{path}"
    );
    response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec()
}

#[tokio::test]
async fn health_meta_module_owns_phase1_handlers() {
    let (_home, app) = test_app();

    assert_eq!(
        assert_json_200(app.clone(), "/health").await,
        br#"{"ok":true,"service":"codoxear-backend-rs"}"#
    );
    assert_eq!(assert_json_200(app.clone(), "/me").await, br#"{"ok":true}"#);
    let bootstrap = assert_json_200(app, "/sessions/bootstrap").await;
    let parsed: serde_json::Value = serde_json::from_slice(&bootstrap).unwrap();
    assert_eq!(parsed["recent_cwds"], serde_json::json!([]));
    assert_eq!(parsed["cwd_groups"], serde_json::json!({}));
}
