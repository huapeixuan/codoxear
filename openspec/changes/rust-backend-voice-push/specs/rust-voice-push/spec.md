## ADDED Requirements

### Requirement: Rust voice backend selection
The system SHALL support selecting the Rust voice push worker through an explicit environment flag while preserving the Python voice push worker as the default and rollback path.

#### Scenario: Default startup uses Python voice worker
- **WHEN** the server starts without the Rust voice env flag enabled
- **THEN** the server MUST initialize the existing Python voice worker
- **AND** it MUST NOT start the Rust voice worker for the same app directory

#### Scenario: Rust flag starts exactly one Rust voice worker
- **WHEN** the server starts with the Rust voice env flag enabled
- **THEN** the server MUST initialize the Rust voice worker
- **AND** it MUST NOT initialize the Python voice worker for the same app directory
- **AND** logs or diagnostics MUST identify Rust as the active voice backend

#### Scenario: Rollback returns to Python worker
- **WHEN** the Rust voice env flag is disabled after Rust voice worker has written state files
- **THEN** the Python voice worker MUST be able to start using the same app directory
- **AND** it MUST read `voice_settings.json`, `push_subscriptions.json`, and `voice_delivery_ledger.json` without migration

### Requirement: Voice settings disk compatibility
The Rust voice worker SHALL read, normalize, and persist voice settings using the existing `voice_settings.json` file and the same field semantics as the Python implementation.

#### Scenario: Missing settings file uses defaults
- **WHEN** `voice_settings.json` does not exist
- **THEN** the Rust voice worker MUST use default values for narration enablement, final-response enablement, TTS base URL, TTS API key, summarization model, and TTS model
- **AND** settings snapshots MUST include the same public fields returned by the existing `/api/settings/voice` endpoint

#### Scenario: Invalid base URL is rejected on update
- **WHEN** a settings update contains a `tts_base_url` that does not start with `http://` or `https://`
- **THEN** the Rust voice worker MUST reject the update with a validation error
- **AND** it MUST NOT corrupt the existing `voice_settings.json`

#### Scenario: Settings save is atomic and rollback-compatible
- **WHEN** settings are updated successfully through the Rust voice worker
- **THEN** the worker MUST write `voice_settings.json` by atomic replacement
- **AND** the JSON MUST be readable by the Python voice worker

### Requirement: Push subscription disk compatibility
The Rust voice worker SHALL read, normalize, update, and persist push subscriptions using the existing `push_subscriptions.json` file and subscription identity semantics.

#### Scenario: Subscription upsert preserves endpoint-derived identity
- **WHEN** a browser subscription with `endpoint`, `keys.p256dh`, and `keys.auth` is upserted
- **THEN** the Rust voice worker MUST compute the same subscription id as Python by hashing the endpoint and taking the first 24 hex characters
- **AND** the stored record MUST include `subscription`, `notifications_enabled`, timestamps, last success/failure fields, user agent, device label, and device class

#### Scenario: Invalid subscription is rejected
- **WHEN** a subscription is missing `endpoint`, `keys.p256dh`, or `keys.auth`
- **THEN** the Rust voice worker MUST reject it with a validation error
- **AND** it MUST NOT write an invalid record to `push_subscriptions.json`

#### Scenario: Stale endpoint is dropped after failed push
- **WHEN** sending a push notification to a stored endpoint whose host ends with `.invalid` fails
- **THEN** the Rust voice worker MUST remove that subscription from `push_subscriptions.json`

#### Scenario: Toggle unknown subscription fails safely
- **WHEN** a toggle request references an endpoint that is not in `push_subscriptions.json`
- **THEN** the Rust voice worker MUST return an unknown-subscription error
- **AND** it MUST NOT change any stored subscription

### Requirement: Delivery ledger compatibility
The Rust voice worker SHALL maintain delivery state in `voice_delivery_ledger.json` with the existing field names, status values, trimming behavior, and final-response/narration semantics.

#### Scenario: Final response creates compatible ledger row
- **WHEN** a new final assistant response is observed
- **THEN** the Rust voice worker MUST create a ledger row keyed by message id
- **AND** the row MUST include `message_id`, `session_id`, `session_display_name`, `message_class`, `preview_text`, `notification_text`, `summary_text`, `summary_status`, `narrated_status`, `push_status`, `voice`, `created_ts`, `updated_ts`, and `last_error`

#### Scenario: Narration without listener is skipped
- **WHEN** narration TTS is enabled and a narration message is observed while no active listener exists
- **THEN** the Rust voice worker MUST mark narration as skipped with `last_error` set to `no active listener`
- **AND** it MUST NOT append audio to HLS

#### Scenario: Newer final response replaces queued older final response
- **WHEN** a final response for a session/message class is queued and a newer final response for the same slot arrives before playback
- **THEN** the older task MUST be marked skipped/replaced in `voice_delivery_ledger.json`
- **AND** only the newer task MUST remain eligible for voice playback

#### Scenario: Ledger is trimmed to maximum size
- **WHEN** the delivery ledger exceeds the configured maximum row count
- **THEN** the Rust voice worker MUST remove the oldest rows by update/create timestamp
- **AND** it MUST preserve the newest rows in `voice_delivery_ledger.json`

### Requirement: HLS audio artifact compatibility
The Rust voice worker SHALL produce the same HLS artifact contract as the Python worker under the app directory `audio/` tree.

#### Scenario: Live playlist path is stable
- **WHEN** a client requests the live audio playlist
- **THEN** the Rust voice worker MUST serve bytes for `audio/live.m3u8`
- **AND** the playlist MUST be a valid live HLS media playlist containing `#EXTM3U`, target duration, media sequence, and segment references in `segments/<name>.ts` form

#### Scenario: Segment access is path-safe
- **WHEN** a client requests an audio segment
- **THEN** the Rust voice worker MUST only allow basename `.ts` segment names under `audio/segments/`
- **AND** it MUST reject traversal paths or non-`.ts` names as not found

#### Scenario: Segment retention keeps playlist bounded
- **WHEN** more than the configured maximum number of HLS segments have been appended
- **THEN** the Rust voice worker MUST keep only the newest segment entries in `live.m3u8`
- **AND** it MUST remove expired segment artifacts when safe

#### Scenario: No ffmpeg is reported without corrupting playlist
- **WHEN** `ffmpeg` or `ffprobe` is unavailable during audio append
- **THEN** the Rust voice worker MUST report a diagnostic HLS error
- **AND** it MUST NOT write a malformed playlist

### Requirement: WebPush and VAPID equivalence
The Rust voice worker SHALL implement WebPush/VAPID behavior that is equivalent to the Python `py_vapid` and `pywebpush` behavior used by the current worker.

#### Scenario: Existing VAPID PEM yields same public key
- **WHEN** Rust reads an existing `webpush_vapid_private.pem`
- **THEN** it MUST derive the same URL-safe base64 no-padding public key as Python derives from the uncompressed X9.62 public point
- **AND** `/api/settings/voice` and `/api/notifications/subscription` snapshots MUST expose that key as `vapid_public_key`

#### Scenario: Missing VAPID PEM is generated compatibly
- **WHEN** no VAPID private key file exists
- **THEN** Rust MUST generate a P-256 ECDSA VAPID private key at `webpush_vapid_private.pem`
- **AND** the generated key MUST be loadable by the Python worker during rollback

#### Scenario: Push payload preserves notification semantics
- **WHEN** a final response triggers push delivery
- **THEN** Rust MUST send a WebPush payload JSON containing `session_id`, `session_display_name`, `message_id`, `notification_text`, and `timestamp`
- **AND** the payload MUST preserve the existing final-response push notification text semantics

#### Scenario: Push send updates subscription and ledger state
- **WHEN** at least one enabled mobile subscription accepts a push
- **THEN** Rust MUST mark the message ledger `push_status` as `sent`
- **AND** it MUST update the subscription `last_success_ts`, clear `last_error`, and update `updated_ts`

#### Scenario: Push send failure updates failure metadata
- **WHEN** all enabled mobile push sends fail
- **THEN** Rust MUST mark the message ledger `push_status` as `error`
- **AND** each failed subscription MUST record `last_failure_ts`, clipped `last_error`, and updated timestamp unless it is dropped as stale

### Requirement: HTTP API compatibility under Rust worker
The server SHALL expose the same authenticated HTTP API behavior when the Rust voice worker is active as when the Python voice worker is active.

#### Scenario: Voice settings endpoints remain authenticated
- **WHEN** an unauthenticated client calls `GET /api/settings/voice` or `POST /api/settings/voice`
- **THEN** the server MUST reject the request using the existing authentication gate

#### Scenario: Subscription endpoints retain response shape
- **WHEN** an authenticated client calls subscription get/upsert/toggle endpoints while Rust voice worker is active
- **THEN** the server MUST return JSON fields compatible with the existing frontend API types

#### Scenario: Notification feed remains stable
- **WHEN** an authenticated client calls `GET /api/notifications/feed?since=<timestamp>`
- **THEN** the server MUST return final-response feed items ordered by update timestamp
- **AND** each item MUST include `message_id`, `session_id`, `session_display_name`, `notification_text`, and `updated_ts`

#### Scenario: HLS routes retain content types
- **WHEN** an authenticated client requests `/api/audio/live.m3u8`
- **THEN** the server MUST return `Content-Type: application/vnd.apple.mpegurl`
- **WHEN** an authenticated client requests an existing `/api/audio/segments/<name>.ts`
- **THEN** the server MUST return `Content-Type: video/mp2t`

### Requirement: TTS and summary processing compatibility
The Rust voice worker SHALL preserve the existing summary/TTS behavior for narration and final response announcements.

#### Scenario: Final response summary target is 30 words
- **WHEN** a final response is summarized with an API key configured
- **THEN** Rust MUST request a spoken mobile notification summary with the existing roughly 24 to 36 word target semantics
- **AND** the spoken TTS text MUST use `Turn summary from <session>. <summary>`

#### Scenario: Narration summary target is 15 words
- **WHEN** narration TTS is enabled and a narration task is processed
- **THEN** Rust MUST request a spoken mobile notification summary with the existing roughly 12 to 18 word target semantics
- **AND** the spoken TTS text MUST use `From <session>. <summary>`

#### Scenario: Missing API key skips or errors consistently
- **WHEN** no TTS API key is configured and a final response is observed
- **THEN** Rust MUST still populate notification text from the raw final response
- **AND** final-response narration MUST be marked error if TTS playback is enabled, or skipped if it is disabled

### Requirement: Observability and failure diagnostics
The Rust voice worker SHALL expose failures through the same user-visible state surfaces as the Python worker.

#### Scenario: HLS failure appears in settings snapshot
- **WHEN** HLS generation fails
- **THEN** `settings_snapshot` MUST include the HLS `last_error` in the `audio` object

#### Scenario: Task failure updates ledger
- **WHEN** summary, TTS, HLS append, or push processing fails for a task
- **THEN** Rust MUST update the corresponding ledger row statuses and clipped `last_error`
- **AND** the worker MUST continue processing later tasks when possible
