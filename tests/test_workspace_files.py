from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

from codoxear import server
from codoxear import workspace_files


def _text_kind(path: Path, raw: bytes) -> tuple[str, str | None]:
    if path.suffix == ".png" or raw.startswith(b"PNG"):
        return "image", "image/png"
    return "text", None


def test_list_session_directory_entries_rejects_absolute_and_escaping_paths(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="path must be relative"):
        workspace_files.list_session_directory_entries(tmp_path, "/tmp")

    with pytest.raises(ValueError, match="escapes session cwd"):
        workspace_files.list_session_directory_entries(tmp_path, "..")


def test_list_session_directory_entries_applies_builtin_and_gitignore_filters(tmp_path: Path) -> None:
    (tmp_path / ".gitignore").write_text("ignored.txt\ncache/\n", encoding="utf-8")
    (tmp_path / ".git").mkdir()
    (tmp_path / "cache").mkdir()
    (tmp_path / "src").mkdir()
    (tmp_path / "ignored.txt").write_text("ignored", encoding="utf-8")
    (tmp_path / "visible.txt").write_text("visible", encoding="utf-8")

    assert workspace_files.list_session_directory_entries(tmp_path) == [
        {"name": "src", "path": "src", "kind": "dir"},
        {"name": ".gitignore", "path": ".gitignore", "kind": "file"},
        {"name": "visible.txt", "path": "visible.txt", "kind": "file"},
    ]


def test_read_client_file_view_marks_large_text_download_only(tmp_path: Path) -> None:
    path = tmp_path / "large.md"
    path.write_text("a" * 11, encoding="utf-8")

    view = workspace_files.read_client_file_view(path, max_bytes=10, file_kind=_text_kind)

    assert view.kind == "download_only"
    assert view.blocked_reason == "too_large"
    assert view.viewer_max_bytes == 10


def test_read_text_file_for_client_hashes_and_marks_invalid_utf8_readonly(tmp_path: Path) -> None:
    path = tmp_path / "note.txt"
    raw = b"broken:\xff\n"
    path.write_bytes(raw)

    text, size, editable, version = workspace_files.read_text_file_for_client(
        path, max_bytes=1024
    )

    assert size == len(raw)
    assert not editable
    assert "broken:" in text
    assert version == hashlib.sha256(raw).hexdigest()


def test_resolve_git_path_uses_repo_root_and_rejects_outside_path(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir()
    target = repo / "app.py"
    target.write_text("print('ok')\n", encoding="utf-8")

    def run_git(cwd: Path, args: list[str]) -> str:
        assert cwd == repo
        assert args == ["rev-parse", "--show-toplevel"]
        return str(repo) + "\n"

    path, root, rel = workspace_files.resolve_git_path(repo, "app.py", run_git=run_git)
    assert path == target.resolve()
    assert root == repo.resolve()
    assert rel == "app.py"

    with pytest.raises(ValueError, match="outside git repo"):
        workspace_files.resolve_git_path(repo, "../outside.py", run_git=run_git)


def test_server_file_list_wrapper_delegates_to_workspace_files(monkeypatch, tmp_path: Path) -> None:
    calls: dict[str, object] = {}

    def fake_list(base, raw_path="", *, ignored_dirs):
        calls["base"] = base
        calls["raw_path"] = raw_path
        calls["ignored_dirs"] = ignored_dirs
        return [{"name": "a.py", "path": "src/a.py", "kind": "file"}]

    monkeypatch.setattr(workspace_files, "list_session_directory_entries", fake_list)

    assert server._list_session_directory_entries(tmp_path, "src") == [
        {"name": "a.py", "path": "src/a.py", "kind": "file"}
    ]
    assert calls == {
        "base": tmp_path,
        "raw_path": "src",
        "ignored_dirs": server.FILE_LIST_IGNORED_DIRS,
    }


def test_server_read_view_wrapper_delegates_to_workspace_files(monkeypatch, tmp_path: Path) -> None:
    calls: dict[str, object] = {}
    expected = workspace_files.ClientFileView(kind="text", size=1, text="x")

    def fake_read(path, *, max_bytes, file_kind):
        calls["path"] = path
        calls["max_bytes"] = max_bytes
        calls["file_kind"] = file_kind
        return expected

    monkeypatch.setattr(workspace_files, "read_client_file_view", fake_read)

    assert server._read_client_file_view(tmp_path / "a.txt") is expected
    assert calls == {
        "path": tmp_path / "a.txt",
        "max_bytes": server.FILE_READ_MAX_BYTES,
        "file_kind": server._file_kind,
    }
