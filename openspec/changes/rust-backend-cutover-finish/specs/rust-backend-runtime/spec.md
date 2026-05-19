## MODIFIED Requirements

### Requirement: Rust binary serves all current `/api/*` routes with byte-level response parity

The system SHALL provide a Rust HTTP server (binary `codoxear-backend-rs`, built from `backend-rs/` crate, axum 0.7) that exposes every supported `/api/<path>` endpoint at both `/api/<path>` (legacy alias) and `/api/v1/<path>` (canonical), with response status code, headers (`Content-Type`, `Content-Length`, `Cache-Control` semantics), JSON field set, field types, and field ordering matching the Phase 5 accepted contract for the same input. No Python HTTP server SHALL be required or supported for runtime parity after this cutover.

#### Scenario: Bootstrap endpoint parity

- **WHEN** the Rust server is started against `~/.local/share/codoxear/` with existing `socks/*.{sock,json}` metadata and an authenticated request `GET /api/sessions/bootstrap` is sent
- **THEN** the response has status `200`, `Content-Type: application/json`, and the JSON keys `recent_cwds`, `cwd_groups`, `new_session_defaults`, `tmux_available` are present with the same structural contract accepted by Phase 1–5 parity tests

#### Scenario: Session list parity

- **WHEN** `GET /api/sessions?view=directories` is sent to the Rust server
- **THEN** the response includes `app_version`, `sessions[]`, `recent_cwds[]`, `new_session_defaults`, `tmux_available`, `tmux_session_name` keys; each `sessions[i]` carries the Phase 5 accepted fields including `session_id`, `agent_backend`, `cwd`, `start_ts`, `updated_ts`, `pid`, `broker_pid`, `busy`, `queue_len`, `harness_enabled`, `git_branch`, `pr_summary`, `priority_offset`, `final_priority`; sort order follows the Rust session loader contract

#### Scenario: Auth cookie remains stable across Rust restarts

- **WHEN** a user logs in via `POST /api/login` against the Rust server and the server restarts with the same `~/.local/share/codoxear/hmac_secret`
- **THEN** the existing `codoxear_auth` cookie authenticates successfully after restart using the same HMAC-SHA256 signing scheme and `hmac_secret` file format

#### Scenario: Unknown route returns 404

- **WHEN** any request hits a path that is not registered in either the canonical or legacy router (e.g. `GET /api/v2/unknown`)
- **THEN** the server returns `404 Not Found` without leaking any internal path information in the body

#### Scenario: Python server module is absent after cutover

- **WHEN** a developer or operator attempts to start `python -m codoxear.server` or import `codoxear.server` as the backend runtime after Phase 6
- **THEN** that path is not a supported runtime; packaging and documentation MUST direct them to `codoxear-backend-rs` instead

### Requirement: Background workers are individually opt-in via environment flags

The Rust system SHALL gate each in-process background worker (harness sweep, queue sweep, voice scan, voice delivery) on a dedicated environment flag, default off, and SHALL NOT start a worker whose flag is unset or set to a falsy value (`0`, empty, `false`, case-insensitive). Workers that are off MUST NOT consume CPU and MUST NOT touch worker-owned files. After Phase 6 there is no Python worker fallback; disabling a Rust worker disables that side effect until the Rust flag is re-enabled or the deployment is reverted.

#### Scenario: Harness sweep gated by env flag

- **WHEN** `CODOXEAR_ENABLE_HARNESS_SWEEP=1` is set in the Rust server environment
- **THEN** a tokio task running the harness sweep loop is spawned at startup, it polls at most once per `CODEX_WEB_HARNESS_SWEEP_SECONDS` (default 2.5s), and it writes harness injection markers to `~/.local/share/codoxear/harness.json` with the existing JSON schema

#### Scenario: Voice delivery worker disabled

- **WHEN** `CODOXEAR_ENABLE_VOICE_WORKER` is unset (or `0`) after Phase 6
- **THEN** no voice delivery task is spawned, no WebPush request is sent from the Rust process, and no Python `voice_push.py` fallback is available or started

#### Scenario: Python and Rust workers are never co-active

- **WHEN** the Phase 6 codebase is built and run
- **THEN** there is no Python `SessionManager` or Python `VoicePushCoordinator` capable of co-owning `harness.json`, `session_queues.json`, `voice_delivery_ledger.json`, or HLS files with Rust workers

### Requirement: Disk metadata format is preserved byte-for-byte across the cutover

The system SHALL read and write all on-disk state (`~/.local/share/codoxear/socks/<id>.{sock,json}`, `session_aliases.json`, `harness.json`, `session_queues.json`, `session_sidebar.json`, `session_files.json`, `hidden_sessions.json`, `recent_cwds.json`, `cwd_groups.json`, `voice_settings.json`, `push_subscriptions.json`, `voice_delivery_ledger.json`, `hmac_secret`, `webpush_vapid_private.pem`, and `audio/` HLS artifacts) using the JSON keys, value types, key ordering rules, atomic-rename semantics, and file-mode bits accepted by the Phase 5 Rust implementation and documented in `docs/cutover/disk-contracts.md`. Existing user state written by Python before Phase 6 MUST remain readable by Rust after Phase 6.

#### Scenario: Read pre-cutover socket sidecar through Rust

- **WHEN** a Python broker from a pre-Phase-6 deployment wrote `socks/sess-A.json` for a codex or pi session
- **THEN** the Rust server SHALL parse that file using the Phase 5 `SessionMeta` shape without dropping unknown-compatible fields and SHALL list the session when the corresponding socket/log state is still valid

#### Scenario: Rust broker writes the only new sidecars

- **WHEN** a new codex or pi session is created after Phase 6
- **THEN** `codoxear-broker-rs` writes `socks/<id>.{sock,json}` using the Rust broker schema accepted in Phase 4, and no Python broker module is invoked

#### Scenario: Atomic file write semantics

- **WHEN** any worker-owned JSON file is updated by the Rust process
- **THEN** the update SHALL go through `write tmp + fsync + rename` (POSIX atomic rename), with file mode `0o600` for secret-bearing files such as `hmac_secret` / VAPID private key and documented modes for other JSON files

### Requirement: Rust broker accepts current broker CLI and environment variables

The system SHALL provide a Rust broker binary `codoxear-broker-rs` that is the only supported broker entry point after Phase 6. It SHALL accept the CLI arguments and environment variables (`CODEX_WEB_OWNER`, `CODEX_WEB_AGENT_BACKEND`, `CODEX_WEB_TMUX_SESSION`, `CODEX_WEB_TMUX_WINDOW`, `CODEX_BIN`, `PI_BIN`, `CODEX_HOME`, `PI_HOME`, `CODEX_WEB_FD_POLL_SECONDS`, `CODEX_WEB_DISCOVER_MIN_INTERVAL_SECONDS`) that were accepted through Phase 5, and it SHALL behave indistinguishably from the perspective of the Rust server, terminal user, and on-disk metadata.

#### Scenario: Codex backend session via Rust broker

- **WHEN** the user runs `codoxear-broker-rs -- <codex args>` from a real terminal with `CODEX_WEB_OWNER` unset
- **THEN** the broker spawns the codex CLI under a PTY, writes `socks/<id>.{sock,json}` with terminal-owned metadata, discovers the active rollout log under `~/.codex/sessions/rollout-*.jsonl`, and accepts inject/interrupt/shutdown JSON commands on the Unix socket

#### Scenario: Pi backend session via Rust broker

- **WHEN** the user runs `CODEX_WEB_AGENT_BACKEND=pi codoxear-broker-rs -- <pi args>` from a real terminal
- **THEN** the broker spawns the pi CLI/RPC path, writes a sidecar with `agent_backend=pi`, and discovers or binds logs under `~/.pi/agent/sessions/<id>.jsonl`, ignoring `~/.codex/sessions/`

#### Scenario: Web-owned session always uses Rust broker

- **WHEN** the UI requests `POST /api/sessions` to create a web-owned codex or pi session after Phase 6
- **THEN** the Rust server spawns `codoxear-broker-rs` directly (not `python -m codoxear.broker` or `python -m codoxear.pi_broker`) and the resulting session appears in `GET /api/sessions` with `owner=web`

#### Scenario: Broker env rollback is no longer supported

- **WHEN** `CODOXEAR_RUST_BROKER_BIN` is unset after Phase 6
- **THEN** new web-owned sessions still use the configured Rust broker path/default binary; unsetting this variable MUST NOT fall back to deleted Python broker modules

### Requirement: Frontend assets are served unchanged

The system SHALL serve every static asset that the Python server previously served (`/`, `/static/...`, `/manifest.webmanifest`, `/service-worker.js`, `/favicon.ico`, `/favicon.png`, `/assets/<vite-hashed>...`) from the existing `codoxear/static/` directory and the existing `codoxear/static/dist/` Vite build output, with the same `Cache-Control`, `Content-Type`, ETag, and `URL prefix` (`CODEX_WEB_URL_PREFIX`) handling, through the Rust server only.

#### Scenario: Vite-hashed asset request

- **WHEN** `GET /assets/main-abc123.js` is requested
- **THEN** the Rust server returns the file from `codoxear/static/dist/assets/main-abc123.js` with `Content-Type: application/javascript` and `Cache-Control: public, max-age=31536000, immutable`

#### Scenario: URL prefix configured

- **WHEN** `CODEX_WEB_URL_PREFIX=/codoxear` is set
- **THEN** the UI is served at `/codoxear/`, the API is mounted under `/codoxear/api/v1/...` and `/codoxear/api/...`, and a request to `/codoxear` (no trailing slash) issues a 308 redirect to `/codoxear/`

### Requirement: Rust server preserves existing default bind host and port

The system SHALL default to bind host `::` (IPv6 dual-stack) and port `8743`, matching the current accepted defaults; SHALL respect `CODEX_WEB_HOST` and `CODEX_WEB_PORT` as the primary environment variables; and SHALL accept `CODOXEAR_BIND_HOST` and `CODOXEAR_BIND_PORT` as fallback aliases for compatibility with `san-tian-dev`-style deploys.

#### Scenario: No bind variables set

- **WHEN** the Rust binary is launched with neither `CODEX_WEB_HOST/PORT` nor `CODOXEAR_BIND_HOST/PORT` set
- **THEN** it binds to `[::]:8743`

#### Scenario: Both styles set, Python style wins

- **WHEN** `CODEX_WEB_PORT=9000` and `CODOXEAR_BIND_PORT=8800` are both set
- **THEN** the server binds on port `9000`

### Requirement: Final cutover rollback is git revert only

The system SHALL treat Phase 6 as the irreversible-in-place cutover. After Python backend files and entry points are removed, disabling env flags SHALL NOT restore Python behavior. Operational rollback SHALL be documented as reverting the Phase 6 commit(s) back to the Phase 5 code-reviewer PASS baseline.

#### Scenario: Voice scan disable after final cutover

- **WHEN** the Rust voice scan worker has been running and a regression is observed; the operator sets `CODOXEAR_ENABLE_VOICE_SCAN=0` and restarts the Rust server
- **THEN** Rust stops scanning, existing `voice_delivery_ledger.json` remains intact, and the operator documentation identifies git revert (not Python fallback) as the way to restore pre-Phase-6 Python voice behavior

#### Scenario: Broker rollback after final cutover

- **WHEN** an operator unsets `CODOXEAR_RUST_BROKER_BIN` after Phase 6 and creates a new web-owned session
- **THEN** the new session is still spawned with the Rust broker default; Python broker fallback is unavailable, and rollback requires reverting Phase 6

### Requirement: Python backend surface is removed from runtime and tests

The repository SHALL NOT ship Python backend, broker, voice worker, or sessiond runtime modules after Phase 6. Tests and docs SHALL NOT import or instruct users to run removed Python backend modules. Python may remain only for packaging static assets, development scripts, or non-runtime tooling explicitly documented as such.

#### Scenario: Python backend files are absent

- **WHEN** the Phase 6 branch is inspected
- **THEN** `codoxear/server.py`, `codoxear/broker.py`, `codoxear/pi_broker.py`, `codoxear/sessiond.py`, and `codoxear/voice_push.py` are absent or reduced to non-runtime compatibility stubs that fail closed with a Rust-only migration message

#### Scenario: Python runtime entry points are absent

- **WHEN** `pyproject.toml` is inspected after Phase 6
- **THEN** it does not expose `codoxear-server`, `codoxear-broker`, or `codoxear-pi-broker` console scripts pointing to Python backend modules

#### Scenario: Tests do not depend on deleted backend internals

- **WHEN** the final verification suite is run
- **THEN** no active test imports deleted Python backend internals such as `codoxear.server.SessionManager`, `codoxear.broker`, `codoxear.pi_broker`, or `codoxear.voice_push.VoicePushCoordinator`; any retained compatibility tests exercise Rust binaries or static assets instead

#### Scenario: Documentation names Rust as the only runtime

- **WHEN** README and AGENTS are inspected after Phase 6
- **THEN** quick start, local dev, broker wrapper, systemd/Tailscale, rollback, and troubleshooting sections name `codoxear-backend-rs` / `codoxear-broker-rs` as the supported runtime and do not present Python backend fallback as available
