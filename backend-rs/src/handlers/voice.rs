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

pub async fn settings_voice(State(state): State<AppState>) -> Response {
    json_response(
        StatusCode::OK,
        with_ok(load_voice_settings_snapshot(&state.config.app_dir)),
    )
}

pub async fn notification_subscriptions(State(state): State<AppState>) -> Response {
    json_response(
        StatusCode::OK,
        with_ok(load_subscriptions_snapshot(&state.config.app_dir)),
    )
}

#[derive(Deserialize)]
pub struct NotificationMessageQuery {
    message_id: Option<String>,
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
    let Some(snapshot) = notification_state_for_message(&state.config.app_dir, &message_id) else {
        return json_response(StatusCode::NOT_FOUND, json!({"error": "unknown message"}));
    };
    json_response(StatusCode::OK, with_ok(snapshot))
}

#[derive(Deserialize)]
pub struct NotificationFeedQuery {
    since: Option<String>,
}

pub async fn notification_feed(
    State(state): State<AppState>,
    Query(query): Query<NotificationFeedQuery>,
) -> Response {
    let since = match query.since.as_deref().unwrap_or("0").parse::<f64>() {
        Ok(value) => value,
        Err(_) => return json_response(StatusCode::BAD_REQUEST, json!({"error": "invalid since"})),
    };
    json_response(
        StatusCode::OK,
        json!({"ok": true, "items": notification_feed_since(&state.config.app_dir, since)}),
    )
}

fn with_ok(value: Value) -> Value {
    let mut out = Map::new();
    out.insert("ok".to_string(), Value::Bool(true));
    if let Value::Object(object) = value {
        for (key, value) in object {
            out.insert(key, value);
        }
    }
    Value::Object(out)
}
