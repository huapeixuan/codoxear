use crate::app_state::AppState;
use crate::file_post::{files_inspect_post, files_read_post, session_file_write};
use crate::handlers::files::{
    file_blob, file_download, file_list, file_read, file_search, files_blob,
};
use crate::handlers::git::{changed_files, diff, file_versions};
use crate::handlers::health_meta::{health, me, sessions_bootstrap};
use crate::handlers::messages::{live, messages, tail};
use crate::handlers::metrics::metrics;
use crate::handlers::session_meta::{
    commands, details, diagnostics, harness_get, queue, repo, takeover, ui_state, workspace,
};
use crate::handlers::sessions_list::{session_resume_candidates, sessions};
use crate::handlers::voice::{
    notification_feed, notification_message, notification_subscriptions, settings_voice,
};
use crate::inject_post::session_inject_file;
use crate::lifecycle_post::{session_delete, session_heartbeat};
use crate::post_handlers::{
    cwd_group_edit, hooks_notify, login, logout, queue_delete, queue_update, session_edit,
    session_enqueue, session_harness, session_interrupt, session_rename, session_send,
    session_ui_response,
};
use crate::runtime::{cookie_name, load_or_create_hmac_secret, url_prefix, verify_auth_cookie};
use crate::session_create::session_create;
use crate::takeover_post::takeover_open;
use crate::voice_post::{
    audio_listener, audio_test_announcement, notification_subscription_toggle,
    notification_subscription_upsert, notification_test_push, settings_voice_save,
};
use crate::voice_worker::hls::{audio_playlist, audio_segment};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Redirect, Response};
use axum::routing::{any, get, post};
use axum::Router;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use tower_http::services::ServeDir;

pub fn router(state: AppState) -> Router {
    let static_dir = repo_static_dir();
    let app = unprefixed_router(state.clone(), static_dir.clone());
    match url_prefix().expect("resolve CODEX_WEB_URL_PREFIX") {
        prefix if prefix.is_empty() => app,
        prefix => Router::new()
            .route(&prefix, get(Redirect::permanent(&format!("{prefix}/"))))
            .route(
                &format!("{prefix}/"),
                get({
                    let app = app.clone();
                    move |request| prefixed_request(request, app.clone())
                }),
            )
            .route(
                &format!("{prefix}/*path"),
                any({
                    let app = app.clone();
                    move |request| prefixed_request(request, app.clone())
                }),
            )
            .merge(app),
    }
}

fn unprefixed_router(state: AppState, static_dir: PathBuf) -> Router {
    let protected_v1 = Router::new()
        .route("/api/v1/me", get(me))
        .route("/api/v1/sessions/bootstrap", get(sessions_bootstrap))
        .route("/api/v1/sessions", get(sessions).post(session_create))
        .route(
            "/api/v1/session_resume_candidates",
            get(session_resume_candidates),
        )
        .route("/api/v1/sessions/:session_id/diagnostics", get(diagnostics))
        .route("/api/v1/sessions/:session_id/queue", get(queue))
        .route(
            "/api/v1/sessions/:session_id/enqueue",
            post(session_enqueue),
        )
        .route(
            "/api/v1/sessions/:session_id/queue/delete",
            post(queue_delete),
        )
        .route(
            "/api/v1/sessions/:session_id/queue/update",
            post(queue_update),
        )
        .route(
            "/api/v1/sessions/:session_id/harness",
            get(harness_get).post(session_harness),
        )
        .route("/api/v1/sessions/:session_id/rename", post(session_rename))
        .route("/api/v1/sessions/:session_id/edit", post(session_edit))
        .route("/api/v1/sessions/:session_id/delete", post(session_delete))
        .route("/api/v1/sessions/:session_id/send", post(session_send))
        .route(
            "/api/v1/sessions/:session_id/ui_response",
            post(session_ui_response),
        )
        .route(
            "/api/v1/sessions/:session_id/heartbeat",
            post(session_heartbeat),
        )
        .route(
            "/api/v1/sessions/:session_id/interrupt",
            post(session_interrupt),
        )
        .route(
            "/api/v1/sessions/:session_id/inject_file",
            post(session_inject_file),
        )
        .route(
            "/api/v1/sessions/:session_id/inject_image",
            post(session_inject_file),
        )
        .route("/api/v1/sessions/:session_id/workspace", get(workspace))
        .route("/api/v1/sessions/:session_id/details", get(details))
        .route("/api/v1/sessions/:session_id/ui_state", get(ui_state))
        .route("/api/v1/sessions/:session_id/commands", get(commands))
        .route("/api/v1/sessions/:session_id/takeover", get(takeover))
        .route(
            "/api/v1/sessions/:session_id/takeover/open",
            post(takeover_open),
        )
        .route("/api/v1/sessions/:session_id/repo", get(repo))
        .route("/api/v1/sessions/:session_id/messages", get(messages))
        .route("/api/v1/sessions/:session_id/tail", get(tail))
        .route("/api/v1/sessions/:session_id/live", get(live))
        .route("/api/v1/sessions/:session_id/file/read", get(file_read))
        .route(
            "/api/v1/sessions/:session_id/file/write",
            post(session_file_write),
        )
        .route("/api/v1/sessions/:session_id/file/search", get(file_search))
        .route("/api/v1/sessions/:session_id/file/list", get(file_list))
        .route("/api/v1/sessions/:session_id/file/blob", get(file_blob))
        .route(
            "/api/v1/sessions/:session_id/file/download",
            get(file_download),
        )
        .route("/api/v1/files/blob", get(files_blob).post(files_blob))
        .route("/api/v1/files/read", post(files_read_post))
        .route("/api/v1/files/inspect", post(files_inspect_post))
        .route(
            "/api/v1/sessions/:session_id/git/changed_files",
            get(changed_files),
        )
        .route("/api/v1/sessions/:session_id/git/diff", get(diff))
        .route(
            "/api/v1/sessions/:session_id/git/file_versions",
            get(file_versions),
        )
        .route(
            "/api/v1/settings/voice",
            get(settings_voice).post(settings_voice_save),
        )
        .route(
            "/api/v1/notifications/subscription",
            get(notification_subscriptions).post(notification_subscription_upsert),
        )
        .route(
            "/api/v1/notifications/subscription/toggle",
            post(notification_subscription_toggle),
        )
        .route(
            "/api/v1/notifications/test_push",
            post(notification_test_push),
        )
        .route("/api/v1/notifications/message", get(notification_message))
        .route("/api/v1/notifications/feed", get(notification_feed))
        .route("/api/v1/audio/live.m3u8", get(audio_playlist))
        .route("/api/v1/audio/segments/*segment", get(audio_segment))
        .route("/api/v1/audio/listener", post(audio_listener))
        .route(
            "/api/v1/audio/test_announcement",
            post(audio_test_announcement),
        )
        .route("/api/v1/metrics", get(metrics))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/cwd_groups/edit", post(cwd_group_edit))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_public_api_auth,
        ));

    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/login", post(login))
        .route("/api/v1/hooks/notify", post(hooks_notify))
        .route(
            "/",
            get({
                let static_dir = static_dir.clone();
                move || static_index(static_dir.clone())
            }),
        )
        .nest_service("/static", ServeDir::new(static_dir.clone()))
        .nest_service("/assets", ServeDir::new(static_dir.join("dist/assets")))
        .route(
            "/manifest.webmanifest",
            get({
                let static_dir = static_dir.clone();
                move || {
                    static_file(
                        static_dir.clone(),
                        "manifest.webmanifest",
                        "application/manifest+json",
                    )
                }
            }),
        )
        .route(
            "/service-worker.js",
            get({
                let static_dir = static_dir.clone();
                move || {
                    static_file(
                        static_dir.clone(),
                        "service-worker.js",
                        "text/javascript;charset=utf-8",
                    )
                }
            }),
        )
        .route(
            "/favicon.ico",
            get({
                let static_dir = static_dir.clone();
                move || static_file(static_dir.clone(), "favicon.png", "image/png")
            }),
        )
        .route(
            "/favicon.png",
            get({
                let static_dir = static_dir.clone();
                move || static_file(static_dir.clone(), "favicon.png", "image/png")
            }),
        )
        .route(
            "/codoxear-icon.png",
            get({
                let static_dir = static_dir.clone();
                move || static_file(static_dir.clone(), "codoxear-icon.png", "image/png")
            }),
        )
        .route(
            "/codoxear-icon-192.png",
            get({
                let static_dir = static_dir.clone();
                move || static_file(static_dir.clone(), "codoxear-icon-192.png", "image/png")
            }),
        )
        .merge(protected_v1)
        .nest("/api", public_api_router(state.clone()))
        .fallback(not_found)
        .with_state(state)
}

async fn static_index(static_dir: PathBuf) -> Response {
    let dist_index = static_dir.join("dist/index.html");
    match fs::read(&dist_index) {
        Ok(bytes) => bytes_response(
            StatusCode::OK,
            prefix_index_asset_urls(bytes),
            "text/html;charset=utf-8",
        ),
        Err(_) => {
            let body = format!(
                "<!doctype html><title>Codoxear UI bundle missing</title><h1>Codoxear UI bundle missing</h1><p>Missing <code>{}</code>.</p><p>Run <code>cd web && npm ci && npm run build</code> before starting codoxear-backend-rs from a source checkout, or deploy a package that includes <code>codoxear/static/dist</code>.</p>",
                dist_index.display()
            );
            bytes_response(
                StatusCode::SERVICE_UNAVAILABLE,
                body.into_bytes(),
                "text/html;charset=utf-8",
            )
        }
    }
}

fn prefix_index_asset_urls(bytes: Vec<u8>) -> Vec<u8> {
    let Ok(prefix) = url_prefix() else {
        return bytes;
    };
    if prefix.is_empty() {
        return bytes;
    }
    let Ok(html) = String::from_utf8(bytes) else {
        return Vec::new();
    };
    html.replace("href=\"/", &format!("href=\"{prefix}/"))
        .replace("src=\"/", &format!("src=\"{prefix}/"))
        .replace("href=\"./", &format!("href=\"{prefix}/"))
        .replace("src=\"./", &format!("src=\"{prefix}/"))
        .into_bytes()
}

async fn prefixed_request(mut request: Request<Body>, app: Router) -> Response {
    let Some(prefix) = url_prefix().ok().filter(|prefix| !prefix.is_empty()) else {
        return not_found().await;
    };
    let uri = request.uri().clone();
    let Some(path_and_query) = uri.path_and_query() else {
        return not_found().await;
    };
    let Some(rest) = path_and_query.path().strip_prefix(&prefix) else {
        return not_found().await;
    };
    let stripped_path = if rest.is_empty() { "/" } else { rest };
    let stripped_path_and_query = match path_and_query.query() {
        Some(query) => format!("{stripped_path}?{query}"),
        None => stripped_path.to_string(),
    };
    let mut parts = uri.into_parts();
    let Ok(new_path_and_query) = stripped_path_and_query.parse() else {
        return not_found().await;
    };
    parts.path_and_query = Some(new_path_and_query);
    let Ok(new_uri) = axum::http::Uri::from_parts(parts) else {
        return not_found().await;
    };
    *request.uri_mut() = new_uri;
    match app.oneshot(request).await {
        Ok(response) => response,
        Err(_) => internal_error("prefixed request dispatch failed".to_string()),
    }
}

async fn static_file(
    static_dir: PathBuf,
    name: &'static str,
    content_type: &'static str,
) -> Response {
    let dist_path = static_dir.join("dist").join(name);
    let static_path = static_dir.join(name);
    match read_existing_file(&dist_path).or_else(|| read_existing_file(&static_path)) {
        Some(bytes) => bytes_response(StatusCode::OK, bytes, content_type),
        None => not_found().await,
    }
}

fn read_existing_file(path: &Path) -> Option<Vec<u8>> {
    fs::read(path).ok()
}

fn bytes_response(status: StatusCode, bytes: Vec<u8>, content_type: &'static str) -> Response {
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn repo_static_dir() -> std::path::PathBuf {
    std::env::var("CODOXEAR_STATIC_DIR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("codoxear/static")
        })
}

fn public_api_router(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/me", get(me))
        .route("/sessions/bootstrap", get(sessions_bootstrap))
        .route("/sessions", get(sessions).post(session_create))
        .route("/session_resume_candidates", get(session_resume_candidates))
        .route("/sessions/:session_id/diagnostics", get(diagnostics))
        .route("/sessions/:session_id/queue", get(queue))
        .route("/sessions/:session_id/enqueue", post(session_enqueue))
        .route("/sessions/:session_id/queue/delete", post(queue_delete))
        .route("/sessions/:session_id/queue/update", post(queue_update))
        .route(
            "/sessions/:session_id/harness",
            get(harness_get).post(session_harness),
        )
        .route("/sessions/:session_id/rename", post(session_rename))
        .route("/sessions/:session_id/edit", post(session_edit))
        .route("/sessions/:session_id/delete", post(session_delete))
        .route("/sessions/:session_id/send", post(session_send))
        .route(
            "/sessions/:session_id/ui_response",
            post(session_ui_response),
        )
        .route("/sessions/:session_id/heartbeat", post(session_heartbeat))
        .route("/sessions/:session_id/interrupt", post(session_interrupt))
        .route(
            "/sessions/:session_id/inject_file",
            post(session_inject_file),
        )
        .route(
            "/sessions/:session_id/inject_image",
            post(session_inject_file),
        )
        .route("/sessions/:session_id/workspace", get(workspace))
        .route("/sessions/:session_id/details", get(details))
        .route("/sessions/:session_id/ui_state", get(ui_state))
        .route("/sessions/:session_id/commands", get(commands))
        .route("/sessions/:session_id/takeover", get(takeover))
        .route("/sessions/:session_id/takeover/open", post(takeover_open))
        .route("/sessions/:session_id/repo", get(repo))
        .route("/sessions/:session_id/messages", get(messages))
        .route("/sessions/:session_id/tail", get(tail))
        .route("/sessions/:session_id/live", get(live))
        .route("/sessions/:session_id/file/read", get(file_read))
        .route("/sessions/:session_id/file/write", post(session_file_write))
        .route("/sessions/:session_id/file/search", get(file_search))
        .route("/sessions/:session_id/file/list", get(file_list))
        .route("/sessions/:session_id/file/blob", get(file_blob))
        .route("/sessions/:session_id/file/download", get(file_download))
        .route("/files/blob", get(files_blob).post(files_blob))
        .route("/files/read", post(files_read_post))
        .route("/files/inspect", post(files_inspect_post))
        .route(
            "/sessions/:session_id/git/changed_files",
            get(changed_files),
        )
        .route("/sessions/:session_id/git/diff", get(diff))
        .route(
            "/sessions/:session_id/git/file_versions",
            get(file_versions),
        )
        .route(
            "/settings/voice",
            get(settings_voice).post(settings_voice_save),
        )
        .route(
            "/notifications/subscription",
            get(notification_subscriptions).post(notification_subscription_upsert),
        )
        .route(
            "/notifications/subscription/toggle",
            post(notification_subscription_toggle),
        )
        .route("/notifications/test_push", post(notification_test_push))
        .route("/notifications/message", get(notification_message))
        .route("/notifications/feed", get(notification_feed))
        .route("/audio/live.m3u8", get(audio_playlist))
        .route("/audio/segments/*segment", get(audio_segment))
        .route("/audio/listener", post(audio_listener))
        .route("/audio/test_announcement", post(audio_test_announcement))
        .route("/metrics", get(metrics))
        .route("/logout", post(logout))
        .route("/cwd_groups/edit", post(cwd_group_edit))
        .route_layer(middleware::from_fn_with_state(
            state,
            require_public_api_auth,
        ));

    Router::new()
        .route("/health", get(health))
        .route("/login", post(login))
        .route("/hooks/notify", post(hooks_notify))
        .merge(protected)
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
