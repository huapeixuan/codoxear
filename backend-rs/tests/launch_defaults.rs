use codoxear_backend_rs::runtime::read_new_session_defaults;
use serde_json::json;
use std::env;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn new(home: &TempDir) -> Self {
        let keys = [
            "HOME",
            "CODEX_HOME",
            "PI_HOME",
            "CODEX_WEB_DEFAULT_AGENT_BACKEND",
            "CODEX_WEB_RECENT_CWD_MAX",
        ];
        let saved = keys
            .into_iter()
            .map(|key| (key, env::var(key).ok()))
            .collect();
        env::set_var("HOME", home.path());
        env::remove_var("CODEX_HOME");
        env::remove_var("PI_HOME");
        env::remove_var("CODEX_WEB_DEFAULT_AGENT_BACKEND");
        env::remove_var("CODEX_WEB_RECENT_CWD_MAX");
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(value) => env::set_var(key, value),
                None => env::remove_var(key),
            }
        }
    }
}

#[test]
fn empty_home_returns_all_defaults() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let _env = EnvGuard::new(&home);

    let defaults = read_new_session_defaults().unwrap();

    assert_eq!(defaults["default_backend"], "codex");
    assert_eq!(defaults["backends"]["codex"]["model_provider"], "openai");
    assert_eq!(
        defaults["backends"]["codex"]["preferred_auth_method"],
        "apikey"
    );
    assert_eq!(
        defaults["backends"]["codex"]["provider_choice"],
        "openai-api"
    );
    assert_eq!(
        defaults["backends"]["codex"]["model"],
        serde_json::Value::Null
    );
    assert_eq!(
        defaults["backends"]["codex"]["reasoning_effort"],
        serde_json::Value::Null
    );
    assert_eq!(defaults["backends"]["codex"]["supports_fast"], true);
    assert_eq!(defaults["backends"]["pi"]["reasoning_effort"], "high");
    assert_eq!(defaults["backends"]["pi"]["provider_choices"], json!([]));
}

#[test]
fn codex_config_model_and_reasoning_effort_are_used() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let _env = EnvGuard::new(&home);
    let codex_home = home.path().join(".codex");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        r#"model = "gpt-foo"
model_reasoning_effort = "medium"
"#,
    )
    .unwrap();

    let defaults = read_new_session_defaults().unwrap();

    assert_eq!(defaults["backends"]["codex"]["model"], "gpt-foo");
    assert_eq!(defaults["backends"]["codex"]["reasoning_effort"], "medium");
}

#[test]
fn pi_settings_and_models_select_matching_provider_and_model() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let _env = EnvGuard::new(&home);
    let pi_agent = home.path().join(".pi/agent");
    fs::create_dir_all(&pi_agent).unwrap();
    fs::write(
        pi_agent.join("settings.json"),
        r#"{"defaultProvider":"bar","defaultModel":"pi-baz"}"#,
    )
    .unwrap();
    fs::write(
        pi_agent.join("models.json"),
        r#"{"providers":{"bar":{"models":[{"id":"pi-baz"},{"id":"pi-qux"}]}}}"#,
    )
    .unwrap();

    let defaults = read_new_session_defaults().unwrap();

    assert_eq!(defaults["backends"]["pi"]["model_provider"], "bar");
    assert_eq!(defaults["backends"]["pi"]["provider_choice"], "bar");
    assert_eq!(defaults["backends"]["pi"]["model"], "pi-baz");
    assert_eq!(
        defaults["backends"]["pi"]["provider_models"]["bar"],
        json!(["pi-baz", "pi-qux"])
    );
}

#[test]
fn models_cache_falls_back_to_lowest_priority_when_model_is_missing() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let _env = EnvGuard::new(&home);
    let codex_home = home.path().join(".codex");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(
        codex_home.join("config.toml"),
        r#"model = "missing-model"
"#,
    )
    .unwrap();
    fs::write(
        codex_home.join("models_cache.json"),
        r#"{"models":[{"slug":"z","priority":20,"default_reasoning_level":"low"},{"slug":"a","priority":10,"default_reasoning_level":"xhigh"}]}"#,
    )
    .unwrap();

    let defaults = read_new_session_defaults().unwrap();

    assert_eq!(defaults["backends"]["codex"]["reasoning_effort"], "xhigh");
}

#[test]
fn malformed_codex_config_returns_invalid_config_error() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let _env = EnvGuard::new(&home);
    let codex_home = home.path().join(".codex");
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(codex_home.join("config.toml"), "model = [").unwrap();

    let err = read_new_session_defaults().unwrap_err();

    assert!(err.contains("invalid Codex config in"), "{err}");
}
