from __future__ import annotations

import base64
import contextlib
import hashlib
import hmac
import json
import os
import socket
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Iterator

import pytest


CONTRACT_PASSWORD = "contract-secret"


def _free_port() -> int:
    with contextlib.closing(socket.socket(socket.AF_INET, socket.SOCK_STREAM)) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _wait_for_http(
    url: str, *, health_path: str = "/api/v1/health", timeout: float = 10.0
) -> None:
    import urllib.error
    import urllib.request

    deadline = time.time() + timeout
    last_error: Exception | None = None
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(f"{url}{health_path}", timeout=0.5):
                return
        except urllib.error.HTTPError:
            return
        except Exception as exc:
            last_error = exc
            time.sleep(0.1)
    raise RuntimeError(f"server did not become ready at {url}: {last_error}")


@pytest.fixture(scope="session")
def shared_app_home() -> Iterator[Path]:
    with tempfile.TemporaryDirectory(prefix="cdx-", dir="/tmp") as home:
        yield Path(home)


@pytest.fixture
def shared_app_dir(shared_app_home: Path) -> Path:
    app_dir = shared_app_home / ".local/share/codoxear"
    app_dir.mkdir(parents=True, exist_ok=True)
    return app_dir


@pytest.fixture
def rust_server_url(shared_app_home: Path) -> Iterator[str]:
    repo_root = Path(__file__).resolve().parents[2]
    bin_path = repo_root / "backend-rs/target/release/codoxear-backend-rs"
    if not bin_path.exists():
        raise RuntimeError(
            f"backend-rs binary not found at {bin_path}; run "
            "cargo build --manifest-path backend-rs/Cargo.toml --release --bins first"
        )
    port = _free_port()
    env = os.environ.copy()
    env.update(
        {
            "HOME": str(shared_app_home),
            "CODEX_HOME": str(shared_app_home / ".codex"),
            "PI_HOME": str(shared_app_home / ".pi"),
            "CODEX_WEB_PASSWORD": CONTRACT_PASSWORD,
            "CODEX_WEB_HOST": "127.0.0.1",
            "CODEX_WEB_PORT": str(port),
            "CODEX_WEB_DISCOVER_MIN_INTERVAL_SECONDS": "0",
            "CODEX_WEB_PUSH_VAPID_SUBJECT": "https://localhost",
            "CODOXEAR_APP_DIR": str(shared_app_home / ".local/share/codoxear"),
        }
    )
    url_prefix = os.environ.get("CODEX_WEB_URL_PREFIX")
    if url_prefix:
        env["CODEX_WEB_URL_PREFIX"] = url_prefix
    proc = subprocess.Popen(
        [str(bin_path)],
        cwd=str(repo_root),
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    url = f"http://127.0.0.1:{port}"
    try:
        health_path = f"{url_prefix}/api/v1/health" if url_prefix else "/api/v1/health"
        _wait_for_http(url, health_path=health_path)
        yield url
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)


@pytest.fixture
def signed_auth_cookie(rust_server_url: str) -> str:
    import urllib.request

    body = json.dumps({"password": CONTRACT_PASSWORD}).encode("utf-8")
    request = urllib.request.Request(
        f"{rust_server_url}/api/login",
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        cookie = response.headers.get("Set-Cookie", "")
    return cookie.split(";", 1)[0]


def _load_or_create_hmac_secret(home: Path) -> bytes:
    app_dir = home / ".local/share/codoxear"
    app_dir.mkdir(parents=True, exist_ok=True)
    path = app_dir / "hmac_secret"
    if path.exists():
        raw = path.read_bytes()
        if len(raw) < 32:
            raise ValueError(f"invalid hmac secret (too short): {path}")
        return raw[:64]
    secret = b"x" * 64
    path.write_bytes(secret)
    os.chmod(path, 0o600)
    return secret


def _b64u(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).decode("ascii").rstrip("=")
