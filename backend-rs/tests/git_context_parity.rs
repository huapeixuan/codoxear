use codoxear_backend_rs::git_context::{
    current_git_branch, reset_caches_for_tests, resolve_repo_context,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn python_json(script: &str) -> Value {
    let python = std::env::var("CODOXEAR_CONTRACT_PYTHON")
        .ok()
        .or_else(|| {
            let candidate = PathBuf::from("/tmp/codoxear-venv/bin/python");
            candidate
                .exists()
                .then(|| candidate.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "python3".to_string());
    let output = Command::new(python)
        .arg("-c")
        .arg(script)
        .current_dir(repo_root())
        .output()
        .expect("run python parity helper");
    assert!(
        output.status.success(),
        "python failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("python emitted JSON")
}

#[test]
fn current_git_branch_matches_python_in_real_repo() {
    reset_caches_for_tests();
    let dir = TempDir::new().unwrap();
    init_git_repo(dir.path());

    let rust_branch = current_git_branch(dir.path());
    let py = python_json(&format!(
        r#"
import json
from pathlib import Path
from codoxear import git_context
git_context.reset_caches_for_tests()
print(json.dumps({{"branch": git_context.current_git_branch(Path({path:?}))}}, separators=(",", ":")))
"#,
        path = dir.path().to_string_lossy()
    ));

    assert_eq!(rust_branch, py["branch"].as_str().map(ToString::to_string));
}

#[test]
fn non_repo_availability_matches_python() {
    reset_caches_for_tests();
    let dir = TempDir::new().unwrap();

    let rust = resolve_repo_context(dir.path(), true).to_detail_dict();
    let py = python_json(&format!(
        r#"
import json
from pathlib import Path
from codoxear import git_context
git_context.reset_caches_for_tests()
ctx = git_context.resolve_repo_context(Path({path:?}), refresh=True)
print(json.dumps(ctx.to_detail_dict(), separators=(",", ":")))
"#,
        path = dir.path().to_string_lossy()
    ));

    assert_eq!(rust["availability"], py["availability"]);
    assert_eq!(rust["git_branch"], py["git_branch"]);
    assert_eq!(rust["pr"], py["pr"]);
}

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
