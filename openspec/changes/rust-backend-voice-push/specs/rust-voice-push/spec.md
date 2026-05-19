## ADDED Requirements

### Requirement: Rust voice ownership is gated and single-writer

The Rust backend SHALL own voice scan and voice delivery side effects only when explicit environment flags are truthy. `CODOXEAR_ENABLE_VOICE_SCAN` SHALL enable Rust log scanning / ledger observation, and `CODOXEAR_ENABLE_VOICE_WORKER` SHALL enable Rust announcement generation, HLS append, WebPush dispatch, and debug side-effect endpoints. When either flag is unset or falsy (`0`, empty, `false` case-insensitive), the corresponding Rust component SHALL NOT start, SHALL NOT consume background CPU, and SHALL NOT mutate `voice_delivery_ledger.json`, `push_subscriptions.json`, or `audio/` artifacts except through already-existing explicit HTTP settings/subscription/listener routes.

#### Scenario: Rust voice flags are disabled by default
- **WHEN** the Rust server starts with `CODOXEAR_ENABLE_VOICE_SCAN` and `CODOXEAR_ENABLE_VOICE_WORKER` unset
- **THEN** no Rust voice scan task or voice worker task is spawned, `POST /api/notifications/test_push` and `POST /api/audio/test_announcement` do not return false success, and no WebPush/TTS/HLS side effect occurs in the Rust process

#### Scenario: Voice scan flag does not imply delivery worker
- **WHEN** the Rust server starts with `CODOXEAR_ENABLE_VOICE_SCAN=1` and `CODOXEAR_ENABLE_VOICE_WORKER` unset
- **THEN** Rust may observe assistant messages and update `voice_delivery_ledger.json`, but it MUST NOT call OpenAI TTS, write HLS segments, or send WebPush requests

#### Scenario: Voice worker flag requires scan ownership guard
- **WHEN** `CODOXEAR_ENABLE_VOICE_WORKER=1` is set without `CODOXEAR_ENABLE_VOICE_SCAN=1`
- **THEN** the server fails startup with a clear error or starts the worker in an explicit drain-only mode documented in logs; it MUST NOT silently let Python and Rust both write the same ledger

#### Scenario: Python and Rust are not co-active voice writers
- **WHEN** Python `codoxear-server` starts with `CODOXEAR_ENABLE_VOICE_SCAN=1`
- **THEN** Python does not start thread `voice-push-scan`, and Rust is the only process allowed to scan logs into `voice_delivery_ledger.json`

#### Scenario: Rollback to Python voice worker
- **WHEN** an operator unsets `CODOXEAR_ENABLE_VOICE_SCAN` and `CODOXEAR_ENABLE_VOICE_WORKER`, stops the Rust server, and starts Python `codoxear-server`
- **THEN** Python `VoicePushCoordinator` loads the same `voice_delivery_ledger.json`, `push_subscriptions.json`, `voice_settings.json`, and `webpush_vapid_private.pem` without repair and resumes without duplicate notifications for already-delivered messages

### Requirement: Rust preserves voice disk file contracts

The Rust voice implementation SHALL read and write the live file names `voice_settings.json`, `push_subscriptions.json`, `voice_delivery_ledger.json`, and `webpush_vapid_private.pem` under `~/.local/share/codoxear`. JSON files SHALL preserve Python schemas, `sort_keys=True`-equivalent deterministic ordering, two-space pretty formatting, trailing newline, same-directory temp file, atomic rename, and normal JSON file modes. Ledger rows SHALL preserve the Python fields `message_id`, `session_id`, `session_display_name`, `message_class`, `preview_text`, `notification_text`, `summary_text`, `summary_status`, `narrated_status`, `push_status`, `voice`, `created_ts`, `updated_ts`, and `last_error`.

#### Scenario: Rust reads Python voice files
- **WHEN** Python has written populated `voice_settings.json`, `push_subscriptions.json`, and `voice_delivery_ledger.json`
- **THEN** Rust settings/subscription/feed/message endpoints and the Rust voice worker load the same settings, subscriptions, and ledger rows without schema repair or dropped valid records

#### Scenario: Python reads Rust voice files
- **WHEN** Rust updates settings, subscription success/failure timestamps, or ledger status fields
- **THEN** Python `VoicePushCoordinator._load_settings`, `_load_subscriptions`, and `_load_delivery_ledger` accept the files and expose equivalent snapshots

#### Scenario: Actual filenames are enforced
- **WHEN** Phase 5 voice tests inspect the app directory after Rust voice side effects
- **THEN** the only persistent voice JSON filenames are `voice_settings.json`, `push_subscriptions.json`, and `voice_delivery_ledger.json`; names such as `push_ledger.json` or `audio_announcement_queue.json` are not created

#### Scenario: Ledger trim bound is preserved
- **WHEN** Rust ledger ownership would grow `voice_delivery_ledger.json` beyond `DELIVERY_LEDGER_MAX` entries
- **THEN** Rust trims the oldest rows by `updated_ts` / `created_ts` using the same 4000-row bound as Python, and the resulting file remains Python-readable

### Requirement: Rust voice scan observes assistant messages with Python delivery semantics

The Rust voice scan SHALL classify assistant messages into `narration` and `final_response` using the same message ids, timestamps, text compaction, final-turn detection, and per-session display names exposed by the existing Rust log normalizers. It SHALL create one ledger row per newly observed message id, skip already-ledgered messages, and apply the same initial status rules as Python: final responses are push-pending and summary-pending; narration is only summary/narration-pending when `tts_enabled_for_narration` is true.

#### Scenario: New final response creates pending ledger row
- **WHEN** a Rust-owned voice scan sees a final assistant response not present in `voice_delivery_ledger.json`
- **THEN** it writes a ledger row with `message_class:"final_response"`, clipped `preview_text`, `summary_status:"pending"`, `narrated_status:"pending"` or `skipped` according to settings, and `push_status:"pending"`

#### Scenario: Narration obeys narration setting
- **WHEN** `tts_enabled_for_narration` is false and Rust observes a non-final narration message
- **THEN** Rust writes or leaves the ledger with `summary_status:"skipped"`, `narrated_status:"skipped"`, and `push_status:"skipped"`, matching Python

#### Scenario: Existing ledger rows are not duplicated
- **WHEN** the same message id is observed again after restart or log rescan
- **THEN** Rust does not enqueue another announcement and does not reset sent/error/skipped statuses in the existing ledger row

#### Scenario: Newer final response replaces older queued same-slot response
- **WHEN** two final responses for the same session/slot are observed before the older one is narrated
- **THEN** Rust marks the older pending narration as `skipped` with `last_error:"replaced by newer message"` and only keeps the newer announcement queued, matching Python replacement semantics

### Requirement: Rust voice worker produces OpenAI-compatible summary and TTS output

When Rust owns voice delivery, it SHALL call OpenAI-compatible `/chat/completions` for summaries and `/audio/speech` for AAC audio using `tts_base_url`, `tts_api_key`, `summarization_model`, and `tts_model` from `voice_settings.json`. It SHALL preserve Python prompt intent, target word counts, timeout/error handling, ledger status transitions, and final-response/narration enablement. It SHALL never log or expose `tts_api_key`.

#### Scenario: Final response summary succeeds
- **WHEN** a final response is processed with a valid API key and the summary endpoint returns text
- **THEN** Rust records `summary_status:"sent"`, stores compact `summary_text`, sets clipped `notification_text`, and uses that summary as the basis for notification/HLS narration exactly as Python does

#### Scenario: Summary failure is recorded
- **WHEN** the summary endpoint returns an HTTP error or malformed response
- **THEN** Rust records `summary_status:"error"`, clips `last_error` to the Python limit, sets final-response narration to `error` or `skipped` according to settings, and continues without crashing the worker

#### Scenario: TTS is skipped without API key
- **WHEN** `tts_enabled_for_final_response` is true but `tts_api_key` is empty
- **THEN** Rust records `narrated_status:"error"` and `last_error:"tts_api_key is required"`, and does not call `/audio/speech`

#### Scenario: Narration task summary target is preserved
- **WHEN** narration is enabled for intermediate assistant messages
- **THEN** Rust summarizes to the short narration target, prefixes spoken text with `From <session_display_name>.`, and merges queued narration tasks for the same session/slot like Python

### Requirement: Rust HLS artifacts match Python live audio contract

The Rust voice worker SHALL maintain the merged live HLS stream under `<app_dir>/audio/`, serving playlist bytes at `/api/audio/live.m3u8` and MPEG-TS segments at `/api/audio/segments/<segment>`. Segment names, sequence ordering, playlist format, target duration, rolling cleanup, silence keepalive, and path traversal protection SHALL match Python `MergedHLSStream` behavior. Rust MAY invoke `ffmpeg` and `ffprobe` as Python does, and SHALL return clear errors when they are unavailable.

#### Scenario: Playlist format is compatible
- **WHEN** Rust has appended one or more audio segments
- **THEN** `/api/audio/live.m3u8` returns bytes beginning with `#EXTM3U`, includes `#EXT-X-VERSION:3`, `#EXT-X-TARGETDURATION:<n>`, `#EXT-X-MEDIA-SEQUENCE:<seq>`, `#EXTINF:<duration>,`, and relative `segments/<name>.ts` entries in increasing sequence order

#### Scenario: Segment names and cleanup are compatible
- **WHEN** Rust appends more than `HLS_MAX_SEGMENTS` segments
- **THEN** segment files are named `<six-digit-seq>-<message-prefix>.ts`, only the newest 18 remain referenced and present, and older segment files are removed

#### Scenario: Keepalive silence preserves live listener playback
- **WHEN** at least one listener is active and no announcement is queued, generating, prepared, or playing
- **THEN** Rust appends silence no more often than the Python keepalive interval and updates playlist/segment state without changing ledger message statuses

#### Scenario: Segment path traversal is rejected
- **WHEN** a client requests `/api/audio/segments/../secret` or a non-`.ts` segment name
- **THEN** Rust returns 404 and does not read outside `<app_dir>/audio/segments`

#### Scenario: Missing ffmpeg is surfaced
- **WHEN** `ffmpeg` or `ffprobe` is unavailable and Rust needs to append audio or silence
- **THEN** Rust records an HLS/ledger error equivalent to Python and exposes the message through `audio.last_error` in `/api/settings/voice`

### Requirement: Rust WebPush and VAPID behavior is pywebpush-compatible

The Rust implementation SHALL reuse `webpush_vapid_private.pem` or create a P-256 VAPID key compatible with Python `py_vapid`. The exposed `vapid_public_key` SHALL be base64url of the uncompressed X9.62 public point, identical to Python for the same PEM. WebPush sends SHALL preserve payload JSON fields, TTL 300, VAPID subject selection (`CODEX_WEB_PUSH_VAPID_SUBJECT`, Tailscale HTTPS fallback, default `https://localhost`), mobile/enabled subscription targeting, success/failure timestamp updates, stale endpoint/404/410 removal, and `push_status` transitions.

#### Scenario: Existing Python VAPID PEM is reused
- **WHEN** `webpush_vapid_private.pem` was created by Python
- **THEN** Rust loads it, computes the same `vapid_public_key` returned by Python, and sends VAPID claims with the same `sub` subject

#### Scenario: Rust-created VAPID PEM is Python-readable
- **WHEN** no VAPID PEM exists and Rust voice starts first
- **THEN** Rust creates `webpush_vapid_private.pem` in a format `py_vapid.Vapid.from_file` can load, and Python later computes the same public key

#### Scenario: Final response push payload matches Python
- **WHEN** Rust sends a push for a final response
- **THEN** the WebPush payload JSON contains `session_id`, `session_display_name`, `message_id`, `notification_text`, and `timestamp`, with `notification_text` using the existing default push text semantics where Python does

#### Scenario: Test push endpoint sends real WebPush when enabled
- **WHEN** `CODOXEAR_ENABLE_VOICE_WORKER=1` and an enabled mobile subscription exists
- **THEN** `POST /api/notifications/test_push` sends WebPush through the Rust path and returns `sent_count`, `failed_count`, `target_count`, and `notification_text` with Python-compatible semantics

#### Scenario: Stale subscriptions are dropped
- **WHEN** a WebPush send fails with HTTP 404 or 410, or the endpoint host is `.invalid`
- **THEN** Rust removes that subscription from `push_subscriptions.json`, records remaining failures, and updates `push_status` to `sent` if any target succeeded or `error` otherwise

### Requirement: Rust listener and announcement queue semantics match Python

The Rust voice worker SHALL track listener heartbeats with the same `client_id`, `enabled`, TTL, active listener count, listener epoch, no-listener skip, reset-on-last-listener-drop, prepared/generating/playing states, and queue depth semantics as Python. `POST /api/audio/listener` SHALL update the same in-memory listener state used by the worker; `POST /api/audio/test_announcement` SHALL enqueue a real test announcement when voice worker is enabled and an active listener exists.

#### Scenario: Listener heartbeat updates active count
- **WHEN** a client posts `{client_id:"c1", enabled:true}` and later `{client_id:"c1", enabled:false}`
- **THEN** Rust returns active listener counts equivalent to Python and `/api/settings/voice.audio.active_listener_count` reflects the same count

#### Scenario: Last listener drop clears pending voice state
- **WHEN** the active listener count drops from positive to zero while queue/generating/prepared/playing state exists
- **THEN** Rust increments listener epoch, clears queue/prepared/generating/playing state, resets HLS playlist, and marks affected tasks `skipped` with `last_error:"no active listener"`

#### Scenario: Test announcement requires active listener and API key
- **WHEN** `POST /api/audio/test_announcement` is called with no active listener or missing `tts_api_key`
- **THEN** Rust returns the same 400 error text Python returns and does not create HLS segments

#### Scenario: Test announcement enqueues and narrates
- **WHEN** voice worker is enabled, an active listener exists, and TTS settings are valid
- **THEN** Rust creates a `test-...` ledger row, enqueues it, synthesizes audio, appends to HLS, and returns `message_id`, `queue_depth`, and `voice`

### Requirement: Rust voice HTTP endpoints preserve existing API and auth contracts

The Rust backend SHALL preserve existing auth, paths, status codes, content types, and response shapes for `/api/settings/voice`, `/api/notifications/subscription`, `/api/notifications/subscription/toggle`, `/api/notifications/message`, `/api/notifications/feed`, `/api/notifications/test_push`, `/api/audio/listener`, `/api/audio/test_announcement`, `/api/audio/live.m3u8`, and `/api/audio/segments/<segment>`, with `/api/v1/...` aliases where already registered. Readonly endpoints SHALL work when workers are disabled; side-effect debug endpoints SHALL require `CODOXEAR_ENABLE_VOICE_WORKER=1` for real success.

#### Scenario: Readonly snapshots work without worker
- **WHEN** Rust voice workers are disabled
- **THEN** GET settings/subscription/message/feed endpoints still read disk snapshots and return Python-compatible responses without creating worker state

#### Scenario: HLS GET returns Python content types
- **WHEN** a client requests `/api/audio/live.m3u8` or a valid `/api/audio/segments/<segment>.ts`
- **THEN** Rust returns `application/vnd.apple.mpegurl` for the playlist and `video/mp2t` for segments, with 404 for unknown segments

#### Scenario: Debug endpoints do not bypass auth
- **WHEN** an unauthenticated client posts to `/api/notifications/test_push` or `/api/audio/test_announcement`
- **THEN** Rust applies the same auth middleware behavior as other protected voice endpoints

#### Scenario: Disabled debug endpoints remain explicit
- **WHEN** a client posts to a debug endpoint while `CODOXEAR_ENABLE_VOICE_WORKER` is falsy
- **THEN** Rust returns an explicit disabled error status and no side effect, rather than `ok:true`

### Requirement: Phase 5 tests prove Rust/Python voice compatibility

The implementation SHALL include unit, integration, contract, and parity tests that cover disk schema, VAPID key compatibility, WebPush send behavior with a mock endpoint, OpenAI summary/TTS with a mock HTTP server, HLS playlist/segment artifacts, listener lifecycle, worker flag gating, rollback, and existing Python oracle cases from `tests/test_voice_push.py` where practical.

#### Scenario: Voice parity tests pass
- **WHEN** Phase 5 verification runs
- **THEN** `cargo test --release` includes Rust voice worker/HLS/WebPush tests, Python `tests/test_voice_push.py` still passes, and contract tests include Rust-write→Python-read and Python-write→Rust-read checks for all voice files

#### Scenario: VAPID equivalence test passes
- **WHEN** the same VAPID PEM is loaded by Python and Rust in a test
- **THEN** both compute the same public key string and the Rust-generated authorization headers are accepted by the test WebPush receiver or fixture verifier

#### Scenario: Real-device smoke is documented
- **WHEN** coding-agent hands off Phase 5 implementation
- **THEN** the handoff includes either a real iOS/Tailscale HTTPS notification smoke result through Rust or an explicit note that this manual smoke is pending, while automated mock WebPush tests are green
