use codoxear_backend_rs::voice_state::{
    load_subscriptions_snapshot, load_voice_settings_snapshot, notification_feed_since,
    notification_state_for_message,
};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn voice_settings_empty_home_returns_python_default_snapshot() {
    let dir = TempDir::new().unwrap();

    let snapshot = load_voice_settings_snapshot(dir.path());

    assert_eq!(snapshot["tts_enabled_for_narration"], false);
    assert_eq!(snapshot["tts_enabled_for_final_response"], false);
    assert_eq!(snapshot["tts_base_url"], "https://api.openai.com/v1");
    assert_eq!(snapshot["tts_api_key"], "");
    assert_eq!(snapshot["summarization_model"], "gpt-4.1-mini");
    assert_eq!(snapshot["tts_model"], "gpt-4o-mini-tts");
    assert_eq!(snapshot["audio"]["queue_depth"], 0);
    assert_eq!(snapshot["audio"]["active_listener_count"], 0);
    assert_eq!(snapshot["audio"]["stream_url"], "/api/audio/live.m3u8");
    assert_eq!(snapshot["audio"]["segment_count"], 0);
    assert_eq!(snapshot["audio"]["last_error"], "");
    assert_eq!(snapshot["audio"]["media_sequence"], 1);
    assert_eq!(snapshot["notifications"]["enabled_devices"], 0);
    assert_eq!(snapshot["notifications"]["total_devices"], 0);
    assert_eq!(snapshot["notifications"]["vapid_public_key"], "");
}

#[test]
fn voice_settings_populated_file_is_cleaned_like_python() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("voice_settings.json"),
        json!({
            "tts_enabled_for_narration": true,
            "tts_enabled_for_final_response": true,
            "tts_base_url": " https://voice.example/v1/ ",
            "tts_api_key": " secret ",
            "summarization_model": " gpt-x ",
            "tts_model": " voice-y "
        })
        .to_string(),
    )
    .unwrap();

    let snapshot = load_voice_settings_snapshot(dir.path());

    assert_eq!(snapshot["tts_enabled_for_narration"], true);
    assert_eq!(snapshot["tts_enabled_for_final_response"], true);
    assert_eq!(snapshot["tts_base_url"], "https://voice.example/v1/");
    assert_eq!(snapshot["tts_api_key"], "secret");
    assert_eq!(snapshot["summarization_model"], "gpt-x");
    assert_eq!(snapshot["tts_model"], "voice-y");
}

#[test]
fn subscriptions_snapshot_sorts_by_updated_ts_desc_and_filters_invalid_records() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("push_subscriptions.json"),
        json!([
            {"subscription": {"endpoint": "missing-keys"}},
            {
                "subscription": {"endpoint": "https://old.example", "keys": {"p256dh": "p", "auth": "a"}},
                "notifications_enabled": false,
                "created_ts": 1.0,
                "updated_ts": 2.0,
                "user_agent": "Desktop Browser",
                "device_label": "Desk"
            },
            {
                "subscription": {"endpoint": "https://new.example", "keys": {"p256dh": "p2", "auth": "a2"}},
                "notifications_enabled": true,
                "created_ts": 3.0,
                "updated_ts": 4.0,
                "user_agent": "Mobile Safari",
                "device_label": "Phone"
            }
        ])
        .to_string(),
    )
    .unwrap();

    let snapshot = load_subscriptions_snapshot(dir.path());
    let items = snapshot["subscriptions"].as_array().unwrap();

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["endpoint"], "https://new.example");
    assert_eq!(items[0]["device_class"], "mobile");
    assert_eq!(items[1]["endpoint"], "https://old.example");
    assert_eq!(items[1]["device_class"], "desktop");
}

#[test]
fn notification_message_and_feed_match_python_filtering_rules() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("voice_delivery_ledger.json"),
        json!({
            "m1": {
                "session_id": "s1",
                "session_display_name": "  Alpha  ",
                "message_class": "final_response",
                "notification_text": " hello   world ",
                "summary_status": "sent",
                "push_status": "pending",
                "created_ts": 1.0,
                "updated_ts": 10.0
            },
            "m2": {
                "session_id": "s1",
                "message_class": "narration",
                "notification_text": "skip narration",
                "summary_status": "sent",
                "created_ts": 1.0,
                "updated_ts": 11.0
            },
            "m3": {
                "session_id": "s2",
                "message_class": "final_response",
                "notification_text": "not ready",
                "summary_status": "pending",
                "created_ts": 1.0,
                "updated_ts": 12.0
            }
        })
        .to_string(),
    )
    .unwrap();

    let message = notification_state_for_message(dir.path(), "m1").unwrap();
    assert_eq!(message["message_id"], "m1");
    assert_eq!(message["message_class"], "final_response");
    assert_eq!(message["summary_status"], "sent");
    assert_eq!(message["push_status"], "pending");
    assert_eq!(message["notification_text"], "hello world");
    assert!(notification_state_for_message(dir.path(), "missing").is_none());

    let feed = notification_feed_since(dir.path(), 0.0);
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0]["message_id"], "m1");
    assert_eq!(feed[0]["session_display_name"], "Alpha");
    assert_eq!(feed[0]["notification_text"], "hello world");
}
