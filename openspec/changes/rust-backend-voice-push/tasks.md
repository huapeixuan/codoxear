## 0. Pre-flight and scope guard

- [ ] 0.1 Run and read `openspec status --change rust-backend-voice-push`, this change's `proposal.md`, `design.md`, and both specs before coding.
- [ ] 0.2 Re-read `docs/cutover/endpoint-inventory.md` voice rows and `docs/cutover/disk-contracts.md` voice rows; create an implementation checklist for `voice_settings.json`, `push_subscriptions.json`, `voice_delivery_ledger.json`, `webpush_vapid_private.pem`, `audio/live.m3u8`, and `audio/segments/*.ts`.
- [ ] 0.3 Re-read `codoxear/voice_push.py`, `codoxear/server.py` `_voice_push_scan_loop` / `_observe_rollout_delta`, `backend-rs/src/voice_state.rs`, `backend-rs/src/voice_post.rs`, `backend-rs/src/handlers/voice.rs`, `backend-rs/src/workers.rs`, and current Rust route registration.
- [ ] 0.4 Confirm baseline before changing code: `openspec validate rust-backend-voice-push --strict`, `openspec validate rust-backend-cutover --strict`, and focused Rust/Python voice tests currently pass or have documented Phase 3 501 expectations.
- [ ] 0.5 Keep scope limited to Phase 5 voice push / HLS / WebPush / TTS cutover; do not delete Python backend, do not remove Python voice fallback, and do not alter Phase 4 broker protocol.

## 1. Dependency and equivalence spikes

- [ ] 1.1 Spike Rust WebPush/VAPID crate options in a throwaway test/module; prove reading a Python `webpush_vapid_private.pem` yields the same base64url unpadded uncompressed P-256 public key as Python.
- [ ] 1.2 Add the chosen WebPush/VAPID/ECDSA/PEM/HTTP dependencies to `backend-rs/Cargo.toml`, documenting why they satisfy Linux/macOS and pywebpush equivalence.
- [ ] 1.3 Spike OpenAI-compatible HTTP client with fake server fixtures for `/chat/completions` and `/audio/speech`, including timeout/error response parsing.
- [ ] 1.4 Spike ffmpeg/ffprobe subprocess wrappers for AAC input, silence generation, TS segmentation, and duration extraction; verify missing binary errors match Python text.
- [ ] 1.5 Keep all new Rust source files under the existing ≤800 line gate; decide the final `voice_worker/` module split before implementing business logic.

## 2. Rust voice module skeleton and single-writer guard

- [ ] 2.1 Create `backend-rs/src/voice_worker/` modules (`mod.rs`, `locks.rs`, `settings.rs`, `subscriptions.rs`, `ledger.rs`, `messages.rs`, `hls.rs`, `openai.rs`, `webpush.rs` or equivalent) and export only testable public APIs from `lib.rs`.
- [ ] 2.2 Implement app-dir voice owner lock (`voice_worker.lock` or documented equivalent) using atomic create-new semantics, pid/role/timestamp contents, stale-owner detection if safe, and drop cleanup.
- [ ] 2.3 Modify `backend-rs/src/workers.rs::spawn_enabled_workers` to start Rust voice scan only when `CODOXEAR_ENABLE_VOICE_SCAN` is truthy and delivery/HLS/WebPush worker only when `CODOXEAR_ENABLE_VOICE_WORKER` is truthy.
- [ ] 2.4 Modify Python `VoicePushCoordinator` / `SessionManager` so `CODOXEAR_ENABLE_VOICE_SCAN` suppresses Python scan and `CODOXEAR_ENABLE_VOICE_WORKER` suppresses Python delivery/keepalive threads while leaving settings/subscription HTTP helpers usable.
- [ ] 2.5 Add Python tests proving truthy Rust voice flags prevent Python scan/delivery/keepalive thread startup and falsy/unset flags preserve existing Python fallback behavior.

## 3. Disk contract parity for settings, subscriptions, VAPID, and ledger

- [ ] 3.1 Refactor or extend Rust voice settings cleaner/writer so `voice_settings.json` output matches Python cleaned keys, pretty sorted JSON, trailing newline, atomic rename, and validation error strings.
- [ ] 3.2 Refactor or extend Rust subscription cleaner/writer so `push_subscriptions.json` output matches Python record schema, ordering by newest `updated_ts`, upsert/toggle semantics, and stale/transient failure fields.
- [ ] 3.3 Implement Rust VAPID key manager for `webpush_vapid_private.pem`: read Python PEM, generate Python-readable PEM when absent, derive public key, normalize env/default subject, and expose snapshots.
- [ ] 3.4 Implement Rust ledger cleaner/writer for `voice_delivery_ledger.json`, including all Python row fields, status strings, timestamp updates, `last_error` clipping sites, and `DELIVERY_LEDGER_MAX` trim behavior.
- [ ] 3.5 Add cross-language tests: Python-written settings/subscription/ledger/VAPID files are read by Rust; Rust-written files are read by Python `VoicePushCoordinator` without migration.

## 4. Message extraction, scan worker, and delivery offsets

- [ ] 4.1 Port or reuse delivery message extraction so Rust voice scan classifies narration vs final_response with the same dedupe behavior covered by `tests/test_voice_push.py::TestDeliveryExtraction`.
- [ ] 4.2 Implement Rust voice scan loop: discover sessions, refresh log path via existing session loader, read JSONL from stored offset in bounded chunks, observe messages, and advance offset.
- [ ] 4.3 Implement delivery offset persistence/patching compatible with current Rust/Python session state; handle truncated logs by resetting offset to 0 while still deduping by ledger message id.
- [ ] 4.4 Preserve resume-muted behavior: resumed session logs advance offsets but do not enqueue/notify historical messages.
- [ ] 4.5 Add Rust tests with fixture Codex/Pi logs covering final_response once, narration enabled/disabled, duplicate adjacent messages, memory citation dedupe, resume mute, truncated log, and offset advancement.

## 5. Announcement queue and ledger state machine

- [ ] 5.1 Implement Rust in-memory voice coordinator state: listeners with TTL/epoch, queue, generating/prepared/playing task, stable session voice mapping from `DEFAULT_VOICES`, and queue depth/audio snapshot.
- [ ] 5.2 Port Python `observe_messages` state transitions for final_response and narration, including pending ledger creation, no-listener skip, listener epoch drift, replace-by-newer-message, and narration merge.
- [ ] 5.3 Implement final_response preparation: optional summary, fixed push text, notification text clipping, summary failure handling, TTS enable/disable handling, and ledger field updates.
- [ ] 5.4 Implement narration preparation: 15-word summary when enabled, spoken text composition, no-listener stale skip, and task error handling.
- [ ] 5.5 Add Rust unit tests mirroring the Python voice coordinator tests for final response summary/enqueue, narration summary, no-listener skip, summary failure fixed push, stable voice mapping, and latest narration merge.

## 6. OpenAI-compatible client

- [ ] 6.1 Implement Rust `OpenAICompatibleClient::summarize` with the exact `/chat/completions` route, headers, JSON body, response content parsing, and Python-compatible error strings.
- [ ] 6.2 Implement Rust `OpenAICompatibleClient::synthesize` with the exact `/audio/speech` route, headers, JSON body (`response_format:"aac"`), empty-body handling, and timeout.
- [ ] 6.3 Ensure HTTP requests do not block unrelated HTTP routes: use async client or `spawn_blocking`/dedicated worker thread appropriately.
- [ ] 6.4 Add fake HTTP server tests for successful summary/TTS, string and content-part responses, HTTP error detail propagation, empty audio, missing API key, and timeout/error ledger mapping.

## 7. HLS stream implementation and byte-serving routes

- [ ] 7.1 Implement Rust `MergedHlsStream` equivalent for `<app_dir>/audio/live.m3u8` and `<app_dir>/audio/segments/`, initializing/rebuilding playlist on startup.
- [ ] 7.2 Implement `append_audio` using ffmpeg segment command compatible with Python, temp AAC/chunk cleanup, ffprobe duration extraction, invalid `N/A` chunk discard, and total-duration error behavior.
- [ ] 7.3 Implement `append_silence` using `anullsrc=r=24000:cl=mono`, 6s keepalive throttle, AAC bitrate compatible with Python, duration extraction, and playlist rewrite.
- [ ] 7.4 Implement segment reserve/store/window cleanup: names `%06d-<prefix[:12]>.ts`, max 18 tracked segments, delete only tracked stale files, update `last_error`, `media_sequence`, and `segment_count`.
- [ ] 7.5 Implement Rust `GET /api/audio/live.m3u8` and `GET /api/audio/segments/{segment}` for both `/api/v1` and legacy `/api`, with auth, path traversal rejection, `Content-Type`, `Content-Length`, `Cache-Control`, `Pragma`, and `Expires` matching Python.
- [ ] 7.6 Add Rust integration tests for empty playlist, audio append playlist body, segment serving headers, missing segment 404, path traversal 404, max-window cleanup, and missing ffmpeg/ffprobe last_error behavior.

## 8. WebPush delivery and debug push endpoint

- [ ] 8.1 Implement Rust WebPush send with existing subscription JSON (`endpoint`, `keys.p256dh`, `keys.auth`), VAPID subject, TTL 300, JSON payload fields, timeout 10s, and no prompt/text secret logging.
- [ ] 8.2 Implement stale endpoint/error classification equivalent to Python `_is_stale_push_subscription_endpoint`, HTTP 404/410 handling, subscription drop, and transient error retention.
- [ ] 8.3 Implement final_response push ledger updates: `push_status=skipped` with no enabled mobile subscriptions, `sent` if any succeeds, `error` if targets exist but all fail.
- [ ] 8.4 Replace Rust Phase 3 `POST /api/notifications/test_push` 501 handler with real behavior and Python-compatible response/errors.
- [ ] 8.5 Add tests for enabled mobile target success, stale subscription drop, transient error retention, all-fail ledger status, no enabled mobile 400, and payload JSON fields.

## 9. Test announcement endpoint and listener route parity

- [ ] 9.1 Reconcile Rust `POST /api/audio/listener` with Python listener heartbeat semantics: client_id validation, enabled toggle, listener TTL/epoch, active listener count, and settings snapshot effects.
- [ ] 9.2 Replace Rust Phase 3 `POST /api/audio/test_announcement` 501 handler with Python-compatible test ledger row creation, queueing, response fields, and TTS setting validation errors.
- [ ] 9.3 Add tests for listener enable/disable, listener expiry/prune, test announcement success with fake TTS/HLS, missing `tts_base_url`, missing `tts_api_key`, and no-listener skip behavior.

## 10. Contract tests and documentation

- [ ] 10.1 Extend `tests/contract` with voice state parity: settings save/read, subscription upsert/toggle/read, notification message/feed, and Rust-written ledger read by Python.
- [ ] 10.2 Extend `tests/contract` with HLS artifact parity using fake audio/ffmpeg fixtures or prebuilt segment fixtures: playlist bytes, segment bytes/headers, and path traversal.
- [ ] 10.3 Extend `tests/contract` with WebPush/VAPID equivalence tests: public key equality, Rust-generated PEM Python readability, and fake push service request assertions.
- [ ] 10.4 Update `README.md` with Phase 5 env flags, prerequisites (`ffmpeg`, `ffprobe`, Rust voice binary/server), Tailscale HTTPS/mobile WebPush setup, and rollback steps.
- [ ] 10.5 Update `docs/cutover/disk-contracts.md` with Rust Phase 5 owner/write strategy, lock file name, and explicit statement that `voice_settings.json`, `push_subscriptions.json`, and `voice_delivery_ledger.json` remain the live filenames.
- [ ] 10.6 Update `docs/cutover/endpoint-inventory.md` footer to note that `POST /api/notifications/test_push`, `POST /api/audio/test_announcement`, and HLS byte-serving routes are implemented in Phase 5 Rust.

## 11. Verification gate and handoff

- [ ] 11.1 `openspec validate rust-backend-voice-push --strict` passes after implementation updates.
- [ ] 11.2 `openspec validate rust-backend-cutover --strict` still passes.
- [ ] 11.3 `cd backend-rs && cargo fmt --all -- --check` passes.
- [ ] 11.4 `cd backend-rs && cargo clippy --all-targets -- -D warnings` passes.
- [ ] 11.5 `cd backend-rs && cargo test --release voice` or the final documented Rust voice selector passes.
- [ ] 11.6 `cd backend-rs && cargo test --release` passes unless a documented external dependency skip is approved.
- [ ] 11.7 `python3 -m pytest tests/test_voice_push.py tests/test_voice_push_source.py -q` passes, proving Python fallback remains intact.
- [ ] 11.8 `python3 -m pytest tests/contract -q -k 'voice or notification or audio'` passes with no unexpected skips for Phase 5-owned cases.
- [ ] 11.9 Line-count gate `find backend-rs/src -name '*.rs' -print0 | xargs -0 wc -l | awk '$1 > 800'` returns no source file over 800 lines.
- [ ] 11.10 Smoke with fake services: `CODOXEAR_ENABLE_VOICE_SCAN=1 CODOXEAR_ENABLE_VOICE_WORKER=1` observes a fixture session log, writes ledger, generates HLS, sends fake WebPush, and rollback to Python reads the same files.
- [ ] 11.11 Real-device smoke: on Tailscale HTTPS with a mobile subscription and OpenAI-compatible TTS key, Rust path sends a WebPush notification and produces playable HLS; record environment, commands, and result in handoff.
- [ ] 11.12 Before declaring ready, request independent `code-reviewer` review; handoff must include Rust build/test results, voice ledger/subscription/HLS contract test results, WebPush/VAPID equivalence evidence, real-device smoke status, and rollback verification.
