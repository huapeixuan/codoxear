use crate::app_state::AppState;
use crate::handlers::git::{changed_files, diff, file_versions};
use crate::handlers::health_meta::{health, me, sessions_bootstrap};
use crate::handlers::metrics::metrics;
use crate::handlers::session_meta::{
    commands, details, diagnostics, harness_get, queue, repo, takeover, ui_state, workspace,
};
use crate::handlers::sessions_list::{session_resume_candidates, sessions};
use crate::handlers::voice::{
    notification_feed, notification_message, notification_subscriptions, settings_voice,
};
use crate::runtime::{cookie_name, load_or_create_hmac_secret, verify_auth_cookie};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde_json::{json, Value};

pub fn router(state: AppState) -> Router {
    let protected_v1 = Router::new()
        .route("/api/v1/me", get(me))
        .route("/api/v1/sessions/bootstrap", get(sessions_bootstrap))
        .route("/api/v1/sessions", get(sessions))
        .route(
            "/api/v1/session_resume_candidates",
            get(session_resume_candidates),
        )
        .route("/api/v1/sessions/:session_id/diagnostics", get(diagnostics))
        .route("/api/v1/sessions/:session_id/queue", get(queue))
        .route("/api/v1/sessions/:session_id/harness", get(harness_get))
        .route("/api/v1/sessions/:session_id/workspace", get(workspace))
        .route("/api/v1/sessions/:session_id/details", get(details))
        .route("/api/v1/sessions/:session_id/ui_state", get(ui_state))
        .route("/api/v1/sessions/:session_id/commands", get(commands))
        .route("/api/v1/sessions/:session_id/takeover", get(takeover))
        .route("/api/v1/sessions/:session_id/repo", get(repo))
        .route(
            "/api/v1/sessions/:session_id/git/changed_files",
            get(changed_files),
        )
        .route("/api/v1/sessions/:session_id/git/diff", get(diff))
        .route(
            "/api/v1/sessions/:session_id/git/file_versions",
            get(file_versions),
        )
        .route("/api/v1/settings/voice", get(settings_voice))
        .route(
            "/api/v1/notifications/subscription",
            get(notification_subscriptions),
        )
        .route("/api/v1/notifications/message", get(notification_message))
        .route("/api/v1/notifications/feed", get(notification_feed))
        .route("/api/v1/metrics", get(metrics))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_public_api_auth,
        ));

    Router::new()
        .route("/api/v1/health", get(health))
        .merge(protected_v1)
        .nest("/api", public_api_router(state.clone()))
        .fallback(not_found)
        .with_state(state)
}

fn public_api_router(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/me", get(me))
        .route("/sessions/bootstrap", get(sessions_bootstrap))
        .route("/sessions", get(sessions))
        .route("/session_resume_candidates", get(session_resume_candidates))
        .route("/sessions/:session_id/diagnostics", get(diagnostics))
        .route("/sessions/:session_id/queue", get(queue))
        .route("/sessions/:session_id/harness", get(harness_get))
        .route("/sessions/:session_id/workspace", get(workspace))
        .route("/sessions/:session_id/details", get(details))
        .route("/sessions/:session_id/ui_state", get(ui_state))
        .route("/sessions/:session_id/commands", get(commands))
        .route("/sessions/:session_id/takeover", get(takeover))
        .route("/sessions/:session_id/repo", get(repo))
        .route(
            "/sessions/:session_id/git/changed_files",
            get(changed_files),
        )
        .route("/sessions/:session_id/git/diff", get(diff))
        .route(
            "/sessions/:session_id/git/file_versions",
            get(file_versions),
        )
        .route("/settings/voice", get(settings_voice))
        .route(
            "/notifications/subscription",
            get(notification_subscriptions),
        )
        .route("/notifications/message", get(notification_message))
        .route("/notifications/feed", get(notification_feed))
        .route("/metrics", get(metrics))
        .route_layer(middleware::from_fn_with_state(
            state,
            require_public_api_auth,
        ));

    Router::new().route("/health", get(health)).merge(protected)
}

async fn not_found() -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NOT_FOUND;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html;charset=utf-8"),
    );
    response
}

pub async fn require_public_api_auth(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let authenticated = request
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| cookie_value(raw, cookie_name()))
        .map(
            |token| match load_or_create_hmac_secret(&state.config.app_dir) {
                Ok(secret) => Ok(verify_auth_cookie(&token, &secret)),
                Err(message) => Err(message),
            },
        );

    match authenticated {
        Some(Ok(true)) => next.run(request).await,
        Some(Err(message)) => {
            json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": message}))
        }
        _ => json_response(StatusCode::UNAUTHORIZED, json!({"error": "unauthorized"})),
    }
}

pub fn internal_error(message: String) -> Response {
    json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": message}))
}

fn cookie_value(raw: &str, name: &str) -> Option<String> {
    raw.split(';').find_map(|part| {
        let trimmed = part.trim();
        let (key, value) = trimmed.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_string())
    })
}

pub fn json_response(status: StatusCode, value: Value) -> Response {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response
}
