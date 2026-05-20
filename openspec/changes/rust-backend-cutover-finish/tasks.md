## 0. Pre-flight and scope guard

- [x] 0.1 Confirm the implementation branch is based on Phase 5 code-reviewer PASS commit `8f489b0` or a descendant; record `git rev-parse --short HEAD` in the handoff.
- [x] 0.2 Run and record baseline validation before deleting Python: `openspec validate rust-backend-cutover-finish --strict`, `openspec validate rust-backend-cutover --strict`, `openspec validate rust-backend-voice-push --strict`.
- [x] 0.3 Run a focused Rust baseline: `cargo test --manifest-path backend-rs/Cargo.toml --release --test voice_worker_phase5 final_response_summary_error` and `cargo test --manifest-path backend-rs/Cargo.toml --release --test broker_contract_phase4` (or the closest current selectors) to prove Phase 4/5 fixes are present.
- [x] 0.4 Generate an active Python runtime inventory with `find codoxear -maxdepth 2 -type f -name '*.py'`, `rg 'codoxear\.(server|broker|pi_broker|sessiond|voice_push|rollout_log|pi_log|pi_rpc|agent_backend|git_context|util|pty_util)' tests scripts README.md AGENTS.md docs/cutover pyproject.toml .github -S`, and save the actionable result in the implementation handoff or `docs/cutover/cutover-log.md`.
- [x] 0.5 Keep scope limited to Phase 6 final cutover: do not remove `/api/*` legacy alias, do not refactor Rust runtime architecture beyond what is required by Python deletion, and do not change frontend UX.

## 1. Python runtime surface removal

- [x] 1.1 Delete `codoxear/server.py`, `codoxear/broker.py`, `codoxear/pi_broker.py`, `codoxear/sessiond.py`, and `codoxear/voice_push.py`.
- [x] 1.2 Delete Python backend helper modules that are only used by the removed runtime (`agent_backend.py`, `git_context.py`, `rollout_log.py`, `pi_log.py`, `pi_messages.py`, `pi_rpc.py`, `pty_util.py`, `util.py`) after confirming no retained static packaging/test path imports them.
- [x] 1.3 Keep `codoxear/static/`, `codoxear/pi_extensions/ask_user_bridge.ts`, image assets, and `codoxear/__init__.py` only if still needed by Rust broker/server packaging or docs.
- [x] 1.4 Remove Python `__pycache__`/runtime artifacts from git tracking if any appear, and confirm `.gitignore` still excludes `backend-rs/target/`, runtime app dirs, socks, logs, secrets.
- [x] 1.5 Run `rg 'python -m codoxear\.(server|broker|pi_broker|sessiond)|codoxear-server|codoxear-broker|codoxear-pi-broker' README.md AGENTS.md docs/cutover pyproject.toml .github scripts -S` and update every active runtime reference to Rust binaries.

## 2. Packaging and entry points

- [x] 2.1 Update `pyproject.toml` to remove `codoxear-server`, `codoxear-broker`, and `codoxear-pi-broker` Python console scripts.
- [x] 2.2 Remove Python runtime dependencies (`Pillow`, `py-vapid`, `pywebpush`) unless a retained non-runtime packaging/test path explicitly needs them; if retained, document why they are not backend runtime dependencies.
- [x] 2.3 Decide and implement the final Python package role: either static-assets-only package data or no runtime wheel path; document the choice in README and handoff.
- [x] 2.4 Ensure Rust binary build instructions are canonical: `cargo build --manifest-path backend-rs/Cargo.toml --release --bins` produces `codoxear-backend-rs` and `codoxear-broker-rs`.
- [x] 2.5 If any install wrapper/script is added for Rust binaries, keep it shell/Rust based and avoid reintroducing Python runtime wrappers.

## 3. Rust server/broker final-state checks

- [x] 3.1 Verify Rust `POST /api/sessions` always selects `codoxear-broker-rs` by default after Phase 6 and does not fall back to `python -m codoxear.broker` / `python -m codoxear.pi_broker` when `CODOXEAR_RUST_BROKER_BIN` is unset.
- [x] 3.2 Add or update Rust tests for broker selection final state: Codex create default uses Rust broker, Pi create default uses Rust broker with `--session-file`, tmux inline command uses Rust broker, and setting `CODOXEAR_RUST_BROKER_BIN` can override the Rust broker path without enabling Python fallback.
- [x] 3.3 Verify `codoxear-broker-rs` terminal-owned Codex and Pi paths remain documented as replacements for `codox` / `piox` wrappers.
- [x] 3.4 Confirm Rust parses pre-Phase-6 Python-written sidecars and voice files using existing Phase 4/5 compatibility tests; keep those tests active even after Python modules are deleted by using fixtures rather than imports of removed modules.
- [x] 3.5 Confirm `/api/v1/*` canonical and `/api/*` legacy alias route sets still cover endpoint inventory rows; add a route-table or contract test if an alias can regress silently.

## 4. Test cleanup and migration

- [x] 4.1 Delete Python unit tests that import removed backend modules (`tests/test_*server*.py`, `tests/test_*broker*.py`, `tests/test_voice_push.py`, `tests/test_pi_*.py`, etc.) unless they are rewritten to target Rust binaries or static fixtures.
- [x] 4.2 Migrate any still-critical behavior coverage from deleted Python tests into `backend-rs/tests/` or `tests/contract/` before deleting the original test.
- [x] 4.3 Update `tests/contract` fixtures so `python_server_url` is removed, skipped, or renamed to historical fixture data; active parity tests must target Rust server behavior, not a live Python server.
- [x] 4.4 Update pytest markers / selectors in `pyproject.toml` and CI so they do not reference removed Python backend tests.
- [x] 4.5 Run `rg 'from codoxear\.(server|broker|pi_broker|sessiond|voice_push|rollout_log|pi_log|pi_rpc|agent_backend|git_context|util|pty_util)|import codoxear\.(server|broker|pi_broker|sessiond|voice_push|rollout_log|pi_log|pi_rpc|agent_backend|git_context|util|pty_util)' tests backend-rs scripts -S` and ensure no active test imports deleted modules.

## 5. Documentation cutover

- [x] 5.1 Update `README.md` Quick start, Configuration, Frontend development, Session ownership, Tailscale/systemd, Known limitations, and rollback sections to describe Rust-only server/broker operation.
- [x] 5.2 Update `AGENTS.md` Components, Local dev, Ops notes, and safe restart examples to use `codoxear-backend-rs` and `codoxear-broker-rs` only.
- [x] 5.3 Update `docs/cutover/disk-contracts.md` with a Phase 6 note: schemas stay compatible, but Python writers/readers are no longer shipped.
- [x] 5.4 Create or update `docs/cutover/cutover-log.md` with rows for Phase 0–6: date, change id, commit/PR if known, env flags safe/default, rollback status, reviewer result; Phase 6 row must state rollback is git revert.
- [x] 5.5 Update `openspec/changes/rust-backend-cutover/tasks.md` Phase 6 and cross-phase rows to reflect this change id and final verification status.
- [x] 5.6 Do not rewrite historical `docs/superpowers/plans/*` unless they are linked from active quick-start docs; if left unchanged, exclude them from runtime grep gates in handoff.

## 6. CI and build workflow

- [x] 6.1 Update `.github/workflows/backend-rs.yml` to remove language implying Python backend installation; Python may remain only as pytest runner for contract/static tests.
- [x] 6.2 Ensure CI runs `cargo fmt`, `cargo clippy`, `cargo test --release`, `cargo build --release --bins`, and the Rust source line-count gate.
- [x] 6.3 Ensure CI or documented local verification runs contract/static tests that remain valid after Python deletion.
- [x] 6.4 Ensure frontend verification remains present: `cd web && npm run test` (or the current test command) and `cd web && npm run build` before handoff.
- [x] 6.5 If macOS broker subset remains in CI from Phase 4, keep it running against Rust-only code; otherwise document the current macOS validation status in cutover log.

## 7. Smoke and compatibility verification

- [x] 7.1 Run `openspec validate rust-backend-cutover-finish --strict`, `openspec validate rust-backend-cutover --strict`, `openspec validate rust-backend-broker --strict`, and `openspec validate rust-backend-voice-push --strict`.
- [x] 7.2 Run `cargo fmt --manifest-path backend-rs/Cargo.toml --all -- --check`.
- [x] 7.3 Run `cargo clippy --manifest-path backend-rs/Cargo.toml --all-targets -- -D warnings`.
- [x] 7.4 Run `cargo test --manifest-path backend-rs/Cargo.toml --release`.
- [x] 7.5 Run `cargo build --manifest-path backend-rs/Cargo.toml --release --bins`.
- [x] 7.6 Run the retained contract suite, e.g. `pytest tests/contract -q`, with no dependency on a live Python server.
- [x] 7.7 Run `cd web && npm run test` and `cd web && npm run build` if web package scripts are available.
- [x] 7.8 Run grep gates: no active runtime docs mention Python server/broker commands; no active tests import deleted Python modules; no source references `VoicePushCoordinator`, `SessionManager` from Python, or `codoxear.server` except historical docs explicitly called out.
- [x] 7.9 Smoke Rust server startup with a temporary app dir: login/auth, `/api/v1/health`, `/api/v1/me`, `/api/v1/sessions/bootstrap`, `/api/sessions` legacy alias, static `/`, and one HLS/notification readonly endpoint.
- [x] 7.10 Smoke core session lifecycle with fake or real Codex/Pi binaries as available: create session, list, send/enqueue, interrupt/delete, broker socket cleanup.
- [x] 7.11 Smoke worker ownership flags: queue/harness disabled no side effects, queue/harness enabled one sweep, voice scan/worker disabled no side effects, voice enabled with mock/fake external services if real devices are unavailable.
- [x] 7.12 Document real iOS/Tailscale WebPush/HLS smoke status. If not run, state the same accepted reason from Phase 5 (no real iOS Home Screen app, Tailscale HTTPS device, or browser push endpoint credentials) and list automated substitutes.

## 8. Review handoff

- [x] 8.1 Summarize removed Python files, retained Python/static files, packaging decision, and all verification commands/results in the issue comment.
- [x] 8.2 Confirm Phase 6 did not remove `/api/*` alias and did not change frontend UX.
- [x] 8.3 Confirm rollback instructions in README/cutover log say git revert, not env flag fallback.
- [x] 8.4 Switch issue to `in_review`, assign to `code-reviewer`, and request review of the Phase 6 final cutover.
- [x] 8.5 Independent `code-reviewer` review completes with PASS before parent issue PEI-46 can be closed.
