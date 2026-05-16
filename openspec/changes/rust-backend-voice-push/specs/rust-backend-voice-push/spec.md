## ADDED Requirements

### Requirement: Rust voice workers are opt-in and single-writer

The system SHALL provide Rust voice scan and Rust voice delivery workers gated by `CODOXEAR_ENABLE_VOICE_SCAN` and `CODOXEAR_ENABLE_VOICE_WORKER`, both default off. When either Rust voice worker is enabled, the system MUST ensure Python voice scan/delivery workers are not concurrently writing the same `voice_delivery_ledger.json` or `audio/` HLS output.

#### Scenario: Voice workers disabled by default

- **WHEN** the Rust server starts with `CODOXEAR_ENABLE_VOICE_SCAN` and `CODOXEAR_ENABLE_VOICE_WORKER` unset, empty, `0`, or `false`
- **THEN** no Rust voice scan loop, delivery loop, HLS keepalive loop, WebPush request, OpenAI-compatible TTS request, `voice_delivery_ledger.json` write, or `audio/` write is performed by the Rust process

#### Scenario: Rust voice owner blocks Python worker co-activity

- **WHEN** `CODOXEAR_ENABLE_VOICE_WORKER=1` is set and a Python `codoxear-server` is started in the same app dir for fallback HTTP compatibility
- **THEN** the Python process MUST NOT start `VoicePushCoordinator` delivery/keepalive threads that write `voice_delivery_ledger.json` or `audio/`, while its settings/subscription HTTP state readers remain usable

#### Scenario: Voice lock conflict fails closed

- **WHEN** a Rust voice worker starts while an existing live voice owner lock for the same app dir is held by another process
- **THEN** the Rust worker SHALL fail closed by not starting voice side effects, logging the conflict, and leaving existing `voice_delivery_ledger.json`, `push_subscriptions.json`, and `audio/` artifacts unchanged

#### Scenario: Rollback to Python voice worker

- **WHEN** Rust voice workers have written ledger/subscription/HLS artifacts, then the operator unsets `CODOXEAR_ENABLE_VOICE_SCAN` and `CODOXEAR_ENABLE_VOICE_WORKER` and starts Python `codoxear-server`
- **THEN** Python SHALL read the existing `voice_settings.json`, `push_subscriptions.json`, and `voice_delivery_ledger.json` without migration and SHALL NOT duplicate notifications for message ids already present in the ledger

### Requirement: Voice disk contracts remain byte-compatible

The Rust implementation SHALL read and write the existing voice disk files with the exact live filenames `voice_settings.json`, `push_subscriptions.json`, and `voice_delivery_ledger.json`. JSON writes MUST be pretty formatted with sorted keys and trailing newline in a way compatible with Python `json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n"`, and updates MUST use same-directory temp file plus atomic rename.

#### Scenario: Settings filename and schema are preserved

- **WHEN** `POST /api/settings/voice` is handled by Rust with a valid settings object
- **THEN** Rust writes `<app_dir>/voice_settings.json` with keys `tts_enabled_for_narration`, `tts_enabled_for_final_response`, `tts_base_url`, `tts_api_key`, `summarization_model`, and `tts_model`, and Python `VoicePushCoordinator._load_settings` can read it without data loss

#### Scenario: Subscription filename and schema are preserved

- **WHEN** Rust upserts or toggles a WebPush subscription
- **THEN** Rust writes `<app_dir>/push_subscriptions.json` as an array of records containing `id`, `subscription.endpoint`, `subscription.keys.p256dh`, `subscription.keys.auth`, `notifications_enabled`, `created_ts`, `updated_ts`, `last_success_ts`, `last_failure_ts`, `last_error`, `user_agent`, `device_label`, and `device_class`, and Python can read/toggle the same record by endpoint

#### Scenario: Ledger filename and status schema are preserved

- **WHEN** Rust observes, summarizes, narrates, pushes, skips, replaces, or errors a voice message
- **THEN** Rust writes `<app_dir>/voice_delivery_ledger.json` as an object keyed by message id, with row fields `message_id`, `session_id`, `session_display_name`, `message_class`, `preview_text`, `notification_text`, `summary_text`, `summary_status`, `narrated_status`, `push_status`, `voice`, `created_ts`, `updated_ts`, and `last_error`, using the same status strings as Python (`pending`, `sent`, `skipped`, `error`)

#### Scenario: Ledger trim is compatible

- **WHEN** the number of ledger records exceeds Python `DELIVERY_LEDGER_MAX`
- **THEN** Rust SHALL remove the oldest records by `updated_ts` falling back to `created_ts`, matching Python trim behavior, before writing the ledger

### Requirement: Rust voice scan observes backend logs without changing broker protocol

The Rust voice scan worker SHALL discover live sessions from existing Rust/Python broker sidecars and scan their session logs for assistant delivery messages using the same classification/deduplication semantics as Python `_rollout_log._extract_delivery_messages` and `VoicePushCoordinator.observe_messages`, without adding new broker socket commands.

#### Scenario: Final response observed once

- **WHEN** a session log gains a final assistant response and `CODOXEAR_ENABLE_VOICE_SCAN=1` is active
- **THEN** Rust SHALL create exactly one `final_response` ledger row for that message id, set `summary_status`, `narrated_status`, and `push_status` to `pending` or `skipped` according to current settings, and advance the delivery offset so later scans do not re-observe the same bytes

#### Scenario: Resume sessions are muted

- **WHEN** the session sidecar indicates a resumed session that Python would treat as `resume_muted`
- **THEN** Rust voice scan SHALL advance delivery offsets but MUST NOT enqueue announcements or send WebPush/TTS for historical messages from the resumed log

#### Scenario: Truncated log resets offset safely

- **WHEN** a session log size becomes smaller than the stored delivery offset
- **THEN** Rust SHALL restart scanning from offset 0, but MUST still skip any message ids already present in `voice_delivery_ledger.json`

#### Scenario: Narration merge and replacement match Python

- **WHEN** multiple narration messages for the same session are queued before playback and narration is enabled
- **THEN** Rust SHALL merge pending narration text for the same `(session_id, narration)` slot, while newer final_response tasks for the same slot replace older pending tasks with `last_error="replaced by newer message"` as Python does

### Requirement: Rust OpenAI-compatible summary and TTS match Python boundaries

The Rust voice delivery worker SHALL call only the OpenAI-compatible endpoints currently used by Python: `POST /chat/completions` for summary and `POST /audio/speech` for AAC TTS. Request payloads, timeout, response parsing, and error-to-ledger mapping MUST match Python behavior.

#### Scenario: Summary request parity

- **WHEN** Rust summarizes a narration or final response with `tts_api_key` configured
- **THEN** it sends the same model, temperature `0.2`, `max_completion_tokens: 90`, system prompt, and user content shape as Python, and accepts either string `choices[0].message.content` or text/output_text content parts

#### Scenario: TTS request parity

- **WHEN** Rust synthesizes speech for an announcement
- **THEN** it sends `POST {tts_base_url}/audio/speech` with JSON fields `model`, `voice`, `input`, and `response_format:"aac"`, uses the configured `tts_model` and stable session voice mapping, and treats an empty body as `audio/speech returned empty body`

#### Scenario: Missing API key is not retried

- **WHEN** final-response narration is enabled but `tts_api_key` is empty
- **THEN** Rust SHALL set `narrated_status="error"` and `last_error="tts_api_key is required"` for the message, without retrying or calling the network

#### Scenario: Summary failure still allows fixed push text

- **WHEN** final response summarization fails but a mobile push subscription is enabled
- **THEN** Rust SHALL record `summary_status="error"` and clipped `last_error`, SHALL NOT enqueue final-response TTS, and SHALL still attempt WebPush using the fixed default push notification text like Python

### Requirement: Rust HLS artifacts match Python playlist and segment contracts

The Rust voice delivery worker SHALL write HLS artifacts under `<app_dir>/audio/` with the same playlist syntax, segment naming, ffmpeg/ffprobe behavior, no-store serving semantics, and cleanup policy as Python `MergedHLSStream`.

#### Scenario: Empty playlist is valid

- **WHEN** no audio segment has been appended and `GET /api/audio/live.m3u8` is requested
- **THEN** Rust returns HTTP 200 with `Content-Type: application/vnd.apple.mpegurl`, `Cache-Control: no-store`, and a playlist body containing at least `#EXTM3U` plus current target-duration/media-sequence lines compatible with Python

#### Scenario: Audio append creates numbered TS segments

- **WHEN** Rust appends AAC bytes for message id `abcdef1234567890`
- **THEN** it writes one or more TS segments named like `%06d-abcdef123456.ts` under `<app_dir>/audio/segments/`, rewrites `live.m3u8` with `#EXT-X-VERSION:3`, `#EXT-X-TARGETDURATION`, `#EXT-X-MEDIA-SEQUENCE`, `#EXTINF:<duration>,`, and `segments/<name>.ts` entries, and updates ledger `narrated_status="sent"` after successful append

#### Scenario: Segment window cleanup matches Python

- **WHEN** appending a segment would make the tracked segment list longer than 18
- **THEN** Rust removes the oldest tracked segment file and keeps the playlist window at at most 18 segments, without deleting unrelated files in `audio/segments/`

#### Scenario: Path traversal is rejected

- **WHEN** `GET /api/audio/segments/../voice_delivery_ledger.json` or a non-`.ts` segment path is requested
- **THEN** Rust returns 404 and MUST NOT read files outside `<app_dir>/audio/segments/`

#### Scenario: Silence keepalive matches listener state

- **WHEN** at least one listener heartbeat is active, there is no queued/prepared/playing task, and no segment has been appended in the last 6 seconds
- **THEN** Rust appends a silence TS segment generated from `anullsrc=r=24000:cl=mono` with AAC bitrate compatible with Python and updates the playlist

### Requirement: Rust WebPush and VAPID are equivalent to pywebpush

The Rust implementation SHALL reuse `webpush_vapid_private.pem`, expose the same base64url unpadded public key to clients, and send WebPush payloads accepted by browser push services with VAPID subject semantics equivalent to Python `pywebpush`/`py_vapid`.

#### Scenario: Existing Python VAPID PEM is reused

- **WHEN** `webpush_vapid_private.pem` already exists and was generated by Python
- **THEN** Rust SHALL load it, derive the same uncompressed P-256 public key bytes, and return the same `vapid_public_key` in settings/subscription snapshots that Python would return

#### Scenario: Rust-generated VAPID PEM is Python-readable

- **WHEN** no VAPID PEM exists and Rust initializes voice push
- **THEN** Rust SHALL generate a P-256 private key PEM at `webpush_vapid_private.pem` such that Python `Vapid.from_file` can read it and derive a public key for existing frontend subscription flows

#### Scenario: Final-response push payload is unchanged

- **WHEN** Rust sends WebPush for a final response
- **THEN** the JSON payload contains `session_id`, `session_display_name`, `message_id`, `notification_text`, and `timestamp`, uses TTL 300, uses VAPID `sub` from the same default/env subject logic as Python, and updates `push_status` to `sent` if any enabled mobile subscription succeeds

#### Scenario: Stale subscriptions are dropped

- **WHEN** a WebPush endpoint is stale by endpoint pattern or push service 404/410-equivalent response
- **THEN** Rust SHALL mark `last_failure_ts`/`last_error`, remove the stale subscription from `push_subscriptions.json`, and continue attempting remaining subscriptions

#### Scenario: Non-stale network errors keep subscription

- **WHEN** a WebPush send fails with a transient network error that Python would not classify as stale
- **THEN** Rust SHALL keep the subscription record, set `last_failure_ts` and `last_error`, and set ledger `push_status` to `error` only if no target subscription succeeded

### Requirement: Voice debug endpoints perform real Phase 5 side effects

The Rust server SHALL implement `POST /api/notifications/test_push` and `POST /api/audio/test_announcement` with Python-compatible side effects and response shape when Phase 5 voice support is built, rather than the Phase 3 explicit 501 placeholder.

#### Scenario: Test push targets enabled mobile subscriptions

- **WHEN** an authenticated client posts to `/api/notifications/test_push` and at least one enabled mobile subscription exists
- **THEN** Rust SHALL send the fixed test notification text to enabled mobile subscriptions, update subscription success/failure fields, and return JSON with `ok`, `sent_count`, `failed_count`, `target_count`, and `notification_text`

#### Scenario: Test push with no mobile target returns bad request

- **WHEN** an authenticated client posts to `/api/notifications/test_push` and no enabled mobile subscriptions exist
- **THEN** Rust SHALL return HTTP 400 with `{"error":"no enabled mobile subscriptions"}`

#### Scenario: Test announcement queues audio

- **WHEN** an authenticated client posts to `/api/audio/test_announcement` with valid TTS settings
- **THEN** Rust SHALL create a test final_response ledger row, enqueue/prepare/append an announcement for active listeners, and return JSON containing `ok`, `message_id`, `notification_text`, and `voice` like Python

#### Scenario: Test announcement without TTS settings returns Python-compatible error

- **WHEN** an authenticated client posts to `/api/audio/test_announcement` without `tts_base_url` or `tts_api_key`
- **THEN** Rust SHALL return HTTP 400 with the same error string Python returns (`tts_base_url is required` or `tts_api_key is required`)
