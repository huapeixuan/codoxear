use codoxear_backend_rs::app_state::build_state;
use codoxear_backend_rs::routes::router;
use codoxear_backend_rs::workers::spawn_enabled_workers;
use std::env;
use std::net::{IpAddr, SocketAddr};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let state = build_state();
    spawn_enabled_workers(state.clone());
    let app = router(state).layer(TraceLayer::new_for_http()).layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );

    let host = env::var("CODEX_WEB_HOST")
        .or_else(|_| env::var("CODOXEAR_BIND_HOST"))
        .ok()
        .and_then(|value| value.parse::<IpAddr>().ok())
        .unwrap_or_else(|| IpAddr::from([0, 0, 0, 0, 0, 0, 0, 0]));
    let port = env::var("CODEX_WEB_PORT")
        .or_else(|_| env::var("CODOXEAR_BIND_PORT"))
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8743);
    let addr = SocketAddr::new(host, port);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    tracing::info!("codoxear-backend-rs (phase 1) listening on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
