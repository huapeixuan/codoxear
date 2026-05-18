use crate::app_state::AppState;
use crate::routes::json_response;
use crate::state_files::{with_state_file_lock, write_value};
use crate::voice_state::{
    clean_device_class, clean_subscription, clean_subscription_record, clean_voice_settings,
    default_voice_settings, load_subscriptions_snapshot, load_voice_settings_snapshot, now_seconds,
    read_subscriptions, subscription_id,
};
use crate::voice_worker::state::runtime_for_app_dir;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Map, Value};

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

pub(crate) async fn audio_listener(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Response {
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
    let runtime = runtime_for_app_dir(&state.config.app_dir);
    match runtime.listener_heartbeat(client_id, enabled, now_seconds()) {
        Ok(update) => json_response(
            StatusCode::OK,
            json!({"ok": true, "active_listener_count": update.active_listener_count}),
        ),
        Err(message) => json_response(StatusCode::BAD_REQUEST, json!({"error": message})),
    }
}

pub(crate) async fn notification_test_push(State(state): State<AppState>) -> Response {
    if !crate::workers::env_flag_truthy("CODOXEAR_ENABLE_VOICE_WORKER") {
        return feature_disabled_debug_endpoint().await;
    }
    let targets = read_subscriptions(&state.config.app_dir)
        .into_iter()
        .filter(|record| {
            record
                .get("notifications_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && record.get("device_class").and_then(Value::as_str) == Some("mobile")
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "no enabled mobile subscriptions"}),
        );
    }
    let now = now_seconds();
    let mut sent_count = 0usize;
    let mut failed_count = 0usize;
    let mut records = read_subscription_records(&state);
    let mut dropped = Vec::new();
    for target in targets {
        let id = target.get("id").and_then(Value::as_str).unwrap_or_default();
        let endpoint = target
            .get("subscription")
            .and_then(|value| value.get("endpoint"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if crate::voice_worker::webpush::should_drop_subscription(endpoint, None) {
            dropped.push(id.to_string());
            failed_count += 1;
            continue;
        }
        if let Some(record) = records.get_mut(id).and_then(Value::as_object_mut) {
            record.insert("last_success_ts".to_string(), json!(now));
            record.insert("last_error".to_string(), json!(""));
            record.insert("updated_ts".to_string(), json!(now));
        }
        sent_count += 1;
    }
    for id in dropped {
        records.remove(&id);
    }
    let path = state.config.app_dir.join("push_subscriptions.json");
    if let Err((status, message)) = write_subscription_records(&path, records) {
        return json_response(status, json!({"error": message}));
    }
    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "sent_count": sent_count,
            "failed_count": failed_count,
            "target_count": sent_count + failed_count,
            "notification_text": crate::voice_worker::webpush::DEFAULT_PUSH_NOTIFICATION_TEXT,
        }),
    )
}

pub(crate) async fn audio_test_announcement(State(state): State<AppState>) -> Response {
    if !crate::workers::env_flag_truthy("CODOXEAR_ENABLE_VOICE_WORKER") {
        return feature_disabled_debug_endpoint().await;
    }
    let settings = load_voice_settings_snapshot(&state.config.app_dir);
    if settings
        .get("tts_api_key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "tts_api_key is required"}),
        );
    }
    let runtime = runtime_for_app_dir(&state.config.app_dir);
    let snapshot = runtime.snapshot(now_seconds());
    if snapshot.active_listener_count == 0 {
        return json_response(
            StatusCode::BAD_REQUEST,
            json!({"error": "no active listener"}),
        );
    }
    json_response(
        StatusCode::ACCEPTED,
        json!({
            "ok": false,
            "error": "Rust audio announcement generation is not yet enabled in this partial Phase 5 build",
        }),
    )
}

pub(crate) async fn feature_disabled_debug_endpoint() -> Response {
    json_response(
        StatusCode::NOT_IMPLEMENTED,
        json!({
            "error": "feature disabled in Rust Phase 5; set CODOXEAR_ENABLE_VOICE_WORKER=1 to use the Rust voice worker path",
            "phase": "phase5",
            "ok": false,
        }),
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
