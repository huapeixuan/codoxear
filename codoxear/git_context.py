"""Resolve git branch and GitHub PR context for a session's working directory.

Used by the session-list endpoint and the per-session repo detail endpoint to
surface branch and PR badges in the workspace UI without blocking session list
polling on subprocess calls.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal

# Per-call timeouts. Kept short so a slow git or network call cannot block
# session-list polling. Branch lookup is local-only and very fast; PR lookup
# may hit the network through `gh`.
BRANCH_TIMEOUT_S = float(os.environ.get("CODEX_WEB_BRANCH_TIMEOUT_S", "2.0"))
PR_TIMEOUT_S = float(os.environ.get("CODEX_WEB_PR_TIMEOUT_S", "4.0"))

# Cache TTLs.
BRANCH_TTL_S = float(os.environ.get("CODEX_WEB_BRANCH_TTL_S", "30.0"))
PR_TTL_S = float(os.environ.get("CODEX_WEB_PR_TTL_S", "120.0"))

# How long to trust a `gh auth status` result.
GH_AUTH_TTL_S = float(os.environ.get("CODEX_WEB_GH_AUTH_TTL_S", "300.0"))


Availability = Literal["ok", "no-gh", "no-pr", "not-a-repo", "error"]


@dataclass(frozen=True)
class PullRequest:
    number: int
    title: str
    state: str  # OPEN | DRAFT | CLOSED | MERGED
    url: str
    is_draft: bool
    head_ref_name: str

    def to_summary(self) -> dict[str, Any]:
        """Compact form for the session-list payload."""
        state = "DRAFT" if self.is_draft and self.state == "OPEN" else self.state
        return {"number": int(self.number), "state": state}

    def to_dict(self) -> dict[str, Any]:
        return {
            "number": int(self.number),
            "title": self.title,
            "state": self.state,
            "url": self.url,
            "is_draft": bool(self.is_draft),
            "head_ref_name": self.head_ref_name,
        }


@dataclass(frozen=True)
class RepoContext:
    git_branch: str | None
    pr: PullRequest | None
    availability: Availability
    cwd: str = ""

    def to_detail_dict(self) -> dict[str, Any]:
        return {
            "cwd": self.cwd,
            "git_branch": self.git_branch,
            "pr": self.pr.to_dict() if self.pr else None,
            "availability": self.availability,
        }

    def pr_summary(self) -> dict[str, Any] | None:
        return self.pr.to_summary() if self.pr else None


@dataclass
class _CacheEntry:
    context: RepoContext
    expires_at: float


# Per-cwd cache + lock dictionaries. Locks ensure only one resolver runs per
# cwd at a time so concurrent pollers do not stampede `git`/`gh`.
_cache: dict[str, _CacheEntry] = {}
_cache_lock = threading.Lock()
_cwd_locks: dict[str, threading.Lock] = {}

# Global gh-auth cache state.
_gh_auth_state: dict[str, Any] = {"ok": False, "checked_at": 0.0}
_gh_auth_lock = threading.Lock()


def _now() -> float:
    return time.monotonic()


def _cwd_lock_for(key: str) -> threading.Lock:
    with _cache_lock:
        lock = _cwd_locks.get(key)
        if lock is None:
            lock = threading.Lock()
            _cwd_locks[key] = lock
        return lock


def _resolve_cwd(cwd: Path) -> Path | None:
    try:
        return Path(cwd).expanduser().resolve()
    except (OSError, RuntimeError):
        return None


def _run(
    cmd: list[str],
    *,
    cwd: Path,
    timeout_s: float,
) -> tuple[int, str, str]:
    """Run a subprocess and return (returncode, stdout, stderr).

    Returns (-1, "", "<reason>") on FileNotFoundError or TimeoutExpired so the
    caller can branch on a single tuple shape.
    """
    try:
        proc = subprocess.run(
            cmd,
            cwd=str(cwd),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout_s,
            check=False,
        )
    except FileNotFoundError as exc:
        return -1, "", f"missing executable: {exc}"
    except subprocess.TimeoutExpired:
        return -1, "", "timeout"
    return (
        proc.returncode,
        proc.stdout.decode("utf-8", errors="replace"),
        proc.stderr.decode("utf-8", errors="replace"),
    )


def _resolve_branch(cwd: Path) -> tuple[str | None, bool]:
    """Return (branch_or_short_sha, is_repo).

    On detached HEAD returns the short SHA so the UI can still render
    something meaningful. Returns (None, False) when cwd is not a git tree.
    """
    code, out, _err = _run(
        ["git", "rev-parse", "--is-inside-work-tree"],
        cwd=cwd,
        timeout_s=BRANCH_TIMEOUT_S,
    )
    if code != 0 or out.strip().lower() != "true":
        return None, False

    code, out, _err = _run(
        ["git", "symbolic-ref", "--quiet", "--short", "HEAD"],
        cwd=cwd,
        timeout_s=BRANCH_TIMEOUT_S,
    )
    if code == 0:
        branch = out.strip()
        if branch:
            return branch, True

    # Detached HEAD: fall back to short SHA.
    code, out, _err = _run(
        ["git", "rev-parse", "--short", "HEAD"],
        cwd=cwd,
        timeout_s=BRANCH_TIMEOUT_S,
    )
    if code == 0:
        sha = out.strip()
        return (sha or None), True
    return None, True


def _gh_available() -> bool:
    return shutil.which("gh") is not None


def _check_gh_auth(*, refresh: bool = False) -> bool:
    """Cached `gh auth status` check. False when gh is missing or unauthenticated."""
    if not _gh_available():
        with _gh_auth_lock:
            _gh_auth_state["ok"] = False
            _gh_auth_state["checked_at"] = _now()
        return False
    with _gh_auth_lock:
        last = float(_gh_auth_state.get("checked_at") or 0.0)
        if not refresh and (_now() - last) < GH_AUTH_TTL_S:
            return bool(_gh_auth_state.get("ok"))
    code, _out, _err = _run(
        ["gh", "auth", "status"],
        cwd=Path.cwd(),
        timeout_s=PR_TIMEOUT_S,
    )
    ok = code == 0
    with _gh_auth_lock:
        _gh_auth_state["ok"] = ok
        _gh_auth_state["checked_at"] = _now()
    return ok


def _resolve_pr(cwd: Path) -> tuple[PullRequest | None, Availability]:
    """Look up PR metadata for the current branch via the gh CLI."""
    if not _gh_available():
        return None, "no-gh"
    if not _check_gh_auth():
        return None, "no-gh"
    code, out, err = _run(
        [
            "gh",
            "pr",
            "view",
            "--json",
            "number,title,state,url,isDraft,headRefName",
        ],
        cwd=cwd,
        timeout_s=PR_TIMEOUT_S,
    )
    if code == -1:
        return None, "error"
    if code != 0:
        # `gh pr view` exits non-zero when the branch has no associated PR.
        msg = (err or out).lower()
        if "no pull requests found" in msg or "no pr" in msg or "could not find" in msg:
            return None, "no-pr"
        return None, "error"
    try:
        data = json.loads(out)
    except ValueError:
        return None, "error"
    if not isinstance(data, dict):
        return None, "error"
    try:
        pr = PullRequest(
            number=int(data.get("number") or 0),
            title=str(data.get("title") or ""),
            state=str(data.get("state") or "").upper(),
            url=str(data.get("url") or ""),
            is_draft=bool(data.get("isDraft") or False),
            head_ref_name=str(data.get("headRefName") or ""),
        )
    except (TypeError, ValueError):
        return None, "error"
    if pr.number <= 0:
        return None, "no-pr"
    return pr, "ok"


def _compute(cwd: Path) -> RepoContext:
    branch, is_repo = _resolve_branch(cwd)
    if not is_repo:
        return RepoContext(
            git_branch=None, pr=None, availability="not-a-repo", cwd=str(cwd)
        )
    pr, availability = _resolve_pr(cwd)
    return RepoContext(
        git_branch=branch,
        pr=pr,
        availability=availability,
        cwd=str(cwd),
    )


def resolve_repo_context(cwd: Path | str, *, refresh: bool = False) -> RepoContext:
    """Resolve repo context for cwd, using a per-cwd TTL cache.

    Set ``refresh=True`` to bypass the cache and re-run `git`/`gh`. Branch
    lookups respect ``BRANCH_TTL_S`` and PR lookups respect ``PR_TTL_S``;
    the longer of the two is used for the combined entry.
    """
    resolved = _resolve_cwd(Path(cwd))
    if resolved is None or not resolved.exists() or not resolved.is_dir():
        return RepoContext(
            git_branch=None,
            pr=None,
            availability="not-a-repo",
            cwd=str(cwd),
        )
    key = str(resolved)
    if not refresh:
        with _cache_lock:
            entry = _cache.get(key)
            if entry is not None and entry.expires_at > _now():
                return entry.context
    lock = _cwd_lock_for(key)
    with lock:
        # Double-check after acquiring the per-cwd lock so concurrent waiters
        # share the freshly-computed result.
        if not refresh:
            with _cache_lock:
                entry = _cache.get(key)
                if entry is not None and entry.expires_at > _now():
                    return entry.context
        ctx = _compute(resolved)
        # Cache TTL: PR TTL when PR data was fetched, branch TTL otherwise.
        ttl = PR_TTL_S if ctx.availability in {"ok", "no-pr"} else BRANCH_TTL_S
        with _cache_lock:
            _cache[key] = _CacheEntry(context=ctx, expires_at=_now() + ttl)
        return ctx


def current_git_branch(cwd: Path) -> str | None:
    """Backwards-compatible thin wrapper used by ``server._current_git_branch``."""
    return resolve_repo_context(cwd).git_branch


def reset_caches_for_tests() -> None:
    """Clear all caches; intended for unit tests only."""
    with _cache_lock:
        _cache.clear()
        _cwd_locks.clear()
    with _gh_auth_lock:
        _gh_auth_state["ok"] = False
        _gh_auth_state["checked_at"] = 0.0


__all__ = [
    "Availability",
    "PullRequest",
    "RepoContext",
    "resolve_repo_context",
    "current_git_branch",
    "reset_caches_for_tests",
]
