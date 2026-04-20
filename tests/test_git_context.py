"""Unit tests for codoxear.git_context."""

from __future__ import annotations

import json
import threading
import time
from pathlib import Path
from unittest.mock import patch

import pytest

from codoxear import git_context


@pytest.fixture(autouse=True)
def _clear_caches():
    git_context.reset_caches_for_tests()
    yield
    git_context.reset_caches_for_tests()


def _stub_run(table):
    """Build a stand-in for git_context._run.

    `table` is a list of tuples (key_substring, returncode, stdout, stderr).
    The first matching entry by command-string substring wins.
    """

    calls: list[list[str]] = []

    def fake(cmd, *, cwd, timeout_s):  # noqa: ARG001
        calls.append(list(cmd))
        joined = " ".join(cmd)
        for key, code, out, err in table:
            if key in joined:
                return code, out, err
        return 1, "", f"unstubbed: {joined}"

    return fake, calls


def _make_repo(tmp_path: Path) -> Path:
    repo = tmp_path / "repo"
    repo.mkdir()
    return repo


def test_branch_resolution_named_branch(tmp_path):
    repo = _make_repo(tmp_path)
    fake, _ = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 0, "feature/login\n", ""),
            ("gh auth status", 0, "Logged in\n", ""),
            ("gh pr view", 1, "", "no pull requests found"),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        ctx = git_context.resolve_repo_context(repo)
    assert ctx.git_branch == "feature/login"
    assert ctx.pr is None
    assert ctx.availability == "no-pr"


def test_detached_head_returns_short_sha(tmp_path):
    repo = _make_repo(tmp_path)
    fake, _ = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 1, "", "fatal: ref HEAD is not a symbolic ref"),
            ("rev-parse --short HEAD", 0, "abc1234\n", ""),
            ("gh auth status", 0, "Logged in\n", ""),
            ("gh pr view", 1, "", "no pull requests found"),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        ctx = git_context.resolve_repo_context(repo)
    assert ctx.git_branch == "abc1234"
    assert ctx.availability == "no-pr"


def test_non_repo_cwd(tmp_path):
    repo = _make_repo(tmp_path)
    fake, _ = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 128, "", "not a git repository"),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake):
        ctx = git_context.resolve_repo_context(repo)
    assert ctx.git_branch is None
    assert ctx.pr is None
    assert ctx.availability == "not-a-repo"


def test_missing_cwd_short_circuits(tmp_path):
    missing = tmp_path / "does-not-exist"
    ctx = git_context.resolve_repo_context(missing)
    assert ctx.availability == "not-a-repo"
    assert ctx.git_branch is None


def test_missing_gh_short_circuits_pr(tmp_path):
    repo = _make_repo(tmp_path)
    fake, _ = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 0, "main\n", ""),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=False
    ):
        ctx = git_context.resolve_repo_context(repo)
    assert ctx.git_branch == "main"
    assert ctx.pr is None
    assert ctx.availability == "no-gh"


def test_pr_view_returns_open_pr(tmp_path):
    repo = _make_repo(tmp_path)
    pr_payload = json.dumps(
        {
            "number": 42,
            "title": "Add login flow",
            "state": "OPEN",
            "url": "https://github.com/example/repo/pull/42",
            "isDraft": False,
            "headRefName": "feature/login",
        }
    )
    fake, _ = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 0, "feature/login\n", ""),
            ("gh auth status", 0, "Logged in\n", ""),
            ("gh pr view", 0, pr_payload, ""),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        ctx = git_context.resolve_repo_context(repo)
    assert ctx.availability == "ok"
    assert ctx.pr is not None
    assert ctx.pr.number == 42
    assert ctx.pr.state == "OPEN"
    assert ctx.pr_summary() == {"number": 42, "state": "OPEN"}


def test_draft_pr_summary_marks_draft(tmp_path):
    repo = _make_repo(tmp_path)
    pr_payload = json.dumps(
        {
            "number": 7,
            "title": "WIP",
            "state": "OPEN",
            "url": "https://github.com/example/repo/pull/7",
            "isDraft": True,
            "headRefName": "wip/x",
        }
    )
    fake, _ = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 0, "wip/x\n", ""),
            ("gh auth status", 0, "Logged in\n", ""),
            ("gh pr view", 0, pr_payload, ""),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        ctx = git_context.resolve_repo_context(repo)
    assert ctx.pr_summary() == {"number": 7, "state": "DRAFT"}


def test_gh_pr_view_timeout_marks_error(tmp_path):
    repo = _make_repo(tmp_path)
    fake, _ = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 0, "main\n", ""),
            ("gh auth status", 0, "Logged in\n", ""),
            ("gh pr view", -1, "", "timeout"),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        ctx = git_context.resolve_repo_context(repo)
    assert ctx.availability == "error"
    assert ctx.git_branch == "main"


def test_cache_hit_avoids_resubprocess(tmp_path):
    repo = _make_repo(tmp_path)
    fake, calls = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 0, "main\n", ""),
            ("gh auth status", 0, "Logged in\n", ""),
            ("gh pr view", 1, "", "no pull requests found"),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        first = git_context.resolve_repo_context(repo)
        n_after_first = len(calls)
        second = git_context.resolve_repo_context(repo)
    assert first == second
    assert len(calls) == n_after_first  # no extra calls on cache hit


def test_refresh_flag_busts_cache(tmp_path):
    repo = _make_repo(tmp_path)
    fake, calls = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 0, "main\n", ""),
            ("gh auth status", 0, "Logged in\n", ""),
            ("gh pr view", 1, "", "no pull requests found"),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        git_context.resolve_repo_context(repo)
        before = len(calls)
        git_context.resolve_repo_context(repo, refresh=True)
    assert len(calls) > before


def test_concurrent_lookups_share_one_resolution(tmp_path):
    repo = _make_repo(tmp_path)
    barrier = threading.Barrier(5)
    started = threading.Event()

    def slow_run(cmd, *, cwd, timeout_s):  # noqa: ARG001
        if "rev-parse --is-inside-work-tree" in " ".join(cmd) and not started.is_set():
            started.set()
            time.sleep(0.05)
            return 0, "true\n", ""
        joined = " ".join(cmd)
        if "rev-parse --is-inside-work-tree" in joined:
            return 0, "true\n", ""
        if "symbolic-ref" in joined:
            return 0, "main\n", ""
        if "gh auth status" in joined:
            return 0, "Logged in\n", ""
        if "gh pr view" in joined:
            return 1, "", "no pull requests found"
        return 1, "", ""

    results: list[git_context.RepoContext] = []
    results_lock = threading.Lock()

    def worker():
        barrier.wait()
        with patch.object(git_context, "_gh_available", return_value=True):
            ctx = git_context.resolve_repo_context(repo)
        with results_lock:
            results.append(ctx)

    with patch.object(git_context, "_run", side_effect=slow_run):
        threads = [threading.Thread(target=worker) for _ in range(5)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()

    # All workers see the same cached object (or equal contexts).
    assert len(results) == 5
    assert all(r.git_branch == "main" for r in results)
    assert all(r.availability == results[0].availability for r in results)


def test_current_git_branch_compat_wrapper(tmp_path):
    repo = _make_repo(tmp_path)
    fake, _ = _stub_run(
        [
            ("rev-parse --is-inside-work-tree", 0, "true\n", ""),
            ("symbolic-ref", 0, "main\n", ""),
            ("gh auth status", 1, "", "not logged in"),
        ]
    )
    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        assert git_context.current_git_branch(repo) == "main"
