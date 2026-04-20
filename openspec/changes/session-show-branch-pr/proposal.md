## Why

Each session in codoxear runs against a `cwd` that is typically a git working tree, but the session UI does not surface which branch the session is on or whether a related GitHub pull request exists. Users juggling many sessions today must `cd` into each repo or open another tool to remember "is this the feature branch with the open PR, or main?". Surfacing branch + PR context in the session row lets the user identify and switch between sessions without leaving the web UI.

## What Changes

- Add a backend helper that resolves, for a session's `cwd`, the current git branch and any associated GitHub pull request (number, title, state, URL) using the `gh` CLI when available.
- Expose this information via a new `GET /api/sessions/<id>/repo` endpoint and include a lightweight summary (`git_branch`, `pr_number`, `pr_state`) in the session list payload that already powers the sidebar.
- Render branch and PR badges in the session card / sidebar entry so users can see them without opening the session.
- Cache results per `cwd` with a short TTL and refresh on session selection / explicit refresh; never block session list rendering on a slow `gh` call.
- Degrade gracefully when `cwd` is not a git repo, when `gh` is missing, or when the user is not authenticated to GitHub.

## Capabilities

### New Capabilities
- `session-repo-context`: Resolves and exposes git branch and GitHub PR information for the working directory of each session, and renders it in the session list UI.

### Modified Capabilities
<!-- None: no existing openspec/specs/ entries today. -->

## Impact

- Backend: `codoxear/server.py` (new `/api/sessions/<id>/repo` route, augmented session list payload), new helper module for `gh`/git probing with caching.
- Frontend: `web/src/lib/types.ts`, `web/src/lib/api.ts`, and `web/src/components/sessions/SessionCard.tsx` / `SessionGroup.tsx` to render the new fields; tests under `web/src/components/sessions/`.
- Dependencies: requires `gh` CLI on the host for PR lookup; PR data is best-effort and absent when `gh` is unavailable or unauthenticated.
- Performance: adds at most one short subprocess call per unique `cwd`, gated by a TTL cache to keep session-list polling cheap.
