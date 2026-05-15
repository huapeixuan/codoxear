from __future__ import annotations

from pathlib import Path
from typing import Any, Callable

SESSION_LIST_ROW_KEYS = (
    "session_id",
    "thread_id",
    "title",
    "alias",
    "first_user_message",
    "cwd",
    "agent_backend",
    "owned",
    "busy",
    "queue_len",
    "git_branch",
    "pr_summary",
    "transport",
    "blocked",
    "snoozed",
    "historical",
)
SESSION_LIST_GROUP_PAGE_SIZE = 5
SESSION_LIST_RECENT_GROUP_LIMIT = 3
SESSION_LIST_RECENT_PAGE_SIZE = 20
SESSION_LIST_FALLBACK_GROUP_KEY = "__no_working_directory__"


def normalize_cwd_group_key(cwd: Any) -> str:
    if not isinstance(cwd, str):
        raise ValueError("cwd must be a string")
    trimmed = cwd.strip()
    if not trimmed:
        raise ValueError("cwd required")
    return str(Path(trimmed).expanduser().resolve(strict=False))


def existing_workspace_dir(cwd: Any) -> str | None:
    try:
        normalized = normalize_cwd_group_key(cwd)
    except ValueError:
        return None
    try:
        if not Path(normalized).is_dir():
            return None
    except OSError:
        return None
    return normalized


def canonical_session_cwd(cwd: Any) -> str | None:
    if not isinstance(cwd, str):
        return None
    trimmed = cwd.strip()
    if not trimmed:
        return None
    try:
        return normalize_cwd_group_key(trimmed)
    except ValueError:
        return trimmed


def normalize_session_cwd_row(row: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(row, dict) or "cwd" not in row:
        return row
    canonical_cwd = canonical_session_cwd(row.get("cwd"))
    if canonical_cwd is None:
        return row
    normalized = dict(row)
    normalized["cwd"] = canonical_cwd
    return normalized


def frontend_session_list_row(row: dict[str, Any]) -> dict[str, Any]:
    normalized = normalize_session_cwd_row(row)
    if not isinstance(normalized, dict):
        return normalized
    return {key: normalized[key] for key in SESSION_LIST_ROW_KEYS if key in normalized}


def session_list_group_key(row: dict[str, Any]) -> str:
    cwd = canonical_session_cwd(row.get("cwd"))
    return cwd or SESSION_LIST_FALLBACK_GROUP_KEY


def session_list_visible_grouped_rows(
    rows: list[dict[str, Any]],
    *,
    cwd_groups: dict[str, dict[str, Any]] | None = None,
    existing_workspace_dir_fn: Callable[[Any], str | None] = existing_workspace_dir,
) -> dict[str, list[dict[str, Any]]]:
    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        key = session_list_group_key(row)
        if key not in grouped:
            grouped[key] = []
        grouped[key].append(row)

    hidden_group_keys = {
        str(cwd)
        for cwd, entry in (cwd_groups or {}).items()
        if isinstance(entry, dict) and bool(entry.get("hidden"))
    }
    if hidden_group_keys:
        grouped = {
            key: group_rows
            for key, group_rows in grouped.items()
            if key not in hidden_group_keys
        }

    return {
        key: group_rows
        for key, group_rows in grouped.items()
        if key == SESSION_LIST_FALLBACK_GROUP_KEY
        or existing_workspace_dir_fn(key) is not None
    }


def session_list_group_sort_key(
    grouped: dict[str, list[dict[str, Any]]], key: str
) -> tuple[int, float]:
    group_rows = grouped[key]
    busy = any(bool(row.get("busy")) for row in group_rows)
    latest_updated = max(float(row.get("updated_ts") or 0.0) for row in group_rows)
    return (0 if busy else 1, -latest_updated)


def session_list_payload(
    rows: list[dict[str, Any]],
    *,
    cwd_groups: dict[str, dict[str, Any]] | None = None,
    group_key: str | None = None,
    offset: int = 0,
    limit: int = SESSION_LIST_GROUP_PAGE_SIZE,
    group_offset: int = 0,
    group_limit: int = SESSION_LIST_RECENT_GROUP_LIMIT,
    existing_workspace_dir_fn: Callable[[Any], str | None] = existing_workspace_dir,
) -> dict[str, Any]:
    grouped = session_list_visible_grouped_rows(
        rows,
        cwd_groups=cwd_groups,
        existing_workspace_dir_fn=existing_workspace_dir_fn,
    )
    group_order = sorted(
        grouped.keys(), key=lambda key: session_list_group_sort_key(grouped, key)
    )

    if group_key is not None:
        group_rows = grouped.get(group_key, [])
        start = max(0, int(offset))
        stop = start + max(1, int(limit))
        page_rows = [frontend_session_list_row(row) for row in group_rows[start:stop]]
        remaining = max(0, len(group_rows) - stop)
        return {
            "sessions": page_rows,
            "remaining_by_group": {group_key: remaining} if remaining > 0 else {},
        }

    selected_group_keys = set(group_order[:SESSION_LIST_RECENT_GROUP_LIMIT])
    omitted_group_count = 0
    for key, group_rows in grouped.items():
        if any(bool(row.get("busy")) for row in group_rows):
            selected_group_keys.add(key)

    if group_offset > 0 or group_limit != SESSION_LIST_RECENT_GROUP_LIMIT:
        group_stop = max(group_offset, 0) + max(1, int(group_limit))
        extra_group_order = group_order[group_offset:group_stop]
        selected_group_keys = set(extra_group_order)
        omitted_group_count = max(0, len(group_order) - group_stop)

    sessions: list[dict[str, Any]] = []
    remaining_by_group: dict[str, int] = {}
    for key in group_order:
        if key not in selected_group_keys:
            continue
        group_rows = grouped[key]
        page_rows = group_rows[:SESSION_LIST_GROUP_PAGE_SIZE]
        sessions.extend(frontend_session_list_row(row) for row in page_rows)
        remaining = len(group_rows) - len(page_rows)
        if remaining > 0:
            remaining_by_group[key] = remaining
    payload: dict[str, Any] = {
        "sessions": sessions,
        "remaining_by_group": remaining_by_group,
    }
    if group_offset <= 0 and group_limit == SESSION_LIST_RECENT_GROUP_LIMIT:
        omitted_group_count = max(0, len(group_order) - len(selected_group_keys))
    payload["omitted_group_count"] = omitted_group_count
    return payload


def session_recent_sort_key(
    row: dict[str, Any], row_index: int
) -> tuple[int, float, float, int]:
    updated_raw = row.get("updated_ts")
    start_raw = row.get("start_ts")
    updated_ts = float(updated_raw) if isinstance(updated_raw, (int, float)) else 0.0
    start_ts = float(start_raw) if isinstance(start_raw, (int, float)) else 0.0
    busy = bool(row.get("busy"))
    return (0 if busy else 1, -updated_ts, -start_ts, row_index)


def session_recent_payload(
    rows: list[dict[str, Any]],
    *,
    cwd_groups: dict[str, dict[str, Any]] | None = None,
    offset: int = 0,
    limit: int = SESSION_LIST_RECENT_PAGE_SIZE,
    existing_workspace_dir_fn: Callable[[Any], str | None] = existing_workspace_dir,
) -> dict[str, Any]:
    grouped = session_list_visible_grouped_rows(
        rows,
        cwd_groups=cwd_groups,
        existing_workspace_dir_fn=existing_workspace_dir_fn,
    )
    ordered_rows = [
        row
        for _group_key in sorted(
            grouped.keys(), key=lambda key: session_list_group_sort_key(grouped, key)
        )
        for row in grouped[_group_key]
    ]
    recent_rows = [
        row
        for _, row in sorted(
            enumerate(ordered_rows),
            key=lambda item: session_recent_sort_key(item[1], item[0]),
        )
    ]
    start = max(0, int(offset))
    stop = start + max(1, int(limit))
    page_rows = [frontend_session_list_row(row) for row in recent_rows[start:stop]]
    remaining = max(0, len(recent_rows) - stop)
    return {
        "sessions": page_rows,
        "remaining": remaining,
    }
