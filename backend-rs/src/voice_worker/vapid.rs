use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use p256::elliptic_curve::rand_core::OsRng;
use p256::pkcs8::{EncodePrivateKey, LineEnding};
use p256::SecretKey;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const VAPID_PRIVATE_KEY_FILE: &str = "webpush_vapid_private.pem";

pub fn public_key_from_pem(path: &Path) -> Result<String, String> {
    let pem = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let builder = web_push::VapidSignatureBuilder::from_pem_no_sub(pem.as_bytes())
        .map_err(|err| format!("parse VAPID private key {}: {err}", path.display()))?;
    Ok(URL_SAFE_NO_PAD.encode(builder.get_public_key()))
}

pub fn load_or_create_public_key(app_dir: &Path) -> Result<String, String> {
    let path = app_dir.join(VAPID_PRIVATE_KEY_FILE);
    if path.exists() {
        return public_key_from_pem(&path);
    }
    fs::create_dir_all(app_dir).map_err(|err| format!("create {}: {err}", app_dir.display()))?;
    let secret = SecretKey::random(&mut OsRng);
    let pem = secret
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(|err| format!("encode VAPID private key: {err}"))?;
    let mut file =
        fs::File::create(&path).map_err(|err| format!("write {}: {err}", path.display()))?;
    file.write_all(pem.as_bytes())
        .map_err(|err| format!("write {}: {err}", path.display()))?;
    file.sync_all()
        .map_err(|err| format!("sync {}: {err}", path.display()))?;
    Ok(public_key_from_secret(&secret))
}

pub fn public_key_from_secret(secret: &SecretKey) -> String {
    let pem = secret
        .to_pkcs8_pem(LineEnding::LF)
        .expect("generated P-256 key encodes as SEC1 PEM");
    let builder = web_push::VapidSignatureBuilder::from_pem_no_sub(pem.as_bytes())
        .expect("generated P-256 key parses as WebPush VAPID PEM");
    URL_SAFE_NO_PAD.encode(builder.get_public_key())
}

pub fn normalize_vapid_subject(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("empty vapid subject".to_string());
    }
    if value.starts_with("mailto:") {
        return Ok(value.to_string());
    }
    if value.starts_with("http://") || value.starts_with("https://") {
        return Ok(value.trim_end_matches('/').to_string());
    }
    Err("vapid subject must start with https://, http://, or mailto:".to_string())
}

pub fn default_vapid_subject_from_env() -> String {
    if let Some(subject) = std::env::var("CODEX_WEB_PUSH_VAPID_SUBJECT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| normalize_vapid_subject(&value).ok())
    {
        return subject;
    }
    tailscale_https_subject_with_runner(default_tailscale_status_runner)
        .unwrap_or_else(|| "https://localhost".to_string())
}

pub fn tailscale_https_subject_with_runner<F>(mut runner: F) -> Option<String>
where
    F: FnMut(Duration) -> Result<String, String>,
{
    let raw = runner(Duration::from_secs(5)).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    let dns_name = value
        .get("Self")
        .and_then(|node| node.get("DNSName"))
        .and_then(serde_json::Value::as_str)?
        .trim()
        .trim_end_matches('.')
        .to_string();
    if dns_name.is_empty() {
        return None;
    }
    Some(format!("https://{dns_name}"))
}

fn default_tailscale_status_runner(timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("tailscale")
        .args(["status", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("run tailscale status --json: {err}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|err| format!("read tailscale status output: {err}"))?;
                if !output.status.success() {
                    return Err(format!("tailscale status exited {}", output.status));
                }
                return String::from_utf8(output.stdout)
                    .map_err(|err| format!("tailscale output utf8: {err}"));
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("tailscale status timed out".to_string());
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(err) => return Err(format!("wait tailscale status: {err}")),
        }
    }
}
