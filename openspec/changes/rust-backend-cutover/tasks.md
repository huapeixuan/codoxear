> Tasks below are intentionally split between two granularities:
>
> - Sections 1–2 are the **Phase 0 deliverables for this umbrella change** itself: an endpoint inventory and a contract-test harness skeleton. Both should be done before any Rust code is written, so each subsequent phase change can mark off real work.
> - Sections 3–9 are **phase pointers**: each phase becomes its own future OpenSpec change (names listed in design.md). The single checkbox per phase is "open the OpenSpec change and meet its Definition of Done"; do **not** treat these as 2-hour tasks. Coding-agent should pick up Phase 1 first.

## 1. Phase 0 deliverables — endpoint inventory

- [x] 1.1 Generate `docs/cutover/endpoint-inventory.md` listing, for each `/api/...` route in `codoxear/server.py`: method, full path pattern, query params consumed, JSON request fields parsed, response shape (top-level keys), auth requirement (`_require_auth`), and the file:line range of the handler. Generate by mechanically grepping `path == "/api/...` and `path.startswith("/api/sessions/") and path.endswith(...)` in `do_GET` / `do_POST`; do not paraphrase — include the exact match expression so reviewers can re-derive the inventory.
- [x] 1.2 In the same document add a "ref-comparison" column that maps each row to the corresponding handler in `san-tian/codoxear@san-tian-dev:backend-rs/src/routes.rs`; flag rows present in target but missing in ref (huapeixuan-only features such as PR badge, image composer drafts, context usage display, hooks notify) and create a `huapeixuan-only-deltas` subsection enumerating them with rationale "must be ported into Phase X".
- [x] 1.3 Write `docs/cutover/disk-contracts.md` listing every JSON file under `~/.local/share/codoxear/` that the Python implementation reads or writes, with: filename, top-level schema (keys + types), atomic-write strategy (`os.replace` vs `os.rename` vs append-only), file mode, and the python module that owns writes. This is the contract the Rust implementation must round-trip.
- [x] 1.4 Validate the inventory by spot-checking: pick 5 random routes from `endpoint-inventory.md`, verify each row's handler line range still matches the file, and have a second reviewer (`coding-agent` or human) sign off in the doc footer.

## 2. Phase 0 deliverables — contract-test harness skeleton

- [x] 2.1 Create `tests/contract/__init__.py` and `tests/contract/conftest.py` with two pytest fixtures: `python_server_url` (spawns `python -m codoxear.server` on a random port with a tmp `~/.local/share/codoxear` dir, yields the URL, terminates cleanly) and `rust_server_url` (skipped with `pytest.skip` until Phase 1 ships, but the fixture signature is final).
- [x] 2.2 Write three skipped placeholder tests in `tests/contract/test_endpoint_parity.py` covering `/api/me`, `/api/sessions/bootstrap`, `/api/sessions`. Each test should use the fixture pattern that will become `assert_json_equivalent(python_response, rust_response, ignore_keys={"app_version"})` so the shape is locked in now even though the Rust side is `pytest.skip`.
- [x] 2.3 Add `tests/contract/README.md` explaining how to run the suite once Rust is available: `cargo build --release --bins -C backend-rs`, then `pytest tests/contract -k parity`. Document expected env vars and fail-modes.
- [x] 2.4 Wire `tests/contract` into the existing pytest discovery (no special CI run yet — they all skip until Phase 1) and run `pytest tests/contract` to confirm zero collection errors.

## 3. Phase 1 — Rust skeleton + 3 read-only endpoints

- [ ] 3.1 Open OpenSpec change `rust-backend-skeleton` (delta capability `rust-backend-runtime`, scope: skeleton + `/api/v1/health`, `/api/v1/me`, `/api/v1/sessions/bootstrap`, both legacy `/api/...` aliases). Definition of Done: `cargo test` green; the three placeholder contract tests in `tests/contract/test_endpoint_parity.py` pass when run against `rust_server_url`; CI builds the crate; README documents `cargo build --release --bins`.

## 4. Phase 2 — Read-only routes parity

- [ ] 4.1 Phase 2 owner change is `rust-backend-readonly-routes` (covers all GET endpoints from inventory § 1.1; updates spec scenarios for messages tail/history/live, queue read, harness GET, git diff/changed_files/file_versions, file read/search/blob, settings/voice GET, notifications GET, metrics, sessions list, session_resume_candidates). Definition of Done: every GET row in `docs/cutover/endpoint-inventory.md` has a green parity contract test; 30 minutes of dual-server polling on a real workstation surfaces zero JSON diff (excluding documented ignore-keys).

## 5. Phase 3 — Write routes parity + worker handoff

- [x] 5.1 Open OpenSpec change `rust-backend-write-routes` (covers POST endpoints from inventory § 1.1: session create/edit/rename/delete, send, enqueue, queue mutate, harness POST, ui_response, heartbeat, interrupt, login/logout, settings save, notifications subscribe/toggle/test_push, audio listener/test_announcement, file write, files inspect/blob, hooks notify, cwd_groups edit). Same change must update `codoxear/server.py:SessionManager` so Python sweeps decline to start when the matching `CODOXEAR_ENABLE_*` flag is set, satisfying spec scenario "Both Rust and Python workers are never co-active". Phase 3 validation status: `pytest tests/contract -q -k 'parity and post'` passes; Rust voice debug/test POSTs remain explicit Phase 5 `501` feature-disabled side-effect routes.

## 6. Phase 4 — Broker port

- [x] 6.1 Open OpenSpec change `rust-backend-broker` (port `codoxear/broker.py` + `codoxear/pi_broker.py` to `backend-rs/src/broker.rs` + bin `codoxear-broker-rs`; introduce `CODOXEAR_RUST_BROKER_BIN` plumbing in `codoxear/server.py:SessionManager._spawn_broker`). Phase 4 coding status: `CODOXEAR_RUST_BROKER_BIN` selection is wired for Python/Rust session-create paths and `codoxear-broker-rs` delegates to the compatible Python broker implementation while Rust broker contract modules/tests land incrementally; full native PTY/Pi RPC replacement and macOS real-broker CI remain reviewer-visible follow-up scope before Phase 6 default cutover.

## 7. Phase 5 — Voice push port

- [ ] 7.1 OpenSpec change `rust-backend-voice-push` is in implementation. Partial coding status: Rust `voice_worker` scaffolding, `web-push`/VAPID public-key spike, HLS artifact serving routes, ledger trim helper, Python delivery-thread handoff, and single-writer lock tests are present; full scan loop, OpenAI TTS, WebPush send, HLS ffmpeg append, and real-device smoke remain before DoD. Definition of Done: real iOS device receives a notification through the Rust path on Tailscale HTTPS; ledger written by Rust is readable by Python and vice versa; concurrent-write advisory lock prevents double delivery.

## 8. Phase 6 — Remove Python backend

- [ ] 8.1 Open OpenSpec change `rust-backend-cutover-finish`. Delete `codoxear/server.py`, `codoxear/broker.py`, `codoxear/pi_broker.py`, `codoxear/voice_push.py`, `codoxear/sessiond.py`, `codoxear/pi_*.py`, `codoxear/rollout_log.py`, `codoxear/agent_backend.py`, `codoxear/git_context.py`, `codoxear/util.py`, `codoxear/pty_util.py`, and the matching `tests/test_*.py`. Update `pyproject.toml` to either drop Python entry points or repurpose the wheel as static-asset-only. Update `README.md`/`AGENTS.md` to describe Rust-only deploy. Definition of Done: only `codoxear/static/` remains under `codoxear/`; `git grep -l 'codex_web\|VoicePushCoordinator'` returns 0 matches; no broken imports.

## 9. Cross-phase verification

- [ ] 9.1 At the end of every phase change, append one row to `docs/cutover/cutover-log.md` (date, phase, env flags now safe to enable in production, rollback verified ✅/❌, link to PR). The umbrella change is "complete" only when this log is full through Phase 6.
- [ ] 9.2 Before archiving this umbrella change, confirm: (a) `openspec list` shows zero remaining `rust-backend-*` changes in flight; (b) `coding-agent` has run the contract suite and posted a green CI link; (c) `go-reviewer` is not used because there is no Go code, but `rust-reviewer` has signed off on the final state of `backend-rs/`.
