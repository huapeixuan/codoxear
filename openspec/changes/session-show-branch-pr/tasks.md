## 1. Backend repo-context helper

- [x] 1.1 Create `codoxear/git_context.py` with a frozen `RepoContext` dataclass (`git_branch`, `pr`, `availability`) and a `resolve_repo_context(cwd: Path, *, refresh: bool = False) -> RepoContext` entry point
- [x] 1.2 Implement branch resolution by reusing `_run_git` with a strict timeout; handle detached HEAD (return short SHA or None)
- [x] 1.3 Implement PR resolution via `gh pr view --json number,title,state,url,isDraft,mergeable,headRefName` with timeout, capturing stderr to classify `no-gh`, `no-pr`, `error`
- [x] 1.4 Add a TTL cache (branch 30s, PR 120s) keyed by resolved absolute `cwd`, with a per-`cwd` lock to prevent thundering herds
- [x] 1.5 Cache `gh auth status` globally with a 5-minute TTL; short-circuit PR lookup to `availability="no-gh"` when unauthenticated
- [x] 1.6 Add unit tests in `tests/test_git_context.py` covering: named branch, detached HEAD, non-repo cwd, missing `gh`, `gh pr view` returning no-PR, `gh` timeout, cache hit/miss, refresh flag, concurrent lookups

## 2. Backend API surface

- [x] 2.1 Refactor `_current_git_branch` in `codoxear/server.py` to delegate to `git_context.resolve_repo_context(...).git_branch` so existing callers stay backwards compatible
- [x] 2.2 Augment the session-list payload builder to include `pr_summary: { number, state } | null` derived from `RepoContext.pr`; ensure absent fields serialize as `null`
- [x] 2.3 Add route `GET /api/sessions/<id>/repo` that returns `{ git_branch, pr, availability }`; honor `?refresh=1` and return 404 for unknown session ids
- [x] 2.4 Update existing pytest coverage for the session list endpoint to assert the new fields and the new endpoint contract (mock `git_context.resolve_repo_context`)

## 3. Frontend types and API client

- [x] 3.1 Extend `web/src/lib/types.ts` `SessionSummary` with `pr_summary?: { number: number; state: "OPEN" | "DRAFT" | "CLOSED" | "MERGED" } | null`
- [x] 3.2 Add `SessionRepoDetail` type and `fetchSessionRepoDetail(sessionId, { refresh })` helper in `web/src/lib/api.ts`
- [x] 3.3 Update `web/src/lib/api.test.ts` with the new helper (success, refresh flag, 404)

## 4. Frontend session card UI

- [x] 4.1 Add a small `BranchBadge` component (with branch icon) to `web/src/components/sessions/` and render it in `SessionCard.tsx` when `session.git_branch` is set
- [x] 4.2 Add a `PrBadge` component that renders `#<number>` styled by state (OPEN, DRAFT, MERGED, CLOSED), and integrate it in `SessionCard.tsx` when `session.pr_summary` is present
- [x] 4.3 Make the PR badge a clickable link that fetches `repo` detail on first hover/click to obtain `pr.url`, then opens it in a new tab without changing the active session
- [x] 4.4 Update `web/src/components/sessions/SessionsPane.test.tsx` (or add a focused `SessionCard.test.tsx`) to cover: branch + PR badge present, branch only, neither, click opens URL in new tab, no badge regression for cards without git context

## 5. Verification

- [x] 5.1 Run `pytest tests/test_git_context.py` and the touched server tests; ensure 80%+ coverage on `codoxear/git_context.py`
- [x] 5.2 Run `cd web && npm run test` and `cd web && npm run build`
- [ ] 5.3 Manual smoke: spin up a session in a repo with an open PR, a repo without a PR, and a non-git directory; confirm sidebar badges and detail endpoint output match the spec scenarios
- [x] 5.4 Update `AGENTS.md` "Components" section with one line about the new `git_context` helper and the new endpoint
