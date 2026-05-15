from __future__ import annotations

import json
import math
import os
from pathlib import Path
from typing import Any, Callable


def write_json_atomic(path: Path, obj: Any, *, sort_keys: bool = True) -> None:
    os.makedirs(path.parent, exist_ok=True)
    tmp = path.with_suffix(".json.tmp")
    tmp.write_text(
        json.dumps(obj, ensure_ascii=False, sort_keys=sort_keys, indent=2) + "\n",
        encoding="utf-8",
    )
    os.replace(tmp, path)


def clean_recent_cwd(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    out = value.strip()
    return out or None


def clean_hidden_after_live_start_ts(value: Any) -> float | None:
    if value is None or isinstance(value, bool):
        return None
    try:
        out = float(value)
    except (TypeError, ValueError):
        return None
    if not math.isfinite(out) or out <= 0:
        return None
    return out


def clean_hidden_session_cutoff_ts(value: Any) -> float | None:
    return clean_hidden_after_live_start_ts(value)


def load_mapping(path: Path, *, error_name: str) -> dict[str, Any] | None:
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    obj = json.loads(raw)
    if not isinstance(obj, dict):
        raise ValueError(f"invalid {error_name} (expected object)")
    return obj


def load_aliases(path: Path, *, clean_alias: Callable[[Any], str]) -> dict[str, str] | None:
    obj = load_mapping(path, error_name="session_aliases.json")
    if obj is None:
        return None
    cleaned: dict[str, str] = {}
    for sid, v in obj.items():
        if not isinstance(sid, str) or not sid:
            continue
        if not isinstance(v, str):
            continue
        alias = clean_alias(v)
        if alias:
            cleaned[sid] = alias
    return cleaned


def save_mapping(path: Path, obj: dict[str, Any]) -> None:
    write_json_atomic(path, obj, sort_keys=True)


def load_files(path: Path, *, file_history_max: int) -> dict[str, list[str]] | None:
    obj = load_mapping(path, error_name="session_files.json")
    if obj is None:
        return None
    cleaned: dict[str, list[str]] = {}
    for sid, arr in obj.items():
        if not isinstance(sid, str) or not sid:
            continue
        if sid.startswith("cwd:"):
            continue
        key = sid if sid.startswith("sid:") else f"sid:{sid}"
        if not isinstance(arr, list):
            continue
        out: list[str] = []
        for v in arr:
            if not isinstance(v, str):
                continue
            p = v.strip()
            if not p or p in out:
                continue
            out.append(p)
            if len(out) >= file_history_max:
                break
        if out:
            cleaned[key] = out
    return cleaned


def load_recent_cwds(path: Path, *, recent_cwd_max: int) -> dict[str, float] | None:
    obj = load_mapping(path, error_name="recent_cwds.json")
    if obj is None:
        return None
    cleaned: dict[str, float] = {}
    for raw_cwd, raw_ts in obj.items():
        cwd = clean_recent_cwd(raw_cwd)
        if cwd is None or isinstance(raw_ts, bool):
            continue
        try:
            ts = float(raw_ts)
        except (TypeError, ValueError):
            continue
        if not math.isfinite(ts) or ts <= 0:
            continue
        prev = cleaned.get(cwd)
        if prev is None or ts > prev:
            cleaned[cwd] = ts
    top = sorted(cleaned.items(), key=lambda item: (-item[1], item[0]))[
        :recent_cwd_max
    ]
    return dict(top)


def recent_cwds_save_payload(recent_cwds: dict[str, float], *, recent_cwd_max: int) -> dict[str, float]:
    items = sorted(
        recent_cwds.items(),
        key=lambda item: (-float(item[1]), item[0]),
    )[:recent_cwd_max]
    return {cwd: ts for cwd, ts in items}


def load_cwd_groups(
    path: Path,
    *,
    normalize_cwd_group_key: Callable[[Any], str],
    clean_alias: Callable[[Any], str],
    cwd_group_entry: Callable[..., dict[str, Any]],
) -> dict[str, dict[str, Any]]:
    cleaned: dict[str, dict[str, Any]] = {}
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return cleaned
    obj = json.loads(raw)
    if not isinstance(obj, dict):
        raise ValueError("invalid cwd_groups.json (expected object)")
    for cwd, v in obj.items():
        try:
            normalized_cwd = normalize_cwd_group_key(cwd)
        except ValueError:
            continue
        if not isinstance(v, dict):
            continue
        label = clean_alias(v.get("label", ""))
        persisted_collapsed = v.get("collapsed", False)
        collapsed = persisted_collapsed if isinstance(persisted_collapsed, bool) else False
        persisted_hidden = v.get("hidden", False)
        hidden = persisted_hidden if isinstance(persisted_hidden, bool) else False
        hidden_after_live_start_ts = clean_hidden_after_live_start_ts(
            v.get("hidden_after_live_start_ts")
        )
        if label or collapsed or hidden:
            cleaned[normalized_cwd] = cwd_group_entry(
                label=label,
                collapsed=collapsed,
                hidden=hidden,
                hidden_after_live_start_ts=hidden_after_live_start_ts,
            )
    return cleaned


def load_hidden_sessions(path: Path) -> tuple[set[str], dict[str, float]] | None:
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    obj = json.loads(raw)
    if not isinstance(obj, list):
        raise ValueError("invalid hidden_sessions.json (expected list)")
    cleaned: set[str] = set()
    cutoffs: dict[str, float] = {}
    for entry in obj:
        if isinstance(entry, str):
            sid = entry.strip()
            if sid:
                cleaned.add(sid)
            continue
        if not isinstance(entry, dict):
            continue
        sid_raw = entry.get("id")
        if not isinstance(sid_raw, str):
            continue
        sid = sid_raw.strip()
        if not sid:
            continue
        cutoff_ts = clean_hidden_session_cutoff_ts(entry.get("cutoff_ts"))
        if cutoff_ts is None:
            cleaned.add(sid)
        else:
            cutoffs[sid] = cutoff_ts
    return cleaned, cutoffs


def hidden_sessions_save_payload(hidden_sessions: set[str], cutoffs: dict[str, float]) -> list[Any]:
    obj: list[Any] = list(sorted(hidden_sessions))
    for key in sorted(cutoffs):
        cutoff_ts = clean_hidden_session_cutoff_ts(cutoffs.get(key))
        if cutoff_ts is None:
            obj.append(key)
            continue
        obj.append({"id": key, "cutoff_ts": cutoff_ts})
    return obj
