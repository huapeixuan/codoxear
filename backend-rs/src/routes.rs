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
use crate::runtime::{cookie_name, load_or_create_hmac_secret, verify_auth_cookie};
use crate::session_create::session_create;
use crate::takeover_post::takeover_open;
use crate::voice_post::{
    audio_listener, audio_test_announcement, notification_subscription_toggle,
    notification_subscription_upsert, notification_test_push, settings_voice_save,
};
use crate::voice_worker::hls::{audio_playlist, audio_segment};
use axum::body::Body;
use axum::extract::OriginalUri;
use axum::extract::{Path as AxumPath, Request, State};
use axum::http::header;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{any, get, post};
use axum::Router;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

pub fn router(state: AppState) -> Router {
    let prefix = crate::runtime::url_prefix().expect("invalid CODEX_WEB_URL_PREFIX");
    router_with_url_prefix(state, &prefix).expect("invalid CODEX_WEB_URL_PREFIX")
}

pub fn router_with_url_prefix(state: AppState, raw_prefix: &str) -> Result<Router, String> {
    let root = root_router(state.clone());
    let prefix = crate::runtime::normalize_url_prefix(Some(raw_prefix))?;
    if prefix.is_empty() {
        return Ok(root);
    }

    let exact_prefix = prefix.clone();
    let prefixed_exact = get(move || {
        let location = format!("{exact_prefix}/");
        async move { Redirect::permanent(&location) }
    });

    let slash_prefix = prefix.clone();
    let slash_root = root.clone();
    let prefixed_slash =
        any(move |request| prefix_dispatch(request, slash_root, slash_prefix, None));

    let wildcard_prefix = prefix.clone();
    let wildcard_root = root.clone();
    let prefixed_wildcard = any(move |AxumPath(rest): AxumPath<String>, request| {
        prefix_dispatch(request, wildcard_root, wildcard_prefix, Some(rest))
    });

    Ok(Router::new()
        .route(&prefix, prefixed_exact)
        .route(&format!("{prefix}/"), prefixed_slash)
        .route(&format!("{prefix}/*rest"), prefixed_wildcard)
        .merge(root))
}

async fn prefix_dispatch(
    mut request: Request<Body>,
    root: Router,
    prefix: String,
    wildcard_rest: Option<String>,
) -> Response {
    let original = request
        .extensions()
        .get::<OriginalUri>()
        .map(|uri| uri.0.clone())
        .unwrap_or_else(|| request.uri().clone());
    let Some(path_and_query) = original.path_and_query() else {
        return not_found().await;
    };
    let raw = path_and_query.as_str();
    if let Some(rest) = wildcard_rest.as_deref() {
        let query = original
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default();
        let rewritten = if rest.is_empty() {
            format!("/{query}")
        } else {
            format!("/{rest}{query}")
        };
        let Ok(uri) = rewritten.parse() else {
            return not_found().await;
        };
        *request.uri_mut() = uri;
        return root.oneshot(request).await.unwrap_or_else(|_| {
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": "prefixed route dispatch failed"}),
            )
        });
    }
    let Some(rest) = raw.strip_prefix(&prefix) else {
        return not_found().await;
    };
    let rewritten = if rest.is_empty() || rest == "/" || rest == "/*rest" {
        "/".to_string()
    } else if rest.starts_with('/') || rest.starts_with('?') {
        rest.to_string()
    } else {
        return not_found().await;
    };
    let Ok(uri) = rewritten.parse() else {
        return not_found().await;
    };
    *request.uri_mut() = uri;
    root.oneshot(request).await.unwrap_or_else(|_| {
        json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({"error": "prefixed route dispatch failed"}),
        )
    })
}

fn root_router(state: AppState) -> Router {
    let static_dir = repo_static_dir();
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
        .route("/", get(static_index))
        .route("/:dist_asset", get(static_dist_asset_redirect))
        .nest_service("/static", ServeDir::new(static_dir.clone()))
        .nest_service("/assets", ServeDir::new(static_dir.join("dist/assets")))
        .route_service(
            "/manifest.webmanifest",
            ServeFile::new(static_dir.join("manifest.webmanifest")),
        )
        .route_service(
            "/service-worker.js",
            ServeFile::new(static_dir.join("service-worker.js")),
        )
        .route_service(
            "/favicon.ico",
            ServeFile::new(static_dir.join("favicon.png")),
        )
        .route_service(
            "/favicon.png",
            ServeFile::new(static_dir.join("favicon.png")),
        )
        .merge(protected_v1)
        .nest("/api", public_api_router(state.clone()))
        .fallback(not_found)
        .with_state(state)
}

async fn static_index() -> Response {
    let static_dir = repo_static_dir();
    let dist_index = static_dir.join("dist/index.html");
    match fs::read(&dist_index) {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html;charset=utf-8"),
            );
            response
        }
        Err(_) => not_found().await,
    }
}

async fn static_dist_asset_redirect(AxumPath(dist_asset): AxumPath<String>) -> Response {
    if !is_static_dist_asset_name(&dist_asset) {
        return not_found().await;
    }
    Redirect::temporary(&format!("assets/{dist_asset}")).into_response()
}

fn is_static_dist_asset_name(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && (value.ends_with(".js") || value.ends_with(".css") || value.ends_with(".map"))
}

pub fn assert_static_dist_ready() -> Result<(), String> {
    let dist_index = repo_static_dir().join("dist/index.html");
    let bytes = fs::read(&dist_index).map_err(|err| {
        format!(
            "missing production web bundle at {}: {err}; run `cd web && npm install && npm run build` before starting codoxear-backend-rs from a source checkout",
            dist_index.display()
        )
    })?;
    validate_dist_index_bytes(&dist_index, &bytes)
}

fn validate_dist_index_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let text = String::from_utf8_lossy(bytes);
    if text.contains("./src/main.tsx") || text.contains("/src/main.tsx") {
        return Err(format!(
            "{} is a Vite source index, not a production bundle; run `cd web && npm run build`",
            path.display()
        ));
    }
    Ok(())
}

fn repo_static_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("codoxear/static")
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

#[cfg(test)]
mod tests {
    use super::validate_dist_index_bytes;
    use std::path::Path;

    #[test]
    fn dist_index_validator_rejects_vite_source_entrypoint() {
        let err = validate_dist_index_bytes(
            Path::new("index.html"),
            br#"<script type="module" src="./src/main.tsx"></script>"#,
        )
        .unwrap_err();
        assert!(err.contains("production bundle"));
    }

    #[test]
    fn dist_index_validator_accepts_built_asset_entrypoint() {
        validate_dist_index_bytes(
            Path::new("index.html"),
            br#"<script type="module" src="./assets/index-abcd.js"></script>"#,
        )
        .unwrap();
    }
}
