use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::routes::router;
use codoxear_backend_rs::runtime::{
    load_or_create_hmac_secret, sign_auth_cookie_value, unix_now_seconds, RuntimeConfig,
};
use http_body_util::BodyExt;
use std::fs;
use std::sync::{Mutex, MutexGuard, OnceLock};
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

fn signed_cookie(home: &TempDir) -> String {
    let app_dir = home.path().join(".local/share/codoxear");
    let secret = load_or_create_hmac_secret(&app_dir).expect("hmac secret");
    let token = sign_auth_cookie_value(&secret, unix_now_seconds() + 3600).expect("sign cookie");
    format!("codoxear_auth={token}")
}

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

fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
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

#[test]
fn url_prefix_serves_ui_api_and_legacy_alias() {
    let _lock = env_lock();
    let _prefix = EnvGuard::set("CODEX_WEB_URL_PREFIX", "/codoxear");
    let static_dir = TempDir::new().expect("temp static dir");
    let _static_dir = EnvGuard::set(
        "CODOXEAR_STATIC_DIR",
        static_dir.path().to_str().expect("utf8 temp path"),
    );
    fs::create_dir_all(static_dir.path().join("dist/assets")).unwrap();
    fs::write(
        static_dir.path().join("dist/index.html"),
        r#"<!doctype html><div id="root"></div><script type="module" src="./assets/app.js"></script>"#,
    )
    .unwrap();
    fs::write(
        static_dir.path().join("dist/assets/app.js"),
        "console.log('ok');",
    )
    .unwrap();

    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (_home, app) = test_app();
        let prefixed_health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/codoxear/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(prefixed_health.status(), StatusCode::OK);

        let prefixed_legacy = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/codoxear/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(prefixed_legacy.status(), StatusCode::OK);

        let legacy_alias = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(legacy_alias.status(), StatusCode::OK);

        let bare_prefix = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/codoxear")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bare_prefix.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            bare_prefix
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("/codoxear/")
        );

        let root = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/codoxear/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(root.status(), StatusCode::OK);
        assert!(root
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .contains("text/html"));
        let root_body = String::from_utf8(response_bytes(root).await).unwrap();
        assert!(root_body.contains("/codoxear/assets/app.js"));
        assert!(!root_body.contains("src=\"/assets/"));

        let asset = app
            .oneshot(
                Request::builder()
                    .uri("/codoxear/assets/app.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(asset.status(), StatusCode::OK);
    });
}

#[test]
fn source_checkout_without_built_dist_fails_closed_for_ui() {
    let _lock = env_lock();
    let static_dir = TempDir::new().expect("temp static dir");
    let _static_dir = EnvGuard::set(
        "CODOXEAR_STATIC_DIR",
        static_dir.path().to_str().expect("utf8 temp path"),
    );
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let (_home, app) = test_app();
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = String::from_utf8(response_bytes(response).await).unwrap();
        assert!(body.contains("Codoxear UI bundle missing"));
        assert!(body.contains("npm run build"));
        assert!(!body.contains("./src/main.tsx"));
    });
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
