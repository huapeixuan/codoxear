"""Tests for repo/PR context exposure in session list and /repo endpoint."""

from __future__ import annotations

from unittest.mock import patch

from codoxear import git_context, server


def _row(session_id: str, cwd: str) -> dict:
    return {
        "session_id": session_id,
        "cwd": cwd,
        "title": "t",
        "alias": "",
        "first_user_message": "",
        "agent_backend": "codex",
        "owned": True,
        "busy": False,
        "queue_len": 0,
        "git_branch": "feature/login",
        "pr_summary": {"number": 42, "state": "OPEN"},
        "transport": "sock",
        "blocked": False,
        "snoozed": False,
        "historical": False,
    }


def test_session_list_payload_includes_pr_summary(tmp_path):
    cwd = str(tmp_path)
    payload = server._session_list_payload([_row("s1", cwd)])
    assert payload["sessions"], "expected at least one session row"
    row = payload["sessions"][0]
    assert row["git_branch"] == "feature/login"
    assert row["pr_summary"] == {"number": 42, "state": "OPEN"}


def test_frontend_session_list_row_drops_unknown_fields_but_keeps_pr_summary():
    row = _row("s2", "/tmp/x")
    row["secret_internal"] = "nope"
    frontend = server._frontend_session_list_row(row)
    assert "pr_summary" in frontend
    assert frontend["pr_summary"] == {"number": 42, "state": "OPEN"}
    assert "secret_internal" not in frontend


def test_pr_summary_missing_serializes_as_none():
    row = _row("s3", "/tmp/x")
    row["pr_summary"] = None
    frontend = server._frontend_session_list_row(row)
    assert frontend["pr_summary"] is None


def test_resolve_repo_context_detail_dict_shape(tmp_path):
    git_context.reset_caches_for_tests()

    def fake(cmd, *, cwd, timeout_s):  # noqa: ARG001
        joined = " ".join(cmd)
        if "rev-parse --is-inside-work-tree" in joined:
            return 0, "true\n", ""
        if "symbolic-ref" in joined:
            return 0, "main\n", ""
        if "gh auth status" in joined:
            return 0, "ok\n", ""
        if "gh pr view" in joined:
            return 1, "", "no pull requests found"
        return 1, "", ""

    with patch.object(git_context, "_run", side_effect=fake), patch.object(
        git_context, "_gh_available", return_value=True
    ):
        ctx = git_context.resolve_repo_context(tmp_path)
    detail = ctx.to_detail_dict()
    assert detail["git_branch"] == "main"
    assert detail["pr"] is None
    assert detail["availability"] == "no-pr"
    assert detail["cwd"].endswith(tmp_path.name)
