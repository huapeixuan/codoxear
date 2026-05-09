use crate::runtime::{load_or_create_hmac_secret, RuntimeConfig};

#[derive(Clone)]
pub struct AppState {
    pub config: RuntimeConfig,
}

pub fn build_state() -> AppState {
    let config = RuntimeConfig::from_env().expect("resolve Codoxear runtime config");
    load_or_create_hmac_secret(&config.app_dir).expect("load or create Codoxear hmac_secret");
    AppState { config }
}
