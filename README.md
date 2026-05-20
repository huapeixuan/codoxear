# Codoxear

<p align="center">
  <img src="codoxear/static/codoxear-icon.png" alt="Codoxear icon" width="140" />
</p>

Codoxear is a Rust-only local web companion for Codex and Pi CLI agent sessions.
It runs on your computer, serves a phone-friendly UI, and keeps filesystem,
tools, credentials, and agent logs local to that machine.

Supported agent backends:

- Codex
- Pi

Not affiliated with OpenAI or the Pi Coding Agent project. "Codex" and "Pi" are
referenced only for CLI compatibility.

## Platform support

Supported:

- Linux (PTYs, `/proc`-based Codex log discovery)
- macOS (PTYs, `pgrep`/`lsof` discovery paths)

Not supported:

- Windows native terminals. Use WSL2 for a Linux environment.

## Quick start

Requires a Rust toolchain and the `codex` and/or `pi` CLI you want to control.
Python is not a supported backend runtime after Phase 6; it is used only as an
optional test runner/static-assets packaging helper.

Build the server and broker binaries:

```sh
cargo build --manifest-path backend-rs/Cargo.toml --release --bins
```

If you are running from a source checkout, build the web app before starting the
server:

```sh
cd web
npm install
npm run build
```

`npm run build` writes the Vite production bundle to `web/dist/` and copies it to
`codoxear/static/dist/`, which `codoxear-backend-rs` serves. The Rust server
refuses to start from a source checkout when this production bundle is missing or
when `codoxear/static/dist/index.html` is still the Vite source index.

1. Create `.env` or export environment variables:

   - Set `CODEX_WEB_PASSWORD` (required).
   - Optional: set `CODEX_WEB_HOST` (default `::`) and `CODEX_WEB_PORT` (default `8743`).

2. Start the Rust server from the repository root or a deployment directory that
   can locate `codoxear/static`:

   ```sh
   ./backend-rs/target/release/codoxear-backend-rs
   ```

   Default bind is `[::]:8743`.

3. Add separate wrappers for terminal-owned brokered sessions (zsh/bash
   functions, not aliases):

   ```sh
   codox() {
     /path/to/codoxear-broker-rs -- "$@"
   }

   piox() {
     CODEX_WEB_AGENT_BACKEND=pi /path/to/codoxear-broker-rs -- "$@"
   }
   ```

   Do not wrap or replace `codex()` or `pi()` themselves. Web-owned sessions
   launch the underlying CLI directly through `codoxear-broker-rs`; wrapping the
   original command can recurse back into the broker.

4. Use `codox` for terminal-owned Codex sessions and `piox` for terminal-owned
   Pi sessions when you want them registered with Codoxear. Leave plain `codex`
   and `pi` unwrapped.

5. On your phone, open `http://<your-computer>:8743`, enter the password, and
   select or create a session.

## Runtime model

- `codoxear-backend-rs` is the only supported HTTP server.
- `codoxear-broker-rs` is the only supported broker/session launcher.
- `/api/v1/*` is the canonical API namespace.
- `/api/*` remains a legacy compatibility alias served by the Rust server.
- Runtime state lives under `~/.local/share/codoxear` by default. Set
  `CODOXEAR_APP_DIR` to override it for tests or isolated deployments.
- Existing pre-Phase-6 sidecars and JSON state files are still readable by Rust.
  New sidecars are written by `codoxear-broker-rs`.

There is no Python fallback. Legacy Python entry points are not part of the supported runtime after Phase 6.

## Configuration

Set these in `.env` or the process environment:

- `CODEX_WEB_PASSWORD` (required)
- `CODEX_WEB_HOST` (default `::`)
- `CODEX_WEB_PORT` (default `8743`)
- `CODOXEAR_BIND_HOST` / `CODOXEAR_BIND_PORT` (fallback aliases when the
  `CODEX_WEB_*` bind variables are unset)
- `CODEX_WEB_URL_PREFIX` (default empty). Example: `/codoxear` serves the UI at
  `/codoxear/` and the API under `/codoxear/api/v1/*` and `/codoxear/api/*`.
- `CODEX_WEB_DEFAULT_AGENT_BACKEND` (default `codex`)
- `CODEX_HOME` (default `~/.codex`)
- `CODEX_BIN` (default `codex`)
- `PI_HOME` (default `~/.pi`)
- `PI_BIN` (default `pi`)
- `CODEX_WEB_COOKIE_TTL_SECONDS` (default `2592000`, 30 days)
- `CODEX_WEB_COOKIE_SECURE` (default `0`; set to `1` behind HTTPS)
- `CODEX_WEB_HARNESS_SWEEP_SECONDS` (default `2.5`)
- `CODEX_WEB_QUEUE_SWEEP_SECONDS` (default `1.0`)
- `CODEX_WEB_QUEUE_IDLE_GRACE_SECONDS` (default `10.0`)
- `CODEX_WEB_DISCOVER_MIN_INTERVAL_SECONDS` (default `1.0`)
- `CODEX_WEB_METRICS_WINDOW` (default `256`)
- `CODEX_WEB_FILE_READ_MAX_BYTES` (default `2097152`)
- `CODEX_WEB_FILE_HISTORY_MAX` (default `20`)
- `CODEX_WEB_GIT_DIFF_MAX_BYTES` (default `819200`)
- `CODEX_WEB_GIT_DIFF_TIMEOUT_SECONDS` (default `4.0`)
- `CODEX_WEB_GIT_CHANGED_FILES_MAX` (default `400`)
- `CODEX_WEB_BRANCH_TIMEOUT_S` (default `2.0`)
- `CODEX_WEB_PR_TIMEOUT_S` (default `4.0`)
- `CODEX_WEB_GH_AUTH_TTL_S` (default `300`)
- `CODEX_WEB_FD_POLL_SECONDS` (default `1.0`)
- `CODOXEAR_ENABLE_QUEUE_SWEEP=1` to enable the Rust queue worker
- `CODOXEAR_ENABLE_HARNESS_SWEEP=1` to enable the Rust harness worker
- `CODOXEAR_ENABLE_VOICE_SCAN=1` to enable voice scan
- `CODOXEAR_ENABLE_VOICE_WORKER=1` to enable voice delivery/HLS/WebPush

Worker flags remain individually opt-in. Disabling a flag disables that Rust side
effect; it does not restore a Python worker.

## Tailscale HTTPS

Browser notifications and iOS Web Push require HTTPS (or localhost). One simple
setup is Tailscale Serve on port `8443`:

```sh
tailscale serve --bg --yes --https=8443 http://127.0.0.1:8743
```

Then open:

```text
https://<device>.<tailnet>.ts.net:8443/
```

Systemd user service example:

```ini
[Service]
ExecStart=/path/to/codoxear-backend-rs
ExecStartPost=/usr/bin/tailscale serve --bg --yes --https=8443 http://127.0.0.1:8743
ExecStopPost=-/usr/bin/tailscale serve --bg --yes --https=8443 off
```

## Session ownership

Codoxear shows:

- Terminal-owned sessions started through `codox` or `piox`.
- Web-owned sessions started from the UI.
- Web-owned tmux sessions started from the UI with `Create in tmux` enabled.

Delete sends a shutdown request to the Rust broker. Deleting a terminal-owned
session stops the corresponding terminal session.

## Frontend development

1. Start the Rust server:

   ```sh
   ./backend-rs/target/release/codoxear-backend-rs
   ```

2. Start Vite:

   ```sh
   cd web
   npm install
   npm run dev
   ```

3. Open the Vite URL for frontend work; `/api/*` proxies to the Rust server.
4. Build production assets with `npm run build` before handing off UI changes.

## Verification

Local full check:

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

## Rollback

Phase 6 has no environment-flag rollback to Python. Operational rollback is a
git revert of the Phase 6 commit(s) to the Phase 5 PASS baseline (or a descendant
known-good commit), followed by the Phase 5 verification suite. Do not delete
`~/.local/share/codoxear` state files during rollback.

## Security model

Codoxear provides password gating only and does not provide TLS by itself.
Assume anyone who can reach the plain HTTP port can observe or modify traffic.
Use VPN, SSH port-forwarding, Tailscale HTTPS, or another TLS reverse proxy for
network security.

## License

MIT, see `LICENSE`.
