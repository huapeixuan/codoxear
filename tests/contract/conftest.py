from __future__ import annotations

import contextlib
import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Iterator

import pytest


CONTRACT_PASSWORD = "codoxear-contract-password"


def _free_port() -> int:
    with contextlib.closing(socket.socket(socket.AF_INET, socket.SOCK_STREAM)) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _wait_for_http(url: str, *, timeout: float = 10.0) -> None:
    import urllib.error
    import urllib.request

    deadline = time.time() + timeout
    last_error: Exception | None = None
    while time.time() < deadline:
        try:
            # Any HTTP response proves the process is accepting connections; /api/me
            # is expected to be 401 before login.
            with urllib.request.urlopen(f"{url}/api/me", timeout=0.5):
                return
        except urllib.error.HTTPError:
            return
        except Exception as exc:  # pragma: no cover - diagnostic path
            last_error = exc
            time.sleep(0.1)
    raise RuntimeError(f"server did not become ready at {url}: {last_error}")


@pytest.fixture
def python_server_url() -> Iterator[str]:
    """Run the current Python server on a random local port.

    The production code currently derives its app dir from HOME, so the fixture
    gives the subprocess an isolated HOME and therefore an isolated
    ~/.local/share/codoxear tree.
    """

    port = _free_port()
    with tempfile.TemporaryDirectory(prefix="codoxear-contract-home-") as home:
        env = os.environ.copy()
        env.update(
            {
                "HOME": home,
                "CODEX_WEB_PASSWORD": CONTRACT_PASSWORD,
                "CODEX_WEB_HOST": "127.0.0.1",
                "CODEX_WEB_PORT": str(port),
                "CODOXEAR_USE_LEGACY_WEB": "1",
            }
        )
        proc = subprocess.Popen(
            [sys.executable, "-m", "codoxear.server"],
            cwd=str(Path(__file__).resolve().parents[2]),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        url = f"http://127.0.0.1:{port}"
        try:
            _wait_for_http(url)
            yield url
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:  # pragma: no cover - cleanup fallback
                proc.kill()
                proc.wait(timeout=5)


@pytest.fixture
def rust_server_url() -> str:
    """Final Rust server fixture signature.

    Phase 1 will replace this skip with a subprocess launch of
    backend-rs/target/release/codoxear-backend-rs on a random local port.
    """

    pytest.skip("Rust backend is not available until Phase 1 (rust-backend-skeleton)")
