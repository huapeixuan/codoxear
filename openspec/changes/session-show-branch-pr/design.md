## Context

Sessions are discovered from `~/.local/share/codoxear/socks/*.json` and exposed through `GET /api/sessions`. The server already computes `git_branch` for the session payload via `_current_git_branch(cwd)` (`codoxear/server.py:2137`) and returns it as part of the session row, but the React workspace UI (`web/src/components/sessions/SessionCard.tsx`, `SessionGroup.tsx`) does not render it. There is no existing mechanism for surfacing GitHub PR state for a session's `cwd`. Session rows are repolled frequently for sidebar updates, so any new lookup must be cheap.

## Goals / Non-Goals

**Goals:**
- Show the current git branch for each session row in the workspace sidebar.
- Show whether the cwd has an open or recently-merged GitHub PR for the current branch, with number and state.
- Provide a richer detail endpoint (`/api/sessions/<id>/repo`) that the active session header / detail view can call for branch + full PR metadata (title, URL, draft, mergeable).
- Keep the session list endpoint cheap: branch + minimal PR summary only, served from a TTL cache.
- Degrade silently when `cwd` is missing, not a git repo, or `gh` is not available / not authenticated.

**Non-Goals:**
- Creating, updating, or merging pull requests from the UI.
- Showing CI / check-run details, comments, or reviews.
- Supporting non-GitHub remotes (GitLab, Bitbucket) in this change. The data shape will allow extension later.
- Real-time push updates; data refreshes on poll, on session selection, and on explicit refresh.

## Decisions

### Decision 1: Use `gh` CLI rather than the GitHub REST/GraphQL API directly
- **What**: Resolve PR metadata by shelling out to `gh pr view --json number,title,state,url,isDraft,mergeable,headRefName --jq .` from the session's `cwd`.
- **Why**: `gh` already handles auth (token, SSO, enterprise hosts), rate limiting, and remote URL parsing. The codebase has no GitHub HTTP client today, and shelling out to a small set of git/gh commands matches the existing pattern used for `git diff` and `git rev-parse`.
- **Alternatives considered**: 
  - Direct REST calls with `requests` and `GH_TOKEN` env — adds an HTTP dependency, requires hand-rolling host detection from `git remote get-url`, and forces us to manage credentials.
  - Parsing `git config --get remote.origin.url` + cached PAT — same downsides as REST.

### Decision 2: New helper module `codoxear/git_context.py`
- **What**: Add a small module that exposes `resolve_repo_context(cwd: Path) -> RepoContext` returning a frozen dataclass with `git_branch`, `pr` (optional dict), `has_remote`, and `error_kind`. Keep `_current_git_branch` as a thin re-export so existing callers don't move.
- **Why**: `server.py` is already very large (~9k lines). A single-purpose module keeps the new logic testable in isolation and makes it easy to mock subprocess calls in pytest.
- **Alternatives considered**: Inline functions in `server.py` — rejected because we want fast unit tests and parallel testability; existing git helpers in `server.py` are difficult to mock in isolation.

### Decision 3: TTL cache keyed by `cwd`
- **What**: In-process `dict[Path, (RepoContext, expires_at)]` with a default TTL of 30 seconds for branch (cheap) and 120 seconds for PR (gh CLI call). Cache busts on session selection and on an explicit `?refresh=1` query param.
- **Why**: Session list polling is frequent (multi-second cadence). Without caching, every poll fans out to multiple `git`/`gh` subprocesses per session. 30s/120s TTL is short enough that a branch switch is reflected within a poll or two, but long enough to keep `gh` calls rare.
- **Alternatives considered**: 
  - No cache — slow and forks subprocesses per poll per session.
  - File-system mtime cache on `.git/HEAD` — works for branch but not for PR state; adds complexity.

### Decision 4: Session list payload carries only a minimal PR summary
- **What**: Augment the session-list row with `git_branch` (already present) plus a new optional `pr_summary: { number: int, state: "OPEN"|"CLOSED"|"MERGED"|"DRAFT" } | null`. Full PR detail (title, URL, mergeable, etc.) lives only on `/api/sessions/<id>/repo`.
- **Why**: Keeps the polled payload small and avoids leaking long PR titles into the sidebar when the user has many sessions. The detail endpoint is fetched on demand by the active session view.
- **Alternatives considered**: Putting full PR details in the list — rejected on payload size and on principle that polling endpoints should stay slim.

### Decision 5: Soft failures degrade to "no PR" without surfacing errors in the sidebar
- **What**: Distinguish three failure modes internally: `not_a_repo`, `no_gh`, `gh_error` (auth, network, no PR found). The list endpoint exposes only branch + optional `pr_summary`. The detail endpoint returns a small `availability` field (`"ok" | "no-gh" | "no-pr" | "not-a-repo" | "error"`) so the UI can show a one-line hint in the detail panel only.
- **Why**: We do not want the sidebar to flash error badges for sessions where PR state is simply unavailable. Detail view is the right place to explain "PR lookup unavailable: gh CLI not installed".

## Risks / Trade-offs

- **Risk**: `gh` CLI subprocess calls block the request thread.
  - **Mitigation**: Wrap calls with a strict `timeout_s` (e.g. 4s, matching existing `GIT_DIFF_TIMEOUT_SECONDS` style) and return `gh_error` on timeout. TTL cache absorbs repeat cost.
- **Risk**: Many concurrent sessions on the same `cwd` could trigger thundering-herd `gh` calls when the cache expires.
  - **Mitigation**: Per-`cwd` `threading.Lock` so only one resolution runs at a time; other callers wait for or reuse the in-flight result.
- **Risk**: PR data leak — PR titles can contain confidential branch names.
  - **Mitigation**: Same trust boundary as existing session metadata (any user with the `codoxear_auth` cookie already sees `cwd`, file contents, and git diffs). No new exposure.
- **Risk**: `gh` auth state varies per host user; running as the server process may not have a token.
  - **Mitigation**: Detect missing auth (`gh auth status` non-zero) once at startup and short-circuit subsequent PR lookups for the session lifetime, refreshing periodically. Surface as `availability: "no-gh"`.
- **Trade-off**: We ship GitHub-only support first. The `RepoContext` dataclass and `pr_summary` field shape are deliberately generic enough to allow a `host` field later, but other hosts will require a follow-up change.

## Migration Plan

- **Forward**: Pure additive change — new endpoint, additive fields on existing payload, additive UI badges. No schema migrations.
- **Rollback**: Remove the new endpoint, drop the `pr_summary` field from the list payload, and revert UI badges. Existing `git_branch` remains untouched.
- **Compatibility**: Older clients that ignore unknown fields (current React UI does this) keep working unchanged during rollout.

## Open Questions

- Should the `pr_summary` show the *target* branch's PR (e.g. when user is on a detached HEAD or on a non-PR branch), or only PRs whose head is the current branch? Initial implementation: only the current branch's PR. Revisit if users ask for "PR I'm reviewing on this branch".
- Should we cache `gh auth status` globally or per-cwd? Initial: global with a 5-minute refresh, since `gh` auth is a per-OS-user concern.
