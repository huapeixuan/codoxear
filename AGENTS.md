# Codoxear architecture notes

This repo is a Rust-only, Linux/macOS companion UI for continuing local CLI agent
sessions from a phone or laptop browser.

Supported agent backends:

- `codex`
- `pi`

## Components

### `codoxear-backend-rs`

- Rust HTTP server built from `backend-rs/`.
- Serves the UI/static assets from `codoxear/static/` and JSON APIs at both
  canonical `/api/v1/*` and legacy alias `/api/*` paths.
- Auth: password gate using `CODEX_WEB_PASSWORD` (required) and the
  `codoxear_auth` cookie signed from `~/.local/share/codoxear/hmac_secret`.
- Session discovery: scans `~/.local/share/codoxear/socks/*.sock` and adjacent
  `*.json` sidecars, including pre-Phase-6 sidecars written by the removed
  Python runtime.
- Web-owned sessions: `POST /api/v1/sessions` or `/api/sessions` spawns
  `codoxear-broker-rs` with `CODEX_WEB_OWNER=web` and the selected
  `agent_backend`.
- Runtime state directory: `~/.local/share/codoxear` by default;
  `CODOXEAR_APP_DIR` may override it for tests/isolated deployments.
- Persisted UI/worker state includes `session_sidebar.json`,
  `session_files.json`, `session_queues.json`, `harness.json`,
  `session_aliases.json`, `voice_settings.json`, `push_subscriptions.json`,
  `voice_delivery_ledger.json`, VAPID PEM, and HLS artifacts.

### `codoxear-broker-rs`

- Rust PTY/RPC wrapper intended to be run from a real terminal or spawned by the
  Rust server.
- Starts the selected backend CLI (`codex` or `pi`), preserves terminal UX, and
  creates a Unix socket control channel under `~/.local/share/codoxear/socks/`.
- Writes a `*.json` sidecar with backend, session/thread id, pid(s), cwd,
  log/session path, socket path, owner tag, launch settings, tmux metadata, and
  spawn nonce.
- Detects active Codex rollout logs and Pi session files through the Rust log
  discovery/normalization modules.
- Linux and macOS.

### `codoxear/static` and `web/`

- `web/` is the Vite/Preact source tree.
- `cd web && npm run build` writes `web/dist/` and copies it into
  `codoxear/static/dist/`.
- `codoxear/static/` is retained for Rust static serving and optional
  static-assets-only Python packaging metadata. It is not a Python runtime.

## Removed legacy surface

Phase 6 removed the Python backend runtime. The repository no longer supports:

- `codoxear/server.py`
- `codoxear/broker.py`
- `codoxear/pi_broker.py`
- `codoxear/sessiond.py`
- `codoxear/voice_push.py`
- Legacy Python console scripts

Do not reintroduce Python runtime wrappers or fallback paths. If rollback is
needed, use git revert to the Phase 5 PASS baseline rather than an env flag.

## Data flow

1. Terminal: `codoxear-broker-rs` runs the selected backend CLI and registers a
   control socket + metadata file.
2. Server: `codoxear-backend-rs` lists sockets/metadata, serves session content,
   and owns opted-in queue/harness/voice workers.
3. Browser: selects a session, sends prompts via API routes, renders normalized
   backend messages, and reads files/git state through Rust handlers.

## Development reminders

- Do not commit secrets: `.env`, `env`, keys, tokens, logs.
- Do not commit runtime artifacts: `codex-homes/`, `socks/`, `root-repo/`,
  `server.log`, `hmac_secret`, `backend-rs/target/`.
- Keep runtime behavior in Rust. Python may remain only for pytest or static
  packaging helper metadata.
- Do not remove `/api/*` aliases without a separate API deprecation change.
- When a subsystem is semantically wrong, replace it instead of layering more
  patches onto a broken structure.
- After modifying the web UI, run `cd web && npm run build` before considering
  the work complete.

## Local dev

Install/build:

```sh
cargo build --manifest-path backend-rs/Cargo.toml --release --bins
```

Run server:

```sh
CODEX_WEB_PASSWORD=... ./backend-rs/target/release/codoxear-backend-rs
```

Broker wrappers:

```sh
codox() {
  /path/to/codoxear-broker-rs -- "$@"
}

piox() {
  CODEX_WEB_AGENT_BACKEND=pi /path/to/codoxear-broker-rs -- "$@"
}
```

Frontend:

```sh
cd web
npm install
npm run dev
npm run build
```

## Ops notes

- Restarting `codoxear-backend-rs` does **not** lose session content. Sessions
  live in backend log/session files and Rust sidecars on disk.
- To avoid losing live sessions, stop only the server process. Do **not** kill
  `codoxear-broker-rs` or the underlying backend CLI unless you intend to stop
  the session.
- Safe restart example:

```sh
pgrep -f "codoxear-backend-rs" | xargs -r kill
CODEX_WEB_PASSWORD=... CODEX_WEB_PORT=13780 CODEX_WEB_HOST=0.0.0.0 \
  ./backend-rs/target/release/codoxear-backend-rs >/tmp/codoxear-13780.log 2>&1 &
```

## Verification before handoff

Run the relevant subset, and for Phase 6/final runtime changes prefer the full
suite:

```sh
openspec validate rust-backend-cutover-finish --strict
openspec validate rust-backend-cutover --strict
openspec validate rust-backend-broker --strict
openspec validate rust-backend-voice-push --strict
cargo fmt --manifest-path backend-rs/Cargo.toml --all -- --check
cargo clippy --manifest-path backend-rs/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path backend-rs/Cargo.toml --release
cargo build --manifest-path backend-rs/Cargo.toml --release --bins
pytest tests/contract -q
(cd web && npm run test && npm run build)
```
