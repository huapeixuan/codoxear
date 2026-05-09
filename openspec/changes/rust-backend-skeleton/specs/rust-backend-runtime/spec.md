## ADDED Requirements

### Requirement: Rust backend crate exists with the canonical module layout and a runnable HTTP binary

The system SHALL include a Rust crate at `backend-rs/` named `codoxear-backend-rs` (edition 2021), buildable with `cargo build --release --bins`, producing two binaries: `codoxear-backend-rs` (an axum HTTP server) and `codoxear-broker-rs` (a Phase-1 stub that prints a "not implemented in phase 1" diagnostic and exits non-zero). The crate SHALL define modules `app_state`, `models`, `routes`, and `runtime` under `src/`, plus `src/main.rs` and `src/bin/codoxear-broker-rs.rs`, mirroring the file layout of `san-tian/codoxear@san-tian-dev:backend-rs/src/` minus the `broker.rs` and `voice_worker.rs` modules which are introduced in later phases.

#### Scenario: cargo build --release --bins succeeds on a clean checkout

- **WHEN** a developer runs `cd backend-rs && cargo build --release --bins` against a freshly cloned `huapeixuan/codoxear` working tree on a host with rustup stable >= 1.78
- **THEN** the build SHALL complete with exit code 0, producing files `backend-rs/target/release/codoxear-backend-rs` and `backend-rs/target/release/codoxear-broker-rs`, and SHALL emit no `cargo` warnings (`cargo clippy --all-targets -- -D warnings` is part of CI and passes)

#### Scenario: stub broker binary exits non-zero with diagnostic

- **WHEN** the user runs `backend-rs/target/release/codoxear-broker-rs --any-args`
- **THEN** the process SHALL print a diagnostic on stderr indicating that the broker is not implemented in Phase 1 and SHALL exit with a non-zero status, so that any caller relying on the broker contract fails loudly rather than silently misbehaving

#### Scenario: workspace layout matches the umbrella plan

- **WHEN** `find backend-rs/src -type f -name '*.rs' | sort` is run on the Phase 1 deliverable
- **THEN** the result SHALL contain exactly `backend-rs/src/app_state.rs`, `backend-rs/src/bin/codoxear-broker-rs.rs`, `backend-rs/src/lib.rs`, `backend-rs/src/main.rs`, `backend-rs/src/models.rs`, `backend-rs/src/routes.rs`, and `backend-rs/src/runtime.rs` (no `broker.rs`, no `voice_worker.rs` in Phase 1; those are introduced by Phase 4 and Phase 5 respectively)

### Requirement: Rust server defaults to bind host `::` and port `8743`, honoring Python and ref env aliases

The system SHALL default to bind host `::` (IPv6 dual-stack) and port `8743` when launched without bind environment variables, matching the current Python defaults. The `codoxear-backend-rs` binary SHALL read `CODEX_WEB_HOST` and `CODEX_WEB_PORT` first, then fall back to `CODOXEAR_BIND_HOST` and `CODOXEAR_BIND_PORT`, so that existing systemd unit and Tailscale Serve configurations continue to work unchanged.

#### Scenario: No bind env vars set

- **WHEN** the binary is launched with neither `CODEX_WEB_HOST/PORT` nor `CODOXEAR_BIND_HOST/PORT` set
- **THEN** it SHALL bind to `[::]:8743`

#### Scenario: Python-style env wins over ref-style env

- **WHEN** both `CODEX_WEB_PORT=9000` and `CODOXEAR_BIND_PORT=8800` are set
- **THEN** the server SHALL bind on port `9000`

#### Scenario: ref-style env used when Python-style absent

- **WHEN** `CODEX_WEB_PORT` is unset and `CODOXEAR_BIND_PORT=8800` is set
- **THEN** the server SHALL bind on port `8800`

### Requirement: Phase 1 startup logs a warning for unimplemented worker flags but does not spawn workers

The system SHALL inspect the four worker feature-flag environment variables (`CODOXEAR_ENABLE_HARNESS_SWEEP`, `CODOXEAR_ENABLE_QUEUE_SWEEP`, `CODOXEAR_ENABLE_VOICE_SCAN`, `CODOXEAR_ENABLE_VOICE_WORKER`) at startup and, for each one set to a truthy value (any value other than empty / `"0"` / `"false"` case-insensitive), SHALL emit a `tracing::warn!` log message stating that the corresponding worker is not yet implemented in Phase 1 and that the flag is being ignored. The Rust process SHALL NOT spawn any of these workers in Phase 1, regardless of flag state, and SHALL NOT touch any file owned by those workers (`harness.json`, `session_queues.json`, `voice_delivery_ledger.json`, etc.).

#### Scenario: Truthy harness sweep flag is logged and ignored

- **WHEN** `CODOXEAR_ENABLE_HARNESS_SWEEP=1` is set in the environment of a Phase 1 `codoxear-backend-rs` process
- **THEN** a single `WARN` line containing the string `CODOXEAR_ENABLE_HARNESS_SWEEP` and a phrase indicating "not implemented in phase 1" SHALL be written via `tracing` at startup, and `~/.local/share/codoxear/harness.json` SHALL NOT be created, modified, or deleted by the Rust process during its lifetime

#### Scenario: All worker flags falsy or unset is silent

- **WHEN** none of the four `CODOXEAR_ENABLE_*` flags are set, or each is set to one of `""`, `"0"`, `"false"` (case-insensitive)
- **THEN** no warning is emitted, and no worker is spawned

### Requirement: Auth middleware uses the same HMAC scheme and `hmac_secret` file as the Python backend

The system SHALL implement an axum middleware that protects routes registered inside the `public_api_router` protected scope. The middleware SHALL parse the `Cookie` request header, look up the `codoxear_auth` cookie value, base64url-decode the payload and signature, and verify the signature using HMAC-SHA256 with the secret stored at `<app_dir>/hmac_secret`. The middleware SHALL accept any cookie that the Python backend would accept, and the cookies it (or downstream phases) emit SHALL be accepted by the Python backend, without either side modifying the secret file format or signing algorithm.

The system SHALL provide a function `load_or_create_hmac_secret(app_dir)` that, when the file does not exist, generates 64 bytes from `/dev/urandom`, writes them atomically (or as close to atomically as the Python implementation already does — direct write is acceptable in Phase 1, matching `codoxear/server.py:_load_or_create_hmac_secret`), and `chmod`s the file to mode `0o600`. When the file exists with at least 32 bytes, the function SHALL read the first 64 bytes and use them as the HMAC key.

#### Scenario: Missing cookie returns 401 unauthorized JSON

- **WHEN** a `GET /api/me` (or `GET /api/v1/me`, or `GET /api/sessions/bootstrap`, or `GET /api/v1/sessions/bootstrap`) request is sent to the Rust server with no `Cookie` header at all
- **THEN** the server SHALL respond with HTTP status `401`, `Content-Type: application/json; charset=utf-8`, and body bytes `{"error":"unauthorized"}` (compact JSON, no trailing newline), matching `codoxear/server.py:_unauthorized`

#### Scenario: Cookie issued by Python is accepted by Rust

- **WHEN** a Python `codoxear-server` and a Rust `codoxear-backend-rs` are both started against the same `<app_dir>` such that `<app_dir>/hmac_secret` is shared, and the test client constructs a `codoxear_auth` cookie by signing `{"exp":<future_unix_ts>}` (UTF-8, JSON encoded with `separators=(",", ":")` and `ensure_ascii=False`) using HMAC-SHA256 with the secret bytes, base64url-encoding payload and signature without padding, joined by `.`
- **THEN** sending that cookie to either server's `GET /api/me` returns HTTP `200` with body `{"ok":true}`

#### Scenario: Cookie with expired exp is rejected by Rust

- **WHEN** a `codoxear_auth` cookie carries `{"exp": <unix_ts_in_the_past>}` signed with the correct secret
- **THEN** the Rust server SHALL respond `401 {"error":"unauthorized"}` (the same as if the cookie were absent), matching the Python `_verify_cookie` behavior

#### Scenario: hmac_secret file is created on first need and not overwritten on second start

- **WHEN** the Rust binary is launched against an `<app_dir>` that does not yet contain `hmac_secret`
- **THEN** a `hmac_secret` file SHALL be created with mode `0o600` and exactly 64 bytes of `/dev/urandom` content, and a subsequent restart of the Rust binary (or a Python `codoxear-server` instance pointed at the same directory) SHALL read the existing file rather than overwriting it

### Requirement: `/api/health` and `/api/v1/health` are public health endpoints implemented by the Rust skeleton

The system SHALL expose `GET /api/health` and `GET /api/v1/health` from the Rust server. Both routes SHALL be public (no auth middleware applied) and SHALL respond with HTTP status `200`, `Content-Type: application/json; charset=utf-8`, and a JSON body of the form `{"ok": true, "service": "codoxear-backend-rs"}`. These endpoints have no Python equivalent and exist to support deployment liveness checks and contract test smoke verification.

The endpoint inventory at `docs/cutover/endpoint-inventory.md` SHALL be updated in Phase 1 to mark `GET /api/health` and `GET /api/v1/health` under the "Ref-only routes not present in huapeixuan Python baseline" section as "implemented by `rust-backend-skeleton` Phase 1; no Python parity expected".

#### Scenario: Public access without cookie

- **WHEN** an unauthenticated `GET /api/health` request is sent
- **THEN** the server SHALL respond with status `200` and body `{"ok":true,"service":"codoxear-backend-rs"}` (compact JSON with that exact key order), and SHALL NOT consult the auth middleware

#### Scenario: Canonical /api/v1/health returns the same body

- **WHEN** `GET /api/v1/health` is sent
- **THEN** the response body and headers SHALL be byte-identical to `GET /api/health`

### Requirement: `/api/me` and `/api/v1/me` parity with the Python `_require_auth` + `{"ok": true}` behavior

The system SHALL expose `GET /api/me` and `GET /api/v1/me` from the Rust server. Both routes SHALL be protected by the auth middleware. When authenticated, the response SHALL be HTTP `200`, `Content-Type: application/json; charset=utf-8`, and body `{"ok":true}` (compact JSON, no other keys, no `server_pid`, no `app_version`, no trailing newline), matching `codoxear/server.py:8775-8780` byte-for-byte.

#### Scenario: Authenticated /api/me parity vs Python

- **GIVEN** a Python `codoxear-server` and a Rust `codoxear-backend-rs` are running against the same shared `<app_dir>` and a valid `codoxear_auth` cookie
- **WHEN** the same authenticated `GET /api/me` request is sent to both servers
- **THEN** both responses have status `200`, identical `Content-Type` headers, and identical JSON body bytes `{"ok":true}` (after gzip decoding if applicable)

#### Scenario: /api/me does NOT include ref-only fields

- **WHEN** the Rust server receives `GET /api/me`
- **THEN** the response JSON body SHALL contain exactly one key, `"ok"` with value `true`, and SHALL NOT include `"server_pid"`, `"app_version"`, `"timestamp"`, or any other field that the san-tian reference adds; this is required for byte-level parity with the Python target

### Requirement: `/api/sessions/bootstrap` and `/api/v1/sessions/bootstrap` parity with Python implementation

The system SHALL expose `GET /api/sessions/bootstrap` and `GET /api/v1/sessions/bootstrap` from the Rust server, both protected by the auth middleware. The response SHALL be HTTP `200`, `Content-Type: application/json; charset=utf-8`, and a JSON body with exactly the four top-level keys `recent_cwds`, `cwd_groups`, `new_session_defaults`, `tmux_available`, in that order, matching `codoxear/server.py:8869-8883`. The system SHALL NOT register `/api/v1/bootstrap` or `/api/bootstrap` (the ref's path with a different shape); only the Python-target `/api/sessions/bootstrap` form is in scope.

The contents of each top-level key SHALL be derived from the same on-disk sources the Python implementation uses:

- `recent_cwds`: array of cwd strings sorted by descending `recent_cwds.json` timestamp then ascending path, truncated to `RECENT_CWD_MAX` (default 256, overridable by `CODEX_WEB_RECENT_CWD_MAX`).
- `cwd_groups`: object map keyed by normalized cwd, with values `{label?, collapsed, hidden, hidden_after_live_start_ts?}`. Phase 1 reads `cwd_groups.json` directly without performing the Python `_reconcile_hidden_cwd_groups` reconciliation pass; reconciliation is deferred to Phase 3 when the matching POST endpoint is ported.
- `new_session_defaults`: object `{default_backend, backends: {codex, pi}}` produced from `~/.codex/config.toml`, `~/.codex/models_cache.json`, `~/.pi/agent/settings.json`, `~/.pi/agent/models.json`, with codex-side `provider_choices`, `reasoning_efforts`, `supports_fast` and pi-side `provider_choices`, `models`, `provider_models`, `reasoning_efforts`, `supports_fast` fields populated identically to `_read_codex_launch_defaults` and `_read_pi_launch_defaults`.
- `tmux_available`: boolean from `which("tmux").is_some()`.

#### Scenario: Empty HOME parity

- **GIVEN** a freshly created shared HOME with no `~/.codex/`, no `~/.pi/`, no `~/.local/share/codoxear/recent_cwds.json`, and no `~/.local/share/codoxear/cwd_groups.json`
- **WHEN** the same authenticated `GET /api/sessions/bootstrap` request is sent to both Python and Rust servers running against this HOME
- **THEN** both responses SHALL be identical compact JSON with `recent_cwds=[]`, `cwd_groups={}`, `new_session_defaults` containing `default_backend="codex"` and `backends.codex` with the configured-default-fallback values (provider="openai", preferred_auth_method="apikey", service_tier="flex", model=null, reasoning_effort=null, model_providers=["chatgpt","openai-api"], provider_choices=["chatgpt","openai-api"], reasoning_efforts=["xhigh","high","medium","low"], supports_fast=true, agent_backend="codex") and `backends.pi` with empty provider_choices/models/provider_models and the Pi default reasoning_effort="high"

#### Scenario: Populated recent_cwds parity

- **GIVEN** a shared HOME where `~/.local/share/codoxear/recent_cwds.json` contains `{"/a":1700000200, "/b":1700000100, "/c":1700000300}` (pretty JSON with sort_keys=true and trailing newline, matching Python's writer)
- **WHEN** the bootstrap request is sent to both servers
- **THEN** both `recent_cwds` arrays SHALL equal `["/c","/a","/b"]` in that order

#### Scenario: cwd_groups round-trip parity (read-only)

- **GIVEN** a shared HOME where `~/.local/share/codoxear/cwd_groups.json` contains `{"/x":{"label":"alpha","collapsed":true,"hidden":false}}` (pretty JSON with sort_keys=true and trailing newline)
- **WHEN** the bootstrap request is sent to both servers
- **THEN** the `cwd_groups["/x"]` object in both responses SHALL contain the same keys (`label`, `collapsed`, `hidden`) with the same values; the Rust response SHALL omit any optional key (`hidden_after_live_start_ts`) that the Python `_cwd_group_entry` would also omit, so `JSON.stringify` of both bodies is byte-identical

#### Scenario: tmux_available reflects actual `which` lookup

- **WHEN** the bootstrap request is sent on a host where `tmux` is on PATH
- **THEN** both Python and Rust SHALL return `tmux_available: true`; on a host where `tmux` is not on PATH (e.g., a Docker test container without the tmux binary), both SHALL return `tmux_available: false`

#### Scenario: ref-style /api/v1/bootstrap is NOT registered

- **WHEN** `GET /api/v1/bootstrap` (without the `/sessions` segment) is sent to the Rust server
- **THEN** the server SHALL respond with status `404`, because Phase 1 deliberately does not register the ref's bootstrap path; only the Python-target `/api/sessions/bootstrap` form is in scope

### Requirement: Phase 1 contract harness exercises Python and Rust against shared on-disk state

The repository SHALL ship updates to `tests/contract/` that, taken together, prove byte-level parity for `/api/me` and `/api/sessions/bootstrap` between the Python and Rust backends. The updates SHALL include: (a) a shared `shared_app_home` fixture used by both `python_server_url` and `rust_server_url`, so both servers see the same `~/.local/share/codoxear/` and the same `hmac_secret`; (b) a `signed_auth_cookie` fixture that constructs a valid `codoxear_auth` cookie directly, without requiring `/api/login` to be implemented in Phase 1; (c) the `rust_server_url` fixture launching `backend-rs/target/release/codoxear-backend-rs` as a subprocess on a random port, with the same `CODEX_WEB_PASSWORD` and `CODEX_WEB_HOST=127.0.0.1`; (d) two parity tests no longer marked `@pytest.mark.skip`, covering `/api/me` and `/api/sessions/bootstrap`.

The `test_sessions_parity` test (Phase 2 scope) SHALL remain `@pytest.mark.skip`. The fixture SHALL fail loudly (`RuntimeError`) when the Rust binary is missing, except when the environment variable `CODOXEAR_SKIP_RUST_FIXTURE=1` is set, in which case it SHALL `pytest.skip` so a Python-only contributor can still run the rest of the suite.

#### Scenario: pytest -k parity passes after cargo build

- **GIVEN** a clean working tree on a host with rustup stable installed
- **WHEN** the developer runs `(cd backend-rs && cargo build --release --bins) && pytest tests/contract -q -k parity`
- **THEN** the contract suite SHALL report two passing tests (`test_me_parity`, `test_sessions_bootstrap_parity`) and one skipped test (`test_sessions_parity`) and zero failures or errors

#### Scenario: Missing Rust binary fails the fixture by default

- **GIVEN** a host where `backend-rs/target/release/codoxear-backend-rs` does not exist (no `cargo build` has been run)
- **WHEN** `pytest tests/contract -q -k parity` is run with `CODOXEAR_SKIP_RUST_FIXTURE` unset
- **THEN** the `rust_server_url` fixture SHALL raise `RuntimeError` containing the missing binary path and a hint to run `cargo build`, causing the parity tests to fail (not skip)

#### Scenario: CODOXEAR_SKIP_RUST_FIXTURE=1 escape hatch skips rather than fails

- **WHEN** `CODOXEAR_SKIP_RUST_FIXTURE=1 pytest tests/contract -q -k parity` is run on the same host without the Rust binary
- **THEN** both parity tests SHALL be reported as `SKIPPED`, not `FAILED`

### Requirement: CI builds and tests `backend-rs/` on Linux and macOS

The repository SHALL include a CI workflow (either an extension to an existing GitHub Actions workflow or a new file under `.github/workflows/`) that, on every push and pull request that touches `backend-rs/**`, `tests/contract/**`, or `pyproject.toml`, runs the following on a matrix of `ubuntu-latest` and `macos-latest`:

- `cd backend-rs && cargo fmt --all -- --check`
- `cd backend-rs && cargo clippy --all-targets -- -D warnings`
- `cd backend-rs && cargo test --release`
- `cd backend-rs && cargo build --release --bins`
- `pytest tests/contract -q -k parity` (with the binary built in the previous step on PATH or at the canonical `target/release/...` location)

The workflow SHALL use `Swatinem/rust-cache@v2` (or an equivalent) keyed on `backend-rs/Cargo.lock` to keep CI runtime under 10 minutes per matrix leg after the first warm cache.

#### Scenario: CI is green on a Phase 1 PR

- **WHEN** a pull request introduces the Phase 1 `backend-rs/` crate plus the contract harness updates and the new CI workflow
- **THEN** the CI run SHALL complete green on both `ubuntu-latest` and `macos-latest`, and the resulting check status SHALL show all five steps passing

### Requirement: Phase 1 deliverable rolls back without Python backend changes

The system SHALL keep the Python backend (`codoxear/server.py`, `codoxear/broker.py`, `codoxear/pi_broker.py`, `codoxear/voice_push.py`, all other `codoxear/*.py` modules, and `pyproject.toml` entry points) byte-for-byte unchanged in this Phase 1 change. Disabling the Rust binary (by not running it, or by `git revert` of the Phase 1 commit) SHALL leave Python-driven behavior fully intact, with no migrated state, no schema changes, and no incompatible cookie or disk artifacts to undo.

#### Scenario: Reverting Phase 1 leaves Python untouched

- **GIVEN** the Phase 1 commit is merged to `main`
- **WHEN** the commit is reverted (`git revert <phase1-sha>`)
- **THEN** `git diff <pre-phase1>..<post-revert> -- codoxear/ pyproject.toml` SHALL produce zero output, and the Python `codoxear-server` CLI SHALL continue to start and serve requests exactly as it did before the Phase 1 commit
