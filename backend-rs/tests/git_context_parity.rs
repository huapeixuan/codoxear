use codoxear_backend_rs::git_context::{reset_caches_for_tests, resolve_repo_context};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn detail_dict_contains_no_gh_or_no_pr_for_repo() {
    reset_caches_for_tests();
    let dir = TempDir::new().unwrap();
    init_git_repo(dir.path());

    let detail = resolve_repo_context(dir.path(), true).to_detail_dict();

    assert_eq!(detail["git_branch"], "main");
    assert!(matches!(
        detail["availability"].as_str(),
        Some("no-gh" | "no-pr" | "error" | "ok")
    ));
}

fn init_git_repo(path: &Path) {
    run_git(path, &["init", "-b", "main"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
    run_git(path, &["config", "user.name", "Test User"]);
    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    run_git(path, &["add", "README.md"]);
    run_git(path, &["commit", "-m", "init"]);
}

fn run_git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
