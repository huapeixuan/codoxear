use crate::app_state::AppState;
use crate::routes::json_response;
use crate::state_files::{with_state_file_lock, write_value};
use crate::voice_state::{
    clean_device_class, clean_subscription, clean_subscription_record, clean_voice_settings,
    default_voice_settings, load_subscriptions_snapshot, load_voice_settings_snapshot, now_seconds,
    read_subscriptions, subscription_id,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

static AUDIO_LISTENERS: OnceLock<Mutex<HashMap<String, ()>>> = OnceLock::new();

pub(crate) async fn settings_voice_save(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(settings) = clean_voice_settings(&payload) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "tts_base_url must start with http:// or https://"}),
        );
    };
    let path = state.config.app_dir.join("voice_settings.json");
    if let Err((status, message)) = with_state_file_lock(&path, || {
        write_value(&path, &Value::Object(settings.clone()))
    }) {
        return json_response(status, json!({"error": message}));
    }
    json_response(
        StatusCode::OK,
        with_ok(load_voice_settings_snapshot(&state.config.app_dir)),
    )
}

pub(crate) async fn notification_subscription_upsert(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(subscription) = payload.get("subscription").and_then(clean_subscription) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "subscription required"}),
        );
    };
    let sid = subscription_id(&subscription);
    let now = now_seconds();
    let user_agent = clean_string(payload.get("user_agent")).unwrap_or_default();
    let device_label = clean_string(payload.get("device_label")).unwrap_or_default();
    let device_class = clean_device_class(payload.get("device_class"), &user_agent);
    let path = state.config.app_dir.join("push_subscriptions.json");
    let write_result = with_state_file_lock(&path, || {
        let mut records = read_subscription_records(&state);
        let current = records.get(&sid).cloned().unwrap_or_default();
        let created_ts = current
            .get("created_ts")
            .and_then(Value::as_f64)
            .unwrap_or(now);
        let mut record = Map::new();
        record.insert("id".to_string(), json!(sid));
        record.insert("subscription".to_string(), Value::Object(subscription));
        record.insert("notifications_enabled".to_string(), json!(true));
        record.insert("created_ts".to_string(), json!(created_ts));
        record.insert("updated_ts".to_string(), json!(now));
        record.insert(
            "last_success_ts".to_string(),
            current
                .get("last_success_ts")
                .cloned()
                .unwrap_or(Value::Null),
        );
        record.insert(
            "last_failure_ts".to_string(),
            current
                .get("last_failure_ts")
                .cloned()
                .unwrap_or(Value::Null),
        );
        record.insert(
            "last_error".to_string(),
            json!(current
                .get("last_error")
                .and_then(Value::as_str)
                .unwrap_or("")),
        );
        record.insert("user_agent".to_string(), json!(user_agent));
        record.insert("device_label".to_string(), json!(device_label));
        record.insert("device_class".to_string(), json!(device_class));
        records.insert(sid.clone(), Value::Object(record));
        write_subscription_records(&path, records)
    });
    if let Err((status, message)) = write_result {
        return json_response(status, json!({"error": message}));
    }
    json_response(
        StatusCode::OK,
        with_ok(load_subscriptions_snapshot(&state.config.app_dir)),
    )
}

pub(crate) async fn notification_subscription_toggle(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
    let Some(endpoint) = payload
        .get("endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "endpoint required"}),
        );
    };
    let Some(enabled) = payload.get("enabled").and_then(Value::as_bool) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "enabled must be a boolean"}),
        );
    };
    let path = state.config.app_dir.join("push_subscriptions.json");
    let write_result = with_state_file_lock(&path, || {
        let mut records = read_subscription_records(&state);
        let target_id = subscription_id_for_endpoint(endpoint);
        let Some(record) = records.get_mut(&target_id) else {
            return Err((StatusCode::NOT_FOUND, "unknown subscription".to_string()));
        };
        if record
            .get("subscription")
            .and_then(|value| value.get("endpoint"))
            .and_then(Value::as_str)
            != Some(endpoint)
        {
            return Err((StatusCode::NOT_FOUND, "unknown subscription".to_string()));
        }
        let Some(record) = record.as_object_mut() else {
            return Err((StatusCode::NOT_FOUND, "unknown subscription".to_string()));
        };
        record.insert("notifications_enabled".to_string(), json!(enabled));
        record.insert("updated_ts".to_string(), json!(now_seconds()));
        write_subscription_records(&path, records)
    });
    if let Err((status, message)) = write_result {
        return json_response(status, json!({"error": message}));
    }
    json_response(
        StatusCode::OK,
        with_ok(load_subscriptions_snapshot(&state.config.app_dir)),
    )
}

pub(crate) async fn audio_listener(Json(payload): Json<Value>) -> Response {
    let Some(client_id) = payload
        .get("client_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "client_id required"}),
        );
    };
    let Some(enabled) = payload.get("enabled").and_then(Value::as_bool) else {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "enabled must be a boolean"}),
        );
    };
    let listeners = AUDIO_LISTENERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut listeners = match listeners.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"error": "audio listener lock poisoned"}),
            )
        }
    };
    if enabled {
        listeners.insert(client_id.to_string(), ());
    } else {
        listeners.remove(client_id);
    }
    json_response(
        StatusCode::OK,
        json!({"ok": true, "active_listener_count": listeners.len()}),
    )
}

fn read_subscription_records(state: &AppState) -> Map<String, Value> {
    let mut records = Map::new();
    for item in read_subscriptions(&state.config.app_dir) {
        let Some(object) = item.as_object() else {
            continue;
        };
        let Some(id) = object.get("id").and_then(Value::as_str) else {
            continue;
        };
        records.insert(id.to_string(), Value::Object(object.clone()));
    }
    records
}

fn write_subscription_records(
    path: &std::path::Path,
    records: Map<String, Value>,
) -> Result<(), (StatusCode, String)> {
    let mut items: Vec<Value> = records
        .into_values()
        .filter_map(|value| clean_subscription_record(&value).map(Value::Object))
        .collect();
    items.sort_by(|left, right| {
        let l = left
            .get("updated_ts")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let r = right
            .get("updated_ts")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        r.partial_cmp(&l).unwrap_or(std::cmp::Ordering::Equal)
    });
    write_value(path, &Value::Array(items))
}

fn subscription_id_for_endpoint(endpoint: &str) -> String {
    let mut subscription = Map::new();
    subscription.insert("endpoint".to_string(), json!(endpoint));
    subscription.insert("keys".to_string(), json!({"p256dh": "x", "auth": "x"}));
    subscription_id(&subscription)
}

fn with_ok(mut value: Value) -> Value {
    if let Value::Object(ref mut object) = value {
        object.insert("ok".to_string(), json!(true));
        return value;
    }
    let mut object = default_voice_settings();
    object.insert("ok".to_string(), json!(true));
    Value::Object(object)
}

fn clean_string(value: Option<&Value>) -> Option<String> {
    let trimmed = match value? {
        Value::Null => String::new(),
        Value::String(raw) => raw.trim().to_string(),
        other => other.to_string().trim().trim_matches('"').to_string(),
    };
    (!trimmed.is_empty()).then_some(trimmed)
}
