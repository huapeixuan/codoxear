use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use p256::elliptic_curve::rand_core::OsRng;
use p256::pkcs8::{EncodePrivateKey, LineEnding};
use p256::SecretKey;
use std::fs;
use std::io::Write;
use std::path::Path;

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
    std::env::var("CODEX_WEB_PUSH_VAPID_SUBJECT")
        .ok()
        .and_then(|value| normalize_vapid_subject(&value).ok())
        .unwrap_or_else(|| "https://localhost".to_string())
}
