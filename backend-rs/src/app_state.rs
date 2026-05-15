use crate::runtime::{load_or_create_hmac_secret, RuntimeConfig};

#[derive(Clone)]
pub struct AppState {
    pub config: RuntimeConfig,
    pub fake_spawn_for_tests: bool,
    pub fake_spawn_session_id_for_tests: Option<String>,
}

pub fn build_state() -> AppState {
    let config = RuntimeConfig::from_env().expect("resolve Codoxear runtime config");
    load_or_create_hmac_secret(&config.app_dir).expect("load or create Codoxear hmac_secret");
    AppState {
        config,
        fake_spawn_for_tests: false,
        fake_spawn_session_id_for_tests: None,
    }
}
