## ADDED Requirements

### Requirement: Rust server exposes auth POST endpoints with Python-compatible cookies

The Rust backend SHALL expose `POST /api/login`, `POST /api/v1/login`, `POST /api/logout`, and `POST /api/v1/logout`. `login` SHALL parse JSON `{password}` without requiring auth, verify it against the same password source as Python, and on success return `200 {"ok":true}` with a `Set-Cookie` header for `codoxear_auth` signed by the same HMAC-SHA256 scheme and scoped to the same cookie path / SameSite / HttpOnly / Secure rules as Python. `logout` SHALL require auth and clear the same cookie with `Max-Age=0`.

#### Scenario: Successful Rust login cookie authenticates Python
- **WHEN** a client posts the correct password to Rust `POST /api/login` while Python and Rust share the same `hmac_secret`
- **THEN** Rust returns `200 {"ok":true}` with `Set-Cookie: codoxear_auth=...`, and the same cookie authenticates Python `GET /api/me`

#### Scenario: Bad password returns 403
- **WHEN** a client posts `{ "password": "wrong" }` to Rust `POST /api/login`
- **THEN** the response is `403 {"error":"bad password"}` and no valid auth cookie is issued

#### Scenario: Logout clears cookie
- **WHEN** an authenticated client posts to Rust `POST /api/logout`
- **THEN** the response is `200 {"ok":true}` and the `Set-Cookie` header clears `codoxear_auth` using the same path Python uses

### Requirement: Rust server writes cwd group state with Python parity

The Rust backend SHALL expose `POST /api/cwd_groups/edit` and `/api/v1/cwd_groups/edit`, require auth, parse JSON fields `cwd`, `label`, `collapsed`, `hidden`, `hidden_after_live_start_ts`, validate them using the same rules as Python `SessionManager.cwd_group_set`, update `cwd_groups.json` using the existing schema, and return `200 {"ok": true, "cwd": <normalized>, ...entry}`.

#### Scenario: Known cwd group edit persists to disk
- **WHEN** an authenticated client posts a known session cwd with `label`, `collapsed`, and `hidden` values
- **THEN** Rust returns the same JSON body as Python and `cwd_groups.json` is readable by Python `GET /api/sessions/bootstrap`

#### Scenario: Unknown cwd without visible state is allowed as no-op entry
- **WHEN** a client posts an unknown cwd with no label, `collapsed=false`, and `hidden=false`
- **THEN** Rust returns the normalized cwd and a default uncollapsed visible entry without persisting unnecessary state, matching Python

#### Scenario: Invalid field types return 400
- **WHEN** `collapsed` or `hidden` is not a boolean, or `label` is not a string when provided
- **THEN** Rust returns the same `400 {"error": ...}` body Python returns

### Requirement: Rust server updates session aliases and sidebar metadata

The Rust backend SHALL expose `POST /api/sessions/{session_id}/edit`, `/api/v1/sessions/{session_id}/edit`, `POST /api/sessions/{session_id}/rename`, and `/api/v1/.../rename`. `edit` SHALL parse `name`, `priority_offset`, `snooze_until`, and `dependency_session_id`, update both `session_aliases.json` and `session_sidebar.json`, and return `ok`, `alias`, `priority_offset`, `snooze_until`, `dependency_session_id`. `rename` SHALL update only the alias file and return `ok`, `alias`.

#### Scenario: Edit updates alias and sidebar fields
- **WHEN** a session exists and the client posts `{name, priority_offset, snooze_until, dependency_session_id}` to Rust `/edit`
- **THEN** the response matches Python and subsequent Rust/Python `GET /api/sessions/{id}/ui_state` returns the edited sidebar values

#### Scenario: Dependency cannot point to self
- **WHEN** `dependency_session_id` equals the edited session id
- **THEN** Rust returns `400 {"error":"session cannot depend on itself"}` matching Python

#### Scenario: Rename clears blank alias
- **WHEN** the client posts a blank `name` to `/rename`
- **THEN** Rust removes the alias entry and returns `alias:""`, matching Python

### Requirement: Rust server mutates persistent session queues with Python parity

The Rust backend SHALL expose `POST /api/sessions/{session_id}/enqueue`, `/queue/delete`, and `/queue/update` at both legacy and v1 paths. It SHALL validate `text`, `images`, and `index` like Python, update `session_queues.json` using the Python schema (string item for text-only, object `{text, images}` for Pi image composer items), and return the same `queued` / `ok` / `queue_len` shapes as Python.

#### Scenario: Enqueue text-only item
- **WHEN** an authenticated client posts `{ "text": "hello" }` to Rust `/enqueue`
- **THEN** Rust returns `200 {"queued":true,"queue_len":1}` and Python `GET /api/sessions/{id}/queue` sees the item

#### Scenario: Enqueue Pi images preserves object shape
- **WHEN** a Pi session receives `/enqueue` with valid `images`
- **THEN** `session_queues.json` stores an object with `text` and `images`, and `/queue/update` preserves the images while changing text, matching Python

#### Scenario: Queue delete/update invalid index returns 502
- **WHEN** `index` is out of range for `/queue/delete` or `/queue/update`
- **THEN** Rust returns the same status and `{"error":"index out of range"}` shape as Python

### Requirement: Rust server sends live broker commands through the existing socket protocol

The Rust backend SHALL expose `POST /api/sessions/{session_id}/send`, `/ui_response`, and `/interrupt` at both legacy and v1 paths. It SHALL use the existing broker socket protocol to send `{"cmd":"send","text":...,"images"?:...}`, `{"cmd":"ui_response",...}`, or `{"cmd":"keys","seq":"\\x1b"}` with Python-equivalent timeouts and error mapping. If `send` cannot reach a still-live broker, Rust SHALL persist the prompt into `session_queues.json` as Python does.

#### Scenario: Send reaches broker
- **WHEN** a stub broker responds to `cmd=send` with `{ "queued": false, "queue_len": 0 }`
- **THEN** Rust `/send` returns that broker response and updates the in-memory/session row busy/queue fields consistently with subsequent `/api/sessions`

#### Scenario: Send broker unavailable falls back to enqueue
- **WHEN** the broker pid is alive but the socket connection fails
- **THEN** Rust `/send` returns a queued response and `session_queues.json` contains the prompt, matching Python fallback behavior

#### Scenario: Pi ui_response legacy fallback
- **WHEN** a Pi broker returns `{"error":"unknown cmd"}` for `ui_response` and the payload is `cancelled:true`
- **THEN** Rust sends ESC through `keys` and returns `{ "ok": true, "legacy_fallback": true }`, matching Python

#### Scenario: Interrupt sends ESC
- **WHEN** a client posts `/interrupt`
- **THEN** Rust sends `cmd=keys` with a literal ESC sequence and returns `200 {"ok":true,"broker":...}` or the same 404/502 errors Python would return

### Requirement: Rust server updates harness configuration and optionally runs the harness sweep worker

The Rust backend SHALL expose `POST /api/sessions/{session_id}/harness` at both paths, reject legacy `text`, validate `enabled`, `request`, `cooldown_minutes`, and `remaining_injections`, update `harness.json`, and return `ok` plus the normalized config. When `CODOXEAR_ENABLE_HARNESS_SWEEP` is truthy, the Rust process SHALL start one harness sweep worker that follows Python `_harness_sweep` cooldown, idle, assistant-last-message, and remaining-injection semantics.

#### Scenario: Harness config write round-trips to Python
- **WHEN** Rust `/harness` enables a session with a custom request and cooldown
- **THEN** Python `GET /api/sessions/{id}/harness` reads the same normalized config

#### Scenario: Legacy text field is rejected
- **WHEN** the request body contains `text`
- **THEN** Rust returns `400 {"error":"unknown field: text (use request)"}` matching Python

#### Scenario: Rust harness worker disabled by default
- **WHEN** `CODOXEAR_ENABLE_HARNESS_SWEEP` is unset
- **THEN** the Rust process does not start the harness sweep loop and does not inject prompts automatically

#### Scenario: Rust harness worker injects once after cooldown
- **WHEN** the flag is enabled, the broker is idle, the last chat role is assistant, cooldown elapsed, and remaining injections is positive
- **THEN** the Rust worker sends the rendered harness prompt once, decrements `remaining_injections`, and persists `harness.json` in Python-readable format

### Requirement: Rust server supports session lifecycle mutation endpoints

The Rust backend SHALL expose `POST /api/sessions`, `/api/v1/sessions`, `POST /api/sessions/{session_id}/delete`, `/heartbeat`, and `/takeover/open`. Session create SHALL parse the same request fields as Python, validate cwd/worktree/tmux/backend options, spawn the existing Python broker or pi_broker process in Phase 3, wait for sidecar metadata, and return the same spawn payload. Delete SHALL request broker shutdown and hide/cleanup session metadata like Python. Heartbeat and takeover/open SHALL match Python status and JSON behavior.

#### Scenario: Create codex web session through Python broker
- **WHEN** Rust receives a valid `POST /api/sessions` for `backend=codex` and non-tmux cwd
- **THEN** it spawns `python -m codoxear.broker`, waits for `socks/<id>.json`, and returns `200 {"ok":true,...spawn payload}` matching Python

#### Scenario: Create pi web session includes AskUser extension
- **WHEN** Rust creates a `backend=pi` session
- **THEN** the spawned command includes the existing `pi_extensions/ask_user_bridge.ts` extension and writes a sidecar readable by Python

#### Scenario: Delete hides and cleans session state
- **WHEN** Rust `/delete` succeeds
- **THEN** the session is hidden, alias/sidebar/files/queue/harness state for that session is cleaned, and Python `GET /api/sessions` no longer lists it

#### Scenario: Heartbeat unsupported session returns 409
- **WHEN** `/heartbeat` targets a session that is not a web-owned pi-rpc session with idle auto-stop support
- **THEN** Rust returns `409` with Python's error text

### Requirement: Rust server supports file write, global file inspect/read/blob POST, and attachment injection

The Rust backend SHALL expose `POST /api/files/read`, `/api/files/inspect`, `/api/files/blob`, `POST /api/sessions/{session_id}/file/write`, `/inject_file`, and `/inject_image` at both paths where applicable. File write SHALL enforce version conflict checks, create/update semantics, path traversal protections, UTF-8/editable constraints, atomic writes, and `session_files.json` history updates matching Python. Attachment injection SHALL enforce body and decoded byte limits, sanitize filenames, write `uploads/<session_id>/<filename>` with mode `0600`, and send bracketed paste text through broker `keys` for non-Pi sessions.

#### Scenario: Existing text file write checks version
- **WHEN** the client posts a stale `version` to `/file/write`
- **THEN** Rust returns `409 {"error":"file changed on disk","conflict":true,"path":...,"version":...}` matching Python and does not modify the file

#### Scenario: Create file conflict returns current version when possible
- **WHEN** `create=true` targets an existing editable file
- **THEN** Rust returns `409 {"error":"file already exists","conflict":true,"path":...,"version":...}` matching Python

#### Scenario: Global files/read adds file history when session_id is provided
- **WHEN** Rust `POST /api/files/read` includes a valid `session_id`
- **THEN** the file path is added to `session_files.json` and subsequent Python session file history includes it

#### Scenario: Inject image rejects Pi sessions
- **WHEN** `/inject_image` targets a Pi session
- **THEN** Rust returns `409` with `backend:"pi"` and `operation:"attachment_injection"`, matching Python

#### Scenario: Uploaded attachment is staged with safe name and 0600 mode
- **WHEN** `/inject_file` receives valid base64 data and filename
- **THEN** Rust writes the decoded bytes under `uploads/<session_id>/` with a sanitized filename, mode `0600`, and returns `path`, `inject_text`, and broker response matching Python

### Requirement: Rust server supports lightweight voice and notification write endpoints without starting Phase 5 workers

The Rust backend SHALL expose `POST /api/settings/voice`, `/api/notifications/subscription`, `/api/notifications/subscription/toggle`, and `/api/audio/listener` at both paths, with auth, validation, JSON persistence, and response shapes matching Python for settings/subscription/listener state. Phase 3 SHALL NOT pretend to send WebPush or synthesize TTS; `POST /api/notifications/test_push` and `POST /api/audio/test_announcement` SHALL either remain unregistered in Rust with documented Phase 5 ownership or return an explicit feature-disabled error, not `ok:true`.

#### Scenario: Voice settings update persists
- **WHEN** Rust receives valid voice settings
- **THEN** it writes `voice_settings.json` and Rust/Python `GET /api/settings/voice` return the same snapshot

#### Scenario: Subscription upsert and toggle persist
- **WHEN** Rust upserts a valid WebPush subscription and then toggles its endpoint disabled
- **THEN** `push_subscriptions.json` contains the same record shape Python writes and both servers list it as disabled

#### Scenario: Unknown subscription toggle returns 404
- **WHEN** `/notifications/subscription/toggle` targets an endpoint not in `push_subscriptions.json`
- **THEN** Rust returns `404 {"error":"unknown subscription"}` matching Python

#### Scenario: Voice debug endpoints are not falsely successful
- **WHEN** a client posts to `/api/notifications/test_push` or `/api/audio/test_announcement` on Rust during Phase 3
- **THEN** the response is either 404/501 feature-disabled per the implemented routing decision, and no WebPush, TTS, HLS, or ledger side effect occurs

### Requirement: Rust server preserves hooks notify no-op behavior

The Rust backend SHALL expose public unauthenticated `POST /api/hooks/notify` and `/api/v1/hooks/notify`. It SHALL read and discard the request body and return `200 {"ignored":true}`, matching Python's optional integration no-op.

#### Scenario: Hooks notify does not require auth
- **WHEN** an unauthenticated client posts any body to Rust `/api/hooks/notify`
- **THEN** Rust returns `200 {"ignored":true}` and does not consult auth middleware

### Requirement: Python workers yield to Rust worker flags

The Python `SessionManager` SHALL detect `CODOXEAR_ENABLE_HARNESS_SWEEP`, `CODOXEAR_ENABLE_QUEUE_SWEEP`, and `CODOXEAR_ENABLE_VOICE_SCAN` at startup. When a flag is truthy (any value except empty, `0`, or case-insensitive `false`), Python SHALL NOT start the corresponding `_harness_loop`, `_queue_loop`, or `_voice_push_scan_loop` thread. This handoff SHALL prevent Python and Rust workers from being co-active for `harness.json`, `session_queues.json`, and voice delivery ledger scan side effects.

#### Scenario: Harness flag prevents Python harness thread
- **WHEN** Python `codoxear-server` starts with `CODOXEAR_ENABLE_HARNESS_SWEEP=1`
- **THEN** no Python thread named `harness` is started and Python does not run `_harness_sweep`

#### Scenario: Queue flag prevents Python queue thread
- **WHEN** Python starts with `CODOXEAR_ENABLE_QUEUE_SWEEP=true`
- **THEN** no Python thread named `queue` is started and Python does not drain `session_queues.json`

#### Scenario: Voice scan flag prevents Python voice scan thread
- **WHEN** Python starts with `CODOXEAR_ENABLE_VOICE_SCAN=1`
- **THEN** no Python thread named `voice-push-scan` is started and Python does not scan logs for voice delivery observations

#### Scenario: Falsy flags preserve Python behavior
- **WHEN** the flags are unset, empty, `0`, or `false`
- **THEN** Python starts the same worker threads it started before Phase 3

### Requirement: Phase 3 POST contract tests prove response parity and disk round-trip compatibility

The contract test suite SHALL include POST parity tests for every Phase 3 endpoint. For each endpoint that writes disk state, the test SHALL verify both the immediate POST response and a follow-up read through the opposite implementation (Rust write then Python read; Python write then Rust read) to prove schema compatibility. For broker mutation endpoints, tests SHALL use stub brokers that assert the exact JSON command sent by Rust.

#### Scenario: Rust-written queue is read by Python
- **WHEN** Rust `/enqueue` writes `session_queues.json`
- **THEN** Python `/api/sessions/{id}/queue` returns the same queue item without repair

#### Scenario: Rust broker command matches Python protocol
- **WHEN** Rust `/send`, `/interrupt`, or `/ui_response` is exercised against a stub broker
- **THEN** the stub receives the exact command JSON Python would have sent for the same request

#### Scenario: No Phase 3 POST endpoint remains untested
- **WHEN** the contract suite is run with `pytest tests/contract -q -k 'parity and post'`
- **THEN** every Phase 3 POST route has at least one passing test and no Phase 3 POST test is skipped or xfailed
