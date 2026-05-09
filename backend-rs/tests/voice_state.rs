use codoxear_backend_rs::voice_state::{
    load_subscriptions_snapshot, load_voice_settings_snapshot, notification_feed_since,
    notification_state_for_message,
};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn voice_settings_empty_home_returns_defaults() {
    let dir = TempDir::new().unwrap();

    let snapshot = load_voice_settings_snapshot(dir.path());

    assert_eq!(snapshot["tts_enabled_for_narration"], false);
    assert_eq!(snapshot["tts_enabled_for_final_response"], false);
    assert_eq!(snapshot["tts_base_url"], "https://api.openai.com/v1");
    assert_eq!(snapshot["tts_model"], "gpt-4o-mini-tts");
    assert_eq!(snapshot["audio"]["stream_url"], "/api/audio/live.m3u8");
    assert_eq!(snapshot["notifications"]["total_devices"], 0);
}

#[test]
fn subscription_snapshot_sorts_by_updated_ts_desc() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("push_subscriptions.json"),
        serde_json::to_string(&json!([
            {
                "subscription": {"endpoint": "https://one", "keys": {"p256dh": "p1", "auth": "a1"}},
                "updated_ts": 1.0,
                "user_agent": "desktop",
                "device_label": "one"
            },
            {
                "subscription": {"endpoint": "https://two", "keys": {"p256dh": "p2", "auth": "a2"}},
                "updated_ts": 3.0,
                "notifications_enabled": false,
                "device_class": "mobile",
                "device_label": "two"
            }
        ]))
        .unwrap(),
    )
    .unwrap();

    let snapshot = load_subscriptions_snapshot(dir.path());

    let items = snapshot["subscriptions"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["endpoint"], "https://two");
    assert_eq!(items[1]["endpoint"], "https://one");
}

#[test]
fn notification_state_and_feed_filter_ledger_like_python() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("voice_delivery_ledger.json"),
        serde_json::to_string(&json!({
            "m1": {
                "session_id": "s1",
                "session_display_name": "Alpha",
                "message_class": "final_response",
                "notification_text": "  hello   world ",
                "summary_status": "sent",
                "narrated_status": "pending",
                "push_status": "skipped",
                "updated_ts": 5.0
            },
            "m2": {
                "session_id": "s2",
                "message_class": "narration",
                "notification_text": "ignored",
                "summary_status": "sent",
                "push_status": "sent",
                "updated_ts": 6.0
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let state = notification_state_for_message(dir.path(), "m1").unwrap();
    assert_eq!(state["message_id"], "m1");
    assert_eq!(state["notification_text"], "hello world");
    assert_eq!(state["summary_status"], "sent");

    let feed = notification_feed_since(dir.path(), 0.0);
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0]["message_id"], "m1");
    assert_eq!(feed[0]["session_display_name"], "Alpha");
    assert!(notification_state_for_message(dir.path(), "missing").is_none());
}
