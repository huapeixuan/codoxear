use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;
use std::collections::HashMap;

use crate::app_state::AppState;
use crate::routes::json_response;
use crate::voice_state::{
    load_subscriptions_snapshot, load_voice_settings_snapshot, notification_feed_since,
    notification_state_for_message,
};

pub async fn settings_voice(State(state): State<AppState>) -> Response {
    let mut value = load_voice_settings_snapshot(&state.config.app_dir);
    if let Some(object) = value.as_object_mut() {
        object.insert("ok".to_string(), json!(true));
    }
    json_response(StatusCode::OK, value)
}

pub async fn notification_subscriptions(State(state): State<AppState>) -> Response {
    let mut value = load_subscriptions_snapshot(&state.config.app_dir);
    if let Some(object) = value.as_object_mut() {
        object.insert("ok".to_string(), json!(true));
    }
    json_response(StatusCode::OK, value)
}

pub async fn notification_message(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let message_id = query
        .get("message_id")
        .map(|value| value.trim())
        .unwrap_or("");
    if message_id.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "message_id required"}),
        );
    }
    let Some(mut value) = notification_state_for_message(&state.config.app_dir, message_id) else {
        return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown message"}));
    };
    if let Some(object) = value.as_object_mut() {
        object.insert("ok".to_string(), json!(true));
    }
    json_response(StatusCode::OK, value)
}

pub async fn notification_feed(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let since_raw = query.get("since").map(|value| value.trim()).unwrap_or("0");
    let since = match if since_raw.is_empty() { "0" } else { since_raw }.parse::<f64>() {
        Ok(value) => value,
        Err(_) => return json_response(StatusCode::BAD_REQUEST, json!({"error": "invalid since"})),
    };
    let items = notification_feed_since(&state.config.app_dir, since);
    json_response(StatusCode::OK, json!({"ok": true, "items": items}))
}
