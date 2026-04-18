# Session List View Modes

## Goal

Add a user-selectable session-list view mode toggle so the sidebar can switch between:

- `Directories`: the current grouped-by-working-directory presentation
- `Recent`: a flat list sorted by most recently updated sessions

The selected mode should persist in the browser so the UI restores the user's last choice on reload.

## User Intent

The current working-directory grouping is useful for project-oriented navigation, but sometimes the user wants a simple global timeline of sessions regardless of directory.

The sidebar should support both mental models:

- project view: group related sessions by working directory
- timeline view: show the freshest sessions first, regardless of project

## Current State

### Frontend

`web/src/components/sessions/SessionsPane.tsx` currently renders a grouped sidebar by:

- deriving groups from the flat `items` array using `session.cwd`
- rendering `SessionGroup` for each working directory bucket
- using cwd-group metadata for label, collapse, and hide behavior

`web/src/domains/sessions/store.ts` currently assumes a directory-grouped pagination model:

- `remainingByGroup`
- `omittedGroupCount`
- `loadMoreGroup()`
- `loadMoreGroups()`

### Backend

`/api/sessions` currently serves the grouped sidebar model.

The response shape and pagination behavior are directory-oriented:

- sessions are returned in the current sidebar order
- extra response fields support grouped rendering and grouped incremental loading
- frontend callers do not currently ask for alternate list semantics

## Desired Behavior

### View toggle

The session sidebar should provide a small toggle control with two modes:

- `Directories`
- `Recent`

The toggle should be visible near the session list header and should feel like a presentation setting for the sidebar, not a global app setting.

### Persisted preference

The selected mode should be stored in browser local storage and restored on page load.

Rules:

- missing preference defaults to `Directories`
- invalid stored values fall back to `Directories`
- this preference is frontend-only and does not need server persistence

### Directories mode

This mode preserves the current behavior:

- group sessions by normalized `cwd`
- show cwd labels, subtitles, collapse state, and hide behavior
- keep group rename and collapse interactions
- keep group-based pagination behavior
- keep the existing `SessionGroup` rendering path

### Recent mode

This mode changes the list semantics:

- do not group by working directory
- show a single flat list of sessions
- order sessions by most recent update, descending
- paginate by sessions globally, not by directory group
- show a `Load more sessions` action when more recent rows are available

Recent mode does not remove cwd information entirely. Each session card should still expose enough cwd context to let the user recognize which project a session belongs to.

## Recommended Approach

Implement view-mode semantics in the backend API instead of treating this as a frontend-only reshuffle.

This approach is preferred because:

- `Recent` becomes a true global recent-session feed, not a flattened snapshot of currently loaded groups
- pagination semantics stay aligned with the selected view
- the backend defines one authoritative recent-ordering rule
- the frontend can remain a view consumer instead of inventing its own sorting truth
- future view modes can be added with the same `view=` contract

## API Design

### Extend `GET /api/sessions`

Add a query parameter:

- `view=directories|recent`

Defaults:

- missing `view` means `directories`

Validation:

- any other value returns `400`

### `Directories` response semantics

Keep the existing grouped-list semantics and response shape as the default behavior.

Expected response still includes:

- `sessions`
- `remaining_by_group`
- `omitted_group_count`

Allowed query parameters remain:

- `groupKey`
- `groupOffset`
- `groupLimit`

This mode should preserve backward compatibility for current callers.

### `Recent` response semantics

Return a flat list of globally recent sessions.

Suggested response shape:

```json
{
  "ok": true,
  "sessions": [...],
  "remaining": 12
}
```

Allowed query parameters:

- `offset`
- `limit`

Disallowed parameters:

- `groupKey`
- `groupOffset`
- `groupLimit`

If a caller supplies grouped-pagination parameters while `view=recent`, the backend should return `400` instead of silently ignoring them. This keeps the contract explicit and prevents accidental mixed semantics.

## Ordering Rules

### Directories mode

Keep the current grouped ordering behavior:

- preserve backend-emitted session ordering inside groups
- preserve current group freshness behavior

This feature must not regress existing group order semantics.

### Recent mode

Define one backend-owned recency key for flat-list sorting.

Recommended timestamp priority:

1. `updated_ts`
2. fallback to `last_activity_ts` if present in the session row model
3. fallback to `start_ts`

The exact rule should be implemented once in the backend and reused consistently.

The frontend should not compute an alternate sort order for recent mode.

## Backend Design

### Separate raw session collection from view projection

Do not overload one function with both grouped and recent pagination semantics.

Preferred structure:

- one internal layer produces normalized session rows with the fields needed by the sidebar
- one projection path builds the grouped `Directories` response
- one projection path builds the flat `Recent` response
- the `/api/sessions` handler validates `view` and dispatches accordingly

This avoids entangling two incompatible pagination models in one implementation path.

### Directories projection

This path should continue to support:

- cwd grouping
- hidden/revealed groups
- per-group pagination
- omitted group counts
- existing cwd-group metadata interactions

No behavioral change is intended beyond making the mode explicit.

### Recent projection

This path should:

- start from the same normalized session row source
- sort by the backend-defined recency rule
- apply global `offset`/`limit`
- return `remaining` based on the number of undisclosed rows

`cwd_groups` metadata should not shape the recent response.

## Frontend Design

### Store state

Update `web/src/domains/sessions/store.ts` to track the selected view mode.

Suggested additions:

- `viewMode: "directories" | "recent"`
- a setter such as `setViewMode()`
- recent-specific pagination state, separate from grouped pagination bookkeeping

Do not force grouped and recent pagination to share the same counters.

The store should maintain separate pagination state for:

- directories-mode grouped loading
- recent-mode flat loading

This avoids state pollution when the user switches back and forth.

### API client

Update `web/src/lib/api.ts` so `listSessions()` can send:

- `view`
- `offset`
- `limit`
- existing group parameters for directories mode

The client should not send group parameters in recent mode.

### UI rendering

Update `web/src/components/sessions/SessionsPane.tsx` to render one of two paths:

- `Directories`: current grouped rendering with `SessionGroup`
- `Recent`: flat list of `SessionCard`

The toggle should:

- be visible near the pane header
- switch the current mode immediately
- persist the selection to local storage
- trigger a refresh using the selected backend view

### Session cards in recent mode

Reuse the existing `SessionCard` behavior and actions.

Session interactions must remain unchanged:

- select
- edit
- delete
- duplicate
- resume historical session

If the current card layout does not clearly expose cwd context, add a lightweight cwd subtitle or metadata line so recent mode remains navigable across projects.

## Selection Behavior

Switching views should not unnecessarily disrupt the active session.

Rules:

- keep the current `activeSessionId` when possible
- if the active session is still present in the fetched rows, keep it selected
- switching views should not automatically redirect the user to a different session unless the current selection no longer exists

This prevents the sidebar from feeling unstable when the user changes the presentation mode.

## Error Handling

### Backend

- unknown `view` returns `400`
- grouped pagination parameters with `view=recent` return `400`
- malformed offsets or limits continue to follow existing validation rules

### Frontend

- invalid persisted `viewMode` falls back to `directories`
- if a recent-mode fetch fails, keep the last rendered state visible and surface an error message
- toggling view modes should not clear the pane before the replacement data is ready unless existing store behavior already requires it

## Testing

### Backend tests

Add coverage for:

- default `/api/sessions` behavior still matching `directories`
- `view=recent` returning a flat recent response
- recent-mode ordering by the canonical recency rule
- `view=recent` rejecting grouped-pagination query parameters
- invalid `view` returning `400`

### Frontend store tests

Add coverage for:

- reading the persisted view mode from local storage
- falling back to `directories` on invalid persisted values
- requesting `view=recent` after mode switch
- preserving independent pagination state for directories and recent modes
- preserving active selection when switching modes

### Frontend component tests

Add coverage for:

- rendering grouped sessions in `Directories`
- rendering a flat session list in `Recent`
- showing the view toggle
- persisting the selected mode
- showing `Load more sessions` in recent mode
- ensuring group controls do not appear in recent mode

## Risks

- If recent-mode sorting duplicates but does not match the backend's current sidebar freshness logic, users may see inconsistent ordering across views.
- If grouped and recent pagination state share the same counters, switching modes may produce missing or duplicated rows.
- If recent mode does not show enough cwd context, the flat list may become harder to scan across projects.
- If the backend silently accepts mixed recent/group query parameters, future regressions will be harder to diagnose.

## Out of Scope

This design does not include:

- changing session-card visual design beyond small metadata adjustments needed for recent mode
- adding more view modes beyond `Directories` and `Recent`
- server-side persistence of the user's preferred view mode
- replacing cwd-group metadata or cwd-group editing semantics
- broader session-sidebar redesign

## Acceptance Criteria

- The session sidebar exposes a `Directories` / `Recent` toggle.
- The selected mode persists across page reloads in browser storage.
- `Directories` mode preserves current grouped-by-working-directory behavior.
- `Recent` mode shows a flat list ordered by backend-defined recency.
- `Recent` mode uses global session pagination, not group pagination.
- Invalid `view` values and recent/group mixed parameters return `400`.
- Switching modes does not break existing session-card actions.
- Group rename, collapse, and hide remain functional in `Directories` mode.
- The implementation is covered by backend and frontend tests.
- `cd web && npm run build` succeeds after the implementation.
