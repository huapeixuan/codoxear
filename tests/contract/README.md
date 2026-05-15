# Contract parity harness

This directory contains cross-backend parity tests for the Rust cutover. Phase 1
runs the Python server and the new Rust backend against a shared temporary HOME
so both processes see the same `~/.local/share/codoxear` state and `hmac_secret`.

## Parity checks

Build the Rust backend binaries first:

```bash
(cd backend-rs && cargo build --release --bins)
pytest tests/contract -q -k 'parity and not readonly'
pytest tests/contract -q -k 'parity and readonly'
pytest tests/contract -q -k 'parity and post'
```

Phase 2 readonly parity requires `gh` on `PATH` (CI installs it on Linux and
macOS) so git-context tests exercise the no-auth/no-PR paths instead of skipping
due to a missing CLI.

Current implementation status: Phase 1 retest covers `/api/me` and
`/api/sessions/bootstrap` including populated bootstrap state; the broader
readonly endpoint tests are unlocked wave-by-wave by
`rust-backend-readonly-routes`.

Phase 3 POST parity uses the same dual-server fixture for disk-writing routes
and lightweight Rust-only stub broker coverage for live broker mutations. The
`post` selector covers auth, cwd group edits, alias/sidebar writes, queue and
harness mutations, send/ui/interrupt/heartbeat socket calls including broker
error and legacy fallback branches, file write/read/inspect, attachment
rejection boundaries, voice/subscription/listener writes, feature-disabled
voice debug endpoints, hooks no-auth behavior, and session-create
validation/tmux-unavailable boundaries.

Queue and harness workers remain opt-in during Phase 3. To test or operate Rust
workers, start Rust with `CODOXEAR_ENABLE_QUEUE_SWEEP=1` and/or
`CODOXEAR_ENABLE_HARNESS_SWEEP=1`; Python `SessionManager` yields the matching
thread when the same flag is truthy. Rollback order is: unset the Rust flag,
stop/restart the Rust process, then restart Python so Python starts the default
sweep thread and resumes ownership of `session_queues.json` / `harness.json`.

If you need to run Python-only contract collection without the Rust binary, set:

```bash
CODOXEAR_SKIP_RUST_FIXTURE=1 pytest tests/contract -q -k parity
```

Without that escape hatch, a missing Rust binary is a fixture error with a hint to
run `(cd backend-rs && cargo build --release --bins)`.

## Fixtures

- `shared_app_home`: session-scoped temporary HOME shared by Python and Rust.
- `python_server_url`: starts `python -m codoxear.server` on a random localhost
  port with `CODEX_WEB_PASSWORD`, `CODEX_WEB_HOST=127.0.0.1`, and the shared HOME.
- `rust_server_url`: starts `backend-rs/target/release/codoxear-backend-rs` on a
  random localhost port with the same password and HOME.
- `signed_auth_cookie`: lazily creates/loads
  `<shared_app_home>/.local/share/codoxear/hmac_secret` and signs a
  `codoxear_auth` cookie directly. Login parity is now covered separately by
  cross-authenticating Python-issued and Rust-issued cookies.
- `_write_contract_session`: creates a sidecar plus a configurable Unix-socket
  stub broker. Tests can pass `broker_handler` to assert exact write commands or
  force broker errors/legacy fallbacks without starting real Codex/Pi brokers.

Common fail modes:

- Python fixture startup timeout: `CODEX_WEB_PASSWORD`, static asset mode, or port
  binding regressed.
- Rust fixture error: build the release binaries, or set
  `CODOXEAR_SKIP_RUST_FIXTURE=1` for Python-only runs.
- JSON diff: compare the failing key against `docs/cutover/endpoint-inventory.md`
  and `docs/cutover/disk-contracts.md`; add an ignore key only for documented
  volatile values such as `app_version`.
