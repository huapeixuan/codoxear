## ADDED Requirements

### Requirement: Session list payload exposes git branch
The session list endpoint (`GET /api/sessions`) SHALL include a `git_branch` field for every session row whose `cwd` resolves to a git working tree.

#### Scenario: Session cwd is a git repo on a named branch
- **WHEN** a session's `cwd` resolves to a git repository checked out on a named branch
- **THEN** the session row in `GET /api/sessions` MUST include `git_branch` set to the branch name (e.g. `"feature/login"`)

#### Scenario: Session cwd is a git repo in detached HEAD
- **WHEN** a session's `cwd` resolves to a git repository in a detached HEAD state
- **THEN** the session row MUST include `git_branch` set to either the short commit SHA or `null`, and MUST NOT raise an error

#### Scenario: Session cwd is not a git repository
- **WHEN** a session's `cwd` does not exist or is not inside a git working tree
- **THEN** `git_branch` MUST be `null` and the session row MUST still be returned

### Requirement: Session list payload exposes pull-request summary
The session list endpoint SHALL include an optional `pr_summary` field for every session row whose `cwd` is a GitHub-backed repository with an associated pull request for the current branch.

#### Scenario: Branch has an open pull request
- **WHEN** a session's current branch has an open GitHub pull request
- **THEN** `pr_summary` MUST be an object with `number` (integer) and `state` (one of `"OPEN"`, `"DRAFT"`, `"CLOSED"`, `"MERGED"`)

#### Scenario: Branch has no associated pull request
- **WHEN** a session's current branch has no associated pull request
- **THEN** `pr_summary` MUST be `null`

#### Scenario: GitHub CLI is unavailable or unauthenticated
- **WHEN** the `gh` CLI is not installed, not authenticated, or its lookup times out
- **THEN** `pr_summary` MUST be `null` and the session list response MUST still succeed

### Requirement: Session repo detail endpoint
The system SHALL provide `GET /api/sessions/<session_id>/repo` returning detailed repo + PR context for a single session.

#### Scenario: Session has an open PR with full metadata
- **WHEN** the client calls `GET /api/sessions/<id>/repo` for a session whose branch has an open PR
- **THEN** the response MUST include `git_branch`, `availability: "ok"`, and `pr` with `number`, `title`, `state`, `url`, `is_draft`, and `head_ref_name`

#### Scenario: Session repo lookup forced refresh
- **WHEN** the client calls `GET /api/sessions/<id>/repo?refresh=1`
- **THEN** the server MUST bypass the cache for that `cwd` and re-resolve branch and PR data before responding

#### Scenario: Session repo unavailable due to missing gh CLI
- **WHEN** the host has no `gh` CLI installed
- **THEN** the response MUST set `availability: "no-gh"`, populate `git_branch` if a git repo is present, and set `pr` to `null`

#### Scenario: Session cwd is not a git repository
- **WHEN** the session's `cwd` is not a git working tree
- **THEN** the response MUST set `availability: "not-a-repo"`, with `git_branch: null` and `pr: null`

#### Scenario: Unknown session id
- **WHEN** the session id is not found
- **THEN** the response MUST be HTTP 404 with a JSON `{ "error": "session not found" }` body

### Requirement: Repo context resolution is cached and time-bounded
The system SHALL cache repo context per `cwd` and bound the cost of any single lookup so polling the session list does not block on git or network operations.

#### Scenario: Repeated lookup within TTL
- **WHEN** repo context for a `cwd` is requested twice within the configured TTL
- **THEN** the second request MUST be served from cache without invoking `git` or `gh` subprocesses

#### Scenario: External tool exceeds timeout
- **WHEN** a `git` or `gh` subprocess does not return within the configured per-call timeout
- **THEN** the lookup MUST be aborted, the result MUST be treated as unavailable for that field, and no exception MUST propagate to the HTTP response

#### Scenario: Concurrent lookups for the same cwd
- **WHEN** multiple requests trigger a cache miss for the same `cwd` simultaneously
- **THEN** at most one resolution subprocess pipeline MUST run concurrently for that `cwd`, and other waiters MUST receive its result

### Requirement: Workspace UI surfaces branch and PR badges
The workspace session UI SHALL render branch and PR information for each session row when the data is available, and SHALL omit the badges when it is not.

#### Scenario: Session row with branch and open PR
- **WHEN** the session payload includes `git_branch` and `pr_summary` with state `"OPEN"`
- **THEN** the session card MUST display a branch badge (e.g. with a branch icon and the branch name) and a PR badge showing `#<number>` styled as "open"

#### Scenario: Session row with branch but no PR
- **WHEN** the session payload includes `git_branch` but `pr_summary` is `null`
- **THEN** the session card MUST display only the branch badge and MUST NOT show a PR badge

#### Scenario: Session row without git context
- **WHEN** the session payload has `git_branch: null` and `pr_summary: null`
- **THEN** the session card MUST NOT show a branch or PR badge and MUST keep its existing layout

#### Scenario: PR badge click opens the PR URL
- **WHEN** the user clicks the PR badge on a session card
- **THEN** the UI MUST open the PR URL (resolved via the detail endpoint) in a new browser tab and MUST NOT change the active session
