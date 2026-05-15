from __future__ import annotations

import json
from pathlib import Path

import pytest

from codoxear import session_state_store


def test_load_files_preserves_sid_shape_and_skips_legacy_cwd_entries(tmp_path: Path) -> None:
    path = tmp_path / "session_files.json"
    path.write_text(
        json.dumps(
            {
                "plain": ["/a.py", "/a.py", "", 123, "/b.py"],
                "sid:kept": ["/c.py"],
                "cwd:/old": ["/ignored.py"],
            }
        ),
        encoding="utf-8",
    )

    assert session_state_store.load_files(path, file_history_max=2) == {
        "sid:plain": ["/a.py", "/b.py"],
        "sid:kept": ["/c.py"],
    }


def test_load_recent_cwds_sorts_dedupes_and_limits(tmp_path: Path) -> None:
    path = tmp_path / "recent_cwds.json"
    path.write_text(
        json.dumps({"/older": 10, "/newer": 30, " ": 99, "/bad": True}),
        encoding="utf-8",
    )

    assert session_state_store.load_recent_cwds(path, recent_cwd_max=1) == {
        "/newer": 30.0
    }


def test_hidden_sessions_loads_legacy_strings_and_cutoff_objects(tmp_path: Path) -> None:
    path = tmp_path / "hidden_sessions.json"
    path.write_text(
        json.dumps(["old", {"id": "new", "cutoff_ts": 123.5}, {"id": "bad", "cutoff_ts": -1}]),
        encoding="utf-8",
    )

    hidden, cutoffs = session_state_store.load_hidden_sessions(path) or (set(), {})

    assert hidden == {"old", "bad"}
    assert cutoffs == {"new": 123.5}
    assert session_state_store.hidden_sessions_save_payload(hidden, cutoffs) == [
        "bad",
        "old",
        {"id": "new", "cutoff_ts": 123.5},
    ]


def test_load_cwd_groups_normalizes_and_recovers_fields(tmp_path: Path) -> None:
    path = tmp_path / "cwd_groups.json"
    path.write_text(
        json.dumps({str(tmp_path): {"label": " Work ", "collapsed": True, "hidden": True, "hidden_after_live_start_ts": 9}}),
        encoding="utf-8",
    )

    def normalize(cwd):
        return str(Path(cwd).resolve())

    def clean_alias(value):
        return str(value).strip()

    def entry(**kwargs):
        return kwargs

    assert session_state_store.load_cwd_groups(
        path,
        normalize_cwd_group_key=normalize,
        clean_alias=clean_alias,
        cwd_group_entry=entry,
    ) == {
        str(tmp_path.resolve()): {
            "label": "Work",
            "collapsed": True,
            "hidden": True,
            "hidden_after_live_start_ts": 9.0,
        }
    }


def test_invalid_mapping_shape_raises_compatible_error(tmp_path: Path) -> None:
    path = tmp_path / "session_aliases.json"
    path.write_text("[]", encoding="utf-8")

    with pytest.raises(ValueError, match="invalid session_aliases.json"):
        session_state_store.load_aliases(path, clean_alias=lambda value: str(value).strip())


def test_write_json_atomic_preserves_json_object_shape(tmp_path: Path) -> None:
    path = tmp_path / "session_aliases.json"

    session_state_store.save_mapping(path, {"sid": "Alias"})

    assert json.loads(path.read_text(encoding="utf-8")) == {"sid": "Alias"}
    assert path.read_text(encoding="utf-8").endswith("\n")
