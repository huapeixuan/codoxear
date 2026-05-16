## ADDED Requirements

### Requirement: Rust runtime exposes Phase 5 voice HTTP parity

The Rust HTTP runtime SHALL expose the existing voice, notification, and audio endpoints at both `/api/v1/...` and legacy `/api/...`, preserving authentication requirements, status codes, JSON response shapes, and media response headers while delegating Phase 5 side effects to the Rust voice worker when enabled.

#### Scenario: Voice settings snapshot includes live Rust audio and notification status

- **WHEN** an authenticated client sends `GET /api/settings/voice` to Rust while Rust voice worker is active
- **THEN** the response includes `ok:true`, voice settings fields, `audio.queue_depth`, `audio.active_listener_count`, `audio.stream_url`, `audio.segment_count`, `audio.last_error`, `audio.media_sequence`, `notifications.enabled_devices`, `notifications.total_devices`, and `notifications.vapid_public_key` with values from the Rust voice coordinator

#### Scenario: Subscription snapshot includes Rust VAPID public key

- **WHEN** an authenticated client sends `GET /api/notifications/subscription` to Rust
- **THEN** the response includes `ok:true`, the Rust/Python-compatible `vapid_public_key`, and `subscriptions[]` records sorted by newest `updated_ts`, with the same fields returned by Python

#### Scenario: Notification message and feed read Rust-written ledger rows

- **WHEN** Rust voice scan has written a final_response ledger row and the client calls `GET /api/notifications/message?message_id=<id>` or `GET /api/notifications/feed?since=<ts>`
- **THEN** Rust returns the same message state/feed shape as Python for that row, filtering feed items to completed/error/skipped final_response notification text records

#### Scenario: HLS playlist and segment routes serve bytes with no-store headers

- **WHEN** an authenticated client requests `GET /api/audio/live.m3u8` or `GET /api/audio/segments/<segment>.ts`
- **THEN** Rust serves the corresponding bytes from `<app_dir>/audio/` with Python-compatible `Content-Type`, `Content-Length`, `Cache-Control: no-store`, `Pragma: no-cache`, and `Expires: 0`; missing/invalid segments return 404

#### Scenario: Legacy and v1 routes are equivalent

- **WHEN** the same authenticated request is sent to `/api/audio/live.m3u8` and `/api/v1/audio/live.m3u8`, or to any `/api/notifications/*` Phase 5 route and its `/api/v1/...` counterpart
- **THEN** both responses have the same status code, body semantics, and relevant headers except for documented path/prefix differences
