## MODIFIED Requirements

### Requirement: Rust server supports session lifecycle mutation endpoints

The Rust backend SHALL expose `POST /api/sessions`, `/api/v1/sessions`, `POST /api/sessions/{session_id}/delete`, `/heartbeat`, and `/takeover/open`. Session create SHALL parse the same request fields as Python, validate cwd/worktree/tmux/backend options, choose the broker executable according to the `CODOXEAR_RUST_BROKER_BIN` rollout flag, wait for sidecar metadata, and return the same spawn payload. When `CODOXEAR_RUST_BROKER_BIN` is unset or blank, session create SHALL keep the Phase 3 Python broker fallback (`python -m codoxear.broker` or `python -m codoxear.pi_broker`). When the flag is set, session create SHALL spawn that Rust broker binary for both Codex and Pi sessions while preserving the same argv/env/nonce/tmux/session-file semantics. Delete SHALL request broker shutdown and hide/cleanup session metadata like Python. Heartbeat and takeover/open SHALL match Python status and JSON behavior.

#### Scenario: Create codex web session through Python broker fallback
- **WHEN** Rust receives a valid `POST /api/sessions` for `backend=codex`, non-tmux cwd, and `CODOXEAR_RUST_BROKER_BIN` is unset
- **THEN** it spawns `python -m codoxear.broker`, waits for `socks/<id>.json`, and returns `200 {"ok":true,...spawn payload}` matching Python

#### Scenario: Create codex web session through Rust broker
- **WHEN** Rust receives a valid `POST /api/sessions` for `backend=codex` and `CODOXEAR_RUST_BROKER_BIN=/path/to/codoxear-broker-rs`
- **THEN** it spawns `/path/to/codoxear-broker-rs --cwd <cwd> -- <codex args...>` with the same `CODEX_WEB_*`/`CODEX_HOME` env values, waits for the nonce-matching sidecar, and returns the same spawn payload shape

#### Scenario: Create pi web session includes AskUser extension
- **WHEN** Rust creates a `backend=pi` session with Python fallback enabled
- **THEN** the spawned command includes the existing `pi_extensions/ask_user_bridge.ts` extension and writes a sidecar readable by Python

#### Scenario: Create pi web session through Rust broker includes session file and AskUser extension
- **WHEN** Rust creates a `backend=pi` session with `CODOXEAR_RUST_BROKER_BIN` set
- **THEN** the spawned Rust broker command includes `--cwd`, `--session-file <resolved pi session file>`, `--`, `-e <ask_user_bridge.ts>`, and any requested Pi model/provider/thinking args, and the resulting sidecar is readable by Python

#### Scenario: Rust broker flag rollback
- **WHEN** `CODOXEAR_RUST_BROKER_BIN` is unset after a previous Rust broker rollout
- **THEN** newly created sessions use Python broker fallback again without requiring code changes or schema migration

#### Scenario: Delete hides and cleans session state
- **WHEN** Rust `/delete` succeeds for either a Python-brokered or Rust-brokered live session
- **THEN** the session is hidden, alias/sidebar/files/queue/harness state for that session is cleaned, and Python `GET /api/sessions` no longer lists it

#### Scenario: Heartbeat unsupported session returns 409
- **WHEN** `/heartbeat` targets a session that is not a web-owned pi-rpc session with idle auto-stop support
- **THEN** Rust returns `409` with Python's error text
