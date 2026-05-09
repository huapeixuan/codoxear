## ADDED Requirements

### Requirement: Rust binary serves all current `/api/*` routes with byte-level response parity

The system SHALL provide a Rust HTTP server (binary `codoxear-backend-rs`, built from `backend-rs/` crate, axum 0.7) that exposes every existing Python `/api/<path>` endpoint at both `/api/<path>` (legacy alias) and `/api/v1/<path>` (canonical), with response status code, headers (`Content-Type`, `Content-Length`, `Cache-Control` semantics), JSON field set, field types, and field ordering identical to the Python `codoxear-server` for the same input.

#### Scenario: Bootstrap endpoint parity

- **WHEN** the Rust server is started against the same `~/.local/share/codoxear/` directory and the same `socks/*.{sock,json}` set as a comparison Python server, and the same authenticated request `GET /api/sessions/bootstrap` is sent to both
- **THEN** both responses have status `200`, `Content-Type: application/json`, and the JSON keys `recent_cwds`, `cwd_groups`, `new_session_defaults`, `tmux_available` are present in both responses with structurally identical values; `recent_cwds` ordering is identical; `new_session_defaults.backends` keys are the same set

#### Scenario: Session list parity

- **WHEN** the same `GET /api/sessions?view=directories` request is sent to both servers
- **THEN** both responses include `app_version`, `sessions[]`, `recent_cwds[]`, `new_session_defaults`, `tmux_available`, `tmux_session_name` keys; each `sessions[i]` carries identical `session_id`, `agent_backend`, `cwd`, `start_ts`, `updated_ts`, `pid`, `broker_pid`, `busy`, `queue_len`, `harness_enabled`, `git_branch`, `pr_summary`, `priority_offset`, `final_priority`; sort order across responses is identical

#### Scenario: Auth cookie cross-compatibility

- **WHEN** a user logs in via `POST /api/login` against Rust server (cookie `codoxear_auth` is set)
- **THEN** the same cookie value, when sent to a Python `codoxear-server` running with the same `hmac_secret`, MUST authenticate successfully (and vice versa); both servers SHALL share the same `~/.local/share/codoxear/hmac_secret` file format and HMAC-SHA256 signing scheme

#### Scenario: Unknown route returns 404

- **WHEN** any request hits a path that is not registered in either the canonical or legacy router (e.g. `GET /api/v2/unknown`)
- **THEN** the server returns `404 Not Found` without leaking any internal path information in the body

### Requirement: Background workers are individually opt-in via environment flags

The system SHALL gate each in-process background worker (harness sweep, queue sweep, voice scan, voice delivery) on a dedicated environment flag, default off, and SHALL NOT start a worker whose flag is unset or set to a falsy value (`0`, empty, `false`, case-insensitive). Workers that are off MUST NOT consume CPU and MUST NOT touch worker-owned files.

#### Scenario: Harness sweep gated by env flag

- **WHEN** `CODOXEAR_ENABLE_HARNESS_SWEEP=1` is set in the Rust server environment
- **THEN** a tokio task running the harness sweep loop is spawned at startup, it polls at most once per `CODEX_WEB_HARNESS_SWEEP_SECONDS` (default 2.5s), and it writes harness injection markers to `~/.local/share/codoxear/harness.json` with the same JSON schema the Python sweep currently uses

#### Scenario: Voice delivery worker disabled

- **WHEN** `CODOXEAR_ENABLE_VOICE_WORKER` is unset (or `0`)
- **THEN** no voice delivery task is spawned, no WebPush request is sent from the Rust process, and the Python `voice_push.py` (if running) remains the sole writer to `voice_ledger.json`

#### Scenario: Both Rust and Python workers are never co-active

- **WHEN** any `CODOXEAR_ENABLE_*` flag is set on the Rust process
- **THEN** the Python `SessionManager` SHALL detect the same flag at startup and decline to spawn its own equivalent thread, so that a given on-disk file (`harness.json`, `session_queues.json`, `voice_ledger.json`) is written by exactly one process at a time

### Requirement: Disk metadata format is preserved byte-for-byte across the cutover

The system SHALL read and write all on-disk state (`~/.local/share/codoxear/socks/<id>.{sock,json}`, `session_aliases.json`, `harness.json`, `session_queues.json`, `session_sidebar.json`, `session_files.json`, `hidden_sessions.json`, `recent_cwds.json`, `voice_ledger.json`, `notification_subscriptions.json`, `voice_settings.json`, `hmac_secret`) using the existing JSON keys, value types, key ordering rules, atomic-rename semantics, and file-mode bits used by the Python implementation as of the head of `main` at the time of cutover.

#### Scenario: Round-trip socket sidecar through Rust

- **WHEN** the Python broker writes `socks/sess-A.json` for a freshly-spawned codex session
- **THEN** the Rust server SHALL parse that file using the same `SessionMeta` shape (fields: `session_id`, `codex_pid`, `broker_pid`, `agent_backend`, `owner`, `transport`, `cwd`, `workspace_cwd`, `log_path`, `start_ts`, `updated_ts`, `model_provider`, `preferred_auth_method`, `model`, `reasoning_effort`, `service_tier`, `tmux_session`, `tmux_window`, `resume_session_id`) without losing any field

#### Scenario: Round-trip socket sidecar through Python

- **WHEN** the Rust broker writes `socks/sess-B.json` for a freshly-spawned pi session
- **THEN** a Python `codoxear-server` instance SHALL list session `sess-B` in `GET /api/sessions` with the same `agent_backend=pi`, `cwd`, `start_ts`, `tmux_session`/`tmux_window` fields the Rust broker wrote

#### Scenario: Atomic file write semantics

- **WHEN** any `_*.json` worker file is updated by the Rust process
- **THEN** the update SHALL go through `write tmp + fsync + rename` (POSIX atomic rename), with file mode `0o600` for `hmac_secret` and `0o644` for all other JSON files, matching current Python behavior

### Requirement: Rust broker accepts current Python broker CLI and environment variables

The system SHALL provide a Rust broker binary `codoxear-broker-rs` that, when invoked with the same CLI arguments and same environment variables (`CODEX_WEB_OWNER`, `CODEX_WEB_AGENT_BACKEND`, `CODEX_WEB_TMUX_SESSION`, `CODEX_WEB_TMUX_WINDOW`, `CODEX_BIN`, `PI_BIN`, `CODEX_HOME`, `PI_HOME`, `CODEX_WEB_FD_POLL_SECONDS`, `CODEX_WEB_DISCOVER_MIN_INTERVAL_SECONDS`) as the existing `codoxear-broker` Python entry point, behaves indistinguishably from the perspective of the server, terminal user, and on-disk metadata.

#### Scenario: Codex backend session via Rust broker

- **WHEN** the user runs `codoxear-broker-rs -- codex --some-arg` from a real terminal with `CODEX_WEB_OWNER` unset
- **THEN** the broker spawns the codex CLI under a PTY, writes `socks/<id>.{sock,json}` with `owner` field absent (terminal-owned), discovers the active rollout log under `~/.codex/sessions/rollout-*.jsonl`, and accepts inject/interrupt/shutdown JSON commands on the unix socket

#### Scenario: Pi backend session via Rust broker

- **WHEN** the user runs `CODEX_WEB_AGENT_BACKEND=pi codoxear-broker-rs -- pi --foo` from a real terminal
- **THEN** the broker spawns the pi CLI, writes a sidecar with `agent_backend=pi`, and discovers logs under `~/.pi/agent/sessions/<id>.jsonl`, ignoring `~/.codex/sessions/`

#### Scenario: Web-owned session via Python server through Rust broker

- **WHEN** `CODOXEAR_RUST_BROKER_BIN=/abs/path/codoxear-broker-rs` is set in the Python `codoxear-server` environment, and the UI requests `POST /api/sessions` to create a web-owned codex session
- **THEN** the Python server spawns the Rust broker binary (not `python -m codoxear.broker`) and the resulting session appears in `GET /api/sessions` with `owner=web`

### Requirement: Frontend assets are served unchanged

The system SHALL serve every static asset that the Python server currently serves (`/`, `/static/...`, `/manifest.webmanifest`, `/service-worker.js`, `/favicon.ico`, `/favicon.png`, `/assets/<vite-hashed>...`) from the existing `codoxear/static/` directory and the existing `codoxear/static/dist/` Vite build output, with the same `Cache-Control`, `Content-Type`, ETag, and `URL prefix` (`CODEX_WEB_URL_PREFIX`) handling.

#### Scenario: Vite-hashed asset request

- **WHEN** `GET /assets/main-abc123.js` is requested
- **THEN** the Rust server returns the file from `codoxear/static/dist/assets/main-abc123.js` with `Content-Type: application/javascript` and `Cache-Control: public, max-age=31536000, immutable`

#### Scenario: URL prefix configured

- **WHEN** `CODEX_WEB_URL_PREFIX=/codoxear` is set
- **THEN** the UI is served at `/codoxear/`, the API is mounted under `/codoxear/api/v1/...` and `/codoxear/api/...`, and a request to `/codoxear` (no trailing slash) issues a 308 redirect to `/codoxear/`

### Requirement: Rust server preserves existing default bind host and port

The system SHALL default to bind host `::` (IPv6 dual-stack) and port `8743`, matching the current Python defaults; SHALL respect `CODEX_WEB_HOST` and `CODEX_WEB_PORT` as the primary environment variables; and SHALL accept `CODOXEAR_BIND_HOST` and `CODOXEAR_BIND_PORT` as fallback aliases for compatibility with `san-tian-dev`-style deploys.

#### Scenario: No bind variables set

- **WHEN** the Rust binary is launched with neither `CODEX_WEB_HOST/PORT` nor `CODOXEAR_BIND_HOST/PORT` set
- **THEN** it binds to `[::]:8743`

#### Scenario: Both styles set, Python style wins

- **WHEN** `CODEX_WEB_PORT=9000` and `CODOXEAR_BIND_PORT=8800` are both set
- **THEN** the server binds on port `9000`

### Requirement: Each migration phase is independently rollback-able through env flags

The system SHALL ensure that for every migration phase listed in `design.md`'s Phase Plan, disabling the corresponding `CODOXEAR_ENABLE_*` flag (or unsetting `CODOXEAR_RUST_BROKER_BIN`) and restarting the Rust process is sufficient to return all on-disk and in-memory behavior to the Python implementation, without manual file repair.

#### Scenario: Voice scan disable rollback

- **WHEN** the Rust voice scan worker has been running for an hour and a regression is observed; the operator sets `CODOXEAR_ENABLE_VOICE_SCAN=0` and restarts the Rust server, then starts the Python `codoxear-server`
- **THEN** the Python `voice_push.VoicePushCoordinator` resumes scanning from `voice_ledger.json` without producing duplicate notifications, dropped notifications, or corrupted ledger entries

#### Scenario: Broker rollback mid-flight

- **WHEN** `CODOXEAR_RUST_BROKER_BIN` is unset on the running Python server (no restart) and a new web-owned session is created
- **THEN** the new session is spawned with the Python broker; existing sessions whose Rust broker process is still alive remain reachable through the same `socks/<id>.sock`
