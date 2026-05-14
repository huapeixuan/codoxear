## 0. Pre-flight and scope guard

- [x] 0.1 Run `openspec status --change rust-backend-write-routes` and confirm proposal/design/specs are present before coding.
- [ ] 0.2 Re-read `docs/cutover/endpoint-inventory.md` POST rows and mark each row owner as Phase 3 or Phase 5; explicitly leave `/api/notifications/test_push` and `/api/audio/test_announcement` as Phase 5 unless implementing real side effects.
- [ ] 0.3 Re-read `docs/cutover/disk-contracts.md` and create a checklist of every Phase 3-written file, including serialization (`sort_keys`, indent, trailing newline), mode, and atomic write strategy.
- [x] 0.4 Run baseline verification: `cd backend-rs && cargo test --release`, `pytest tests/contract -q -k 'parity and readonly'`, and `openspec validate rust-backend-write-routes --strict` after artifacts are written.

## 1. Shared write infrastructure

- [ ] 1.1 Add `backend-rs/src/state_files.rs` or equivalent with `read_modify_write_json`, `write_json_atomic`, file-mode preservation, temp-file + fsync + rename, and parent fsync best effort. _(partial: added atomic JSON writer + parent fsync + lock wrapper; mode preservation/read_modify helper still pending)_
- [x] 1.2 Add per-file in-process mutex/advisory lock helper for `harness.json`, `session_queues.json`, `session_aliases.json`, `session_sidebar.json`, `session_files.json`, `cwd_groups.json`, `voice_settings.json`, and `push_subscriptions.json`.
- [ ] 1.3 Add Rust helpers mirroring Python cleaners: alias, priority offset, snooze_until, dependency_session_id, harness cooldown, harness remaining, queue item images, hidden_after_live_start_ts, safe filename, attachment inject text. _(partial: metadata/queue/harness cleaners added; file/attachment cleaners pending)_
- [ ] 1.4 Add unit tests for all cleaners using Python edge cases from `codoxear/server.py` and `codoxear/voice_push.py`.
- [x] 1.5 Extend line-count CI gate if new modules are added; ensure every `backend-rs/src/**/*.rs` file remains ≤ 800 lines.

## 2. Broker mutation client

- [x] 2.1 Extend `backend-rs/src/broker_client.rs` with typed write wrappers: `broker_send`, `broker_keys`, `broker_ui_response`, `broker_shutdown`, and optionally `broker_tail` if existing message/tail code needs parity reuse.
- [x] 2.2 Implement Python-equivalent timeouts: state 1.5s, send/ui_response 3.0s, keys/interrupt 2.0s, shutdown 1.0s unless Python uses a different timeout at the call site.
- [x] 2.3 Add stub Unix socket tests asserting exact outbound JSON for `send` with/without Pi images, `keys` ESC, `ui_response` value/confirmed/cancelled, and `shutdown`.
- [ ] 2.4 Add error mapping tests: broker `{error:"..."}` becomes handler 502 where Python raises `ValueError`; dead session cleanup path returns 404 where Python would remove stale sidecar.

## 3. Router and auth POST endpoints

- [x] 3.1 Register POST routes in `backend-rs/src/routes.rs` for both `/api/v1/...` and legacy `/api/...`; keep `POST /api/hooks/notify` outside auth middleware.
- [x] 3.2 Implement auth login handler: parse JSON body, verify password using Python-compatible source/env, sign `codoxear_auth`, emit matching `Set-Cookie`, return compact `{"ok":true}`.
- [x] 3.3 Implement auth logout handler: require auth, clear cookie with same path / Max-Age / HttpOnly / SameSite / Secure behavior as Python.
- [ ] 3.4 Add contract tests for login success cookie cross-authenticates Python, bad password 403, malformed/empty body behavior, and logout cookie clearing.

## 4. CWD groups, aliases, and sidebar writes

- [x] 4.1 Implement `POST /api/cwd_groups/edit`: known cwd validation, hidden reconciliation inputs, normalized key, and `cwd_groups.json` persistence.
- [x] 4.2 Implement `POST /api/sessions/{id}/rename`: `session_aliases.json` update/removal and response `ok/alias` parity.
- [x] 4.3 Implement `POST /api/sessions/{id}/edit`: update alias plus `session_sidebar.json` fields `priority_offset`, `snooze_until`, `dependency_session_id`; validate dependency exists and is not self.
- [ ] 4.4 Add Rust unit tests for JSON write shape and key ordering for `cwd_groups.json`, `session_aliases.json`, and `session_sidebar.json`.
- [ ] 4.5 Add contract tests: Rust edit → Python GET `/ui_state`/`/sessions`; Python edit → Rust GET `/ui_state`/`/sessions`; include invalid field 400 and unknown session 404.

## 5. Queue mutation endpoints

- [x] 5.1 Implement queue item read-modify-write helpers preserving Python schema: text-only as string, image queue item as `{text, images}`.
- [x] 5.2 Implement `POST /api/sessions/{id}/enqueue` with text/images validation, Pi session file touch behavior where applicable, `queued/queue_len` response.
- [x] 5.3 Implement `POST /api/sessions/{id}/queue/delete` and `/queue/update`, including index required, out-of-range mapping, image preservation on update, empty queue cleanup.
- [ ] 5.4 Add contract tests for enqueue text, enqueue images, update preserves images, delete last item removes session key, invalid index/text, unknown session.
- [ ] 5.5 Add a concurrent enqueue Rust test to ensure no lost update under multiple simultaneous requests to the same queue file.

## 6. Harness config endpoint

- [x] 6.1 Implement harness config normalization and `harness.json` writer matching Python `_save_harness` (`sort_keys=True`, indent=2, trailing newline).
- [x] 6.2 Implement `POST /api/sessions/{id}/harness`, including unknown legacy `text` rejection, cooldown/remaining validation, missing session behavior, and normalized response.
- [ ] 6.3 Add contract tests: Rust write → Python GET `/harness`; Python write → Rust GET `/harness`; invalid `text`, cooldown, remaining, and unknown session cases.

## 7. Send, ui_response, interrupt, heartbeat

- [ ] 7.1 Implement `POST /api/sessions/{id}/send`: historical Pi resume handling if feasible, live broker `send`, Pi images forwarding, idle auto-stop heartbeat refresh, dead broker cleanup, and fallback enqueue when broker socket is unavailable but process is alive. _(partial: live send, Pi images, and stale-socket fallback enqueue implemented; historical resume/heartbeat refresh/dead cleanup pending)_
- [x] 7.2 Implement `POST /api/sessions/{id}/ui_response`: Pi-only validation, forward allowed fields (`id`, `value`, `confirmed`, `cancelled`), live UI success, `unknown cmd` legacy fallback via ESC/text, and matching 404/502 errors.
- [x] 7.3 Implement `POST /api/sessions/{id}/interrupt`: broker `keys` with ESC and response `ok/broker` parity.
- [x] 7.4 Implement `POST /api/sessions/{id}/heartbeat`: update web activity for supported web-owned pi-rpc sessions and return `session_id`, `idle_timeout_seconds`, `last_web_activity_ts`; unsupported sessions return 409.
- [ ] 7.5 Add stub-broker contract tests for send success, send broker-error, send fallback enqueue, ui_response success/fallback/cancelled, interrupt ESC payload, heartbeat supported/unsupported.

## 8. File write, global file POST, and attachment injection

- [x] 8.1 Reuse or extend Phase 2 path helpers for write-safe `_resolve_session_path`, `_resolve_under`, global `_resolve_client_file_path`, symlink escape rejection, and absolute-path handling.
- [x] 8.2 Implement `POST /api/sessions/{id}/file/write` create/update paths: text validation, version required, conflict responses, atomic UTF-8 write, `session_files.json` history update, response `ok/path/rel/size/version/editable`.
- [x] 8.3 Implement `POST /api/files/read` and `/api/files/inspect` using Phase 2 file view helper and add file history when `session_id` is valid.
- [x] 8.4 Implement `POST /api/files/blob` alias behavior, including query `path`, inline image/pdf bytes, Content-Disposition, Cache-Control, and errors.
- [x] 8.5 Implement `POST /api/sessions/{id}/inject_file` and `/inject_image`: body size cap, base64 validation, filename sanitization, decoded byte limit, upload mode `0600`, Pi 409 rejection, bracketed paste broker `keys` command.
- [ ] 8.6 Add contract tests for file update success, stale version conflict, create success/conflict, path traversal, non-editable file, global read/inspect/blob, file history round-trip, upload too large, invalid base64, Pi injection rejection, and successful non-Pi injection command. _(partial: Rust integration coverage added for global read/inspect history, file update success, stale conflict, history round-trip, Pi injection rejection, non-Pi >2MiB upload success, and decoded upload too large)_

## 9. Session create/delete/takeover lifecycle

- [x] 9.1 Port request parser `_parse_create_session_request` to Rust, covering `cwd`, `backend`/`agent_backend`, `args`, `resume_session_id`, `worktree_branch`, model/provider/reasoning/service tier, and `create_in_tmux`.
- [ ] 9.2 Implement non-tmux `POST /api/sessions` for codex by spawning `python -m codoxear.broker` with the same env and args Python uses, waiting for `CODEX_WEB_SPAWN_NONCE` sidecar, seeding resumed alias when needed.
- [ ] 9.3 Implement non-tmux `POST /api/sessions` for pi by spawning `python -m codoxear.pi_broker` with `ask_user_bridge.ts`, session file selection/resume validation, and Pi env parity.
- [ ] 9.4 Implement worktree branch creation parity for codex or explicitly gate unsupported cases with the same error until covered; add tests for `worktree_branch` with resume rejection.
- [ ] 9.5 Implement tmux create path parity or at minimum no-tmux + tmux-unavailable parity; if tmux support is deferred, update spec/tasks before coding and mark as a blocker for full Phase 3 DoD.
- [ ] 9.6 Implement `POST /api/sessions/{id}/delete`: historical row hide, broker shutdown via `shutdown`, fallback kill if needed, hidden_sessions/session state cleanup. _(partial: active session shutdown/kill, hidden_sessions, and state cleanup implemented; historical row handling pending)_
- [x] 9.7 Implement `POST /api/sessions/{id}/takeover/open`: descriptor eligibility check and terminal open behavior; if platform-specific open cannot run in CI, cover descriptor-not-eligible and mock open command. _(terminal launch is best-effort and CI covers descriptor-not-eligible path)_
- [ ] 9.8 Add contract tests for create codex/pi happy path with stub/fake broker command where possible, cwd required/creation errors, resume not found, delete unknown/success cleanup, and takeover not eligible. _(partial: Rust integration coverage includes delete success cleanup and takeover not eligible)_

## 10. Lightweight voice/subscription writes and hooks

- [x] 10.1 Implement `POST /api/settings/voice`: clean settings with Python-compatible defaults and persist `voice_settings.json`; notify no worker in Phase 3.
- [x] 10.2 Implement `POST /api/notifications/subscription`: clean subscription endpoint/keys, compute Python-compatible subscription id, persist record with timestamps/device fields, return subscription snapshot.
- [x] 10.3 Implement `POST /api/notifications/subscription/toggle`: endpoint required, enabled bool, unknown subscription 404, persist updated timestamp and enabled state.
- [x] 10.4 Implement `POST /api/audio/listener`: client_id/enabled validation and active listener heartbeat state if in-memory state exists; do not start HLS/TTS worker.
- [x] 10.5 Decide and implement Phase 3 behavior for `/api/notifications/test_push` and `/api/audio/test_announcement`: either leave unregistered/404 or return explicit 501 feature-disabled; update endpoint inventory to keep Phase 5 owner.
- [x] 10.6 Implement unauthenticated `POST /api/hooks/notify` and `/api/v1/hooks/notify` returning `{"ignored":true}`.
- [ ] 10.7 Add contract tests for settings update, subscription upsert/toggle/unknown, listener heartbeat validation, debug endpoints non-success/no side effects, and hooks no-auth behavior. _(partial: Rust integration coverage added for settings update, subscription upsert/toggle, listener heartbeat, hooks no-auth, and debug endpoint feature-disabled/no-side-effect; unknown toggle contract coverage pending)_

## 11. Rust queue and harness workers

- [ ] 11.1 Add `queue_worker.rs`: gated by `CODOXEAR_ENABLE_QUEUE_SWEEP`, periodically discovers sessions, prunes missing queues, checks broker busy/queue_len and log idle state, waits `QUEUE_IDLE_GRACE_SECONDS`, sends one queued item, then pops it from `session_queues.json`. _(partial: gated Rust worker loop added; prunes missing queues, checks broker/session busy and log idle, waits idle grace, sends one item and pops it)_
- [ ] 11.2 Add `harness_worker.rs`: gated by `CODOXEAR_ENABLE_HARNESS_SWEEP`, implements Python `_harness_sweep` cooldown, remaining injection, assistant-last-message, broker idle, local queue empty, and rendered prompt send behavior. _(partial: gated Rust worker loop added; checks broker/local queue idle, cooldown, scope cooldown, assistant-last-message gate, sends Python-parity prompt, and decrements remaining)_
- [x] 11.3 Wire workers in `backend-rs/src/main.rs` after state build; default off; truthy/falsy parsing matches Python helper.
- [ ] 11.4 Add worker unit/integration tests with temp app dir, log fixtures, and stub broker: disabled no side effects, enabled sends once, cooldown prevents duplicate, remaining reaches zero disables harness, queue drain pops exactly one item. _(partial: truthy/falsy parser, queue idle grace/pop exactly one, harness assistant/cooldown/prompt/decrement behavior covered; disabled/no-side-effect and remaining-zero-disable cases still pending)_

## 12. Python worker handoff

- [x] 12.1 Modify `codoxear/server.py:SessionManager.__init__` to start `_harness_thr` only when `CODOXEAR_ENABLE_HARNESS_SWEEP` is falsy.
- [x] 12.2 Modify `SessionManager.__init__` to start `_queue_thr` only when `CODOXEAR_ENABLE_QUEUE_SWEEP` is falsy.
- [x] 12.3 Modify `SessionManager.__init__` to start `_voice_push_scan_thr` only when `CODOXEAR_ENABLE_VOICE_SCAN` is falsy; leave voice worker/TTS ownership for Phase 5.
- [x] 12.4 Add Python tests verifying truthy flags prevent thread creation and falsy/unset flags preserve existing thread startup; assert no corresponding file is touched during a short wait when yielded.
- [x] 12.5 Document worker handoff env vars in README or cutover docs with rollback order: disable Rust flag, restart Python, verify Python thread active.

## 13. Contract tests, docs, and inventory updates

- [ ] 13.1 Extend `tests/contract/test_endpoint_parity.py` or split `test_post_parity.py` with `@pytest.mark.post` covering every Phase 3 POST route.
- [ ] 13.2 For every disk-writing endpoint, add Rust-write→Python-read and Python-write→Rust-read round-trip assertions.
- [ ] 13.3 For every broker endpoint, add stub broker assertions for exact JSON command and timeout/error mapping.
- [ ] 13.4 Update `tests/contract/README.md` with `pytest tests/contract -q -k 'parity and post'`, worker flag usage, and test fixtures for stub brokers.
- [ ] 13.5 Update `docs/cutover/endpoint-inventory.md`: mark Phase 3 implemented rows, keep Phase 5 voice debug/HLS rows separate, and link to this tasks file.
- [ ] 13.6 Update `docs/cutover/disk-contracts.md` if Rust atomic lock/temp strategy differs from Python direct writes, without changing schema.
- [ ] 13.7 Update `openspec/changes/rust-backend-cutover/tasks.md` Phase 3 row to reference this change and its validation status.

## 14. Verification gate

- [x] 14.1 `cd backend-rs && cargo fmt --all -- --check` passes.
- [x] 14.2 `cd backend-rs && cargo clippy --all-targets -- -D warnings` passes.
- [x] 14.3 `cd backend-rs && cargo test --release` passes, including broker mutation, state file, queue/harness worker, auth, file write, and lifecycle tests.
- [x] 14.4 `cd backend-rs && cargo build --release --bins` passes.
- [x] 14.5 `find backend-rs/src -name '*.rs' -print0 | xargs -0 wc -l | awk '$1 > 800'` returns no source file over limit.
- [x] 14.6 `pytest tests/contract -q -k 'parity and readonly'` still passes (Phase 2 regression check).
- [ ] 14.7 `pytest tests/contract -q -k 'parity and post'` passes with no skipped/xfailed Phase 3 endpoint tests.
- [x] 14.8 Python worker handoff tests pass with truthy and falsy `CODOXEAR_ENABLE_*` env flags.
- [x] 14.9 `openspec validate rust-backend-write-routes --strict` returns valid.
- [x] 14.10 `openspec validate rust-backend-cutover --strict` still returns valid.
- [ ] 14.11 Independent `code-reviewer` review completes with PASS or no HIGH findings before implementation is declared ready for merge.
