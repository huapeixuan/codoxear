## ADDED Requirements

### Requirement: All 200 JSON responses use the canonical `application/json; charset=utf-8` Content-Type

The Rust backend SHALL emit `Content-Type: application/json; charset=utf-8` on every successful (status 200) JSON response, including the Phase 1 endpoints `/api/me`, `/api/v1/me`, `/api/sessions/bootstrap`, and `/api/v1/sessions/bootstrap`. The implementation SHALL route every 200 JSON path through a shared `json_response(status, value) -> Response` helper rather than depending on `axum::Json`'s default IntoResponse, so that any new handler automatically inherits the correct charset header.

#### Scenario: `/api/me` 200 response includes `charset=utf-8`

- **WHEN** an authenticated `GET /api/me` request is sent to the Rust server with a valid `codoxear_auth` cookie
- **THEN** the response status is `200`, the response header `Content-Type` equals exactly `application/json; charset=utf-8`, and the response body is the byte sequence `{"ok":true}` (no trailing newline)

#### Scenario: `/api/sessions/bootstrap` 200 response includes `charset=utf-8`

- **WHEN** an authenticated `GET /api/sessions/bootstrap` request is sent to the Rust server with a valid cookie against an empty HOME
- **THEN** the response status is `200`, the response header `Content-Type` equals exactly `application/json; charset=utf-8`, and the parsed JSON body matches the empty-state Python parity shape

#### Scenario: New 200 handler regression guard

- **WHEN** the Rust integration test `backend-rs/tests/json_response_helper.rs` (or an equivalent fixture) inspects each 200 path registered in `routes::router()`
- **THEN** every 200 response carries the exact `Content-Type: application/json; charset=utf-8` header, so that future handlers cannot regress the charset suffix unnoticed

### Requirement: `serde_json` preserves insertion order on every nested JSON object

The Rust backend SHALL build with `serde_json` configured for `preserve_order` so that nested `Map<String, Value>` fields serialize in the order keys were inserted, matching Python `dict` insertion order. Handlers that build dynamic objects (notably `BootstrapResponse.new_session_defaults.backends.codex` and `.backends.pi`, `/api/sessions` per-session entries, `/api/diagnostics`, `voice settings_snapshot`) SHALL insert keys in the same sequence the Python implementation uses, enabling byte-level body equivalence on deterministic responses.

#### Scenario: Bootstrap nested dict order matches Python

- **WHEN** an authenticated `GET /api/sessions/bootstrap` is issued against an HOME with a populated `~/.codex/config.toml` and `~/.pi/agent/settings.json`
- **THEN** the byte sequence of `new_session_defaults.backends.codex` returned by the Rust server is identical to the byte sequence of the same field returned by the Python server (verified via `assert_response_bytes_equal` in the contract harness)

#### Scenario: serde_json feature is enabled in the build

- **WHEN** `cargo metadata --format-version 1 --no-deps` is executed inside `backend-rs/`
- **THEN** the `serde_json` dependency entry contains the `preserve_order` feature

### Requirement: `cwd_groups.json` parse failures recover with a warning instead of HTTP 500

The Rust backend SHALL treat a missing, empty, malformed, or non-Object `~/.local/share/codoxear/cwd_groups.json` as an empty grouping configuration: the loader SHALL emit a single `tracing::warn!` line that includes the file path and the underlying error, return an empty map, and let the caller proceed normally. The behavior SHALL match Python `_load_cwd_groups` (`codoxear/server.py:5052-5096`).

#### Scenario: Malformed cwd_groups.json yields 200 with empty grouping

- **WHEN** `~/.local/share/codoxear/cwd_groups.json` contains the bytes `not-json` and an authenticated `GET /api/sessions/bootstrap` is issued
- **THEN** the Rust server responds `200` with `cwd_groups: {}` in the body and a single `WARN`-level log line referencing the file path; the Python server returns the same status and shape for the same input

#### Scenario: Top-level non-Object cwd_groups.json recovers gracefully

- **WHEN** `cwd_groups.json` contains `[1,2,3]`
- **THEN** the Rust server responds `200` with `cwd_groups: {}` in the body, exactly like Python

### Requirement: `normalize_cwd_group_key` performs progressive canonicalization equivalent to Python `Path.resolve(strict=False)`

The Rust backend SHALL canonicalize candidate cwd-group keys using a progressive algorithm: expand `~`, then walk the path from the longest existing prefix downward, applying `fs::canonicalize` on the existing portion and joining the remaining unresolved tail back. The result SHALL equal `Path(trimmed).expanduser().resolve(strict=False)` from Python for the same input on the same filesystem.

#### Scenario: Existing parent with non-existent child resolves the parent

- **WHEN** the host has `/tmp/exists` as a real directory and `/tmp/exists/missing` does not exist, and the input key is `/tmp/exists/missing/child`
- **THEN** the Rust normalizer returns the canonical form of `/tmp/exists` joined with `missing/child`, matching Python's `resolve(strict=False)`

#### Scenario: Symlinked parent is resolved

- **WHEN** `/tmp/link` is a symlink to `/tmp/real-dir` and the input key is `/tmp/link/child` (where `child` does not yet exist)
- **THEN** the Rust normalizer returns `/tmp/real-dir/child`, matching Python `resolve(strict=False)`

### Requirement: Bootstrap populated-state contract tests cover recent_cwds, cwd_groups, and tmux_available

The contract harness SHALL include populated-state parity tests for `/api/sessions/bootstrap` covering at minimum: (b) `recent_cwds.json` populated with multiple entries — both servers return identically sorted and truncated `recent_cwds`; (c) `cwd_groups.json` round-trip — both servers return structurally identical `cwd_groups` after a hand-crafted on-disk file is placed; (d) `tmux_available` reflects the host's actual `which tmux` lookup — both servers return the same boolean.

#### Scenario: Populated recent_cwds parity

- **WHEN** `recent_cwds.json` on disk contains five entries with varied `ts` and `cwd` values
- **THEN** `GET /api/sessions/bootstrap` from both Python and Rust servers returns identically ordered and truncated `recent_cwds` arrays

#### Scenario: cwd_groups round-trip parity

- **WHEN** `cwd_groups.json` on disk contains three groups with varied `label` / `collapsed` / `hidden` fields
- **THEN** both servers return structurally identical `cwd_groups` maps in `GET /api/sessions/bootstrap`

#### Scenario: tmux_available parity

- **WHEN** the host either has or lacks `tmux` on PATH
- **THEN** `tmux_available` in both servers' `bootstrap` payload is the same boolean

### Requirement: Read-only Unix-socket broker client is implemented in Rust

The Rust backend SHALL provide a synchronous read-only Unix-socket client at `backend-rs/src/broker_client.rs` that connects to a broker socket, writes a single JSON object terminated by `\n`, reads a single line of JSON response, and closes the connection. It SHALL expose at minimum `broker_state(sock_path, timeout)`, `broker_ui_state(sock_path, timeout)`, and `broker_commands(sock_path, timeout)` helpers, each returning structured Rust types. The client SHALL distinguish four error categories — connect refused, timeout, empty response, malformed JSON — and SHALL NOT write any command other than `state` / `ui_state` / `commands` to the broker (write commands are introduced in Phase 4).

#### Scenario: Successful state call

- **WHEN** a Python broker is running and listening on `<app_dir>/socks/<id>.sock`, and the Rust caller invokes `broker_state(sock_path, Duration::from_millis(1500))`
- **THEN** the call returns `Ok(BrokerState { busy: bool, queue_len: usize, token: Option<Value> })` populated from the broker's JSON response, equivalent to Python `_sock_call(sock, {"cmd": "state"}, timeout_s=1.5)`

#### Scenario: Connect refused returns Err

- **WHEN** the socket file does not exist, or exists but no process is listening
- **THEN** `broker_state` returns `Err(_)`, and the caller is responsible for downgrading to the last-known sidecar state

#### Scenario: Timeout returns Err

- **WHEN** a broker accepts the connection but does not respond within the timeout
- **THEN** `broker_state` returns `Err(_)` rather than blocking indefinitely

#### Scenario: Malformed JSON returns Err

- **WHEN** a broker responds with a line that is not valid JSON or is missing the required `busy` / `queue_len` fields for the `state` command
- **THEN** `broker_state` returns `Err(_)` and the caller logs a warning before downgrading

#### Scenario: Phase 2 client is read-only

- **WHEN** the source code of `backend-rs/src/broker_client.rs` is grepped for `cmd` literal values
- **THEN** the only commands present are `"state"`, `"ui_state"`, and `"commands"` — no `inject`, `interrupt`, `shutdown`, `enqueue`, or other write commands appear in this module

### Requirement: `/api/settings/voice` and `/api/v1/settings/voice` return a voice settings snapshot read from disk

The Rust backend SHALL expose `GET /api/settings/voice` and `GET /api/v1/settings/voice`, both protected by the auth middleware. The handler SHALL read `~/.local/share/codoxear/voice_settings.json` (creating no file, starting no worker) and return a JSON body with `ok: true` plus the same key set Python `MANAGER._voice_push.settings_snapshot()` returns: `tts_enabled_for_narration`, `tts_provider`, `tts_model`, `tts_voice`, `tts_base_url`, `tts_speed`, `provider_options`, plus any voice-status keys Python currently emits. Missing or malformed file SHALL produce the same default snapshot Python emits when the file is absent.

#### Scenario: Empty HOME returns Python defaults

- **WHEN** `voice_settings.json` does not exist and an authenticated `GET /api/settings/voice` is issued
- **THEN** the Rust response has status `200`, body `ok: true` plus the default voice settings keys, structurally identical to the Python server's response under the same conditions

#### Scenario: Populated voice_settings.json round-trips

- **WHEN** a hand-crafted `voice_settings.json` with `tts_provider: "openai"`, `tts_voice: "shimmer"`, `tts_speed: 1.1` is placed on disk
- **THEN** both servers return the same `tts_provider`, `tts_voice`, `tts_speed` values in the `/api/settings/voice` response

#### Scenario: Phase 5 worker is not started

- **WHEN** the Rust server handles `GET /api/settings/voice`
- **THEN** no HLS segment file, WebPush request, or TTS subprocess is created during request handling — verified by inspecting `~/.local/share/codoxear/audio/` and process subprocess list before and after the request

### Requirement: `/api/notifications/subscription`, `/api/notifications/message`, and `/api/notifications/feed` (GET) return file-snapshot data

The Rust backend SHALL expose, with auth middleware applied:

- `GET /api/notifications/subscription` and `/api/v1/notifications/subscription`: read `~/.local/share/codoxear/push_subscriptions.json` and return `ok: true` plus the same key set Python `MANAGER._voice_push.subscriptions_snapshot()` emits (subscriptions list and aggregate counters).
- `GET /api/notifications/message?message_id=` and `/api/v1/...`: when `message_id` is empty or missing return `400 {"error":"message_id required"}`; when the id is unknown return `404 {"error":"unknown message"}`; otherwise return `200 {"ok": true, ...notification_state}` derived from `voice_delivery_ledger.json`.
- `GET /api/notifications/feed?since=` and `/api/v1/...`: parse `since` as a `f64` (default `0`); on parse failure return `400 {"error":"invalid since"}`; otherwise return `200 {"ok": true, "items": [...]}` from the ledger feed since the timestamp.

All three handlers SHALL be implemented without instantiating the Phase 5 voice push worker; they only read the on-disk files.

#### Scenario: Subscription snapshot parity

- **WHEN** `push_subscriptions.json` contains two subscriptions with `endpoint`, `device_class`, `device_label`, `ua` fields
- **THEN** Python and Rust servers return structurally identical bodies on `GET /api/notifications/subscription`

#### Scenario: Missing message_id returns 400

- **WHEN** `GET /api/notifications/message` is sent with no `message_id` query param
- **THEN** the Rust server returns status `400` and body `{"error":"message_id required"}`, identical to Python

#### Scenario: Unknown message returns 404

- **WHEN** `GET /api/notifications/message?message_id=does-not-exist` is sent against a populated ledger that does not contain the id
- **THEN** the Rust server returns status `404` and body `{"error":"unknown message"}`

#### Scenario: Invalid since returns 400

- **WHEN** `GET /api/notifications/feed?since=not-a-number` is sent
- **THEN** the Rust server returns status `400` and body `{"error":"invalid since"}`

#### Scenario: Feed items parity

- **WHEN** `voice_delivery_ledger.json` contains five events at different timestamps and `GET /api/notifications/feed?since=0` is issued
- **THEN** both servers return the same list of `items`, in the same order, with identical fields per item

### Requirement: `/api/metrics` and `/api/v1/metrics` return the metrics snapshot used by Python `_metrics_snapshot`

The Rust backend SHALL expose `GET /api/metrics` and `GET /api/v1/metrics`, both protected by the auth middleware. The handler SHALL return `200 {"metrics": ...}` where the `metrics` object contains the same key set Python `_metrics_snapshot()` emits at the time of cutover, including at minimum the rolling latency keys recorded by `_record_metric` (`api_sessions_ms`, `api_messages_init_ms`, `api_messages_poll_ms`). Phase 2 MAY emit empty / zeroed metrics if the Rust server has not yet handled enough requests to populate them — the response SHALL still match the Python schema (key set, value types).

#### Scenario: Metrics endpoint returns the correct top-level shape

- **WHEN** an authenticated `GET /api/metrics` is issued
- **THEN** the response status is `200`, body has exactly the top-level key `metrics` whose value is an object, and each known Python metric key (e.g. `api_sessions_ms`) is either present with the same value type Python uses or absent (Phase 2 zero-history is acceptable)

#### Scenario: Metrics endpoint key set matches Python

- **WHEN** both servers are warmed by issuing identical request sequences (e.g. 10 `/api/sessions` calls), then both `GET /api/metrics` are compared
- **THEN** the Rust response's `metrics` keys are a superset of the Python keys observed in the same warm-up; missing keys produce a clear failure in the contract test

### Requirement: `/api/sessions` and `/api/v1/sessions` return Python-parity session list payloads

The Rust backend SHALL expose `GET /api/sessions` and `GET /api/v1/sessions`, both protected by the auth middleware. The handler SHALL parse the same query parameters Python parses (`view`, `group_key`, `offset`, `limit`, `group_offset`, `group_limit`) with the same defaults, the same lower / upper bound clamping (`limit ∈ [1, 50]`, `group_limit ∈ [1, 20]`), the same `view` validation (`directories` | `recent`), and the same `400` error schema (`{"error": "...", "view"?: "..."}`). For `view=directories`, the response shape SHALL include `sessions[]`, `remaining_by_group`, `omitted_group_count`, `recent_cwds`, `cwd_groups`, `new_session_defaults`, `tmux_available`, `tmux_session_name`, `app_version` — keys, types, and ordering matching Python `_session_list_payload`. For `view=recent`, the response SHALL include `sessions[]`, `remaining`, plus the same auxiliary keys. Each per-session entry SHALL include the same field set the Python frontend reads.

The handler SHALL discover live sessions by scanning `<app_dir>/socks/*.sock`, parse each `<id>.json` sidecar, prune dead PIDs, merge state from `session_aliases.json`, `session_queues.json`, `harness.json`, `session_sidebar.json`, `session_files.json`, `hidden_sessions.json`, and call `broker_client::broker_state` for live `busy/queue_len/token` (degrading to sidecar values on broker error). It SHALL compute `time_priority` / `base_priority` / `final_priority` using the same formulae Python uses (decay over `_priority_from_elapsed_seconds`, clip01, `dependency_session_id` and `snooze_until` zeroing). It SHALL resolve `git_branch` and `pr_summary` via `git_context::resolve_repo_context` (per-cwd cache).

#### Scenario: directories view default parity

- **WHEN** an authenticated `GET /api/sessions` (default `view=directories`) is issued against a HOME containing 3 sessions across 2 cwds, with sidebar / queue / harness / aliases populated
- **THEN** the Rust response has status `200`, the JSON body matches the Python response under `assert_json_equivalent` with structurally identical `sessions` per group, identical `remaining_by_group`, and identical `recent_cwds` ordering

#### Scenario: recent view rejects group pagination

- **WHEN** `GET /api/sessions?view=recent&group_key=foo` is issued
- **THEN** both servers respond `400 {"error":"group pagination is not supported for recent view"}` (or the byte-equal Python message)

#### Scenario: Unsupported view returns 400 with view field

- **WHEN** `GET /api/sessions?view=banana` is issued
- **THEN** both servers respond `400 {"error":"unsupported sessions view","view":"banana"}` with identical body

#### Scenario: Limit clamping matches Python

- **WHEN** `GET /api/sessions?view=directories&limit=999` is issued
- **THEN** both servers cap the returned page size at `50` (the Python upper bound) and the `remaining_by_group` reflects the cap

#### Scenario: Broker timeout falls back to sidecar busy/queue_len

- **WHEN** the broker socket for one of the sessions is unreachable (connection refused or timeout)
- **THEN** the Rust server still returns a `200` response, the affected entry's `busy` / `queue_len` are set to the last known sidecar values rather than failing the entire request, and a single `WARN` log line is emitted referencing the unreachable session

#### Scenario: PR badge resolution per cwd

- **WHEN** two sessions share the same cwd which is a real git repo with an open PR, and `gh` is on PATH
- **THEN** `pr_summary` and `git_branch` resolve via `git_context::resolve_repo_context` exactly once per cwd within the PR cache TTL window, and both Rust and Python responses contain the same `pr_summary` shape (`number`, `title`, `state`, `availability`)

### Requirement: `/api/session_resume_candidates` returns parity with Python `_session_resume_candidates_for_cwd`

The Rust backend SHALL expose `GET /api/session_resume_candidates` and `/api/v1/...`, parsing query parameters `cwd`, `backend`, `agent_backend`. The response SHALL include `ok: true`, the `cwd` info object (`exists`, `is_git_repo`, etc.), `backend` echo, and a `sessions` list of resumable session descriptors mirroring Python output for the same query.

#### Scenario: Empty cwd parity

- **WHEN** `GET /api/session_resume_candidates?cwd=&backend=codex` is issued (empty cwd)
- **THEN** both servers respond with the same shape (`ok: true`, `exists: false`, `sessions: []` or analogous)

#### Scenario: Real cwd with prior sessions parity

- **WHEN** the requested cwd has past codex sessions on disk and `agent_backend=codex` is requested
- **THEN** both servers return identically ordered `sessions[]` arrays with matching `session_id`, `start_ts`, `updated_ts`, `alias` fields

### Requirement: `/api/sessions/{id}/diagnostics` returns Python-parity diagnostics payloads

The Rust backend SHALL expose `GET /api/sessions/{session_id}/diagnostics` and `/api/v1/...`, with auth middleware applied. The handler SHALL refresh session metadata, look up the in-memory session, call `broker_client::broker_state` for live `busy/queue_len/token`, read `session_sidebar.json` for `priority_offset/snooze_until/dependency_session_id`, resolve `git_branch` via `git_context`, optionally read `model_provider/preferred_auth_method/model/reasoning_effort` from log when sidecar is missing them, and emit the full Python field set: `session_id, thread_id, agent_backend, backend, owned, transport, cwd, start_ts, updated_ts, log_path, session_file_path, broker_pid, codex_pid, busy, broker_busy, queue_len, token, model_provider, preferred_auth_method, provider_choice, model, reasoning_effort, service_tier, idle_auto_stop, idle_timeout_seconds, tmux_session, tmux_window, git_branch, pr_summary, time_priority, base_priority, final_priority, priority_offset, snooze_until, dependency_session_id, todo_snapshot`.

#### Scenario: Unknown session returns 404

- **WHEN** `GET /api/sessions/does-not-exist/diagnostics` is issued
- **THEN** both servers respond `404 {"error":"unknown session"}` with identical body

#### Scenario: Live broker state reflected in busy and queue_len

- **WHEN** a real broker is running with `busy=true` and `queue_len=3` on its socket
- **THEN** the Rust diagnostics response contains `broker_busy: true`, `queue_len: 3`, and (for non-Pi backends with a log path) `busy` reflects log-derived idle status combined with broker busy

#### Scenario: All Python fields present

- **WHEN** the diagnostics response is parsed
- **THEN** every Python diagnostics field from `codoxear/server.py:9183-9230` is present with the same value type; absent fields produce a contract-test failure naming the missing key

### Requirement: `/api/sessions/{id}/queue`, `/harness`, `/workspace`, `/details`, `/ui_state`, `/commands`, `/takeover`, `/repo` parity

The Rust backend SHALL expose the following GET endpoints (each on canonical `/api/v1/*` and legacy `/api/*` paths), all behind the auth middleware:

- `/api/sessions/{id}/queue` — return `200 {"ok": true, "queue": [...]}` from `session_queues.json` (no broker call), with `404` for unknown session and `502` `{"error": ...}` for IO errors. Matches `MANAGER.queue_list` + `_queue_list_local`.
- `/api/sessions/{id}/harness` — return `200 {"ok": true, "enabled": ..., "cooldown_minutes": ..., "remaining_injections": ..., "request": ...}` from `harness.json`, with Python defaults for missing fields.
- `/api/sessions/{id}/workspace` — port `_session_workspace_payload`; return `200 <workspace descriptor>` or `404 {"error": "unknown session"}`.
- `/api/sessions/{id}/details` — port `_session_details_payload`; return `200 <details>` or `404 {"error": "unknown session"}`.
- `/api/sessions/{id}/ui_state` — for Pi sessions call `broker_ui_state`; for other backends return `200 {"priority_offset": ..., "snooze_until": ..., "dependency_session_id": ...}` from sidebar; `404` on unknown session, `502` on broker error.
- `/api/sessions/{id}/commands` — port `MANAGER.get_session_commands`; for Pi sessions call `broker_commands`; return `200 <commands snapshot>` or `404`.
- `/api/sessions/{id}/takeover` — port `_session_takeover_payload`; return `200 <takeover descriptor>` or `404`.
- `/api/sessions/{id}/repo` — parse `?refresh=1` flag, call `git_context::resolve_repo_context(cwd, refresh)`, return `200 <to_detail_dict()>` or `404 {"error": "session not found"}`.

#### Scenario: Queue endpoint reads disk only

- **WHEN** `session_queues.json` contains 2 entries for `sess-A` and `GET /api/sessions/sess-A/queue` is issued
- **THEN** the Rust response is `200 {"ok":true,"queue":["...","..."]}`, structurally identical to Python; no Unix-socket broker call is made (verified via `strace`-style log capture or equivalent)

#### Scenario: Harness endpoint missing fields use Python defaults

- **WHEN** `harness.json` for `sess-B` contains only `enabled: true`
- **THEN** both servers return `200 {"ok":true,"enabled":true,"cooldown_minutes":<HARNESS_DEFAULT_IDLE_MINUTES>,"remaining_injections":<HARNESS_DEFAULT_MAX_INJECTIONS>,"request":null}` with identical defaults

#### Scenario: Pi ui_state via broker

- **WHEN** a Pi session is running and `GET /api/sessions/<id>/ui_state` is issued
- **THEN** the Rust handler calls `broker_ui_state` and returns the broker's payload directly; non-Pi sessions skip the broker and return sidebar-only fields

#### Scenario: Broker error on ui_state surfaces as 502

- **WHEN** the broker socket is unreachable while handling `GET /api/sessions/<pi-id>/ui_state`
- **THEN** the Rust response is `502 {"error": "..."}` with a body the contract test treats as Python-equivalent

#### Scenario: `/repo` resolves PR via git_context

- **WHEN** the session cwd is a real git repo with an open PR and `?refresh=1` is sent
- **THEN** both servers' `/repo` responses contain the same `git_branch`, `pr_summary.number`, `pr_summary.title`, `pr_summary.state`, `availability` fields

### Requirement: `/api/sessions/{id}/git/changed_files`, `/git/diff`, `/git/file_versions` parity

The Rust backend SHALL expose `GET /api/sessions/{id}/git/changed_files`, `/git/diff?path=&staged=`, and `/git/file_versions?path=` on canonical and legacy paths, all behind auth. The handlers SHALL invoke `git` subprocess with the same argument lists, timeouts (`GIT_DIFF_TIMEOUT_SECONDS`), and byte limits Python uses, return `409 {"error": "..."}` for non-repo sessions, and produce the same JSON field sets:

- `/git/changed_files`: `ok, cwd, files, entries[{path, additions, deletions, changed}], staged, unstaged`. Files truncated at `GIT_CHANGED_FILES_MAX`.
- `/git/diff`: `ok, cwd, path, staged, diff` — `staged` is parsed from `?staged=1` boolean.
- `/git/file_versions`: `ok, cwd, path, abs_path, base_exists, base_text, current_exists, current_text, current_size`.

#### Scenario: Non-repo cwd returns 409

- **WHEN** the session cwd is not a git working tree and `GET /api/sessions/<id>/git/changed_files` is issued
- **THEN** both servers return `409 {"error": "..."}` with identical message

#### Scenario: Changed files truncated at max

- **WHEN** the cwd has more than `GIT_CHANGED_FILES_MAX` changed files
- **THEN** both servers' `files` arrays are truncated to the same length and `entries` is correspondingly trimmed

#### Scenario: Diff staged flag round-trips

- **WHEN** `?staged=1` is set and the index has staged changes for the requested path
- **THEN** both servers' `diff` body contains the staged diff text and `staged: true`

#### Scenario: file_versions reflects on-disk state

- **WHEN** the requested path exists in the working tree but not in HEAD (newly added file)
- **THEN** both servers return `base_exists: false`, `current_exists: true`, `current_size > 0`, `current_text` populated, `base_text: null`

### Requirement: `/api/sessions/{id}/file/read`, `/file/search`, `/file/list`, `/file/blob`, `/file/download`, `/api/files/blob` parity

The Rust backend SHALL expose the file-viewer GET endpoints on canonical and legacy paths behind auth:

- `/file/read?path=` — return `200 {"ok": true, "kind": ..., "path": ..., "rel": ..., "size": ..., text?, editable?, version?, content_type?, image_url?, pdf_url?, download_only?, reason?, viewer_max_bytes?}` mirroring Python.
- `/file/search?q=&limit=` — return `200 {"ok": true, "query": ..., "cwd": ..., "mode": ..., "matches": [...], "scanned": ..., "truncated": ...}`.
- `/file/list?path=` — return `200 {"ok": true, "cwd": ..., "path": ..., "entries": [...]}` (huapeixuan-only).
- `/file/blob?path=` — return inline image/PDF bytes with appropriate `Content-Type`, or `error` JSON.
- `/file/download?path=` — return attachment bytes with `Content-Disposition: attachment`, or `error` JSON.
- `/api/files/blob?path=` — global file viewer inline blob (huapeixuan-only).

All handlers SHALL respect Python's path-traversal protection (paths must resolve under the session cwd), the same `viewer_max_bytes` policy, the same MIME detection, and the same error shapes.

#### Scenario: Read text file under cwd

- **WHEN** a UTF-8 text file under the session cwd is requested via `/file/read?path=src/main.rs`
- **THEN** both servers return `200` with `kind: "text"`, identical `text` content, identical `size`, identical `version` (mtime nanos or equivalent), and identical `content_type`

#### Scenario: Path traversal rejected

- **WHEN** `/file/read?path=../../etc/passwd` is requested
- **THEN** both servers return an `error` body (Python message, e.g. `{"error": "path outside session cwd"}` or equivalent), at the same status code

#### Scenario: Binary blob returns inline bytes with correct MIME

- **WHEN** an image at `<cwd>/screenshot.png` is requested via `/file/blob?path=screenshot.png`
- **THEN** both servers return raw image bytes with `Content-Type: image/png` (or the agreed MIME-detection result documented in design open-question resolution) and `Content-Length: <size>`

#### Scenario: file/list parity huapeixuan-only

- **WHEN** `/file/list?path=src/` is requested against a real cwd
- **THEN** both servers return `entries[]` with the same set of `name`, `kind` (file / dir), `size`, `mtime` fields per entry, sorted identically

#### Scenario: download forces attachment disposition

- **WHEN** `/file/download?path=README.md` is requested
- **THEN** both servers return raw bytes with `Content-Disposition: attachment; filename="README.md"` (or the exact Python format)

### Requirement: `/api/sessions/{id}/messages`, `/tail`, `/live` parity (legacy aliases preserved)

The Rust backend SHALL expose `GET /api/sessions/{id}/messages?offset=&limit=&before=&init=`, `/tail`, and `/live?offset=&live_offset=&requests_version=` on canonical `/api/v1/*` and legacy `/api/*` paths, all behind auth. The handlers SHALL port Python `MANAGER.get_messages_page`, `MANAGER.get_tail`, and `_session_live_payload` 1:1, including:

- `offset` clamped to `>= 0`.
- `limit` clamped to `[20, 200]`, default `80`.
- `before` clamped to `>= 0`, default `0`.
- `init` parsed as `?init=1` boolean.
- 404 on unknown session (with historical-row fallback if Python has one).
- `502 {"error": "..."}` on upstream parse / IO errors.
- For non-Pi sessions only, `payload.diag.meta_refresh_ms` is set to the `refresh_session_meta` elapsed time.
- `_record_metric("api_messages_init_ms", ...)` / `"api_messages_poll_ms"` recorded on success (Phase 2 may emit a no-op metric collector if metric scaffolding is not yet shared with Python; the metric name SHALL be reserved).
- `/tail` returns `200 {"tail": ...}`.
- `/live` returns `_session_live_payload`.

The Rust implementation SHALL **not** introduce ref's `/messages/{tail,history,live}` split form (those are Phase 6 cleanup if at all); legacy aliases stay user28b45952.

#### Scenario: messages init parity

- **WHEN** `GET /api/sessions/<id>/messages?init=1&limit=50` is issued against a session with a populated codex rollout log
- **THEN** both servers return identically structured payloads (message list, `diag` meta block on non-Pi, identical message ordering)

#### Scenario: messages poll with offset parity

- **WHEN** `GET /api/sessions/<id>/messages?offset=120&limit=80` is issued
- **THEN** both servers return identically structured payloads, identical `next_offset` (or analogous cursor), identical message ordering

#### Scenario: tail returns last events

- **WHEN** `GET /api/sessions/<id>/tail` is issued
- **THEN** both servers return `{"tail": ...}` with identical content (last N events parsed by `MANAGER.get_tail`)

#### Scenario: live with live_offset parity

- **WHEN** `GET /api/sessions/<id>/live?offset=120&live_offset=8&requests_version=v1` is issued
- **THEN** both servers return `_session_live_payload` with identical `messages`, `live_messages`, `requests_version`, and idle/busy fields

#### Scenario: legacy paths return same body as canonical

- **WHEN** `GET /api/sessions/<id>/messages?init=1` and `GET /api/v1/sessions/<id>/messages?init=1` are both sent
- **THEN** both responses are byte-identical (same body, same headers, same status)

### Requirement: Module split keeps every Rust source file ≤ 800 lines

The Rust backend SHALL split `runtime.rs` and any other monolithic file produced by Phase 2 such that no `backend-rs/src/**/*.rs` file exceeds 800 lines (the same hard limit Phase 1 already enforces on `runtime.rs`). The split SHALL introduce explicit modules `broker_client`, `session_loader`, `log_normalizer` (optionally further `log_normalizer/codex` and `log_normalizer/pi`), `git_context`, `voice_state`, plus a `handlers/` directory with files grouped per wave (voice, sessions_list, session_meta, git, files, messages, metrics).

#### Scenario: Every source file is under the limit

- **WHEN** the deliverable is checked with `find backend-rs/src -name '*.rs' -print0 | xargs -0 wc -l`
- **THEN** every line count is `<= 800`

#### Scenario: Required modules exist

- **WHEN** `find backend-rs/src -type f -name '*.rs' | sort` is run
- **THEN** the output contains at least `broker_client.rs`, `session_loader.rs`, a `log_normalizer/` directory or `log_normalizer.rs`, `git_context.rs`, `voice_state.rs`, and a `handlers/` directory or per-wave handler files

### Requirement: contract harness covers readonly parity for every Phase 2 endpoint

The contract harness SHALL gain end-to-end parity tests covering at minimum the following request shapes per endpoint, each comparing Python and Rust servers via the existing shared HOME / shared HMAC secret setup. Each test SHALL assert (a) status code equality, (b) `Content-Type` header equality, (c) body dict-equivalence via `assert_json_equivalent`. Deterministic endpoints (settings/voice empty state, bootstrap empty state, metrics empty state) SHALL additionally assert raw-byte body equality.

The harness SHALL also include unit-level parity tests for the log normalizer using `tests/fixtures/rollout/*.jsonl` and `tests/fixtures/pi/*.jsonl` fixtures, asserting Rust output matches the corresponding Python `python -c "from codoxear.rollout_log import ..."` output for the same fixture file.

CI SHALL provide `gh` on `macos-latest` (via `brew install gh`) so that `git_context` parity tests do not skip on macOS, and SHALL run the readonly parity selector `pytest tests/contract -k 'parity and readonly'` (or equivalent) across `ubuntu-latest` and `macos-latest`.

#### Scenario: Selector runs all readonly parity tests

- **WHEN** `pytest tests/contract -k 'parity and readonly' --collect-only` is invoked at the Phase 2 deliverable HEAD
- **THEN** the collected test list covers every endpoint listed in the wave-A through wave-F enumeration (at least one test per endpoint shape), with no `pytest.skip` decorators on Phase 2 endpoints

#### Scenario: Header equality enforced

- **WHEN** any readonly parity test asserts `response_python.headers["Content-Type"] != response_rust.headers["Content-Type"]`
- **THEN** the test fails with a clear message naming the endpoint and the diverging header values

#### Scenario: macos CI installs gh

- **WHEN** the `.github/workflows/backend-rs.yml` macOS job runs
- **THEN** `gh --version` is available before `pytest` is invoked, and the `git_context` parity test against a temp git repo successfully executes (rather than skipping)
