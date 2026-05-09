use codoxear_backend_rs::voice_state::{
    load_subscriptions_snapshot, load_voice_settings_snapshot, notification_feed_since,
    notification_state_for_message,
};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn voice_settings_empty_app_dir_matches_snapshot_defaults() {
    let dir = TempDir::new().unwrap();

    assert_eq!(
        load_voice_settings_snapshot(dir.path()),
        json!({
            "tts_enabled_for_narration": false,
            "tts_enabled_for_final_response": false,
            "tts_base_url": "https://api.openai.com/v1",
            "tts_api_key": "",
            "summarization_model": "gpt-4.1-mini",
            "tts_model": "gpt-4o-mini-tts",
            "audio": {
                "queue_depth": 0,
                "active_listener_count": 0,
                "stream_url": "/api/audio/live.m3u8",
                "segment_count": 0,
                "last_error": "",
                "media_sequence": 1,
            },
            "notifications": {
                "enabled_devices": 0,
                "total_devices": 0,
                "vapid_public_key": "",
            },
        })
    );
}

#[test]
fn voice_settings_populated_file_overrides_defaults() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("voice_settings.json"),
        json!({
            "tts_enabled_for_narration": true,
            "tts_enabled_for_final_response": true,
            "tts_base_url": "https://example.test/v1/",
            "tts_api_key": " key ",
            "summarization_model": " custom-summarizer ",
            "tts_model": " custom-tts ",
        })
        .to_string(),
    )
    .unwrap();

    let snapshot = load_voice_settings_snapshot(dir.path());

    assert_eq!(snapshot["tts_enabled_for_narration"], json!(true));
    assert_eq!(snapshot["tts_enabled_for_final_response"], json!(true));
    assert_eq!(snapshot["tts_base_url"], json!("https://example.test/v1"));
    assert_eq!(snapshot["tts_api_key"], json!("key"));
    assert_eq!(snapshot["summarization_model"], json!("custom-summarizer"));
    assert_eq!(snapshot["tts_model"], json!("custom-tts"));
}

#[test]
fn subscriptions_snapshot_filters_and_sorts_mobile_counts() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("push_subscriptions.json"),
        json!([
            {
                "subscription": {"endpoint": "https://push.example/a", "keys": {"p256dh": "p", "auth": "a"}},
                "notifications_enabled": true,
                "created_ts": 1.0,
                "updated_ts": 3.0,
                "user_agent": "Mozilla Mobile",
                "device_label": "phone"
            },
            {
                "subscription": {"endpoint": "https://push.example/b", "keys": {"p256dh": "p", "auth": "b"}},
                "notifications_enabled": false,
                "created_ts": 1.0,
                "updated_ts": 5.0,
                "device_class": "desktop"
            },
            {"subscription": {"endpoint": "", "keys": {}}}
        ])
        .to_string(),
    )
    .unwrap();

    let snapshot = load_subscriptions_snapshot(dir.path());

    assert_eq!(snapshot["vapid_public_key"], json!(""));
    assert_eq!(snapshot["subscriptions"].as_array().unwrap().len(), 2);
    assert_eq!(
        snapshot["subscriptions"][0]["endpoint"],
        json!("https://push.example/b")
    );
    assert_eq!(
        snapshot["subscriptions"][1]["device_class"],
        json!("mobile")
    );
}

#[test]
fn notification_state_and_feed_read_cleaned_ledger() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("voice_delivery_ledger.json"),
        json!({
            "msg-1": {
                "session_id": "sess-a",
                "session_display_name": " Alpha ",
                "message_class": "final_response",
                "notification_text": " hello   world ",
                "summary_status": "sent",
                "push_status": "pending",
                "updated_ts": 10.0,
                "created_ts": 9.0
            },
            "msg-2": {
                "session_id": "sess-b",
                "message_class": "final_response",
                "notification_text": "old",
                "summary_status": "pending",
                "updated_ts": 8.0
            },
            "msg-3": {
                "session_id": "sess-c",
                "message_class": "narration",
                "notification_text": "ignored",
                "summary_status": "sent",
                "updated_ts": 11.0
            }
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(
        notification_state_for_message(dir.path(), "msg-1"),
        Some(json!({
            "message_id": "msg-1",
            "message_class": "final_response",
            "summary_status": "sent",
            "push_status": "pending",
            "notification_text": "hello world",
        }))
    );
    assert_eq!(notification_state_for_message(dir.path(), "missing"), None);
    assert_eq!(
        notification_feed_since(dir.path(), 9.0),
        vec![json!({
            "message_id": "msg-1",
            "session_id": "sess-a",
            "session_display_name": "Alpha",
            "notification_text": "hello world",
            "updated_ts": 10.0,
        })]
    );
}
