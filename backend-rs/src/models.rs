use serde::{Deserialize, Serialize};
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SessionRow {
    pub session_id: String,
    pub thread_id: Option<String>,
    pub title: Option<String>,
    pub alias: String,
    pub first_user_message: Option<String>,
    pub agent_backend: String,
    pub backend: String,
    pub owner: Option<String>,
    pub owned: bool,
    pub transport: Option<String>,
    pub supports_live_ui: bool,
    pub ui_protocol_version: Option<i64>,
    pub cwd: String,
    pub workspace_cwd: Option<String>,
    pub log_path: Option<String>,
    pub session_path: Option<String>,
    pub start_ts: f64,
    pub updated_ts: f64,
    pub broker_pid: i64,
    pub codex_pid: i64,
    pub busy: bool,
    pub broker_busy: bool,
    pub queue_len: usize,
    pub queue_items: Vec<Value>,
    pub token: Value,
    pub harness_enabled: bool,
    pub harness_cooldown_minutes: f64,
    pub harness_remaining_injections: i64,
    pub harness_request: String,
    pub files: Vec<String>,
    pub priority_offset: f64,
    pub snooze_until: Option<f64>,
    pub dependency_session_id: Option<String>,
    pub final_priority: f64,
    pub base_priority: f64,
    pub time_priority: f64,
    pub blocked: bool,
    pub snoozed: bool,
    pub git_branch: Option<String>,
    pub pr_summary: Value,
    pub todo_snapshot: Value,
    pub model_provider: Option<String>,
    pub preferred_auth_method: Option<String>,
    pub provider_choice: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub service_tier: Option<String>,
    pub tmux_session: Option<String>,
    pub tmux_window: Option<String>,
}
