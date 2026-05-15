use crate::app_state::AppState;
use crate::models::{BootstrapResponse, MeResponse};
use crate::routes::{internal_error, json_response};
use crate::runtime::{read_cwd_groups, read_recent_cwds, tmux_available};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

pub async fn health() -> impl IntoResponse {
    json_response(
        StatusCode::OK,
        json!({"ok": true, "service": "codoxear-backend-rs"}),
    )
}

pub async fn me() -> Response {
    json_response(StatusCode::OK, json!(MeResponse { ok: true }))
}

pub async fn sessions_bootstrap(State(state): State<AppState>) -> Response {
    match bootstrap_response(&state) {
        Ok(value) => json_response(StatusCode::OK, json!(value)),
        Err(message) => internal_error(message),
    }
}

fn bootstrap_response(state: &AppState) -> Result<BootstrapResponse, String> {
    let recent_cwds = read_recent_cwds(&state.config.app_dir.join("recent_cwds.json"))?;
    let cwd_groups = read_cwd_groups(&state.config.app_dir.join("cwd_groups.json"))?;
    let new_session_defaults = crate::launch_defaults::read_new_session_defaults()?;
    Ok(BootstrapResponse {
        recent_cwds,
        cwd_groups,
        new_session_defaults,
        tmux_available: tmux_available(),
    })
}
