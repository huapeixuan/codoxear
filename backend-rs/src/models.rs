use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Serialize)]
pub struct BootstrapResponse {
    pub recent_cwds: Vec<String>,
    pub cwd_groups: Map<String, Value>,
    pub new_session_defaults: Value,
    pub tmux_available: bool,
}

#[derive(Serialize)]
pub struct MeResponse {
    pub ok: bool,
}
