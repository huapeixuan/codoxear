pub use crate::launch_defaults::{
    read_codex_launch_defaults, read_new_session_defaults, read_pi_launch_defaults,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{json, Map, Value};
use sha2::Sha256;
use std::cmp::Ordering;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct RuntimeConfig {
    pub app_dir: PathBuf,
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, String> {
        Ok(Self {
            app_dir: default_app_dir()?,
        })
    }
}

pub fn default_app_dir() -> Result<PathBuf, String> {
    if let Ok(raw) = env::var("CODOXEAR_APP_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    let home = env::var("HOME")
        .map_err(|_| "HOME is not set; cannot resolve Codoxear app dir".to_string())?;
    let trimmed = home.trim();
    if trimmed.is_empty() {
        return Err("HOME is empty; cannot resolve Codoxear app dir".to_string());
    }
    Ok(PathBuf::from(trimmed)
        .join(".local")
        .join("share")
        .join("codoxear"))
}

pub fn load_or_create_hmac_secret(app_dir: &Path) -> Result<Vec<u8>, String> {
    let path = app_dir.join("hmac_secret");
    fs::create_dir_all(app_dir).map_err(|err| format!("create {}: {err}", app_dir.display()))?;
    if path.exists() {
        let bytes = fs::read(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        if bytes.len() < 32 {
            return Err(format!(
                "invalid hmac secret (too short): {}",
                path.display()
            ));
        }
        return Ok(bytes.into_iter().take(64).collect());
    }

    let mut secret = vec![0_u8; 64];
    fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut secret))
        .map_err(|err| format!("read /dev/urandom: {err}"))?;
    fs::write(&path, &secret).map_err(|err| format!("write {}: {err}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path)
            .map_err(|err| format!("stat {}: {err}", path.display()))?
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&path, permissions)
            .map_err(|err| format!("chmod {}: {err}", path.display()))?;
    }
    Ok(secret)
}

pub fn cookie_name() -> &'static str {
    "codoxear_auth"
}

pub fn cookie_ttl_seconds() -> i64 {
    env::var("CODEX_WEB_COOKIE_TTL_SECONDS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(30 * 24 * 3600)
}

pub fn cookie_secure() -> bool {
    env::var("CODEX_WEB_COOKIE_SECURE").ok().as_deref() == Some("1")
}

pub fn cookie_path() -> Result<String, String> {
    let prefix = normalize_url_prefix(env::var("CODEX_WEB_URL_PREFIX").ok().as_deref())?;
    Ok(if prefix.is_empty() {
        "/".to_string()
    } else {
        format!("{prefix}/")
    })
}

pub fn verify_auth_cookie(value: &str, secret: &[u8]) -> bool {
    let Some((payload_b64, sig_b64)) = value.split_once('.') else {
        return false;
    };
    let Ok(raw_payload) = URL_SAFE_NO_PAD.decode(payload_b64.as_bytes()) else {
        return false;
    };
    let Ok(signature) = URL_SAFE_NO_PAD.decode(sig_b64.as_bytes()) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return false;
    };
    mac.update(&raw_payload);
    if mac.verify_slice(&signature).is_err() {
        return false;
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&raw_payload) else {
        return false;
    };
    let Some(exp) = payload.get("exp").and_then(Value::as_i64) else {
        return false;
    };
    exp > unix_now_seconds()
}

pub fn sign_auth_cookie_value(secret: &[u8], exp: i64) -> Result<String, String> {
    let raw = serde_json::to_vec(&json!({ "exp": exp })).map_err(|err| err.to_string())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|err| err.to_string())?;
    mac.update(&raw);
    let sig = mac.finalize().into_bytes();
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(raw),
        URL_SAFE_NO_PAD.encode(sig)
    ))
}

pub fn unix_now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub fn recent_cwd_max() -> usize {
    env::var("CODEX_WEB_RECENT_CWD_MAX")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(256)
}

pub fn read_recent_cwds(path: &Path) -> Result<Vec<String>, String> {
    let Some(value) = read_optional_json(path)? else {
        return Ok(Vec::new());
    };
    let Value::Object(object) = value else {
        return Err(format!("invalid recent_cwds.json in {}", path.display()));
    };
    let mut cleaned: Map<String, Value> = Map::new();
    for (raw_cwd, raw_ts) in object {
        let cwd = raw_cwd.trim();
        if cwd.is_empty() || raw_ts.is_boolean() {
            continue;
        }
        let Some(ts) = raw_ts.as_f64() else {
            continue;
        };
        if !ts.is_finite() || ts <= 0.0 {
            continue;
        }
        match cleaned.get(cwd).and_then(Value::as_f64) {
            Some(prev) if prev >= ts => {}
            _ => {
                cleaned.insert(cwd.to_string(), json!(ts));
            }
        }
    }
    let mut rows = cleaned
        .into_iter()
        .filter_map(|(cwd, ts)| ts.as_f64().map(|ts| (cwd, ts)))
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    rows.truncate(recent_cwd_max());
    Ok(rows.into_iter().map(|(cwd, _)| cwd).collect())
}

pub fn read_cwd_groups(path: &Path) -> Result<Map<String, Value>, String> {
    let Some(value) = read_optional_json_lenient(path, "cwd_groups.json")? else {
        return Ok(Map::new());
    };
    let Value::Object(object) = value else {
        tracing::warn!(path = %path.display(), "invalid cwd_groups.json: top-level value is not an object");
        return Ok(Map::new());
    };
    let mut cleaned = Map::new();
    for (cwd, value) in object {
        let normalized_cwd = normalize_cwd_group_key(&cwd);
        let Some(normalized_cwd) = normalized_cwd else {
            continue;
        };
        let Value::Object(entry) = value else {
            continue;
        };
        let label = entry
            .get("label")
            .and_then(Value::as_str)
            .map(clean_alias_public)
            .unwrap_or_default();
        let collapsed = entry
            .get("collapsed")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let hidden = entry
            .get("hidden")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let hidden_after_live_start_ts = entry
            .get("hidden_after_live_start_ts")
            .and_then(clean_hidden_after_live_start_ts);
        if !(label.is_empty() && !collapsed && !hidden) {
            // Phase 3 will reconcile via `cwd_groups_save` when POST /api/cwd_groups/edit lands.
            cleaned.insert(
                normalized_cwd,
                cwd_group_entry_public(&label, collapsed, hidden, hidden_after_live_start_ts),
            );
        }
    }
    Ok(cleaned)
}

pub fn tmux_available() -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|dir| {
        let candidate = dir.join("tmux");
        is_executable_file(&candidate)
    })
}

fn normalize_url_prefix(raw: Option<&str>) -> Result<String, String> {
    let Some(value) = raw else {
        return Ok(String::new());
    };
    let mut prefix = value.trim().to_string();
    if prefix.is_empty() || prefix == "/" {
        return Ok(String::new());
    }
    if prefix.contains("://") {
        return Err("CODEX_WEB_URL_PREFIX must be a path prefix (not a URL)".to_string());
    }
    if prefix.contains('?') || prefix.contains('#') {
        return Err("CODEX_WEB_URL_PREFIX must not include '?' or '#'".to_string());
    }
    if !prefix.starts_with('/') {
        return Err("CODEX_WEB_URL_PREFIX must start with '/'".to_string());
    }
    while prefix.len() > 1 && prefix.ends_with('/') {
        prefix.pop();
    }
    if prefix == "/" {
        Ok(String::new())
    } else {
        Ok(prefix)
    }
}

fn read_optional_json(path: &Path) -> Result<Option<Value>, String> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|err| format!("parse {}: {err}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("read {}: {err}", path.display())),
    }
}

fn read_optional_json_lenient(path: &Path, label: &str) -> Result<Option<Value>, String> {
    match fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(value) => Ok(Some(value)),
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "invalid {label}");
                Ok(None)
            }
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "invalid {label}");
            Ok(None)
        }
    }
}

pub fn clean_alias_public(value: &str) -> String {
    let mut out = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.len() > 80 {
        out.truncate(80);
        out = out.trim_end().to_string();
    }
    out
}

pub fn normalize_cwd_group_key(cwd: &str) -> Option<String> {
    let trimmed = cwd.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    let expanded = if let Some(rest) = trimmed.strip_prefix("~/") {
        env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or(path)
    } else if trimmed == "~" {
        env::var("HOME").map(PathBuf::from).unwrap_or(path)
    } else {
        path
    };
    Some(
        progressive_canonicalize(&expanded)
            .to_string_lossy()
            .to_string(),
    )
}

fn progressive_canonicalize(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }

    let mut missing_tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path;
    loop {
        if let Ok(canonical_prefix) = fs::canonicalize(cursor) {
            let mut out = canonical_prefix;
            for component in missing_tail.iter().rev() {
                out.push(component);
            }
            return out;
        }
        match (cursor.parent(), cursor.file_name()) {
            (Some(parent), Some(name)) => {
                missing_tail.push(name.to_os_string());
                cursor = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn clean_hidden_after_live_start_ts(value: &Value) -> Option<f64> {
    if value.is_null() || value.is_boolean() {
        return None;
    }
    let out = value.as_f64()?;
    (out.is_finite() && out > 0.0).then_some(out)
}

pub fn cwd_group_entry_public(
    label: &str,
    collapsed: bool,
    hidden: bool,
    hidden_after_live_start_ts: Option<f64>,
) -> Value {
    let mut entry = Map::new();
    entry.insert("label".to_string(), json!(clean_alias_public(label)));
    entry.insert("collapsed".to_string(), json!(collapsed));
    if hidden {
        entry.insert("hidden".to_string(), json!(true));
        entry.insert(
            "hidden_after_live_start_ts".to_string(),
            hidden_after_live_start_ts.map_or(Value::Null, Value::from),
        );
    }
    Value::Object(entry)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}
