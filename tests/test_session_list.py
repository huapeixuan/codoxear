from __future__ import annotations

from pathlib import Path

from codoxear import server
from codoxear import session_list


def test_frontend_row_slims_internal_fields_and_normalizes_cwd() -> None:
    cwd = str(Path("/tmp/project").resolve(strict=False))
    row = {
        "session_id": "sess-1",
        "thread_id": "thread-1",
        "cwd": "/tmp/project",
        "agent_backend": "pi",
        "busy": True,
        "queue_len": 2,
        "alias": "Active",
        "broker_pid": 1234,
        "model": "internal",
        "provider_choice": "internal",
    }

    assert session_list.frontend_session_list_row(row) == {
        "session_id": "sess-1",
        "thread_id": "thread-1",
        "cwd": cwd,
        "agent_backend": "pi",
        "busy": True,
        "queue_len": 2,
        "alias": "Active",
    }


def test_grouped_payload_paginates_and_omits_hidden_groups(monkeypatch) -> None:
    docs_cwd = str(Path("/work/docs").resolve(strict=False))
    hidden_cwd = str(Path("/work/hidden").resolve(strict=False))
    rows = [
        {"session_id": f"docs-{index}", "cwd": docs_cwd, "agent_backend": "pi"}
        for index in range(1, 8)
    ] + [{"session_id": "hidden-1", "cwd": hidden_cwd, "agent_backend": "pi"}]

    payload = session_list.session_list_payload(
        rows,
        cwd_groups={hidden_cwd: {"hidden": True}},
        existing_workspace_dir_fn=lambda cwd: str(cwd) if str(cwd) == docs_cwd else None,
    )

    assert [row["session_id"] for row in payload["sessions"]] == [
        "docs-1",
        "docs-2",
        "docs-3",
        "docs-4",
        "docs-5",
    ]
    assert payload["remaining_by_group"] == {docs_cwd: 2}
    assert payload["omitted_group_count"] == 0


def test_recent_payload_orders_busy_then_updated_and_reports_remaining() -> None:
    rows = [
        {"session_id": "old", "cwd": "/work/old", "updated_ts": 10, "agent_backend": "pi"},
        {"session_id": "new", "cwd": "/work/new", "updated_ts": 100, "agent_backend": "pi"},
        {"session_id": "busy", "cwd": "/work/busy", "updated_ts": 20, "busy": True, "agent_backend": "pi"},
    ]

    payload = session_list.session_recent_payload(
        rows, limit=2, existing_workspace_dir_fn=lambda cwd: str(cwd)
    )

    assert [row["session_id"] for row in payload["sessions"]] == ["busy", "new"]
    assert payload["remaining"] == 1


def test_server_wrappers_delegate_to_session_list_module(monkeypatch) -> None:
    calls: dict[str, object] = {}

    def fake_payload(rows, **kwargs):
        calls["rows"] = rows
        calls["existing_workspace_dir_fn"] = kwargs["existing_workspace_dir_fn"]
        return {"sessions": []}

    monkeypatch.setattr(session_list, "session_list_payload", fake_payload)

    assert server._session_list_payload([{"session_id": "s1"}]) == {"sessions": []}
    assert calls["rows"] == [{"session_id": "s1"}]
    assert calls["existing_workspace_dir_fn"] is server._existing_workspace_dir
