use codoxear_backend_rs::runtime::clean_alias_public;
use codoxear_backend_rs::state_files::{read_modify_write_json, write_object_file, write_value};
use codoxear_backend_rs::write_cleaners::{
    attachment_inject_text, clean_dependency_session_id, clean_harness_cooldown,
    clean_harness_remaining, clean_hidden_after_live_start_ts, clean_priority_offset,
    clean_queue_items, clean_safe_filename, clean_snooze_until, legacy_ui_response_text,
    normalize_pi_image_inputs,
};
use serde_json::{json, Map, Value};
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

#[test]
fn write_object_file_sorts_keys_and_matches_python_pretty_json_shape() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session_aliases.json");
    let mut object = Map::new();
    object.insert("z".to_string(), json!("last"));
    object.insert("a".to_string(), json!("first"));

    write_object_file(&path, &object).unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"a\": \"first\",\n  \"z\": \"last\"\n}\n"
    );
}

#[test]
fn write_value_creates_parent_and_replaces_existing_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested/cwd_groups.json");

    write_value(&path, &json!({"old": true})).unwrap();
    write_value(&path, &json!({"new": [1, 2]})).unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"new\": [\n    1,\n    2\n  ]\n}\n"
    );
}

#[test]
fn read_modify_write_json_updates_under_lock_and_preserves_existing_mode() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session_sidebar.json");
    write_value(&path, &json!({"old": true})).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    let returned = read_modify_write_json(&path, |value| {
        value["old"] = json!(false);
        value["new"] = json!("ok");
        Ok(value["new"].clone())
    })
    .unwrap();

    assert_eq!(returned, json!("ok"));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"new\": \"ok\",\n  \"old\": false\n}\n"
    );
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn phase3_cleaners_match_python_edge_case_contracts() {
    assert_eq!(clean_alias_public("  hello\n   world  "), "hello world");
    assert_eq!(
        clean_alias_public(&format!("{} tail", "x".repeat(90))),
        "x".repeat(80)
    );
    assert_eq!(
        clean_queue_items(vec![
            json!("  hi  "),
            json!({"text":"  there ", "images":[{"file_name":"a.png"}]}),
            json!(" ")
        ]),
        vec![
            json!("hi"),
            json!({"text":"there","images":[{"file_name":"a.png"}]})
        ]
    );
    assert_eq!(clean_priority_offset(None).unwrap(), 0.0);
    assert_eq!(clean_priority_offset(Some(&json!(0.5))).unwrap(), 0.5);
    assert!(clean_priority_offset(Some(&json!(2))).is_err());
    assert_eq!(clean_snooze_until(Some(&Value::Null)).unwrap(), None);
    assert_eq!(
        clean_snooze_until(Some(&json!(123.5))).unwrap(),
        Some(123.5)
    );
    assert_eq!(
        clean_dependency_session_id(Some(&json!(" sid "))).unwrap(),
        Some("sid".to_string())
    );
    assert_eq!(clean_harness_cooldown(&json!(3)).unwrap(), 3);
    assert_eq!(clean_harness_remaining(&json!(0)).unwrap(), 0);
    assert!(clean_harness_remaining(&json!(-1)).is_err());
    assert_eq!(
        legacy_ui_response_text(&json!({"confirmed": true})).unwrap(),
        "yes"
    );
    assert_eq!(
        legacy_ui_response_text(&json!({"value": [" a ", "", "b"]})).unwrap(),
        "a, b"
    );
    assert_eq!(clean_hidden_after_live_start_ts(&json!(123.5)), Some(123.5));
    assert_eq!(clean_hidden_after_live_start_ts(&json!(0)), None);
    assert_eq!(clean_hidden_after_live_start_ts(&json!(true)), None);
    assert_eq!(
        clean_safe_filename(" ../My File!.txt ", "file"),
        "My_File.txt"
    );
    assert_eq!(clean_safe_filename("???", "file"), "file");
    assert_eq!(
        clean_safe_filename(&format!("{}.txt", "x".repeat(120)), "file").len(),
        96
    );
    assert_eq!(
        attachment_inject_text(2, std::path::Path::new("/tmp/a.txt")).unwrap(),
        "Attachment 2: /tmp/a.txt\n"
    );
    assert!(attachment_inject_text(0, std::path::Path::new("/tmp/a.txt")).is_err());
}

#[test]
fn pi_image_inputs_reject_non_images_and_preserve_python_shape() {
    let images = normalize_pi_image_inputs(&json!([
        {"file_name":" x.png ", "mime_type":"image/png", "data_b64":"abc"}
    ]))
    .unwrap();
    assert_eq!(
        images,
        vec![json!({"file_name":"x.png", "mime_type":"image/png", "data_b64":"abc"})]
    );
    assert!(normalize_pi_image_inputs(
        &json!([{ "file_name":"x.txt", "mime_type":"text/plain", "data_b64":"abc" }])
    )
    .is_err());
}
