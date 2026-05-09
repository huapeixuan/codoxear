use crate::routes::json_response;
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::json;

pub async fn metrics() -> Response {
    json_response(StatusCode::OK, json!({"metrics": {}}))
}
