## ADDED Requirements

### Requirement: Rust broker binary preserves the Python broker command line and environment contract

The `codoxear-broker-rs` binary SHALL accept the same launch shape used by Python broker entry points: `--cwd <path> -- <agent args...>` for Codex-compatible launches and `--cwd <path> --session-file <path> -- <agent args...>` for Pi web-owned launches. It SHALL read the same environment variables as the Python brokers for backend selection and metadata (`CODEX_WEB_AGENT_BACKEND`, `CODEX_WEB_OWNER`, `CODEX_WEB_SPAWN_NONCE`, `CODEX_WEB_TRANSPORT`, `CODEX_WEB_TMUX_SESSION`, `CODEX_WEB_TMUX_WINDOW`, `CODEX_HOME`, `PI_HOME`, `CODEX_BIN`, `PI_BIN`, model/provider/reasoning/service-tier overrides), apply the same defaults, and exit non-zero with a clear stderr message when required inputs are invalid.

#### Scenario: Codex CLI compatibility
- **WHEN** `codoxear-broker-rs --cwd /repo -- -c 'sandbox_mode="danger-full-access"' resume <id>` is launched with `CODEX_WEB_AGENT_BACKEND=codex`
- **THEN** the Rust broker starts the configured Codex binary with the exact post-`--` agent args in `/repo`, using `CODEX_HOME` as the backend home, and records `resume_session_id=<id>` in sidecar metadata

#### Scenario: Pi CLI compatibility with session file
- **WHEN** `codoxear-broker-rs --cwd /repo --session-file /tmp/pi.jsonl -- -e ask_user_bridge.ts --model x` is launched with `CODEX_WEB_AGENT_BACKEND=pi`
- **THEN** the Rust broker starts a Pi RPC-backed session for `/tmp/pi.jsonl`, forwards the post-`--` Pi args, preserves the AskUser extension arg, and records `session_path=/tmp/pi.jsonl` in sidecar metadata

#### Scenario: Environment metadata is preserved
- **WHEN** `CODEX_WEB_OWNER=web`, `CODEX_WEB_SPAWN_NONCE=abc`, `CODEX_WEB_TRANSPORT=tmux`, `CODEX_WEB_TMUX_SESSION=codoxear`, and `CODEX_WEB_TMUX_WINDOW=w1` are set
- **THEN** the Rust broker sidecar contains the same owner, spawn_nonce, transport, tmux_session, and tmux_window values that Python would write

### Requirement: Rust Codex broker writes Python-compatible sidecar metadata

For Codex backend sessions, the Rust broker SHALL create `~/.local/share/codoxear/socks/<id>.sock` and adjacent `<id>.json` metadata readable by current Python and Rust servers. The sidecar SHALL preserve the Python Codex broker schema: `session_id`, `backend`, `owner`, `supports_web_control`, `broker_pid`, `sessiond_pid`, `codex_pid`, `cwd`, `start_ts`, `log_path`, `sock_path`, `agent_backend`, `resume_session_id`, `model_provider`, `preferred_auth_method`, `model`, `reasoning_effort`, `service_tier`, `transport`, `tmux_session`, `tmux_window`, `spawn_nonce`. The metadata file and socket SHALL use mode `0600` where the platform supports chmod.

#### Scenario: Initial sidecar before log discovery
- **WHEN** a Rust Codex broker starts before Codex has opened a rollout log
- **THEN** it writes a sidecar with stable pid/cwd/owner/backend fields, `log_path:null`, `agent_backend:"codex"`, and `codex_pid` set to the managed Codex process id

#### Scenario: Sidecar after rollout discovery
- **WHEN** the broker discovers the active non-subagent Codex rollout log
- **THEN** it updates `session_id` and `log_path` in the same sidecar file and both Python `SessionManager._discover_existing` and Rust `GET /api/sessions` list the session without repair

#### Scenario: Resume id sidecar field
- **WHEN** Codex args include `resume <resume-id>` or `CODEX_WEB_RESUME_SESSION_ID=<resume-id>`
- **THEN** the Rust broker records `resume_session_id` exactly as Python does and does not invent a different id

### Requirement: Rust Pi broker writes Python-compatible Pi sidecar metadata

For Pi backend sessions, the Rust broker SHALL create a Pi sidecar compatible with `codoxear/pi_broker.py`: `backend:"pi"`, `transport:"pi-rpc"`, `owner`, `supports_web_control`, `supports_live_ui:true`, `ui_protocol_version:1`, `broker_pid`, `agent_pid`, compatibility `codex_pid`, `cwd`, `start_ts`, `log_path:null`, `sock_path`, `resume_session_id`, `spawn_nonce`, optional `session_path`, optional tmux fields, and `agent_backend:"pi"` when writing from Rust. The metadata file and socket SHALL use mode `0600`.

#### Scenario: Pi sidecar includes compatibility codex_pid
- **WHEN** a Rust Pi broker starts a Pi RPC child process
- **THEN** the sidecar includes both `agent_pid` and `codex_pid` using the Pi child pid (or broker pid when no child pid is exposed), so Python discovery does not reject the sidecar

#### Scenario: Agent-managed Pi session omits session_path
- **WHEN** Pi args contain `--no-session` or otherwise manage session storage without a broker-owned `--session-file`
- **THEN** the Rust broker omits `session_path` and sets `supports_web_control` consistently with Python

#### Scenario: Pi session id may update after RPC state sync
- **WHEN** Pi RPC later reports a concrete session id different from the initial placeholder
- **THEN** the Rust broker updates the sidecar `session_id` and keeps the same socket path valid

### Requirement: Rust broker exposes the existing Unix socket JSON-line protocol

The Rust broker SHALL listen on a Unix domain socket and handle one JSON request per line, responding with one JSON object plus newline. It SHALL implement the current command surface used by Python and Rust servers: `state`, `tail`, `send`, `keys`, `shutdown` for Codex; and `state`, `tail`, `live_messages`, `ui_state`, `commands`, `ui_response`, `send`, `keys`, `shutdown` for Pi. Unknown commands SHALL return `{"error":"unknown cmd"}`. Command request/response field names and validation errors SHALL match Python brokers.

#### Scenario: State command
- **WHEN** a client sends `{"cmd":"state"}` to a Rust broker socket
- **THEN** the response includes `busy`, `queue_len`, and `token` with the same semantics as Python for that backend

#### Scenario: Codex send command injects prompt
- **WHEN** a client sends `{"cmd":"send","text":"hello"}` to a Rust Codex broker
- **THEN** the response is `{"queued":false,"queue_len":0}`, broker state becomes busy, and the prompt is written to the PTY using bracketed paste plus the same enter suffix Python uses

#### Scenario: Pi ui_response command validates pending request
- **WHEN** a client sends `{"cmd":"ui_response","id":"req1","value":"x"}` for a pending Pi UI request
- **THEN** Rust forwards the response through Pi RPC, marks the request resolved, and returns `{"ok":true}`; unknown or resolved ids return the same error strings as Python

#### Scenario: Shutdown command terminates managed process
- **WHEN** a client sends `{"cmd":"shutdown"}`
- **THEN** Rust returns `{"ok":true}`, closes the broker socket, terminates or closes the managed backend process/session, and removes or allows cleanup of stale socket artifacts consistently with Python server delete behavior

### Requirement: Rust Codex broker preserves PTY and terminal behavior on Linux and macOS

The Rust Codex broker SHALL spawn Codex under a PTY, set terminal size from the invoking terminal when available, set `TERM`, `COLUMNS`, and `LINES`, forward local stdin/stdout for terminal-owned sessions, support SIGWINCH resize propagation, and use process-group cleanup equivalent to Python. Headless web-owned sessions SHALL not require interactive stdin/stdout and SHALL still execute the backend through the current login-shell behavior needed by Python.

#### Scenario: Terminal-owned local input is forwarded
- **WHEN** Rust broker is launched from a real terminal and the user types into stdin
- **THEN** input bytes are forwarded to the Codex PTY and Codex output is written to stdout, matching the current `codoxear-broker` terminal UX

#### Scenario: Web-owned session is headless
- **WHEN** Rust broker is launched by `POST /api/sessions` with `CODEX_WEB_OWNER=web`
- **THEN** it does not depend on inherited terminal stdin/stdout and still creates a controllable broker socket and sidecar

#### Scenario: SIGWINCH resizes PTY
- **WHEN** the parent terminal size changes while a terminal-owned Rust broker is running
- **THEN** the Rust broker updates the managed PTY window size before continuing I/O

### Requirement: Rust broker discovers active Codex and Pi log paths without regressing macOS behavior

The Rust broker SHALL discover active backend logs using platform-appropriate mechanisms. On Linux it MAY use `/proc` fd traversal equivalent to `proc_find_open_rollout_log`; on macOS it SHALL use `pgrep -P` and `lsof -p ... -F n` or an equivalent non-`/proc` strategy. Codex discovery SHALL ignore subagent rollout logs by following `session_meta.payload.source.subagent` to the main parent session when possible. Pi discovery SHALL prefer explicit `--session-file` / `session_path`, then Pi session id/cwd matching fallbacks.

#### Scenario: Linux proc discovery finds child rollout log
- **WHEN** a Codex child process opens `~/.codex/sessions/.../rollout-...<id>.jsonl` on Linux
- **THEN** Rust discovers that path from the process tree and patches sidecar `log_path`

#### Scenario: macOS lsof discovery finds child rollout log
- **WHEN** running on macOS where `/proc` is unavailable and `lsof` lists an open rollout jsonl for the Codex child or descendant
- **THEN** Rust discovers that path and patches sidecar `log_path` without requiring Linux-only files

#### Scenario: Subagent rollout is ignored
- **WHEN** the only newly opened Codex rollout log has `session_meta.payload.source.subagent`
- **THEN** Rust does not bind the UI to the subagent log; if the parent main log is discoverable, it binds to the parent instead

#### Scenario: Pi explicit session path does not drift
- **WHEN** a Pi broker was launched with an explicit `--session-file` and a newer same-cwd Pi log appears
- **THEN** Rust keeps `session_path` and metadata bound to the explicit file, matching the Python drift-prevention tests

### Requirement: Rust broker maintains busy, token, tail, live UI, and command state parity

The Rust broker SHALL update in-memory state from backend output/log/RPC events so socket clients observe Python-equivalent `busy`, `token`, `tail`, Pi `live_messages`, Pi `ui_state`, and Pi `commands`. It SHALL preserve Codex busy quiet/interrupt grace behavior and Pi prompt-sent grace behavior to avoid premature idle dips.

#### Scenario: Codex busy clears after quiet completed turn
- **WHEN** a Codex rollout log reports terminal turn events and no new output appears for the configured quiet period
- **THEN** Rust `state.busy` clears under the same conditions as Python

#### Scenario: Codex token snapshot updates from rollout log
- **WHEN** Codex writes token/context usage events to the rollout log
- **THEN** Rust `state.token` exposes the same normalized token snapshot Python exposes

#### Scenario: Pi pending AskUser request appears in ui_state
- **WHEN** Pi RPC emits an `extension_ui_request` event
- **THEN** Rust `ui_state` returns a pending request object with the same normalized fields as Python until it is resolved or the turn ends

#### Scenario: Pi prompt sent grace prevents idle flicker
- **WHEN** a Pi prompt was just accepted but `get_state()` has not yet reported busy
- **THEN** Rust keeps `busy:true` for the same grace window as Python before allowing idle

### Requirement: Rust broker contract and integration tests prove Python compatibility

The implementation SHALL include tests that exercise Rust broker sidecar writes, socket commands, spawn selection, log discovery, and rollback behavior. Tests SHALL include cross-language compatibility checks where Python server reads Rust sidecars and Rust server reads Python sidecars. macOS-specific PTY/log-discovery behavior SHALL be verified either in `macos-latest` CI or by documented equivalent local macOS validation before handoff.

#### Scenario: Rust sidecar is discovered by Python
- **WHEN** a Rust broker writes Codex and Pi sidecars into a temp app dir
- **THEN** Python `SessionManager._discover_existing` or the contract harness reads them and exposes the same session ids/backends without schema repair

#### Scenario: Socket command protocol matches existing tests
- **WHEN** Rust broker is exercised with the command cases covered by `tests/test_pi_broker.py` and broker mutation contract tests
- **THEN** request validation, response JSON, and error strings match the Python broker for each backend

#### Scenario: CI verifies both platforms
- **WHEN** Phase 4 is submitted for review
- **THEN** Linux CI passes Rust/Python broker tests and either `macos-latest` CI passes the macOS broker subset or the handoff includes a reproducible macOS local validation transcript covering PTY and lsof discovery
