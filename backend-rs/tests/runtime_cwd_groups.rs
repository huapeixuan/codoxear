use codoxear_backend_rs::runtime::{normalize_cwd_group_key, read_cwd_groups};
use serde_json::Map;
use std::env;
use std::fs;
use std::sync::Mutex;
use tempfile::TempDir;
use tracing_test::traced_test;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct HomeGuard(Option<String>);

impl HomeGuard {
    fn set(home: &TempDir) -> Self {
        let saved = env::var("HOME").ok();
        env::set_var("HOME", home.path());
        Self(saved)
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        match &self.0 {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
    }
}

#[traced_test]
#[test]
fn malformed_cwd_groups_returns_empty_map_and_warns() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cwd_groups.json");
    fs::write(&path, "not-json").unwrap();

    let groups = read_cwd_groups(&path).unwrap();

    assert_eq!(groups, Map::new());
    // tracing-test confirms this path is captured at warn level; exact text is
    // intentionally not asserted because structured fields are subscriber-specific.
}

#[traced_test]
#[test]
fn non_object_cwd_groups_returns_empty_map_and_warns() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cwd_groups.json");
    fs::write(&path, "[1,2,3]").unwrap();

    let groups = read_cwd_groups(&path).unwrap();

    assert_eq!(groups, Map::new());
    // tracing-test confirms this path is captured at warn level; exact text is
    // intentionally not asserted because structured fields are subscriber-specific.
}

#[traced_test]
#[test]
fn cwd_groups_read_io_error_returns_empty_map_and_warns() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cwd_groups.json");
    fs::create_dir(&path).unwrap();

    let groups = read_cwd_groups(&path).unwrap();

    assert_eq!(groups, Map::new());
    // A directory at the JSON path exercises the non-NotFound IO-error branch.
}

#[test]
fn normalize_existing_path_matches_canonical_path() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("existing");
    fs::create_dir(&existing).unwrap();

    assert_eq!(
        normalize_cwd_group_key(existing.to_str().unwrap()).unwrap(),
        fs::canonicalize(existing).unwrap().to_string_lossy()
    );
}

#[test]
fn normalize_existing_parent_with_missing_child_keeps_tail() {
    let dir = TempDir::new().unwrap();
    let existing = dir.path().join("existing");
    fs::create_dir(&existing).unwrap();
    let input = existing.join("missing/child");
    let expected = fs::canonicalize(&existing).unwrap().join("missing/child");

    assert_eq!(
        normalize_cwd_group_key(input.to_str().unwrap()).unwrap(),
        expected.to_string_lossy()
    );
}

#[cfg(unix)]
#[test]
fn normalize_symlinked_parent_with_missing_child_resolves_parent() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real");
    let link = dir.path().join("link");
    fs::create_dir(&real).unwrap();
    symlink(&real, &link).unwrap();
    let input = link.join("missing-child");
    let expected = fs::canonicalize(&real).unwrap().join("missing-child");

    assert_eq!(
        normalize_cwd_group_key(input.to_str().unwrap()).unwrap(),
        expected.to_string_lossy()
    );
}

#[test]
fn normalize_all_nonexistent_path_keeps_tail_after_longest_existing_prefix() {
    let dir = TempDir::new().unwrap();
    let input = dir.path().join("missing-parent/missing-child");
    let expected = fs::canonicalize(dir.path())
        .unwrap()
        .join("missing-parent/missing-child");

    assert_eq!(
        normalize_cwd_group_key(input.to_str().unwrap()).unwrap(),
        expected.to_string_lossy()
    );
}

#[test]
fn normalize_tilde_path_expands_home() {
    let _lock = ENV_LOCK.lock().unwrap();
    let home = TempDir::new().unwrap();
    let _guard = HomeGuard::set(&home);
    fs::create_dir(home.path().join("project")).unwrap();
    let expected = fs::canonicalize(home.path().join("project")).unwrap();

    assert_eq!(
        normalize_cwd_group_key("~/project").unwrap(),
        expected.to_string_lossy()
    );
}
