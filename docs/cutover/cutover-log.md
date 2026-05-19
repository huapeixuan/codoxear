# Rust backend cutover log

| Date (UTC) | Phase / change | Commit / PR | Runtime state | Rollback status | Reviewer status |
|---|---|---|---|---|---|
| 2026-05 | Phase 0 / `rust-backend-cutover` | historical umbrella | Endpoint inventory, disk contracts, and contract harness established. Python runtime still primary. | Additive docs/tests; rollback by reverting Phase 0 docs/tests. | Accepted in later phase baseline. |
| 2026-05 | Phase 1 / `rust-backend-skeleton` | historical | Rust skeleton served health/me/bootstrap preview. Python runtime still primary. | Additive Rust preview; rollback by not running Rust binary or reverting Phase 1. | Accepted in Phase 1 review. |
| 2026-05 | Phase 2 / `rust-backend-readonly-routes` | historical | Rust read-only route parity expanded. Python runtime still primary. | Additive/parallel; rollback by routing back to Python. | Accepted in Phase 2 review. |
| 2026-05 | Phase 3 / `rust-backend-write-routes` | historical | Rust write routes and queue/harness handoff landed behind opt-in worker flags. | Stop Rust worker, unset matching flag, restart Python at that phase. | Accepted in Phase 3 review. |
| 2026-05 | Phase 4 / `rust-backend-broker` | historical | `codoxear-broker-rs` introduced and selectable for web-owned sessions. | At Phase 4, unset `CODOXEAR_RUST_BROKER_BIN`; after Phase 6 this rollback is no longer available. | Accepted in Phase 4 review. |
| 2026-05 | Phase 5 / `rust-backend-voice-push` | PR #9 / `8f489b0` PASS baseline | Rust voice scan/delivery/HLS/WebPush implemented behind opt-in flags; Python fallback still present before Phase 6. | Stop Rust, unset voice flags, restart Python at Phase 5. | code-reviewer PASS; real iOS/Tailscale smoke remained manual pending. |
| 2026-05-20 | Phase 6 / `rust-backend-cutover-finish` | this branch | Rust-only runtime. Python backend/broker/sessiond/voice modules and console scripts removed. `/api/v1/*` and `/api/*` remain served by Rust. | **Git revert only** to Phase 5 PASS baseline/descendant; no env flag restores Python fallback. | Pending code-reviewer. |

## Phase 6 inventory summary

Pre-flight active Python runtime inventory found these runtime modules before
removal: `server.py`, `broker.py`, `pi_broker.py`, `sessiond.py`, `voice_push.py`,
`agent_backend.py`, `git_context.py`, `rollout_log.py`, `pi_log.py`,
`pi_messages.py`, `pi_rpc.py`, `pty_util.py`, `util.py`, and `constants.py`.
Active references existed in README, AGENTS, pyproject console scripts, CI
contract setup, and Python unit tests. Phase 6 removes those modules/tests and
keeps only `codoxear/__init__.py`, `codoxear/static/`, and
`codoxear/pi_extensions/ask_user_bridge.ts` under the Python package tree.

Python package role after Phase 6: static-assets-only metadata/package data for
source builds/tests. It exposes no runtime console scripts and declares no Python
backend runtime dependencies.

## Manual smoke status

Automated substitutes cover Rust startup, auth, route aliases, pre-cutover
sidecars, Rust broker fake Codex/Pi flows, WebPush/TTS/HLS mock paths, and
readonly HLS/notification endpoints. Real iOS Home Screen WebPush over Tailscale
HTTPS remains manual pending for the same accepted reason as Phase 5: the local
coding agent has no real iOS Home Screen app, Tailscale HTTPS device, or browser
push endpoint credentials.
