use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn write_fake_bin(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

fn write_fake_codex_bin(dir: &Path, ready: &Path) -> PathBuf {
    let source = dir.join("fake-codex.c");
    let bin = dir.join("fake-codex");
    let ready_literal = ready
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    fs::write(
        &source,
        format!(
            r#"#include <signal.h>
#include <stdio.h>
#include <unistd.h>

static volatile sig_atomic_t done = 0;

static void stop(int sig) {{
  (void)sig;
  done = 1;
}}

int main(void) {{
  FILE *file = fopen("{ready_literal}", "w");
  if (!file) return 10;
  fputs("ready", file);
  fclose(file);
  signal(SIGTERM, stop);
  signal(SIGINT, stop);
  while (!done) pause();
  return 0;
}}
"#
        ),
    )
    .unwrap();
    let status = Command::new("cc")
        .arg(&source)
        .arg("-o")
        .arg(&bin)
        .status()
        .unwrap();
    assert!(status.success());
    bin
}

fn broker_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_codoxear-broker-rs"))
}

#[cfg(target_os = "linux")]
fn ignore_sigchld_then_exec_broker_bin(dir: &Path) -> PathBuf {
    let source = dir.join("ignore-sigchld-then-exec.c");
    let bin = dir.join("ignore-sigchld-then-exec");
    fs::write(
        &source,
        r#"#include <signal.h>
#include <unistd.h>

int main(int argc, char **argv) {
  if (argc < 2) return 64;
  signal(SIGCHLD, SIG_IGN);
  execv(argv[1], argv + 1);
  return 127;
}
"#,
    )
    .unwrap();
    let status = Command::new("cc")
        .arg(&source)
        .arg("-o")
        .arg(&bin)
        .status()
        .unwrap();
    assert!(status.success());
    bin
}

fn wait_for_live_meta(app_dir: &Path) -> Value {
    let socks = app_dir.join("socks");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen_json = Vec::new();
    while Instant::now() < deadline {
        if let Ok(entries) = fs::read_dir(&socks) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let raw = fs::read_to_string(&path).unwrap();
                let value: Value = serde_json::from_str(&raw).unwrap();
                if let Some(sock_path) = value["sock_path"].as_str() {
                    if fs::metadata(sock_path).is_ok() {
                        return value;
                    }
                    seen_json.push(format!(
                        "{} points to missing {}",
                        path.display(),
                        sock_path
                    ));
                } else {
                    seen_json.push(format!("{} missing sock_path", path.display()));
                }
            }
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "live broker metadata not written under {}; seen: {}",
        socks.display(),
        seen_json.join("; ")
    );
}

fn wait_for_file(path: &Path) -> String {
    wait_for_file_result(path).unwrap_or_else(|()| panic!("file not written: {}", path.display()))
}

fn wait_for_file_result(path: &Path) -> Result<String, ()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(raw) = fs::read_to_string(path) {
            return Ok(raw);
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err(())
}

fn collect_child_failure(mut child: Child) -> String {
    let status = child.try_wait().ok().flatten();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    format!("child_status={status:?}; stderr={stderr}")
}

fn wait_for_ready_or_broker_exit(ready: &Path, child: &mut Child) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if fs::read_to_string(ready).is_ok() {
            return Ok(());
        }
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            return Err(format!("broker exited before fake codex ready: {status}"));
        }
        thread::sleep(Duration::from_millis(50));
    }
    Err("fake codex readiness file was not written before timeout".to_string())
}

fn wait_for_ready_or_fail(ready: &Path, child: Child) -> Child {
    let mut child = child;
    if let Err(reason) = wait_for_ready_or_broker_exit(ready, &mut child) {
        let details = collect_child_failure(child);
        panic!("fake codex did not start: {reason}; {details}");
    }
    child
}

fn sock_call(sock_path: &str, payload: Value) -> Value {
    let mut stream = UnixStream::connect(sock_path).unwrap();
    writeln!(stream, "{}", serde_json::to_string(&payload).unwrap()).unwrap();
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response).unwrap();
    serde_json::from_str(&response).unwrap()
}

fn stop_child(mut child: Child, sock_path: Option<&str>) {
    if let Some(sock_path) = sock_path {
        let _ = sock_call(sock_path, serde_json::json!({"cmd":"shutdown"}));
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn rust_broker_codex_writes_sidecar_and_serves_socket() {
    let dir = TempDir::new().unwrap();
    let ready = dir.path().join("codex-ready");
    let fake_codex = write_fake_codex_bin(dir.path(), &ready);
    let child = Command::new(broker_bin())
        .args(["--cwd", dir.path().to_str().unwrap(), "--"])
        .env("CODOXEAR_APP_DIR", dir.path().join("app"))
        .env("CODEX_WEB_AGENT_BACKEND", "codex")
        .env("CODEX_BIN", &fake_codex)
        .env("CODEX_HOME", dir.path().join("codex-home"))
        .env("CODEX_WEB_SPAWN_NONCE", "nonce-codex")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let child = wait_for_ready_or_fail(&ready, child);

    let meta = wait_for_live_meta(&dir.path().join("app"));
    let sock_path = meta["sock_path"].as_str().unwrap().to_string();
    assert_eq!(meta["backend"], "codex");
    assert_eq!(meta["agent_backend"], "codex");
    assert_eq!(meta["spawn_nonce"], "nonce-codex");
    assert_eq!(meta["log_path"], Value::Null);
    assert!(meta["codex_pid"].as_i64().unwrap() > 0);
    assert_eq!(
        fs::metadata(meta["sock_path"].as_str().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    assert_eq!(
        sock_call(&sock_path, serde_json::json!({"cmd":"send","text":""})),
        serde_json::json!({"error":"text required"})
    );
    let state = sock_call(&sock_path, serde_json::json!({"cmd":"state"}));
    assert_eq!(state["queue_len"], 0);
    assert!(state.get("busy").is_some());

    stop_child(child, Some(&sock_path));
}

#[cfg(target_os = "linux")]
#[test]
fn rust_broker_codex_resets_ignored_sigchld_before_waiting_for_pty_child() {
    let dir = TempDir::new().unwrap();
    let ready = dir.path().join("codex-ready");
    let fake_codex = write_fake_codex_bin(dir.path(), &ready);
    let wrapper = ignore_sigchld_then_exec_broker_bin(dir.path());
    let mut child = Command::new(wrapper)
        .arg(broker_bin())
        .args(["--cwd", dir.path().to_str().unwrap(), "--"])
        .env("CODOXEAR_APP_DIR", dir.path().join("app"))
        .env("CODEX_WEB_AGENT_BACKEND", "codex")
        .env("CODEX_BIN", &fake_codex)
        .env("CODEX_HOME", dir.path().join("codex-home"))
        .env("CODEX_WEB_SPAWN_NONCE", "nonce-sigchld")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    if let Err(reason) = wait_for_ready_or_broker_exit(&ready, &mut child) {
        let details = collect_child_failure(child);
        panic!(
            "fake codex did not start when broker inherited ignored SIGCHLD: {reason}; {details}"
        );
    }
    let status = child.try_wait().unwrap();
    assert!(
        status.is_none(),
        "broker exited early after inherited ignored SIGCHLD: {status:?}"
    );
    let meta = wait_for_live_meta(&dir.path().join("app"));
    let sock_path = meta["sock_path"].as_str().unwrap().to_string();
    assert_eq!(meta["spawn_nonce"], "nonce-sigchld");
    assert_eq!(
        sock_call(&sock_path, serde_json::json!({"cmd":"state"}))["queue_len"],
        0
    );

    stop_child(child, Some(&sock_path));
}

#[test]
fn rust_broker_pi_writes_session_path_and_pi_socket_commands() {
    let dir = TempDir::new().unwrap();
    let fake_pi = write_fake_bin(
        dir.path(),
        "fake-pi.sh",
        &format!(
            "#!/bin/sh\nprintf '%s\n' \"$@\" > {}\necho fake-pi-ready\nexec tail -f /dev/null\n",
            dir.path().join("pi-args.txt").display()
        ),
    );
    let session_file = dir.path().join("session.jsonl");
    let child = Command::new(broker_bin())
        .args([
            "--cwd",
            dir.path().to_str().unwrap(),
            "--session-file",
            session_file.to_str().unwrap(),
            "--",
            "--model",
            "fake",
        ])
        .env("CODOXEAR_APP_DIR", dir.path().join("app"))
        .env("CODEX_WEB_AGENT_BACKEND", "pi")
        .env("PI_BIN", &fake_pi)
        .env("PI_HOME", dir.path().join("pi-home"))
        .env("CODEX_WEB_OWNER", "web")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let meta = wait_for_live_meta(&dir.path().join("app"));
    let sock_path = meta["sock_path"].as_str().unwrap().to_string();
    assert_eq!(meta["backend"], "pi");
    assert_eq!(meta["transport"], "pi-rpc");
    assert_eq!(meta["agent_backend"], "pi");
    assert_eq!(
        meta["session_path"].as_str(),
        Some(session_file.to_string_lossy().as_ref())
    );
    assert_eq!(meta["supports_live_ui"], true);
    assert!(meta["agent_pid"].as_i64().unwrap() > 0);
    assert_eq!(meta["codex_pid"], meta["agent_pid"]);
    let pi_args = wait_for_file(&dir.path().join("pi-args.txt"));
    assert!(pi_args.contains("--mode\nrpc\n"));
    assert!(pi_args.contains(&format!("--session\n{}\n", session_file.display())));
    assert!(pi_args.contains("--model\nfake\n"));

    assert_eq!(
        sock_call(
            &sock_path,
            serde_json::json!({"cmd":"live_messages","offset":0})
        ),
        serde_json::json!({"offset":0,"events":[]})
    );
    assert_eq!(
        sock_call(&sock_path, serde_json::json!({"cmd":"ui_state"})),
        serde_json::json!({"requests":[]})
    );
    assert_eq!(
        sock_call(&sock_path, serde_json::json!({"cmd":"commands"})),
        serde_json::json!({"commands":[]})
    );

    stop_child(child, Some(&sock_path));
}
