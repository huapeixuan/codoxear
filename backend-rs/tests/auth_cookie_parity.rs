use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use codoxear_backend_rs::runtime::{sign_auth_cookie_value, unix_now_seconds, verify_auth_cookie};
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;

#[test]
fn rust_signed_cookie_roundtrips() {
    let secret = b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let token = sign_auth_cookie_value(secret, unix_now_seconds() + 60).expect("sign cookie");
    assert!(verify_auth_cookie(&token, secret));
}

#[test]
fn python_style_cookie_verifies_in_rust() {
    let secret = b"fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
    let raw = serde_json::to_vec(&json!({"exp": unix_now_seconds() + 60})).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    mac.update(&raw);
    let sig = mac.finalize().into_bytes();
    let token = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(raw),
        URL_SAFE_NO_PAD.encode(sig)
    );
    assert!(verify_auth_cookie(&token, secret));
}

#[test]
fn expired_cookie_is_rejected() {
    let secret = b"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let token = sign_auth_cookie_value(secret, unix_now_seconds() - 60).expect("sign cookie");
    assert!(!verify_auth_cookie(&token, secret));
}
