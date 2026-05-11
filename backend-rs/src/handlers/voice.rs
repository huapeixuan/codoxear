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
    let mut payload = ok_prefix();
    extend_object(
        &mut payload,
        load_voice_settings_snapshot(&state.config.app_dir),
    );
    json_response(StatusCode::OK, Value::Object(payload))
}

pub async fn notification_subscriptions(State(state): State<AppState>) -> Response {
    let mut payload = ok_prefix();
    extend_object(
        &mut payload,
        load_subscriptions_snapshot(&state.config.app_dir),
    );
    json_response(StatusCode::OK, Value::Object(payload))
}

pub async fn notification_message(
    State(state): State<AppState>,
    Query(query): Query<NotificationMessageQuery>,
) -> Response {
    let message_id = query
        .message_id
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if message_id.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "message_id required"}),
        );
    }

    let Some(snapshot) = notification_state_for_message(&state.config.app_dir, message_id) else {
        return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown message"}));
    };
    let mut payload = ok_prefix();
    extend_object(&mut payload, snapshot);
    json_response(StatusCode::OK, Value::Object(payload))
}

pub async fn notification_feed(
    State(state): State<AppState>,
    Query(query): Query<NotificationFeedQuery>,
) -> Response {
    let raw = query.since.as_deref().unwrap_or("0");
    let Ok(since) = raw.parse::<f64>() else {
        return json_response(StatusCode::BAD_REQUEST, json!({"error": "invalid since"}));
    };
    json_response(
        StatusCode::OK,
        json!({"ok": true, "items": notification_feed_since(&state.config.app_dir, since)}),
    )
}

fn ok_prefix() -> Map<String, Value> {
    let mut payload = Map::new();
    payload.insert("ok".to_string(), Value::Bool(true));
    payload
}

fn extend_object(target: &mut Map<String, Value>, value: Value) {
    if let Value::Object(object) = value {
        target.extend(object);
    }
}
