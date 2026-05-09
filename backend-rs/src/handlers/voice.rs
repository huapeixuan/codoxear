use crate::app_state::AppState;
use crate::routes::json_response;
use crate::voice_state::{
    load_subscriptions_snapshot, load_voice_settings_snapshot, notification_feed_since,
    notification_state_for_message,
};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{json, Map, Value};

#[derive(Deserialize)]
pub struct NotificationMessageQuery {
    message_id: Option<String>,
}

#[derive(Deserialize)]
pub struct NotificationFeedQuery {
    since: Option<String>,
}

pub async fn settings_voice(State(state): State<AppState>) -> Response {
    let snapshot = load_voice_settings_snapshot(&state.config.app_dir);
    json_response(StatusCode::OK, with_ok(snapshot))
}

pub async fn notification_subscriptions(State(state): State<AppState>) -> Response {
    let snapshot = load_subscriptions_snapshot(&state.config.app_dir);
    json_response(StatusCode::OK, with_ok(snapshot))
}

pub async fn notification_message(
    State(state): State<AppState>,
    Query(query): Query<NotificationMessageQuery>,
) -> Response {
    let message_id = query.message_id.unwrap_or_default().trim().to_string();
    if message_id.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "message_id required"}),
        );
    }
    match notification_state_for_message(&state.config.app_dir, &message_id) {
        Some(snapshot) => json_response(StatusCode::OK, with_ok(snapshot)),
        None => json_response(StatusCode::NOT_FOUND, json!({"error": "unknown message"})),
    }
}

pub async fn notification_feed(
    State(state): State<AppState>,
    Query(query): Query<NotificationFeedQuery>,
) -> Response {
    let since_raw = query.since.unwrap_or_else(|| "0".to_string());
    let since_ts = match since_raw.trim().parse::<f64>() {
        Ok(value) => value,
        Err(_) => return json_response(StatusCode::BAD_REQUEST, json!({"error": "invalid since"})),
    };
    json_response(
        StatusCode::OK,
        json!({"ok": true, "items": notification_feed_since(&state.config.app_dir, since_ts)}),
    )
}

fn with_ok(value: Value) -> Value {
    let mut out = Map::new();
    out.insert("ok".to_string(), json!(true));
    if let Value::Object(object) = value {
        out.extend(object);
    }
    Value::Object(out)
}
