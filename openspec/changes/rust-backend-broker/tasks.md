## 0. Pre-flight and scope guard

- [x] 0.1 Run `openspec status --change rust-backend-broker` and read `proposal.md`, `design.md`, and both specs before coding.
- [x] 0.2 Re-read `docs/cutover/disk-contracts.md` rows for Codex broker sidecar, Pi broker sidecar, and sessiond sidecar; create an implementation checklist for every sidecar field, nullable/missing rule, writer mode, and socket path rule.
- [x] 0.3 Re-read `codoxear/broker.py`, `codoxear/pi_broker.py`, `codoxear/util.py` process/log discovery helpers, `tests/test_pi_broker.py`, `tests/test_broker_proc_rollout.py`, and Phase 3 `backend-rs/src/session_create*.rs`.
- [x] 0.4 Confirm Phase 3 baseline still passes enough to start: `openspec validate rust-backend-cutover --strict`, `openspec validate rust-backend-write-routes --strict`, and `cd backend-rs && cargo test --release broker_client session_create` or the closest focused selectors.
- [x] 0.5 Keep scope limited to Phase 4 broker and broker selection; do not implement Phase 5 voice/HLS/WebPush/TTS or Phase 6 Python deletion.

## 1. Rust broker module skeleton and dependency spike

- [x] 1.1 Replace `backend-rs/src/bin/codoxear-broker-rs.rs` placeholder with real CLI parsing for `--cwd`, optional `--session-file`, and post-`--` agent args.
- [x] 1.2 Add broker modules under `backend-rs/src/broker/` or equivalent (`mod.rs`, `config.rs`, `meta.rs`, `ipc.rs`, `log_discovery.rs`, `codex.rs`, `pi.rs`, `pty.rs`) and export them from `backend-rs/src/lib.rs` as needed for tests.
- [x] 1.3 Choose and add PTY/process dependencies after a focused spike proves child pid, PTY read/write, resize, process group cleanup, and macOS support; document the choice in code comments or `design.md` if the final decision differs.
- [x] 1.4 Add a line-count gate for new broker Rust source files and keep every `backend-rs/src/**/*.rs` file ≤ 800 lines.
- [x] 1.5 Add broker debug logging controlled by `CODEX_WEB_BROKER_DEBUG=1` without leaking secrets or full prompt text beyond existing Python behavior.

## 2. Shared broker config, env, and metadata writers

- [x] 2.1 Implement backend/env normalization matching Python: `CODEX_WEB_AGENT_BACKEND`, `CODEX_WEB_OWNER`, `CODEX_WEB_SPAWN_NONCE`, `CODEX_WEB_TRANSPORT`, `CODEX_WEB_TMUX_SESSION`, `CODEX_WEB_TMUX_WINDOW`, `CODEX_HOME`, `PI_HOME`, `CODEX_BIN`, `PI_BIN`, model/provider/reasoning/service-tier overrides.
- [x] 2.2 Implement app dir and socks dir resolution using the same `~/.local/share/codoxear` contract as `codoxear.util.default_app_dir()`.
- [x] 2.3 Implement Codex sidecar writer with all Python Codex keys, compact JSON-compatible schema, atomic same-dir temp+rename, and mode `0600`.
- [x] 2.4 Implement Pi sidecar writer with all Python Pi keys, optional `session_path`, compatibility `codex_pid`, `agent_pid`, `supports_live_ui`, `ui_protocol_version`, `agent_backend:"pi"`, atomic rename, and mode `0600`.
- [x] 2.5 Add Rust unit tests that serialize Codex/Pi sidecars and assert exact field presence/nullability for web-owned, terminal-owned, tmux, resume, and Pi `--no-session` cases.
- [x] 2.6 Add cross-language sidecar tests proving Python discovery accepts Rust-written Codex/Pi sidecars and Rust session loader accepts Python-written sidecars.

## 3. Unix socket JSON-line server

- [x] 3.1 Implement Unix socket creation under `socks/`, cleanup of stale own socket path before bind, chmod `0600`, listen backlog compatible with Python, and one-request-per-connection JSON-line handling.
- [x] 3.2 Implement shared command dispatch and JSON response writer with Python-equivalent malformed/unknown command behavior.
- [x] 3.3 Implement common commands `state`, `tail`, `send`, `keys`, `shutdown` for Codex and Pi; preserve validation error strings (`text required`, `seq required`, `no state`, `unknown cmd`).
- [x] 3.4 Implement Pi-only commands `live_messages`, `ui_state`, `commands`, `ui_response` with Python-equivalent request/response/error shape.
- [x] 3.5 Add broker socket server tests that connect via UnixStream and assert exact JSON responses for every supported command and unknown command.
- [x] 3.6 Add shutdown tests proving the broker returns `{"ok":true}`, stops accepting new socket requests, and terminates/closes the managed backend.

## 4. Codex PTY broker implementation

- [x] 4.1 Implement Codex child spawn in a PTY with cwd, `TERM`, `COLUMNS`, `LINES`, backend home env, configured `CODEX_BIN`, post-`--` args, and headless web-owned login-shell behavior matching Python.
- [x] 4.2 Implement terminal-owned stdin raw-mode forwarding, stdout forwarding, termios restore, and EOF handling.
- [x] 4.3 Implement SIGWINCH resize propagation to the PTY and add a test around the resize abstraction.
- [x] 4.4 Implement `send` prompt injection using the same bracketed paste / enter sequence behavior as Python; include optional `enter_seq` support.
- [x] 4.5 Implement `keys` raw sequence decoding compatible with Python unicode-escape handling and direct PTY write.
- [x] 4.6 Implement process-group and child cleanup for normal exit, socket shutdown, server delete, and broker crash paths; Linux may use pdeathsig where available.
- [x] 4.7 Add PTY/fake-PTY tests for send/keys/state/tail/busy transitions and cleanup; keep real Codex binary out of unit tests.

## 5. Codex rollout log discovery and state tracking

- [x] 5.1 Port Linux `/proc` descendant fd discovery from `codoxear.util.proc_find_open_rollout_log` into Rust with tests using fake proc trees equivalent to `tests/test_broker_proc_rollout.py`.
- [x] 5.2 Port macOS discovery using `pgrep -P` and `lsof -p ... -F n` or an equivalent command-runner abstraction; add tests with fixture command output so the code path is covered on Linux CI too.
- [x] 5.3 Port Codex rollout path validation, UUID extraction, `session_meta` parsing, and subagent-parent resolution/ignore behavior.
- [x] 5.4 Port Codex busy/token/tail state updates from rollout log events, including quiet seconds, interrupt grace, pending calls, token snapshot, and resume delivery mute behavior required by current tests.
- [x] 5.5 Patch sidecar `session_id`/`log_path` whenever log discovery first succeeds or switches from null to concrete path; do not bind to subagent logs.
- [x] 5.6 Add tests for initial `log_path:null`, later log discovery patch, subagent ignore/parent binding, resume session id, and multiple concurrent brokers claiming different logs.

## 6. Pi RPC broker implementation

- [x] 6.1 Investigate `codoxear/pi_rpc.py` and document the minimal Rust Pi RPC operations needed: start session, prompt, abort, get_state, get_commands, send_ui_response, drain events/stderr, close.
- [x] 6.2 Implement or bridge Pi RPC startup with cwd, `--session-file`, `--session`, `--session-dir`, `--no-session`, `PI_BIN`, `PI_HOME`, and post-`--` args matching Python.
- [x] 6.3 Preserve AskUser extension injection semantics: Rust server create path passes `-e codoxear/pi_extensions/ask_user_bridge.ts`, and terminal Pi launches do not duplicate the arg if already present.
- [x] 6.4 Implement Pi state sync: busy extraction, prompt-sent grace, turn id tracking, session id update from RPC state, stderr/tail drain, live message coalescing, terminal turn completion clearing.
- [x] 6.5 Implement pending UI request tracking and `ui_response` forwarding with the same resolution/retry behavior and error strings as Python.
- [x] 6.6 Implement foreground Pi stdin/stdout/SIGINT behavior for terminal-owned sessions, including abort-on-SIGINT while busy and delegation to previous handler when idle.
- [x] 6.7 Add fake Pi RPC tests covering prompt success/error, images validation, commands, ui_state, ui_response success/unknown/already-resolved, live_messages offsets, shutdown, explicit session path drift prevention, and `--no-session` support.

## 7. Broker selection rollout in Python server

- [x] 7.1 Add a helper in `codoxear/server.py` that resolves `CODOXEAR_RUST_BROKER_BIN`: trim whitespace, require non-empty, and leave unset/blank behavior unchanged.
- [x] 7.2 Modify Codex `SessionManager.spawn_web_session` argv construction so non-tmux and tmux inline commands use Rust broker binary when the helper returns a path; otherwise use `[sys.executable, "-m", "codoxear.broker"]`.
- [x] 7.3 Modify Pi web-owned spawn path so non-tmux and tmux inline commands use Rust broker binary with `--cwd`, `--session-file`, `--`, AskUser extension args when the helper returns a path; otherwise use `[sys.executable, "-m", "codoxear.pi_broker"]`.
- [x] 7.4 Preserve all existing env values for owner, backend, spawn_nonce, tmux, model/provider/reasoning/service-tier, homes, resume id, and worktree behavior.
- [x] 7.5 Add Python tests for argv/env selection: unset flag uses Python broker, set flag uses Rust broker for Codex, set flag uses Rust broker + `--session-file` for Pi, and tmux inline shell command contains the selected broker.

## 8. Broker selection rollout in Rust server

- [x] 8.1 Add `CODOXEAR_RUST_BROKER_BIN` resolution to `backend-rs/src/session_create_support.rs` or a broker selection helper.
- [x] 8.2 Modify Rust Codex create path to build argv with Rust broker binary when set and Python fallback when unset; keep trust/model/resume/worktree args after `--` unchanged.
- [x] 8.3 Modify Rust Pi create path to build argv with Rust broker binary + `--session-file` when set and Python `codoxear.pi_broker` fallback when unset; keep AskUser extension and requested Pi args unchanged.
- [x] 8.4 Ensure tmux create path uses the selected broker executable and inline env, not a hard-coded Python module.
- [x] 8.5 Add Rust tests for session_create argv/env in fake spawn mode or by factoring argv builder functions: Codex fallback, Codex Rust broker, Pi fallback, Pi Rust broker, tmux Rust broker.

## 9. Contract and integration test matrix

- [x] 9.1 Add Rust broker unit tests for sidecar JSON, socket protocol, log discovery, PTY abstraction, Pi RPC abstraction, and process cleanup.
- [x] 9.2 Add Python tests for `CODOXEAR_RUST_BROKER_BIN` spawn selection in Python server without launching real Codex/Pi.
- [x] 9.3 Extend `tests/contract` with Rust-brokered session fixtures: Rust sidecar → Python read, Python sidecar → Rust read, Rust broker socket → Rust/Python server mutation endpoints.
- [x] 9.4 Add smoke integration tests using fake Codex/Pi binaries that open/write fixture logs so Rust broker can discover logs and the server can list/send/delete sessions without real external CLIs.
- [x] 9.5 Add or update CI to run broker tests on Linux; add a macOS broker subset job or document why equivalent local macOS validation is used.
- [x] 9.6 Add macOS validation script or README section covering PTY spawn, `lsof` log discovery, Pi session_path behavior, and rollback via unset env.

## 10. Documentation and cutover artifacts

- [x] 10.1 Update `README.md` with `codoxear-broker-rs` build/run instructions, `CODOXEAR_RUST_BROKER_BIN` rollout, and rollback steps.
- [x] 10.2 Update `docs/cutover/disk-contracts.md` with Phase 4 Rust broker write strategy and explicit statement that schemas/filenames/modes remain unchanged.
- [x] 10.3 Update `openspec/changes/rust-backend-cutover/tasks.md` Phase 4 row or footer with this change id and validation status.
- [x] 10.4 If final implementation choices differ from this design (PTY crate, Pi RPC bridge, macOS CI strategy), update `openspec/changes/rust-backend-broker/design.md` before handoff.
- [x] 10.5 Do not update docs to say Rust broker is default unless `CODOXEAR_RUST_BROKER_BIN` is actually set by the user/operator.

## 11. Verification gate

- [x] 11.1 `openspec validate rust-backend-broker --strict` passes.
- [x] 11.2 `openspec validate rust-backend-cutover --strict` still passes.
- [x] 11.3 `cd backend-rs && cargo fmt --all -- --check` passes.
- [x] 11.4 `cd backend-rs && cargo clippy --all-targets -- -D warnings` passes.
- [x] 11.5 `cd backend-rs && cargo test --release` passes, including broker unit/integration tests.
- [x] 11.6 `cd backend-rs && cargo build --release --bins` produces a working `codoxear-broker-rs`.
- [x] 11.7 `python3 -m pytest tests/test_pi_broker.py tests/test_broker_proc_rollout.py tests/test_pi_server_backend.py tests/test_sessiond_fail_closed.py -q` or narrower updated broker/session selectors pass.
- [x] 11.8 `pytest tests/contract -q -k 'broker or parity'` (or the documented Phase 4 contract selector) passes with no skipped/xfailed Phase 4 broker cases.
- [x] 11.9 Line-count gate `find backend-rs/src -name '*.rs' -print0 | xargs -0 wc -l | awk '$1 > 800'` returns no source file over limit.
- [x] 11.10 Manual or CI smoke: with `CODOXEAR_RUST_BROKER_BIN=$(pwd)/backend-rs/target/release/codoxear-broker-rs`, create Codex and Pi web-owned sessions, observe them in `/api/sessions`, send a prompt, interrupt/delete, then unset the env and verify new sessions use Python broker fallback.
- [x] 11.11 macOS validation: `macos-latest` CI broker subset passes or handoff includes local macOS transcript covering PTY and `lsof` discovery.
- [ ] 11.12 Independent `code-reviewer` review completes with PASS or no HIGH findings before implementation is declared ready for merge.
