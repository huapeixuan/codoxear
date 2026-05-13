## 0. Phase 1 follow-up baseline (must be done first)

These tasks address `rust-backend-skeleton` review退回项 and unblock Phase 2 byte-level parity testing.

- [x] 0.1 Add a shared 200-path helper (`json_response(status, value) -> Response` in `backend-rs/src/routes.rs` or `backend-rs/src/json.rs`) that always writes `Content-Type: application/json; charset=utf-8`; refactor `me()` and `sessions_bootstrap()` to use it (remove the `axum::Json` 200 path); reference: code-reviewer #1 on PEI-53.
- [x] 0.2 Update `backend-rs/Cargo.toml`: add `serde_json = { version = "1", features = ["preserve_order"] }`; rebuild; rerun `cargo test --release` and `pytest tests/contract -k parity` to ensure Phase 1 retest stays green; reference: code-reviewer #2 on PEI-53.
- [x] 0.3 In Phase 1 endpoints that build nested `Map<String, Value>` (notably `BootstrapResponse.new_session_defaults.backends.codex` / `.backends.pi`), insert keys in the same order Python's dict insertion order; verify via raw-bytes diff in a new contract test fixture.
- [x] 0.4 Rewrite `backend-rs/src/runtime.rs::read_cwd_groups`: on parse error or non-Object top level, emit `tracing::warn!` with the file path and underlying error and return an empty map; add a unit test reading a malformed `cwd_groups.json` and asserting `Map::new()` plus exactly one warn-level log line; reference: code-reviewer Phase 1 follow-up MEDIUM #2.
- [x] 0.5 Rewrite `backend-rs/src/runtime.rs::normalize_cwd_group_key`: implement progressive canonicalization equivalent to Python `Path.expanduser().resolve(strict=False)` (walk longest existing prefix, canonicalize, join remaining tail); add unit tests for: (a) symlinked parent + non-existent child, (b) all-existing path, (c) all-non-existent path, (d) `~`-expanded path; reference: code-reviewer Phase 1 follow-up MEDIUM #3.
- [x] 0.6 Add bootstrap populated-state contract tests in `tests/contract/test_endpoint_parity.py`: (b) populated `recent_cwds.json` parity, (c) populated `cwd_groups.json` round-trip parity, (d) `tmux_available` reflects `which tmux`; keep existing (a) empty-state and (e) `/api/v1/bootstrap` 404 tests; verify all five scenarios pass; reference: code-reviewer Phase 1 follow-up MEDIUM #4.
- [x] 0.7 Add a regression-guard integration test (`backend-rs/tests/json_response_helper.rs` or extension to `health_me_bootstrap.rs`) that issues every Phase 1 200 path and asserts `Content-Type` equals exactly `application/json; charset=utf-8`.

## 1. Module split & shared infrastructure scaffolding

- [x] 1.1 Create empty modules `backend-rs/src/broker_client.rs`, `backend-rs/src/session_loader.rs`, `backend-rs/src/log_normalizer/mod.rs` (with `codex.rs` and `pi.rs` submodules), `backend-rs/src/git_context.rs`, `backend-rs/src/voice_state.rs`, plus a `backend-rs/src/handlers/` directory with empty `mod.rs` and per-wave files (`voice.rs`, `sessions_list.rs`, `session_meta.rs`, `git.rs`, `files.rs`, `messages.rs`, `metrics.rs`); register them in `lib.rs` and `main.rs`.
- [x] 1.2 Move existing Phase 1 code from `runtime.rs` into the appropriate new modules: `RuntimeConfig` and `load_or_create_hmac_secret` stay in `runtime.rs`; `read_recent_cwds` / `read_cwd_groups` / `normalize_cwd_group_key` / `read_new_session_defaults` / `tmux_available` move into `runtime.rs` or a sub-module; ensure `runtime.rs` line count drops below 500.
- [x] 1.3 Move existing Phase 1 handlers `health` / `me` / `sessions_bootstrap` into `handlers/health_meta.rs` (or equivalent); update `routes::router` accordingly; verify all Phase 1 tests still pass.
- [x] 1.4 Extend `.github/workflows/backend-rs.yml` (or the `wc -l` gate Phase 1 introduced) to enforce `<= 800` lines on **every** `backend-rs/src/**/*.rs` file, not just `runtime.rs`; CI fails fast if any file exceeds the limit.

## 2. Read-only broker IPC client

- [x] 2.1 Implement `backend-rs/src/broker_client.rs::broker_request(sock_path: &Path, request: &Value, timeout: Duration) -> Result<Value, BrokerError>`; reuse the protocol from ref `runtime.rs:7195-7220` (write JSON + `\n`, read one line, parse). Use `std::os::unix::net::UnixStream` with `set_read_timeout`/`set_write_timeout`; close stream on drop.
- [x] 2.2 Implement typed wrappers `broker_state(sock_path, timeout) -> Result<BrokerState, BrokerError>`, `broker_ui_state(sock_path, timeout) -> Result<BrokerUiState, BrokerError>`, `broker_commands(sock_path, timeout) -> Result<BrokerCommands, BrokerError>`. Define `BrokerError { ConnectRefused, Timeout, Empty, Malformed(String), Io(String) }` enum.
- [x] 2.3 Add unit tests `backend-rs/tests/broker_client.rs` covering: (a) successful state call against a stub `nc -lU` listener that returns a canned JSON line, (b) connect refused (path does not exist), (c) timeout (listener accepts but does not respond), (d) malformed response (returns `not-json\n`), (e) missing `busy`/`queue_len` fields → `Malformed`. Use `tempfile::tempdir()` for sock paths.
- [x] 2.4 Verify with `grep -nE '"(state|ui_state|commands|inject|interrupt|shutdown|enqueue)"' backend-rs/src/broker_client.rs` that only `state`, `ui_state`, `commands` literals appear (write commands are Phase 4).

## 3. Session loader

- [x] 3.1 Implement `backend-rs/src/session_loader.rs::load_session_rows(config: &RuntimeConfig) -> Result<Vec<SessionRow>, String>`: scan `<app_dir>/socks/*.sock`, for each parse `<id>.json` sidecar, skip dead PIDs (use `pid_alive` ported from ref `runtime.rs`), skip hidden ids; merge in alias/queue/harness/sidebar/files/hidden state from JSON files. Reference Python `SessionManager.list_sessions` (`codoxear/server.py:6416-6700`) and ref `runtime::load_sessions_response`.
- [x] 3.2 Define `SessionRow` struct in `backend-rs/src/models.rs` carrying every field the frontend reads from `/api/sessions` (≥ 30 fields including `session_id, agent_backend, backend, owner, transport, cwd, log_path, start_ts, updated_ts, broker_pid, codex_pid, busy, broker_busy, queue_len, harness_enabled, alias, files, priority_offset, snooze_until, dependency_session_id, final_priority, base_priority, time_priority, git_branch, pr_summary, todo_snapshot, ...`); use `IndexMap` for any nested object that needs Python-order parity.
- [x] 3.3 For each session row, call `broker_client::broker_state(sock_path, Duration::from_millis(1500))` to get live `busy`/`queue_len`/`token`; on error log at WARN and fall back to sidecar values; emit `broker_busy` separately so callers can distinguish.
- [x] 3.4 Implement `priority_from_elapsed_seconds(elapsed: f64) -> f64`, `clip01`, and `final_priority` computation 1:1 with Python `_priority_from_elapsed_seconds` and `_clip01`. Add unit tests for boundary cases (elapsed=0, very large elapsed, snoozed, blocked).
- [x] 3.5 Implement grouping helpers `_session_list_visible_grouped_rows`, `_session_list_group_sort_key`, `_frontend_session_list_row`, `_session_recent_sort_key` 1:1 with Python `codoxear/server.py:1817-1976`; add unit tests covering at least 3 sessions across 2 cwds with mixed busy / hidden / alias state.
- [x] 3.6 Implement `load_sessions_directories_payload(rows, cwd_groups, group_key, offset, limit, group_offset, group_limit)` and `load_sessions_recent_payload(rows, cwd_groups, offset, limit)` returning `serde_json::Value` with `IndexMap`-backed objects; cover `remaining_by_group`, `omitted_group_count`, `remaining` keys.
- [x] 3.7 Implement `find_session(config, session_id)` for per-session endpoints (returns 404-friendly `Err` if not found); reuse it across diagnostics / queue / harness / messages.

## 4. Log normalizer

- [x] 4.1 Port `codoxear/rollout_log.py::_messages_from_codex_log` into `backend-rs/src/log_normalizer/codex.rs::messages_from_codex_log(log_path: &Path, offset, limit, init, before) -> CodexMessagesPage`; preserve event types, delivery messages, idle markers, token snapshots; carry over Python's branch order and short-circuit conditions.
- [x] 4.2 Port `codoxear/rollout_log.py::_idle_from_log`, `_token_snapshot_from_log`, `_run_settings_from_log`, `_todo_snapshot_payload_for_session` to corresponding Rust functions.
- [x] 4.3 Port `codoxear/pi_log.py` 309 lines into `backend-rs/src/log_normalizer/pi.rs`: `messages_from_pi_log`, `pi_session_header`, `pi_user_text`, `pi_assistant_text`, `pi_final_turn`, `pi_run_settings`, `pi_context_usage`. Read `~/.pi/agent/models.json` for token deltas.
- [x] 4.4 Add fixture files under `tests/fixtures/rollout/*.jsonl` (codex sample sessions: short, paginated, mixed events) and `tests/fixtures/pi/*.jsonl` (pi short, with model-cache, with AskUser flow); commit them.
- [x] 4.5 Add `backend-rs/tests/log_normalizer_parity.rs`: for each fixture, assert Rust output matches Python output produced via `python3 -c "from codoxear.rollout_log import _messages_from_codex_log; ..."` (drive Python via subprocess inside the test or via a small `tools/dump_log_normalizer.py` helper that materializes JSON to a file).
- [x] 4.6 Verify file size: `wc -l backend-rs/src/log_normalizer/codex.rs backend-rs/src/log_normalizer/pi.rs` each `<= 800`; if approaching, split further (e.g. `pi/header.rs`, `pi/usage.rs`).

## 5. Git context

- [x] 5.1 Port `codoxear/git_context.py` (334 lines) into `backend-rs/src/git_context.rs`: `RepoContext { availability, git_branch, pr_summary }`, `resolve_repo_context(cwd: &Path, refresh: bool) -> RepoContext`, plus `to_detail_dict() -> serde_json::Value` matching Python's response shape.
- [x] 5.2 Implement per-cwd lock + TTL cache: use `parking_lot::Mutex<HashMap<PathBuf, CachedContext>>` with `BRANCH_TTL_S = 30s`, `PR_TTL_S = 120s`, `GH_AUTH_TTL_S = 300s`; honor `CODEX_WEB_*` env overrides matching Python.
- [x] 5.3 Implement `git_branch` resolution via `git rev-parse --abbrev-ref HEAD` with timeout 2.0s; PR resolution via `gh pr view --json number,title,state,url,isDraft,baseRefName,headRefName` with timeout 4.0s; downgrade to `availability="no-gh" | "no-pr" | "not-a-repo" | "error"` mirroring Python.
- [x] 5.4 Add `backend-rs/tests/git_context_parity.rs`: in `tempfile::tempdir()` create a real git repo via `git init` + commit; assert `current_git_branch` matches Python's`; on a directory that is not a git repo, assert both Rust and Python return `availability="not-a-repo"`.
- [x] 5.5 Update `.github/workflows/backend-rs.yml` macOS job to run `brew install gh`; document in `tests/contract/README.md` that the parity selector requires `gh` on PATH.

## 6. Voice / notification state snapshot

- [x] 6.1 Implement `backend-rs/src/voice_state.rs::load_voice_settings_snapshot(app_dir) -> serde_json::Value` reading `voice_settings.json` (creating no file, no worker), returning the Python `_voice_push.settings_snapshot()` keyset with default values when the file is absent or malformed.
- [x] 6.2 Implement `load_subscriptions_snapshot(app_dir) -> serde_json::Value` reading `push_subscriptions.json`; mirror Python `_voice_push.subscriptions_snapshot()`.
- [x] 6.3 Implement `notification_state_for_message(app_dir, message_id) -> Option<serde_json::Value>` reading `voice_delivery_ledger.json` and indexing by message id.
- [x] 6.4 Implement `notification_feed_since(app_dir, since: f64) -> Vec<serde_json::Value>` reading the ledger and emitting events with `ts >= since` in the same order Python uses.
- [x] 6.5 Add unit tests for each of 6.1–6.4 with both empty and populated fixtures (place files in `tests/fixtures/voice/` and read into `tempfile::tempdir()`).

## 7. Wave A handlers — voice / notification / metrics

- [x] 7.1 Implement `handlers::voice::settings_voice` for `GET /api/settings/voice` and `/api/v1/...` using `voice_state::load_voice_settings_snapshot`; register in `routes::router` behind auth; emit `application/json; charset=utf-8`.
- [x] 7.2 Implement `handlers::voice::notification_subscriptions` for `GET /api/notifications/subscription` (+ v1).
- [x] 7.3 Implement `handlers::voice::notification_message` for `GET /api/notifications/message?message_id=` (+ v1) with 400 / 404 / 200 paths matching Python.
- [x] 7.4 Implement `handlers::voice::notification_feed` for `GET /api/notifications/feed?since=` (+ v1) with 400 / 200 paths.
- [x] 7.5 Implement `handlers::metrics::metrics` for `GET /api/metrics` (+ v1); Phase 2 may emit zero / empty histories but key set must match Python.
- [x] 7.6 Update `tests/contract/test_endpoint_parity.py`: add `test_settings_voice_parity`, `test_notifications_subscription_parity`, `test_notifications_message_parity` (covering 400 / 404 / 200), `test_notifications_feed_parity` (covering 400 / 200), `test_metrics_parity`. All assert status, Content-Type header, and dict-eq body; deterministic empty-state cases also assert byte-eq.

## 8. Wave B handlers — session list & resume candidates

- [x] 8.1 Implement `handlers::sessions_list::sessions` for `GET /api/sessions` (+ v1) wrapping `session_loader::load_session_rows` + `load_sessions_directories_payload` / `load_sessions_recent_payload`; honor query params with same clamping (`limit ∈ [1,50]`, `group_limit ∈ [1,20]`, `view ∈ {directories,recent}`).
- [x] 8.2 Add 400 paths: unsupported view, group pagination on recent view; emit identical error bodies to Python.
- [x] 8.3 Implement `handlers::sessions_list::session_resume_candidates` for `GET /api/session_resume_candidates` (+ v1) wrapping ref `load_resume_candidates_response` ported to user28b45952 field set.
- [x] 8.4 Add `tests/contract/test_endpoint_parity.py::test_sessions_list_parity` covering: (a) empty HOME, (b) 1 session, (c) 3 sessions across 2 cwds, (d) `?view=recent`, (e) `?limit=999` clamping, (f) `?view=banana` 400, (g) one session with broker socket unreachable (sidecar fallback). Use `monkeypatch` and pre-seeded socket fixtures.
- [x] 8.5 Add `tests/contract/test_endpoint_parity.py::test_session_resume_candidates_parity` covering: (a) empty cwd, (b) cwd with prior codex sessions, (c) Pi backend.

## 9. Wave C handlers — per-session metadata

- [x] 9.1 Implement `handlers::session_meta::diagnostics` for `GET /api/sessions/{id}/diagnostics` (+ v1) producing the full Python field set including `git_branch` / `pr_summary` / `todo_snapshot` / `time_priority` / `base_priority` / `final_priority`.
- [x] 9.2 Implement `handlers::session_meta::queue` for `GET /api/sessions/{id}/queue` (+ v1) reading from `session_queues.json` only — verify with grep that no broker call is made.
- [x] 9.3 Implement `handlers::session_meta::harness_get` for `GET /api/sessions/{id}/harness` (+ v1) reading `harness.json` and applying Python defaults for missing fields.
- [x] 9.4 Implement `handlers::session_meta::workspace`, `details`, `takeover` for the corresponding endpoints; port Python helpers `_session_workspace_payload`, `_session_details_payload`, `_session_takeover_payload`.
- [x] 9.5 Implement `handlers::session_meta::ui_state` for `GET /api/sessions/{id}/ui_state` (+ v1): for Pi sessions call `broker_client::broker_ui_state`; for non-Pi return sidebar fields (`priority_offset`, `snooze_until`, `dependency_session_id`).
- [x] 9.6 Implement `handlers::session_meta::commands` for `GET /api/sessions/{id}/commands` (+ v1) calling `broker_client::broker_commands` for Pi; non-Pi may return `404` or empty list (mirror Python behavior precisely after re-reading `MANAGER.get_session_commands`).
- [x] 9.7 Implement `handlers::session_meta::repo` for `GET /api/sessions/{id}/repo` (+ v1) wrapping `git_context::resolve_repo_context(cwd, refresh=qs.refresh==1)` and emitting `to_detail_dict()`.
- [x] 9.8 Add `tests/contract/test_endpoint_parity.py::test_diagnostics_parity` (live broker fixture vs broker-down fallback), `test_queue_parity`, `test_harness_get_parity`, `test_workspace_parity`, `test_details_parity`, `test_ui_state_parity` (Pi + non-Pi cases), `test_commands_parity`, `test_takeover_parity`, `test_repo_parity`. Use a `pytest` plugin to spin up a stub broker on `nc -lU` for live-state cases.

## 10. Wave D handlers — git endpoints

- [x] 10.1 Implement `handlers::git::changed_files` for `GET /api/sessions/{id}/git/changed_files` (+ v1); port Python git subprocess invocations with same args, timeouts, byte limits (`GIT_DIFF_TIMEOUT_SECONDS`, `64KB` / `128KB` caps); produce `ok/cwd/files/entries/staged/unstaged` shape.
- [x] 10.2 Implement `handlers::git::diff` for `GET /api/sessions/{id}/git/diff?path=&staged=` (+ v1).
- [x] 10.3 Implement `handlers::git::file_versions` for `GET /api/sessions/{id}/git/file_versions?path=` (+ v1) emitting `ok/cwd/path/abs_path/base_exists/base_text/current_exists/current_text/current_size`.
- [x] 10.4 Add `_run_git` helper in `git_context.rs` (or a `git_subprocess.rs`) shared between `git_context::resolve_repo_context` and Wave D handlers; honor timeout + byte cap.
- [x] 10.5 Add `tests/contract/test_endpoint_parity.py::test_git_changed_files_parity`, `test_git_diff_parity`, `test_git_file_versions_parity` using a shared `tempfile`-managed git repo fixture (`fixtures/git_repo` with seeded commits + dirty tree).

## 11. Wave E handlers — file viewer

- [x] 11.1 Implement path-traversal guard `resolve_session_path(cwd: &Path, path: &str) -> Result<PathBuf, ApiError>` rejecting absolute paths, `..` escapes, and symlinks pointing outside the cwd; mirror Python `_safe_join` behavior.
- [x] 11.2 Implement `handlers::files::file_read` for `GET /api/sessions/{id}/file/read?path=` (+ v1) emitting `ok, kind, path, rel, size, text?, editable?, version?, content_type?, image_url?, pdf_url?, download_only?, reason?, viewer_max_bytes?`. Determine MIME via a Rust helper agreed in design open-question (`mime_guess` crate with explicit override list to match Python `mimetypes`).
- [x] 11.3 Implement `handlers::files::file_search` for `GET /api/sessions/{id}/file/search?q=&limit=` (+ v1).
- [x] 11.4 Implement `handlers::files::file_list` for `GET /api/sessions/{id}/file/list?path=` (+ v1).
- [x] 11.5 Implement `handlers::files::file_blob` for `GET /api/sessions/{id}/file/blob?path=` (+ v1) emitting raw bytes with the correct Content-Type and Content-Length.
- [x] 11.6 Implement `handlers::files::file_download` for `GET /api/sessions/{id}/file/download?path=` (+ v1) emitting `Content-Disposition: attachment; filename="<basename>"`.
- [x] 11.7 Implement `handlers::files::files_blob` for `GET /api/files/blob?path=` (+ v1) — global file viewer (huapeixuan-only).
- [x] 11.8 Add `tests/contract/test_endpoint_parity.py::test_file_read_parity` (text + binary + path traversal rejection), `test_file_search_parity`, `test_file_list_parity`, `test_file_blob_parity` (image bytes + correct MIME + 404 path), `test_file_download_parity`, `test_files_blob_parity`.

## 12. Wave F handlers — message log readers

- [x] 12.1 Implement `handlers::messages::messages` for `GET /api/sessions/{id}/messages?offset=&limit=&before=&init=` (+ v1) wrapping `log_normalizer::codex::messages_from_codex_log` / `pi::messages_from_pi_log`; clamp `limit ∈ [20, 200]`, `offset >= 0`, `before >= 0`; for non-Pi sessions inject `payload.diag.meta_refresh_ms`.
- [x] 12.2 Implement `handlers::messages::tail` for `GET /api/sessions/{id}/tail` (+ v1) returning `{"tail": ...}`.
- [x] 12.3 Implement `handlers::messages::live` for `GET /api/sessions/{id}/live?offset=&live_offset=&requests_version=` (+ v1) wrapping a Rust port of `_session_live_payload` (this depends on log normalizer + sidebar + sessions queue).
- [x] 12.4 Verify `/api/v1/...` and `/api/...` aliases produce byte-identical responses for the same input.
- [x] 12.5 Add `tests/contract/test_endpoint_parity.py::test_messages_init_parity`, `test_messages_poll_parity`, `test_tail_parity`, `test_live_parity`. Use the same fixtures introduced in §4.4.

## 13. CI, docs, inventory

- [x] 13.1 Update `.github/workflows/backend-rs.yml`: install `gh` on macOS (`brew install gh`); add a step running `pytest tests/contract -k 'parity and readonly'` (alongside the existing Phase 1 selector); ensure `wc -l` gate covers all `backend-rs/src/**/*.rs`.
- [x] 13.2 Update `tests/contract/README.md` with new readonly selector instructions and the required `gh` toolchain on macOS.
- [x] 13.3 Update `README.md` with any new env vars used by `git_context` (already shared with Python: `CODEX_WEB_BRANCH_TIMEOUT_S`, `CODEX_WEB_PR_TIMEOUT_S`, `CODEX_WEB_GH_AUTH_TTL_S`) and confirm Phase 2 binary still runs on default `[::]:8743`.
- [x] 13.4 Update `docs/cutover/endpoint-inventory.md`: for every Phase 2 endpoint table row, change the "Ref comparison" annotation from "...Phase 2 port" to "implemented by `rust-backend-readonly-routes`"; preserve untouched rows for Phase 3 / Phase 4 / Phase 5 endpoints.
- [x] 13.5 Update inventory's "Must be ported in Phase 2 (GET/read-only parity)" section to link each bullet to its tasks section here (`Wave B`, `Wave C`, etc.).
- [x] 13.6 Update `openspec/changes/rust-backend-cutover/tasks.md` Phase 2 row(s) to reflect that the dedicated change `rust-backend-readonly-routes` is the Phase 2 owner.

## 14. Definition of Done — verification gate (must all be green to ship)

- [ ] 14.1 `cd backend-rs && cargo fmt --all -- --check` passes.
- [ ] 14.2 `cd backend-rs && cargo clippy --all-targets -- -D warnings` passes.
- [ ] 14.3 `cd backend-rs && cargo test --release` passes; coverage includes new unit tests for `broker_client`, `session_loader`, `log_normalizer/{codex,pi}`, `git_context`, `voice_state`, `json_response`, `normalize_cwd_group_key`, `read_cwd_groups`.
- [ ] 14.4 `cd backend-rs && cargo build --release --bins` produces both `codoxear-backend-rs` and `codoxear-broker-rs` (Phase 1 stub still exits non-zero).
- [ ] 14.5 `find backend-rs/src -name '*.rs' -print0 | xargs -0 wc -l | awk '$1 > 800'` returns no lines (every source file ≤ 800 lines).
- [ ] 14.6 `pytest tests/contract -q -k 'parity and readonly'` passes on Linux and macOS; no Phase 2 endpoint test is skipped or xfailed.
- [ ] 14.7 `pytest tests/contract -q -k 'parity and not readonly'` (Phase 1 retest) still passes — no regression on `/api/me`, `/api/sessions/bootstrap`, `/api/health`.
- [ ] 14.8 `openspec validate rust-backend-readonly-routes --strict` returns "Change 'rust-backend-readonly-routes' is valid"; `openspec validate rust-backend-cutover --strict` still returns valid.
- [ ] 14.9 Independent code review by `code-reviewer` produces PASS or WARNING-with-mergeable-fixes; any HIGH must be addressed before declaring Phase 2 done.
