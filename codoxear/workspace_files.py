from __future__ import annotations

import fnmatch
import hashlib
import os
import secrets
import urllib.parse
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

FILE_LIST_IGNORED_DIRS = frozenset(
    {
        ".git",
        ".hg",
        ".mypy_cache",
        ".pytest_cache",
        ".svn",
        "__pycache__",
        "build",
        "dist",
        "node_modules",
        "venv",
        ".venv",
    }
)
MARKDOWN_EXTENSIONS = frozenset({"md", "markdown", "mdown", "mkd"})
TEXTUAL_EXTENSIONS = frozenset(
    {
        "bash",
        "c",
        "cc",
        "cfg",
        "conf",
        "cpp",
        "css",
        "csv",
        "diff",
        "go",
        "h",
        "hpp",
        "htm",
        "html",
        "ini",
        "java",
        "js",
        "json",
        "jsonl",
        "log",
        "md",
        "markdown",
        "mdown",
        "mkd",
        "patch",
        "py",
        "rs",
        "scss",
        "sh",
        "sql",
        "svg",
        "toml",
        "ts",
        "tsx",
        "txt",
        "xml",
        "yaml",
        "yml",
        "zsh",
    }
)
TEXTUAL_FILENAMES = frozenset({"dockerfile", "license", "makefile", "readme"})


@dataclass(frozen=True)
class ClientFileView:
    kind: str
    size: int
    content_type: str | None = None
    text: str | None = None
    editable: bool = False
    version: str | None = None
    blocked_reason: str | None = None
    viewer_max_bytes: int | None = None


def safe_expanduser(p: Path) -> Path:
    try:
        return p.expanduser()
    except RuntimeError:
        return p


def resolve_under(base: Path, rel: str) -> Path:
    if not isinstance(rel, str) or not rel.strip():
        raise ValueError("path required")
    if "\x00" in rel:
        raise ValueError("invalid path")
    p = Path(rel)
    if p.is_absolute():
        raise ValueError("path must be relative")
    resolved_base = base.resolve()
    resolved = (resolved_base / p).resolve()
    if (
        not str(resolved).startswith(str(resolved_base) + os.sep)
        and resolved != resolved_base
    ):
        raise ValueError("path escapes session cwd")
    return resolved


def resolve_session_path(base: Path, raw_path: str) -> Path:
    if not isinstance(raw_path, str) or not raw_path.strip():
        raise ValueError("path required")
    if "\x00" in raw_path:
        raise ValueError("invalid path")
    p = Path(raw_path)
    if p.is_absolute():
        return safe_expanduser(p).resolve()
    resolved_base = safe_expanduser(base)
    if not resolved_base.is_absolute():
        resolved_base = resolved_base.resolve()
    return (resolved_base / p).resolve()


def resolve_git_path(
    cwd: Path,
    raw_path: str,
    *,
    run_git: Callable[[Path, list[str]], str],
) -> tuple[Path, Path, str]:
    repo_root = Path(run_git(cwd, ["rev-parse", "--show-toplevel"]).strip()).resolve()
    target = resolve_session_path(cwd, raw_path)
    try:
        rel = str(target.relative_to(repo_root))
    except ValueError as e:
        raise ValueError("path is outside git repo") from e
    return target, repo_root, rel


def resolve_session_relative_child(base: Path, raw_path: str) -> Path:
    rel = str(raw_path or "").strip()
    if not rel:
        return base.resolve()
    if "\x00" in rel:
        raise ValueError("invalid path")
    p = Path(rel)
    if p.is_absolute():
        raise ValueError("path must be relative")
    resolved_base = base.resolve()
    resolved = (resolved_base / p).resolve()
    if (
        not str(resolved).startswith(str(resolved_base) + os.sep)
        and resolved != resolved_base
    ):
        raise ValueError("path escapes session cwd")
    return resolved


def load_root_gitignore_patterns(root: Path) -> list[str]:
    path = root / ".gitignore"
    try:
        raw = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    except OSError:
        return []
    patterns: list[str] = []
    for line in raw.splitlines():
        pattern = line.strip()
        if not pattern or pattern.startswith("#") or pattern.startswith("!"):
            continue
        patterns.append(pattern)
    return patterns


def gitignore_matches(rel_path: str, *, is_dir: bool, pattern: str) -> bool:
    candidate = rel_path.strip("/")
    if not candidate:
        return False
    rule = pattern.strip()
    if not rule:
        return False
    dir_only = rule.endswith("/")
    if dir_only and not is_dir:
        return False
    rule = rule.rstrip("/")
    if not rule:
        return False
    anchored = rule.startswith("/")
    rule = rule.lstrip("/")
    if not rule:
        return False

    if "/" in rule:
        return fnmatch.fnmatchcase(candidate, rule)

    parts = candidate.split("/")
    if anchored:
        return fnmatch.fnmatchcase(parts[0], rule)
    return any(fnmatch.fnmatchcase(part, rule) for part in parts)


def is_ignored_session_relpath(rel_path: str, *, is_dir: bool, patterns: list[str]) -> bool:
    return any(
        gitignore_matches(rel_path, is_dir=is_dir, pattern=pattern)
        for pattern in patterns
    )


def session_entry_sort_key(entry: dict[str, str]) -> tuple[int, str]:
    return (0 if entry.get("kind") == "dir" else 1, entry.get("name", ""))


def list_session_directory_entries(
    base: Path,
    raw_path: str = "",
    *,
    ignored_dirs: frozenset[str] = FILE_LIST_IGNORED_DIRS,
) -> list[dict[str, str]]:
    root = safe_expanduser(base).resolve()
    if not root.exists():
        raise FileNotFoundError("session cwd not found")
    if not root.is_dir():
        raise ValueError("session cwd is not a directory")
    target = resolve_session_relative_child(root, raw_path)
    if not target.exists():
        raise FileNotFoundError("path not found")
    if not target.is_dir():
        raise ValueError("path is not a directory")

    patterns = load_root_gitignore_patterns(root)
    out: list[dict[str, str]] = []
    for child in target.iterdir():
        rel = child.relative_to(root).as_posix()
        if child.is_dir() and child.name in ignored_dirs:
            continue
        if is_ignored_session_relpath(rel, is_dir=child.is_dir(), patterns=patterns):
            continue
        out.append(
            {
                "name": child.name,
                "path": rel,
                "kind": "dir" if child.is_dir() else "file",
            }
        )
    out.sort(key=session_entry_sort_key)
    return out


def file_content_version(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def file_extension(path: Path) -> str:
    suffix = str(path.suffix or "").lower()
    if not suffix.startswith("."):
        return ""
    return suffix[1:]


def markdown_kind(path: Path) -> str:
    return "markdown" if file_extension(path) in MARKDOWN_EXTENSIONS else "text"


def path_looks_textual(path: Path) -> bool:
    ext = file_extension(path)
    if ext in TEXTUAL_EXTENSIONS:
        return True
    return str(path.name or "").strip().lower() in TEXTUAL_FILENAMES


def looks_like_text_bytes(raw: bytes) -> bool:
    if b"\x00" in raw:
        return False
    for b in raw:
        if b < 32 and b not in (9, 10, 12, 13, 27):
            return False
    return True


def decode_text_for_client(raw: bytes) -> tuple[str, bool]:
    try:
        return raw.decode("utf-8"), True
    except UnicodeDecodeError:
        return raw.decode("utf-8", errors="replace"), False


def decode_text_view_for_client(path: Path, raw: bytes) -> tuple[str, bool, str] | None:
    if b"\x00" in raw:
        return None
    try:
        text = raw.decode("utf-8")
        editable = True
    except UnicodeDecodeError:
        if not path_looks_textual(path) and not looks_like_text_bytes(raw):
            return None
        text = raw.decode("utf-8", errors="replace")
        editable = False
    return text, editable, file_content_version(raw)


def read_text_file_strict(path: Path, *, max_bytes: int) -> tuple[str, int]:
    st = path.stat()
    size = int(st.st_size)
    if size > max_bytes:
        raise ValueError(f"file too large (max {max_bytes} bytes)")
    data = path.read_bytes()
    if b"\x00" in data:
        raise ValueError("binary file not supported")
    text = data.decode("utf-8", errors="replace")
    return text, size


def read_text_file_for_client(path: Path, *, max_bytes: int) -> tuple[str, int, bool, str]:
    st = path.stat()
    size = int(st.st_size)
    if size > max_bytes:
        raise ValueError(f"file too large (max {max_bytes} bytes)")
    data = path.read_bytes()
    if b"\x00" in data:
        raise ValueError("binary file not supported")
    text, editable = decode_text_for_client(data)
    return text, size, editable, file_content_version(data)


def read_text_file_for_write(path: Path, *, max_bytes: int) -> tuple[str, int, str]:
    st = path.stat()
    size = int(st.st_size)
    if size > max_bytes:
        raise ValueError(f"file too large (max {max_bytes} bytes)")
    data = path.read_bytes()
    if b"\x00" in data:
        raise ValueError("binary file not supported")
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as e:
        raise ValueError("file is not editable as utf-8 text") from e
    return text, size, file_content_version(data)


def write_text_file_atomic(path: Path, *, text: str, max_bytes: int) -> tuple[int, str]:
    if not isinstance(text, str):
        raise ValueError("text must be a string")
    if path.is_symlink():
        raise ValueError("symlink file not supported")
    data = text.encode("utf-8")
    size = len(data)
    if size > max_bytes:
        raise ValueError(f"file too large (max {max_bytes} bytes)")
    st = path.stat()
    tmp = path.with_name(f".{path.name}.codoxear-tmp-{secrets.token_hex(6)}")
    try:
        tmp.write_bytes(data)
        os.chmod(tmp, st.st_mode & 0o777)
        os.replace(tmp, path)
    finally:
        try:
            if tmp.exists():
                tmp.unlink()
        except OSError:
            pass
    return size, file_content_version(data)


def write_new_text_file_atomic(path: Path, *, text: str, max_bytes: int) -> tuple[int, str]:
    if not isinstance(text, str):
        raise ValueError("text must be a string")
    if path.is_symlink():
        raise ValueError("symlink file not supported")
    parent = path.parent
    if not parent.exists():
        raise FileNotFoundError("parent directory not found")
    if not parent.is_dir():
        raise ValueError("parent path is not a directory")
    if parent.is_symlink():
        raise ValueError("symlink parent directory not supported")
    if path.exists():
        raise FileExistsError("file already exists")
    data = text.encode("utf-8")
    size = len(data)
    if size > max_bytes:
        raise ValueError(f"file too large (max {max_bytes} bytes)")
    tmp = path.with_name(f".{path.name}.codoxear-tmp-{secrets.token_hex(6)}")
    try:
        fd = os.open(str(tmp), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o666)
        with os.fdopen(fd, "wb") as fh:
            fh.write(data)
        os.link(str(tmp), str(path))
    finally:
        try:
            if tmp.exists():
                tmp.unlink()
        except OSError:
            pass
    return size, file_content_version(data)


def read_client_file_view(
    path_obj: Path,
    *,
    max_bytes: int,
    file_kind: Callable[[Path, bytes], tuple[str, str | None]],
) -> ClientFileView:
    if not path_obj.exists():
        raise FileNotFoundError("file not found")
    if path_obj.is_dir():
        return ClientFileView(kind="directory", size=0)
    if not path_obj.is_file():
        raise ValueError("path is not a file")
    try:
        size = int(path_obj.stat().st_size)
        with path_obj.open("rb") as f:
            prefix = f.read(4096)
    except PermissionError as e:
        raise PermissionError("permission denied") from e
    kind, content_type = file_kind(path_obj, prefix)
    if kind in {"image", "pdf"}:
        return ClientFileView(kind=kind, size=size, content_type=content_type)
    if size > max_bytes:
        return ClientFileView(
            kind="download_only",
            size=size,
            blocked_reason="too_large",
            viewer_max_bytes=max_bytes,
        )
    raw = path_obj.read_bytes()
    text_payload = decode_text_view_for_client(path_obj, raw)
    if text_payload is None:
        return ClientFileView(kind="download_only", size=size, blocked_reason="binary")
    text, editable, version = text_payload
    return ClientFileView(
        kind=markdown_kind(path_obj),
        size=size,
        text=text,
        editable=editable,
        version=version,
    )


def inspect_openable_file(
    path_obj: Path,
    *,
    max_bytes: int,
    file_kind: Callable[[Path, bytes], tuple[str, str | None]],
) -> tuple[bytes, int, str, str | None]:
    view = read_client_file_view(path_obj, max_bytes=max_bytes, file_kind=file_kind)
    if view.kind == "directory":
        raise ValueError("path is not a file")
    if view.kind == "download_only":
        if view.blocked_reason == "too_large":
            raise ValueError(f"file too large (max {max_bytes} bytes)")
        raise ValueError("binary file not supported")
    raw = path_obj.read_bytes()
    return raw, view.size, view.kind, view.content_type


def read_text_or_image(
    path_obj: Path,
    *,
    max_bytes: int,
    file_kind: Callable[[Path, bytes], tuple[str, str | None]],
) -> tuple[str, int, str | None, bytes | None]:
    view = read_client_file_view(path_obj, max_bytes=max_bytes, file_kind=file_kind)
    if view.kind in {"image", "pdf", "download_only", "directory"}:
        return view.kind, view.size, view.content_type, None
    raw = path_obj.read_bytes()
    return view.kind, view.size, view.content_type, raw


def read_downloadable_file(path_obj: Path) -> tuple[bytes, int]:
    if not path_obj.exists():
        raise FileNotFoundError("file not found")
    if not path_obj.is_file():
        raise ValueError("path is not a file")
    try:
        raw = path_obj.read_bytes()
    except PermissionError as e:
        raise PermissionError("permission denied") from e
    return raw, len(raw)


def inspect_client_path(
    path_obj: Path,
    *,
    max_bytes: int,
    file_kind: Callable[[Path, bytes], tuple[str, str | None]],
) -> tuple[int, str, str | None]:
    view = read_client_file_view(path_obj, max_bytes=max_bytes, file_kind=file_kind)
    return view.size, view.kind, view.content_type


def download_disposition(path_obj: Path) -> str:
    return f"attachment; filename*=UTF-8''{urllib.parse.quote(path_obj.name, safe='')}"
