from __future__ import annotations

import base64
import contextlib
import hashlib
import hmac
import json
import os
import secrets
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


@pytest.fixture(scope="session")
def shared_app_home() -> Iterator[Path]:
    """Shared HOME for Python and Rust contract servers."""

    with tempfile.TemporaryDirectory(prefix="codoxear-contract-home-") as home:
        yield Path(home)


@pytest.fixture
def preseed_contract_state(
    shared_app_dir: Path, request: pytest.FixtureRequest
) -> None:
    for name in (
        "recent_cwds.json",
        "cwd_groups.json",
        "voice_settings.json",
        "push_subscriptions.json",
        "voice_delivery_ledger.json",
    ):
        (shared_app_dir / name).unlink(missing_ok=True)
    scenario = getattr(request.node, "callspec", None)
    scenario_name = scenario.params.get("scenario") if scenario else None
    if scenario_name == "recent_cwds":
        (shared_app_dir / "recent_cwds.json").write_text(
            json.dumps(
                {
                    "/tmp/a": 10,
                    "/tmp/b": 30,
                    "/tmp/c": 20,
                    "": 99,
                    "/tmp/ignore-bool": True,
                }
            ),
            encoding="utf-8",
        )
    elif scenario_name == "cwd_groups":
        (shared_app_dir / "cwd_groups.json").write_text(
            json.dumps(
                {
                    "/tmp/project-a": {"label": "A", "collapsed": True},
                    "/tmp/project-b": {"hidden": True},
                    "/tmp/project-c": {
                        "label": "C",
                        "hidden": True,
                        "hidden_after_live_start_ts": 123.5,
                    },
                }
            ),
            encoding="utf-8",
        )

    test_name = request.node.name
    if "settings_voice_parity" in test_name:
        (shared_app_dir / "voice_settings.json").write_text(
            json.dumps(
                {
                    "tts_enabled_for_narration": True,
                    "tts_enabled_for_final_response": True,
                    "tts_base_url": "https://api.openai.com/v1/",
                    "tts_api_key": "test-key",
                    "summarization_model": "summary-model",
                    "tts_model": "tts-model",
                }
            ),
            encoding="utf-8",
        )
    if "notifications_subscription_parity" in test_name:
        (shared_app_dir / "push_subscriptions.json").write_text(
            json.dumps(
                [
                    {
                        "subscription": {
                            "endpoint": "https://push.example/one",
                            "keys": {"p256dh": "p1", "auth": "a1"},
                        },
                        "notifications_enabled": True,
                        "created_ts": 1.0,
                        "updated_ts": 2.0,
                        "user_agent": "Mozilla Mobile",
                        "device_label": "phone",
                    },
                    {
                        "subscription": {
                            "endpoint": "https://push.example/two",
                            "keys": {"p256dh": "p2", "auth": "a2"},
                        },
                        "notifications_enabled": False,
                        "created_ts": 3.0,
                        "updated_ts": 4.0,
                        "device_class": "desktop",
                        "device_label": "desktop",
                    },
                ]
            ),
            encoding="utf-8",
        )
    if (
        "notifications_message_parity" in test_name
        or "notifications_feed_parity" in test_name
    ):
        (shared_app_dir / "voice_delivery_ledger.json").write_text(
            json.dumps(
                {
                    "msg-1": {
                        "session_id": "session-1",
                        "session_display_name": "Alpha",
                        "message_class": "final_response",
                        "notification_text": "  hello   world ",
                        "summary_status": "sent",
                        "narrated_status": "pending",
                        "push_status": "skipped",
                        "updated_ts": 10.0,
                    },
                    "msg-2": {
                        "session_id": "session-2",
                        "message_class": "narration",
                        "notification_text": "ignored",
                        "summary_status": "sent",
                        "push_status": "sent",
                        "updated_ts": 11.0,
                    },
                }
            ),
            encoding="utf-8",
        )


@pytest.fixture
def python_server_url(
    shared_app_home: Path, preseed_contract_state: None
) -> Iterator[str]:
    """Run the current Python server on a random local port."""

    port = _free_port()
    env = os.environ.copy()
    env.update(
        {
            "HOME": str(shared_app_home),
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
def rust_server_url(
    shared_app_home: Path, preseed_contract_state: None
) -> Iterator[str]:
    """Run the Phase 1 Rust backend binary on a random local port."""

    repo_root = Path(__file__).resolve().parents[2]
    bin_path = repo_root / "backend-rs/target/release/codoxear-backend-rs"
    if not bin_path.exists():
        message = (
            f"backend-rs binary not found at {bin_path}; "
            "run (cd backend-rs && cargo build --release --bins) first"
        )
        if os.environ.get("CODOXEAR_SKIP_RUST_FIXTURE") == "1":
            pytest.skip(message)
        raise RuntimeError(message)

    port = _free_port()
    env = os.environ.copy()
    env.update(
        {
            "HOME": str(shared_app_home),
            "CODEX_WEB_PASSWORD": CONTRACT_PASSWORD,
            "CODEX_WEB_HOST": "127.0.0.1",
            "CODEX_WEB_PORT": str(port),
        }
    )
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
def shared_app_dir(shared_app_home: Path) -> Path:
    app_dir = shared_app_home / ".local/share/codoxear"
    app_dir.mkdir(parents=True, exist_ok=True)
    return app_dir


@pytest.fixture
def signed_auth_cookie(shared_app_home: Path) -> str:
    secret = _load_or_create_hmac_secret(shared_app_home)
    payload = json.dumps(
        {"exp": int(time.time()) + 3600}, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")
    sig = hmac.new(secret, payload, hashlib.sha256).digest()
    token = f"{_b64u(payload)}.{_b64u(sig)}"
    return f"codoxear_auth={token}"


def _load_or_create_hmac_secret(home: Path) -> bytes:
    app_dir = home / ".local/share/codoxear"
    app_dir.mkdir(parents=True, exist_ok=True)
    path = app_dir / "hmac_secret"
    if path.exists():
        raw = path.read_bytes()
        if len(raw) < 32:
            raise ValueError(f"invalid hmac secret (too short): {path}")
        return raw[:64]
    secret = secrets.token_bytes(64)
    path.write_bytes(secret)
    os.chmod(path, 0o600)
    return secret


def _b64u(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).decode("ascii").rstrip("=")
