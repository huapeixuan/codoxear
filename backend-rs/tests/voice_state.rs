use codoxear_backend_rs::voice_state::{
    load_subscriptions_snapshot, load_voice_settings_snapshot, notification_feed_since,
    notification_state_for_message,
};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests")
        .join("fixtures")
        .join("voice")
        .join(name)
}

fn copy_fixture(app_dir: &Path, fixture: &str, dest: &str) {
    fs::create_dir_all(app_dir).unwrap();
    fs::copy(fixture_path(fixture), app_dir.join(dest)).unwrap();
}

#[test]
fn voice_settings_snapshot_defaults_when_file_missing_or_malformed() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path().join("app");

    let missing = load_voice_settings_snapshot(&app_dir);
    assert_default_settings_snapshot(&missing);

    fs::create_dir_all(&app_dir).unwrap();
    fs::write(app_dir.join("voice_settings.json"), b"not-json").unwrap();
    let malformed = load_voice_settings_snapshot(&app_dir);
    assert_default_settings_snapshot(&malformed);
}

#[test]
fn voice_settings_snapshot_cleans_populated_fixture_and_counts_mobile_devices() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path().join("app");
    copy_fixture(
        &app_dir,
        "voice_settings_populated.json",
        "voice_settings.json",
    );
    copy_fixture(
        &app_dir,
        "push_subscriptions_populated.json",
        "push_subscriptions.json",
    );

    let snapshot = load_voice_settings_snapshot(&app_dir);

    assert_eq!(snapshot["tts_enabled_for_narration"], true);
    assert_eq!(snapshot["tts_enabled_for_final_response"], false);
    assert_eq!(snapshot["tts_base_url"], "https://tts.example.com/v1");
    assert_eq!(snapshot["tts_api_key"], "sk-test");
    assert_eq!(snapshot["summarization_model"], "summary-model");
    assert_eq!(snapshot["tts_model"], "tts-model");
    assert_eq!(snapshot["audio"]["queue_depth"], 0);
    assert_eq!(snapshot["audio"]["active_listener_count"], 0);
    assert_eq!(snapshot["audio"]["stream_url"], "/api/audio/live.m3u8");
    assert_eq!(snapshot["audio"]["segment_count"], 0);
    assert_eq!(snapshot["audio"]["last_error"], "");
    assert_eq!(snapshot["audio"]["media_sequence"], 1);
    assert_eq!(snapshot["notifications"]["enabled_devices"], 1);
    assert_eq!(snapshot["notifications"]["total_devices"], 2);
    assert!(!snapshot["notifications"]["vapid_public_key"]
        .as_str()
        .unwrap()
        .is_empty());
}

#[test]
fn subscriptions_snapshot_defaults_and_sorts_populated_fixture() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path().join("app");

    assert_eq!(
        load_subscriptions_snapshot(&app_dir),
        json!({"vapid_public_key": load_subscriptions_snapshot(&app_dir)["vapid_public_key"].clone(), "subscriptions": []})
    );

    fs::create_dir_all(&app_dir).unwrap();
    fs::write(app_dir.join("push_subscriptions.json"), b"{}").unwrap();
    assert_eq!(
        load_subscriptions_snapshot(&app_dir),
        json!({"vapid_public_key": load_subscriptions_snapshot(&app_dir)["vapid_public_key"].clone(), "subscriptions": []})
    );

    copy_fixture(
        &app_dir,
        "push_subscriptions_populated.json",
        "push_subscriptions.json",
    );
    let snapshot = load_subscriptions_snapshot(&app_dir);
    let items = snapshot["subscriptions"].as_array().unwrap();

    assert!(!snapshot["vapid_public_key"].as_str().unwrap().is_empty());
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["endpoint"], "https://push.example/newer");
    assert_eq!(items[1]["endpoint"], "https://push.example/older");
    assert_eq!(items[2]["endpoint"], "https://push.example/disabled-mobile");
    assert_eq!(items[0]["device_class"], "desktop");
    assert_eq!(items[1]["device_class"], "mobile");
    assert_eq!(items[2]["notifications_enabled"], false);
    assert!(items[0]["id"].as_str().unwrap().len() == 24);
}

#[test]
fn notification_state_defaults_and_reads_cleaned_ledger() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path().join("app");

    assert!(notification_state_for_message(&app_dir, "m-final").is_none());

    copy_fixture(
        &app_dir,
        "voice_delivery_ledger_populated.json",
        "voice_delivery_ledger.json",
    );

    assert_eq!(
        notification_state_for_message(&app_dir, "m-final"),
        Some(json!({
            "message_id": "m-final",
            "message_class": "final_response",
            "summary_status": "sent",
            "push_status": "sent",
            "notification_text": "Hello world"
        }))
    );
    assert!(notification_state_for_message(&app_dir, "invalid-class").is_none());
    assert!(notification_state_for_message(&app_dir, "missing").is_none());
}

#[test]
fn notification_feed_filters_and_sorts_like_python_snapshot() {
    let dir = TempDir::new().unwrap();
    let app_dir = dir.path().join("app");
    copy_fixture(
        &app_dir,
        "voice_delivery_ledger_populated.json",
        "voice_delivery_ledger.json",
    );

    let items = notification_feed_since(&app_dir, 10.0);

    assert_eq!(
        items,
        vec![
            json!({
                "message_id": "m-final",
                "session_id": "s1",
                "session_display_name": "Session One",
                "notification_text": "Hello world",
                "updated_ts": 20.0
            }),
            json!({
                "message_id": "m-final-2",
                "session_id": "s2",
                "session_display_name": "Session",
                "notification_text": "Done",
                "updated_ts": 20.0
            })
        ]
    );

    assert!(notification_feed_since(&app_dir, 20.0).is_empty());
}

fn assert_default_settings_snapshot(value: &Value) {
    assert_eq!(value["tts_enabled_for_narration"], false);
    assert_eq!(value["tts_enabled_for_final_response"], false);
    assert_eq!(value["tts_base_url"], "https://api.openai.com/v1");
    assert_eq!(value["tts_api_key"], "");
    assert_eq!(value["summarization_model"], "gpt-4.1-mini");
    assert_eq!(value["tts_model"], "gpt-4o-mini-tts");
    assert_eq!(value["audio"]["queue_depth"], 0);
    assert_eq!(value["audio"]["active_listener_count"], 0);
    assert_eq!(value["audio"]["stream_url"], "/api/audio/live.m3u8");
    assert_eq!(value["audio"]["segment_count"], 0);
    assert_eq!(value["audio"]["last_error"], "");
    assert_eq!(value["audio"]["media_sequence"], 1);
    assert_eq!(value["notifications"]["enabled_devices"], 0);
    assert_eq!(value["notifications"]["total_devices"], 0);
    assert!(!value["notifications"]["vapid_public_key"]
        .as_str()
        .unwrap()
        .is_empty());
}
