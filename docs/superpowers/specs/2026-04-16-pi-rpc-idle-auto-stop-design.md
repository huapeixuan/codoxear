# Pi RPC Idle Auto-Stop Design

**Date:** 2026-04-16

## Goal

Automatically stop web-owned `pi-rpc` sessions after a period with no web interaction, while preserving the session log so the conversation naturally remains available in history.

## User Intent

The user wants inactive web Pi sessions to stop lingering in the background.

Approved preferences:

- apply only to `Pi` sessions using `pi-rpc`
- do not log the browser out
- do not delete the session log or historical messages
- after timeout, the session should disappear from the live list and remain available as history
- default idle timeout is `30 minutes`

## Problem

Today Codoxear has two separate concepts of liveness:

- browser authentication is represented by the `codoxear_auth` cookie
- a web-owned Pi session is represented by a live `pi-rpc` broker process and its socket metadata

The cookie already expires eventually, but the Pi RPC process does not stop merely because the browser is no longer interacting with it. That creates the wrong semantic result for web-owned sessions:

- the user closes the tab or stops interacting
- the Pi RPC broker and child process keep running
- the session remains live instead of becoming historical

The repository already has a safe shutdown path:

- `codoxear.server` can delete a session through `delete_session()`
- the delete flow already reaches broker `shutdown`
- broker shutdown stops the managed Pi RPC process
- the session log is preserved on disk

So the missing piece is not process termination itself. The missing piece is a reliable definition of web inactivity and a sweep that converts inactivity into a normal session shutdown.

## Approved Direction

Implement server-enforced idle auto-stop for web-owned `pi-rpc` sessions, driven by explicit web activity timestamps refreshed by both strong interaction endpoints and a lightweight frontend heartbeat.

The design keeps four invariants:

1. only web-owned `Pi` `pi-rpc` sessions participate
2. inactivity is defined by lack of web interaction, not by whether the agent is still producing output
3. timeout uses the existing broker shutdown path rather than a second termination mechanism
4. stopping the live broker must not remove the durable session log, so the session remains discoverable as history

## Scope

### In Scope

- web-owned sessions with `backend == "pi"`
- sessions whose transport is `pi-rpc`
- a per-session web activity timestamp stored in server memory
- frontend heartbeat for the active viewed session
- server sweep that stops timed-out sessions
- a global timeout setting with default `1800` seconds
- regression tests for activity refresh, timeout sweep, and historical retention

### Out of Scope

- changing cookie authentication behavior
- stopping terminal-owned Pi sessions
- stopping Codex sessions
- adding per-session timeout UI controls in the first version
- deleting logs or historical session metadata
- introducing websocket or SSE push transport just for idle tracking

## Design Principles

1. **Web interaction defines session liveness**
   - The auto-stop rule exists to clean up abandoned browser-driven sessions.
   - Agent output alone must not keep a session alive.

2. **Shutdown semantics should stay singular**
   - The server already knows how to stop a session safely.
   - Idle cleanup should reuse that existing path.

3. **History must survive live teardown**
   - Session termination only removes the live runtime.
   - Durable logs remain the source of truth for history.

4. **The browser may hint; the server decides**
   - The frontend reports continued viewing through heartbeat.
   - The server is responsible for authoritative timeout enforcement.

## Current State

### Authentication

`codoxear/server.py` issues the `codoxear_auth` cookie using `_set_auth_cookie()`. That cookie has a TTL, but it is not tied to any single live session.

### Session lifecycle

`codoxear.server` maintains live sessions in memory and already knows how to stop them with `delete_session()`. For Pi RPC sessions, shutdown ultimately reaches the broker and then `PiRpcClient.close()`.

### Historical visibility

Pi history is read from the durable session file, not from the live broker process. Once the broker exits, the session can still be rendered historically as long as the file remains available.

### Missing state

There is currently no session-level notion of:

- when the browser last interacted with a given session
- whether that session should be auto-stopped on inactivity
- a dedicated heartbeat endpoint for passive viewing

## Activity Model

The core design decision is to treat "session is still in use" as a web-session property, not a process property.

### Strong activity

These requests refresh the session's web activity timestamp because they mean the user is actively interacting with the session:

- `POST /api/sessions/<id>/send`
- `POST /api/sessions/<id>/enqueue`
- `POST /api/sessions/<id>/ui_response`
- `POST /api/sessions/<id>/interrupt`

### Passive viewing activity

The frontend sends a heartbeat only for the currently selected session when that session is eligible for idle auto-stop and the page is visible.

Heartbeat exists so that reading an active conversation without sending messages still counts as interaction.

### Non-activity

The following do not extend idle lifetime:

- broker-side streamed output
- background queue processing by the agent
- historical session reads
- global session list polling
- unrelated API traffic for another session

This is intentional. The requested behavior is to stop the Pi RPC process when the user stops interacting with that web session, even if the agent could otherwise continue working.

## Eligibility Rules

A session is eligible for idle auto-stop only when all of the following are true:

- `owned is True`
- `backend == "pi"`
- `transport == "pi-rpc"`
- `auto_stop_on_idle is True`
- `idle_timeout_seconds > 0`

All other sessions are ignored by the idle sweep.

This keeps terminal-owned sessions and non-Pi backends outside the feature by construction.

## Server Data Model

Extend the live `Session` model in `codoxear/server.py` with the following fields:

- `last_web_activity_ts: float | None`
- `idle_timeout_seconds: int | None`
- `auto_stop_on_idle: bool`
- `idle_stop_reason: str | None` optional for diagnostics and tests

### Initialization

When a new eligible web Pi RPC session is spawned:

- `auto_stop_on_idle = True`
- `idle_timeout_seconds = CODEX_WEB_PI_RPC_IDLE_TIMEOUT_SECONDS` or the default `1800`
- `last_web_activity_ts = now`

When sessions are re-discovered from metadata after a server restart, missing values should be reconstructed conservatively:

- if the live session still qualifies, initialize `last_web_activity_ts` to `now`
- do not attempt to infer inactivity from stale browser state

This favors not killing a just-rediscovered session immediately after server restart.

## API Design

### `POST /api/sessions/<id>/heartbeat`

Purpose:

- refresh `last_web_activity_ts` for an eligible active session
- allow passive viewing to keep the session alive

Behavior:

- requires normal auth
- returns `404` for unknown sessions
- returns `409` when the session does not support idle heartbeat semantics, for example non-Pi or non-`pi-rpc`
- refreshes `last_web_activity_ts`
- returns timeout metadata for optional future UI hints

Recommended response shape:

```json
{
  "ok": true,
  "session_id": "...",
  "idle_timeout_seconds": 1800,
  "last_web_activity_ts": 1760000000.0
}
```

### Existing write-like session endpoints

These endpoints should also refresh `last_web_activity_ts` for eligible sessions before they perform their main action:

- `POST /api/sessions/<id>/send`
- `POST /api/sessions/<id>/enqueue`
- `POST /api/sessions/<id>/ui_response`
- `POST /api/sessions/<id>/interrupt`

This keeps strong interaction and passive viewing on the same timing model.

## Sweep Design

The server already performs periodic background maintenance. Idle cleanup should be implemented as another sweep pass in `codoxear/server.py` rather than a new long-lived subsystem.

### Sweep interval

Reuse an existing periodic manager loop cadence if practical. A separate interval is unnecessary because timeout precision only needs to be within a few seconds.

### Sweep rule

For every live session:

1. ignore sessions that are not eligible
2. read `last_web_activity_ts`
3. if `now - last_web_activity_ts < idle_timeout_seconds`, do nothing
4. otherwise stop the session using the normal `delete_session()` path

### Timeout action

Timeout does not perform a custom kill sequence. It calls the same session deletion path already used elsewhere, which:

- asks the broker to shut down
- terminates the managed process if needed
- clears live in-memory state
- preserves the log file on disk

### Post-timeout semantics

After a successful timeout stop:

- the session no longer appears as a live broker-backed session
- the session log remains readable
- existing historical discovery continues to surface it as history

This is the desired "live to history" transition.

## Frontend Design

The frontend owns only activity reporting, not timeout enforcement.

### Heartbeat conditions

Send heartbeat only when all of the following are true:

- the page is visible
- there is an active selected session
- the active session is Pi-backed and live
- the active session indicates idle auto-stop support

### Heartbeat cadence

Send heartbeat approximately every `60 seconds` while the above conditions remain true.

This is far below the idle timeout, cheap to serve, and tolerant of timer jitter.

### Stop conditions

Stop heartbeats when:

- the user switches to another session
- the page becomes hidden
- the session leaves the live list
- the session is no longer eligible for idle auto-stop

### Optional UI hint

The first version does not require a visible countdown or warning banner.

However, the API should return `idle_timeout_seconds` so a later enhancement can warn the user before auto-stop without changing the contract.

## Concurrency and Race Semantics

The design intentionally keeps the race rules simple.

### Recent activity wins

If a request refreshes `last_web_activity_ts` shortly before the sweep evaluates the session, the updated timestamp extends the timeout window and the session survives.

### Timeout is based on web activity, not busy state

A busy agent does not delay timeout.

This is not a bug. It is the explicit semantic choice requested by the user: if no web interaction happens for `30 minutes`, the Pi RPC session should stop even if the agent is still capable of producing output.

### No second durability state machine

The idle feature must not introduce a separate "timing out", "graceful shutdown pending", or "soft idle" state model unless implementation needs demonstrate it is unavoidable. The existing delete path already provides the required shutdown boundary.

## Configuration

Add a new environment variable:

- `CODEX_WEB_PI_RPC_IDLE_TIMEOUT_SECONDS`

Rules:

- default: `1800`
- `0` disables the feature globally
- negative values are invalid and should fall back to the default or raise at startup

This keeps the first version operationally simple while allowing deployments to opt out.

## Error Handling

### Heartbeat on unsupported session

Return `409` with an explanatory error. The frontend can then stop attempting heartbeat for that session.

### Heartbeat after timeout or deletion

Return `404`. The frontend should treat this as the session no longer being live and refresh session state.

### Shutdown failure

If the timeout sweep fails to stop a session, log the failure and retry on the next sweep. Do not mark the session historical until the live deletion path actually succeeds.

## Testing Strategy

### Server tests

Add coverage for:

- session eligibility classification
- activity refresh on `send`, `enqueue`, `ui_response`, and `interrupt`
- heartbeat refreshing `last_web_activity_ts`
- timeout sweep ignoring unsupported sessions
- timeout sweep calling normal deletion for expired eligible sessions
- timed-out session preserving durable history behavior

### Frontend tests

Add coverage for:

- heartbeat starts only for the selected eligible live session
- heartbeat pauses when the document becomes hidden
- heartbeat pauses when the active session changes
- heartbeat stops after `404` or `409` responses

### Regression tests

Verify the user-visible contract:

- an idle Pi RPC session disappears from the live set after timeout
- the same session still appears in history and its messages remain readable

## Alternatives Considered

### Frontend-only timeout

Rejected because browser throttling, tab suspension, and mobile backgrounding make the browser an unreliable enforcer.

### Agent-output-based lifetime

Rejected because it does not match the requested semantic invariant. The user asked for no-interaction timeout, not no-output timeout.

### Cookie logout tied to session timeout

Rejected because the user explicitly wants only the Pi RPC session stopped, not the whole web login cleared.

## Rollout Plan

1. add server-side eligibility fields and timestamp refresh helpers
2. add the heartbeat endpoint
3. integrate timeout sweep into existing manager maintenance
4. wire frontend heartbeat for the selected eligible live session
5. add tests for both server and frontend behavior

## Open Questions

None for the first version. The timeout policy, target session type, and desired post-timeout behavior were explicitly agreed during brainstorming.
