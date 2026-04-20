# Pi tmux takeover design

**Date:** 2026-04-17

## Goal

Make Pi sessions created with Codoxear's `Create in tmux` mode genuinely take-overable from a local terminal while preserving the existing `pi-rpc` web control plane.

The user should be able to click a `Take over in tmux` button in the web UI. Codoxear should first try to open a local terminal and attach to the correct tmux session/window automatically. If that is unavailable or fails, the UI should immediately fall back to showing an exact attach command that can be copied and run manually.

## Problem

Today, `Create in tmux` for Pi creates a tmux-hosted `pi_broker` process, but the feature behaves more like "host the broker in tmux for observability" than "create a tmux session I can later take over".

Observed issues:

- Attaching to the shared tmux session often lands in an apparently empty shell window instead of a clearly take-overable live Pi terminal bridge.
- The current UI wording for Pi explicitly says the session is hosted in tmux while `pi-rpc` handles web control, which matches the implementation but not the user's expectation of terminal takeover.
- There is no first-class API or UI action for takeover. Users have to infer the right `tmux attach` command and then hunt for the correct window.
- The backend records tmux metadata, but the product does not turn that metadata into a reliable takeover workflow.

## Desired semantics

We are targeting the following behavior for Pi sessions launched with `Create in tmux`:

1. `pi_broker` remains the single runtime owner of the live Pi session.
2. `pi-rpc` remains the canonical structured web control plane.
3. The tmux window becomes a durable terminal bridge to that live broker session, not merely a host shell for observability.
4. The user can later take over from a terminal by attaching to the correct tmux session/window and continuing interaction.
5. The web UI exposes a one-click takeover action that first tries to open a terminal automatically, then falls back to an exact attach command.

This is intentionally **not** a requirement to recreate Pi's native full-screen terminal UI exactly. The requirement is a live, reliable, interactive terminal bridge that supports entering prompts, seeing output, and interrupting the current turn while preserving RPC capabilities.

## Non-goals

The first iteration does not attempt to:

- embed a real tmux client inside the browser
- replace `pi-rpc` with PTY text injection
- make tmux the only or authoritative control plane
- guarantee automatic terminal opening for every terminal emulator on every platform
- perfectly emulate every behavior of Pi's native interactive TUI
- solve takeover for terminal-owned non-tmux Pi sessions or Codex sessions

## Current-state findings

Relevant code and tests show the present contract clearly:

- `web/src/components/new-session/NewSessionDialog.tsx` describes Pi tmux mode as hosting the session in tmux while `pi-rpc` handles web control.
- `web/src/components/new-session/NewSessionDialog.test.tsx` asserts that exact wording.
- `codoxear/pi_broker.py` writes metadata with `transport: "pi-rpc"` even when tmux metadata is present.
- `codoxear/server.py` launches Pi tmux sessions by spawning `codoxear.pi_broker` inside a tmux window and recording `CODEX_WEB_TMUX_SESSION` plus `CODEX_WEB_TMUX_WINDOW`.
- Existing server tests validate launch-in-tmux behavior but do not cover any takeover capability.

The gap is therefore not "tmux launch is absent" but "takeover semantics and takeover tooling are absent".

## Architecture direction

### 1. Keep `pi_broker` as the single owner

`pi_broker` already owns:

- Pi RPC subprocess lifecycle
- prompt submission
- abort behavior
- live output sync
- pending UI request state
- socket commands for web control

That ownership should stay intact. We should not split Pi runtime semantics across separate tmux and web session controllers.

### 2. Promote tmux from observability host to takeover target

For web-owned Pi sessions created in tmux, the tmux window should be treated as a durable foreground terminal bridge backed by `pi_broker`.

That means:

- the tmux window must continue to run a long-lived foreground `pi_broker`
- terminal stdin in the tmux pane must keep feeding broker prompt submission
- broker stdout must continue to stream live output into the tmux pane
- terminal `Ctrl+C` should continue to map to broker abort behavior

The essential invariant is:

- **one live broker, two clients**
  - tmux terminal view
  - web RPC client

Clients render or submit through the broker; they do not own independent session state.

### 3. Add a first-class takeover API

The backend should expose a takeover-specific capability instead of forcing the UI to infer commands from raw metadata.

Add a read endpoint for eligible sessions:

- `GET /api/sessions/<id>/takeover`

Suggested response shape:

```json
{
  "ok": true,
  "eligible": true,
  "backend": "pi",
  "transport": "pi-rpc",
  "tmux_session": "codoxear",
  "tmux_window": "codoxear-047445",
  "attach_command": "tmux attach -t codoxear \\; select-window -t codoxear:codoxear-047445",
  "auto_open_supported": true,
  "auto_open_methods": ["terminal"],
  "terminal_launch_command": ["osascript", "... platform specific ..."]
}
```

For ineligible or degraded sessions, return structured reasons, for example:

```json
{
  "ok": true,
  "eligible": false,
  "reason": "tmux window is unavailable"
}
```

Eligibility rules for the first version:

- session is owned by web
- backend is `pi`
- transport is `pi-rpc`
- `tmux_session` and `tmux_window` are present
- the broker socket is still alive
- the tmux window still exists

### 4. Add an auto-open action

Add a write endpoint:

- `POST /api/sessions/<id>/takeover/open`

Behavior:

1. re-resolve takeover descriptor server-side
2. if not eligible, return structured failure
3. attempt to launch a local terminal on the host machine that runs the exact attach command
4. return success/failure plus the same attach command for fallback display

Example success:

```json
{
  "ok": true,
  "opened": true,
  "attach_command": "tmux attach -t codoxear \\; select-window -t codoxear:codoxear-047445"
}
```

Example failure:

```json
{
  "ok": true,
  "opened": false,
  "reason": "no supported GUI terminal launcher found",
  "attach_command": "tmux attach -t codoxear \\; select-window -t codoxear:codoxear-047445"
}
```

## Attach-command strategy

The attach command must target the exact session and window, not only the shared session name.

Preferred command form:

```bash
tmux attach -t <session> \; select-window -t <session>:<window>
```

Rationale:

- attaching only to `codoxear` may land in the wrong active window
- the window name already exists in broker metadata
- generating the command on the server centralizes escaping and compatibility

If needed later, pane targeting can be added, but the first version can treat one tmux window as the takeover unit.

## Host terminal auto-open strategy

### Supported behavior

The web button should attempt to open a terminal on the same machine running Codoxear and execute the attach command.

### Platform policy

- **macOS:** support AppleScript / `osascript` and optionally `open -a Terminal` style launchers.
- **Linux GUI hosts:** detect a small allowlist of common terminal launchers such as `x-terminal-emulator`, `gnome-terminal`, `kitty`, `wezterm`, `ghostty`, or `konsole` when installed.
- **Headless Linux or unsupported hosts:** skip auto-open and return a structured fallback.

### Product rule

Automatic terminal opening is best-effort only. The action is considered usable as long as failure is explicit and the exact attach command is immediately available to copy.

## Frontend interaction design

### Button placement

Add a `Take over in tmux` action for eligible Pi tmux sessions in the session details surface and/or active session toolbar.

The button should only appear when the server says the session is eligible for takeover.

### Click flow

1. User clicks `Take over in tmux`
2. Frontend requests the takeover descriptor or uses already-fetched capability data
3. Frontend calls `takeover/open`
4. If success:
   - show a small success notice such as "Opened terminal and selected tmux window"
   - keep a `Copy command` affordance available anyway
5. If failure:
   - show a dialog or popover with the failure reason
   - show the exact attach command
   - provide a one-click copy action

### UX principle

The UI should never pretend takeover succeeded. Success must mean the server actually launched a terminal process successfully. Otherwise the user gets the fallback command immediately.

## Session-list and capability model

The current session row metadata may continue to expose `tmux_session` and `tmux_window`, but the frontend should not infer takeover readiness from those raw fields alone.

Instead, add an explicit capability such as:

- `can_takeover_in_tmux: true | false`
- optional `takeover_reason_unavailable`

This lets the UI avoid showing a misleading action for:

- dead broker sessions
- missing tmux windows
- non-Pi sessions
- non-tmux Pi sessions

A full takeover descriptor endpoint still remains useful for the exact command and host-launch details.

## Runtime and concurrency semantics

The first version should allow both web and tmux interaction to coexist through the same broker.

Invariants:

- the broker remains the single source of truth for busy state and pending requests
- tmux-submitted prompts and web-submitted prompts both go through broker prompt submission
- `Ctrl+C` in tmux continues to abort the active turn through broker abort semantics
- web-side `ask_user` continues to use structured `ui_response`

Out of scope for the first version:

- hard locking the session to one client after takeover
- conflict arbitration beyond existing broker busy behavior
- remote notification that "someone is typing in tmux right now"

If dual-input confusion later becomes a product problem, a future phase can add advisory status or soft locks.

## Backend implementation outline

### `codoxear/server.py`

Add takeover helpers that:

- validate whether a session is takeover-eligible
- verify that the recorded tmux window still exists
- generate the exact attach command
- detect supported host terminal launchers
- launch the host terminal when requested

Potential helper boundaries:

- `_session_takeover_descriptor(session)`
- `_tmux_window_exists(session_name, window_name)`
- `_build_tmux_attach_command(session_name, window_name)`
- `_detect_terminal_launcher()`
- `_build_terminal_open_argv(attach_command)`

Add routes:

- `GET /api/sessions/<id>/takeover`
- `POST /api/sessions/<id>/takeover/open`

### `codoxear/pi_broker.py`

Keep the current foreground-terminal bridge as the core implementation, but review and harden the assumptions that matter for durable tmux takeover:

- ensure stdin EOF or transient terminal conditions do not accidentally destroy the intended takeover target too eagerly
- ensure stdout streaming continues cleanly for long-lived tmux hosting
- keep signal/abort behavior reliable after later tmux re-attach

If needed, add small metadata or health signals to make server-side eligibility checks more accurate, but do not change `transport` away from `pi-rpc`.

### `web` frontend

Add session action UI and API client methods for takeover descriptor and open action.

Possible touch points:

- API client types and methods
- session row / active session capability mapping
- active session toolbar or session details pane
- dialog/popover for fallback command display and copy action

## Testing strategy

### Backend tests

Add coverage for:

- descriptor generation for eligible Pi tmux sessions
- ineligible responses for missing broker, missing tmux metadata, missing tmux window, or wrong backend/transport
- exact attach-command formatting
- auto-open success path with mocked launcher
- auto-open fallback path when no supported launcher exists

### Frontend tests

Add coverage for:

- `Take over in tmux` visibility only for eligible sessions
- click success path showing success state
- click failure path showing fallback command and copy affordance
- no misleading action shown for ineligible sessions

### Integration/behavior tests

At minimum, verify that sessions created with Pi `Create in tmux` expose takeover metadata/capability and that the UI can consume it without inferring raw tmux fields incorrectly.

## Rollout plan

### Phase 1

- add takeover descriptor and open endpoints
- add exact attach-command generation
- add frontend button and fallback UI
- support common host launchers on macOS and common Linux desktops

### Phase 2 if needed

- improve launcher coverage per terminal emulator
- add richer session-side takeover status
- add advisory client-presence signals if concurrent use becomes confusing

## Risks

- Host terminal launching is platform-specific and may fail often on unusual setups.
- `tmux_window` metadata alone may become stale unless validated live before use.
- Dual web + tmux input can be surprising if the user interacts from both places at once.
- There may be edge cases where the tmux-hosted broker exits and the stored metadata outlives the real takeover target.

These risks are acceptable for the first version as long as the UI exposes explicit failure and always provides the exact fallback command.

## Acceptance criteria

The feature is complete when all of the following are true:

1. A Pi session created with `Create in tmux` can expose a server-validated takeover descriptor.
2. The web UI shows `Take over in tmux` only for eligible sessions.
3. Clicking that action attempts to open a host terminal automatically.
4. If auto-open fails, the user immediately sees a precise copyable attach command.
5. The attach command selects the correct tmux window, not merely the tmux session.
6. The Pi session remains controlled by `pi_broker` and `pi-rpc`; web UI capabilities continue to work.
7. Tests cover descriptor generation, auto-open fallback behavior, and frontend visibility/interaction states.
