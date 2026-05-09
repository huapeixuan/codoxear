use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use codoxear_backend_rs::app_state::AppState;
use codoxear_backend_rs::routes::router;
use codoxear_backend_rs::runtime::{
    load_or_create_hmac_secret, sign_auth_cookie_value, unix_now_seconds, RuntimeConfig,
};
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

fn test_app() -> (TempDir, axum::Router) {
    let home = TempDir::new().expect("temp home");
    let app_dir = home.path().join(".local/share/codoxear");
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

async fn assert_json_200_has_utf8_content_type(
    app: axum::Router,
    path: &str,
    cookie: Option<String>,
) {
    let mut builder = Request::builder().uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header("Cookie", cookie);
    }
    let response = app
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8",
        "{path}"
    );
    let _ = response.into_body().collect().await.unwrap();
}

#[tokio::test]
async fn every_phase1_200_json_path_uses_utf8_content_type() {
    let (home, app) = test_app();
    let cookie = signed_cookie(&home);
    let cases = [
        ("/api/health", None),
        ("/api/v1/health", None),
        ("/api/me", Some(cookie.clone())),
        ("/api/v1/me", Some(cookie.clone())),
        ("/api/sessions/bootstrap", Some(cookie.clone())),
        ("/api/v1/sessions/bootstrap", Some(cookie)),
    ];

    for (path, cookie) in cases {
        assert_json_200_has_utf8_content_type(app.clone(), path, cookie).await;
    }
}
