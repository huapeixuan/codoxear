## 0. Pre-flight and scope guard

- [x] 0.1 Run `openspec status --change rust-backend-voice-push` and read `proposal.md`, `design.md`, `specs/rust-voice-push/spec.md`, and this `tasks.md` before coding.
- [x] 0.2 Confirm implementation branch is based on Phase 4 reviewer PASS commit `92d2a0f` or a descendant containing `rust-backend-broker`; record `git rev-parse --short HEAD` in handoff.
- [x] 0.3 Re-run baseline validation before code changes: `openspec validate rust-backend-cutover --strict`, `openspec validate rust-backend-broker --strict`, and `cd backend-rs && cargo test --release voice_state` or the closest focused selector.
- [x] 0.4 Re-read `codoxear/voice_push.py`, `tests/test_voice_push.py`, `backend-rs/src/voice_state.rs`, `backend-rs/src/voice_post.rs`, `backend-rs/src/handlers/voice.rs`, `backend-rs/src/workers.rs`, `docs/cutover/disk-contracts.md`, and `docs/cutover/endpoint-inventory.md` Phase 5 rows.
- [x] 0.5 Keep scope limited to Phase 5 voice/HLS/WebPush/TTS migration; do not remove Python `voice_push.py`, do not make Rust voice default, and do not change Phase 4 broker protocol.

## 1. Dependency spike and module scaffold

- [x] 1.1 Add `backend-rs/src/voice_worker/` modules (or equivalent split files) for config/state, ledger, scan, OpenAI client, HLS, VAPID/WebPush, tasks, and tests; register in `lib.rs` without starting workers by default.
- [x] 1.2 Evaluate Rust WebPush/VAPID dependency candidates and add the selected crates to `backend-rs/Cargo.toml`; document the final choice and rationale in `design.md` Implementation Notes.
- [x] 1.3 Add trait abstractions for OpenAI HTTP, WebPush sender, HLS media runner, and clock so unit tests can run without real OpenAI/APNs/ffmpeg.
- [x] 1.4 Add line-count guard coverage for new voice modules and keep every `backend-rs/src/**/*.rs` file ≤ 800 lines.
- [x] 1.5 Add debug logging for Rust voice ownership and failures without logging `tts_api_key`, Authorization headers, full prompt text, or push subscription secrets.

## 2. Worker flags, ownership guard, and rollback safety

- [x] 2.1 Extend Rust startup (`main.rs` / `workers.rs`) to parse `CODOXEAR_ENABLE_VOICE_SCAN` and `CODOXEAR_ENABLE_VOICE_WORKER` with the same truthy/falsy semantics as existing worker flags.
- [x] 2.2 Implement scan worker startup only when `CODOXEAR_ENABLE_VOICE_SCAN` is truthy; prove disabled startup has no background task and no voice file/HLS side effects.
- [x] 2.3 Implement delivery worker startup only when `CODOXEAR_ENABLE_VOICE_WORKER` is truthy and scan ownership is explicit; if not, fail fast or log documented drain-only behavior per spec.
- [x] 2.4 Add a Rust voice ownership lock or equivalent fail-fast guard for `voice_delivery_ledger.json` / `audio/` writes so two Rust processes cannot own the same voice worker outputs.
- [x] 2.5 Add tests for truthy/falsy flag parsing, scan-only mode, worker-enabled mode, disabled debug endpoint behavior, and rollback restart with Python-readable files.

## 3. Disk contracts and ledger writer

- [x] 3.1 Implement Rust ledger read/modify/write helpers for `voice_delivery_ledger.json` preserving Python fields, status strings, sort-key pretty JSON, trailing newline, temp-file + fsync + rename.
- [x] 3.2 Implement subscription update helpers that preserve `push_subscriptions.json` schema while recording WebPush success/failure timestamps, last_error clipping, and stale subscription drops.
- [ ] 3.3 Implement VAPID PEM helpers that load existing `webpush_vapid_private.pem`, create a Python-readable PEM if missing, and compute Python-equivalent base64url uncompressed public key.
- [x] 3.4 Add cross-language tests: Python-written voice settings/subscriptions/ledger/VAPID are read by Rust; Rust-written files are read by Python `VoicePushCoordinator` loaders or focused Python helper scripts.
- [x] 3.5 Add ledger trim tests proving Rust enforces the 4000-row `DELIVERY_LEDGER_MAX` bound and keeps newest rows by Python-compatible timestamp ordering.
- [x] 3.6 Update `docs/cutover/disk-contracts.md` with Phase 5 Rust voice write strategy, preserving the exact filenames `voice_settings.json`, `push_subscriptions.json`, `voice_delivery_ledger.json`, and `webpush_vapid_private.pem`.

## 4. Voice scan worker

- [x] 4.1 Implement scan loop interval configuration (for example `CODEX_WEB_VOICE_SCAN_SECONDS`, default matching Python’s practical cadence) and one-shot `voice_scan_once` test hook.
- [x] 4.2 Use `session_loader::load_session_rows` and existing Codex/Pi log normalizers to classify assistant messages into `narration` and `final_response` with stable message ids, text, timestamps, and session display names.
- [x] 4.3 Implement `observe_messages` parity: create pending ledger rows for new messages, skip already-ledgered ids, apply narration/final-response initial status rules, and clip preview text to Python limits.
- [x] 4.4 Implement same-slot replacement/merge behavior for queued final responses and narration tasks, including `last_error:"replaced by newer message"` for superseded pending work.
- [ ] 4.5 Add Rust tests using Codex and Pi log fixtures for new final response, disabled narration, enabled narration, duplicate rescan, replacement, malformed logs, and restart with existing ledger.
- [x] 4.6 Add contract tests proving Rust scan writes `voice_delivery_ledger.json` that Python notification message/feed endpoints can read without repair.

## 5. Listener state and announcement queue

- [x] 5.1 Implement in-memory listener registry with `LISTENER_TTL_SECONDS=45.0`, listener epoch, active listener count, queue depth, generating/prepared/playing state, and test reset hooks.
- [x] 5.2 Wire `POST /api/audio/listener` and `/api/v1/audio/listener` to the same Rust listener state used by the worker while preserving existing validation and response shape.
- [x] 5.3 Implement no-listener skip behavior: pending queued/generating/prepared tasks are marked `skipped` with `last_error:"no active listener"` when no active listener exists.
- [x] 5.4 Implement last-listener-drop cleanup: clear queue/prepared/generating/playing state, increment listener epoch, reset HLS playlist, and notify waiting worker state.
- [x] 5.5 Add tests for heartbeat enable/disable, TTL pruning, last listener drop, listener epoch stale prepared task skip, queue depth snapshot, and `/api/settings/voice.audio.active_listener_count` parity.

## 6. OpenAI-compatible summary and TTS

- [ ] 6.1 Implement OpenAI-compatible `/chat/completions` summary client with Python-equivalent system/user prompts, target word behavior, response parsing for string and content-list formats, timeout, and error mapping.
- [ ] 6.2 Implement OpenAI-compatible `/audio/speech` client posting model/voice/input/`response_format:"aac"` and returning non-empty bytes or Python-compatible errors.
- [ ] 6.3 Implement final-response processing: summary success/skipped/error, ledger `notification_text`, default push text semantics, `tts_enabled_for_final_response`, and `tts_api_key required` narration error.
- [ ] 6.4 Implement narration processing: short summary target, `From <session>.` spoken prefix, merged narration tasks, status transitions, and stale listener checks before/after HTTP calls.
- [ ] 6.5 Add mock HTTP server tests for summary success, summary HTTP error, malformed summary, empty API key, TTS success, TTS empty body, TTS HTTP error, and secret-safe logs.

## 7. HLS live stream output and serving

- [ ] 7.1 Implement Rust `MergedHlsStream` equivalent under `<app_dir>/audio`, including `segments/`, `live.m3u8`, sequence counter, segment metadata, last_error, and snapshot fields.
- [ ] 7.2 Implement append-audio path using `ffmpeg` to split AAC into MPEG-TS segments and `ffprobe` to read duration, matching Python command parameters and invalid `N/A` segment skip behavior.
- [ ] 7.3 Implement append-silence keepalive using `anullsrc`, `HLS_KEEPALIVE_SECONDS`, `HLS_SILENCE_SECONDS`, and active-listener/no-work preconditions.
- [ ] 7.4 Implement playlist rewrite with Python-compatible `#EXTM3U`, version, target duration, media sequence, `#EXTINF`, relative segment paths, rolling 18-segment cleanup, reset behavior.
- [x] 7.5 Wire `GET /api/audio/live.m3u8` and `/api/audio/segments/<segment>` (and v1 aliases if present) to Rust HLS files/state with correct content types and traversal-safe 404 behavior.
- [ ] 7.6 Add HLS tests for sequence ordering, target duration, reset, cleanup, invalid segment names, missing ffmpeg/ffprobe error, fake ffmpeg append, and optional real-ffmpeg integration selector.

## 8. WebPush and VAPID delivery

- [ ] 8.1 Implement VAPID subject selection: `CODEX_WEB_PUSH_VAPID_SUBJECT`, Tailscale HTTPS DNS fallback, and default `https://localhost`, matching Python validation rules.
- [ ] 8.2 Implement WebPush send for enabled mobile subscriptions only, with payload fields, TTL 300, VAPID `sub`, timeout, success/failure timestamp updates, and clipped last_error.
- [ ] 8.3 Preserve Python final-response push semantics: push payload notification text uses `DEFAULT_PUSH_NOTIFICATION_TEXT` where Python does, while ledger `notification_text` may contain summary/preview.
- [ ] 8.4 Implement stale subscription removal for `.invalid` endpoints and HTTP 404/410 WebPush responses.
- [ ] 8.5 Implement `POST /api/notifications/test_push` enabled path returning `sent_count`, `failed_count`, `target_count`, and `notification_text`; keep disabled path explicit with no side effect.
- [ ] 8.6 Add WebPush tests with mock sender/server for success, partial failure, all failure, 404/410 drop, `.invalid` drop, no mobile subscriptions error, payload JSON, TTL, and VAPID subject/public-key equivalence.

## 9. Debug announcement endpoint and worker loop

- [ ] 9.1 Implement Rust worker loop state machine: wait for queued task, generate summary/TTS, prepare audio, append to HLS only when no task is playing and listener epoch matches, then mark `narrated_status:"sent"`.
- [ ] 9.2 Implement playing duration gate using appended HLS duration, so prepared tasks do not overlap and queue order matches Python.
- [ ] 9.3 Implement worker error path: mark task/ledger `error`, update HLS `last_error`, clear generating state, and continue processing future tasks.
- [ ] 9.4 Implement `POST /api/audio/test_announcement` enabled path: validate API key and active listener, create `test-...` ledger row, enqueue task, return `message_id`, `queue_depth`, and `voice`.
- [ ] 9.5 Add worker loop tests for append prepared, stale prepared skip, processing error, queue ordering, playing duration, test announcement no listener, missing API key, and successful test announcement with fake TTS/HLS.

## 10. HTTP snapshots and route parity

- [x] 10.1 Update `load_voice_settings_snapshot` so `audio.queue_depth`, `active_listener_count`, `segment_count`, `last_error`, `media_sequence`, and `notifications.vapid_public_key` reflect Rust voice runtime when workers are enabled, and file-only defaults when disabled.
- [ ] 10.2 Update `load_subscriptions_snapshot`, notification message, and notification feed paths if needed so Rust runtime state and disk state remain Python-compatible after WebPush updates.
- [x] 10.3 Ensure all voice/HLS routes require auth exactly like Python except no public voice debug bypass; verify content type for JSON, HLS playlist, and MPEG-TS segment responses.
- [ ] 10.4 Add contract tests for readonly snapshots with worker disabled, snapshots with worker enabled, HLS GET 200/404, debug endpoints auth, debug disabled no side effect, and v1/legacy alias behavior.

## 11. Python compatibility and fallback checks

- [x] 11.1 Keep `codoxear/voice_push.py` and Python tests intact; do not remove Python fallback code or dependencies in `pyproject.toml` in this phase.
- [x] 11.2 Add or update Python tests proving `CODOXEAR_ENABLE_VOICE_SCAN` truthy still prevents Python `voice-push-scan` thread and falsy values preserve existing Python behavior.
- [x] 11.3 Add a rollback test or documented script: Rust writes a pending/sent/error ledger and subscription updates, then Python `VoicePushCoordinator` loads them and exposes equivalent snapshots without duplicate sends.
- [ ] 11.4 Ensure Python can load Rust-created `webpush_vapid_private.pem` via `py_vapid.Vapid.from_file` and compute the same public key.

## 12. Documentation and OpenSpec updates

- [x] 12.1 Update `docs/cutover/endpoint-inventory.md` Phase 5 section to mark `POST /api/notifications/test_push`, `POST /api/audio/test_announcement`, HLS GET routes, and voice worker ownership as implemented by `rust-backend-voice-push`.
- [x] 12.2 Update `docs/cutover/disk-contracts.md` with Phase 5 voice ownership, writer strategy, lock/rollback rules, exact filenames, and HLS artifact contract.
- [x] 12.3 Update `README.md` with Rust voice flags, ffmpeg/ffprobe requirement, WebPush/VAPID notes, Tailscale HTTPS subject behavior, and rollback steps.
- [x] 12.4 Update `openspec/changes/rust-backend-cutover/tasks.md` Phase 5 row with this change id and validation status.
- [x] 12.5 If implementation choices differ from this design (crate choice, lock behavior, ffmpeg strategy), update `openspec/changes/rust-backend-voice-push/design.md` before handoff.

## 13. Verification gate

- [ ] 13.1 `openspec validate rust-backend-voice-push --strict` passes.
- [ ] 13.2 `openspec validate rust-backend-cutover --strict` still passes.
- [ ] 13.3 `cd backend-rs && cargo fmt --all -- --check` passes.
- [ ] 13.4 `cd backend-rs && cargo clippy --all-targets -- -D warnings` passes.
- [ ] 13.5 `cd backend-rs && cargo test --release` passes, including voice worker, HLS, WebPush/VAPID, OpenAI mock, and worker flag tests.
- [ ] 13.6 `cd backend-rs && cargo build --release --bins` passes.
- [ ] 13.7 `python3 -m pytest tests/test_voice_push.py tests/test_python_worker_handoff.py -q` passes.
- [ ] 13.8 `pytest tests/contract -q -k 'voice or notification or audio'` (or the documented Phase 5 selector) passes with no skipped/xfailed Phase 5-owned cases except explicitly documented real-device smoke.
- [ ] 13.9 Line-count gate `find backend-rs/src -name '*.rs' -print0 | xargs -0 wc -l | awk '$1 > 800'` returns no source file over limit.
- [ ] 13.10 Manual or CI smoke: with `CODOXEAR_ENABLE_VOICE_SCAN=1 CODOXEAR_ENABLE_VOICE_WORKER=1`, a Rust session observes a final response, writes ledger, sends a mock or real WebPush, appends HLS audio, and rollback by unsetting flags lets Python read the same files.
- [ ] 13.11 Real-device smoke: document whether an iOS/Tailscale HTTPS device received a Rust-path notification and played HLS; if not performed, handoff must mark it pending and explain the automated substitute.
- [ ] 13.12 Independent `code-reviewer` review completes with PASS or no HIGH findings before implementation is declared ready for merge.
