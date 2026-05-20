from __future__ import annotations

import json
import os
import socket
import threading
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


def _response(
    base_url: str, path: str, cookie: str
) -> tuple[int, dict[str, str], bytes]:
    request = urllib.request.Request(f"{base_url}{path}", headers={"Cookie": cookie})
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            return (
                response.status,
                {key.lower(): value for key, value in response.headers.items()},
                response.read(),
            )
    except urllib.error.HTTPError as exc:
        return (
            exc.code,
            {key.lower(): value for key, value in exc.headers.items()},
            exc.read(),
        )


def _json(base_url: str, path: str, cookie: str) -> dict[str, Any]:
    status, _headers, body = _response(base_url, path, cookie)
    assert status == 200
    return json.loads(body.decode("utf-8"))


def _start_stub_broker(sock_path: Path) -> None:
    sock_path.unlink(missing_ok=True)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(sock_path))
    server.listen(8)

    def serve() -> None:
        while True:
            try:
                conn, _ = server.accept()
            except OSError:
                return
            with conn:
                data = b""
                while b"\n" not in data:
                    chunk = conn.recv(65536)
                    if not chunk:
                        break
                    data += chunk
                try:
                    request = json.loads(data.split(b"\n", 1)[0].decode("utf-8"))
                except Exception:
                    request = {}
                cmd = request.get("cmd")
                if cmd == "ui_state":
                    response = {"ok": True, "requests": []}
                elif cmd == "commands":
                    response = {"ok": True, "commands": []}
                elif cmd in {"send", "keys", "ui_response", "shutdown"}:
                    response = {"ok": True, "queue_len": 0}
                else:
                    response = {"busy": False, "queue_len": 0, "token": None}
                conn.sendall(json.dumps(response).encode("utf-8") + b"\n")

    threading.Thread(target=serve, daemon=True).start()


def _write_contract_session(app_dir: Path, cwd: Path) -> None:
    socks = app_dir / "socks"
    socks.mkdir(parents=True, exist_ok=True)
    session_id = "sess-contract"
    sock_path = socks / f"{session_id}.sock"
    _start_stub_broker(sock_path)
    (socks / f"{session_id}.json").write_text(
        json.dumps(
            {
                "session_id": f"thread-{session_id}",
                "agent_backend": "codex",
                "backend": "codex",
                "owner": "web",
                "transport": "pty",
                "cwd": str(cwd),
                "start_ts": 100.0,
                "updated_ts": 200.0,
                "broker_pid": 1,
                "codex_pid": 0,
                "busy": False,
                "queue_len": 1,
                "log_path": None,
                "sock_path": str(sock_path),
            }
        ),
        encoding="utf-8",
    )


def test_canonical_and_legacy_aliases_cover_core_routes(
    rust_server_url: str, signed_auth_cookie: str
) -> None:
    for legacy, canonical in (
        ("/api/health", "/api/v1/health"),
        ("/api/me", "/api/v1/me"),
        ("/api/sessions/bootstrap", "/api/v1/sessions/bootstrap"),
        ("/api/sessions", "/api/v1/sessions"),
        ("/manifest.webmanifest", "/manifest.webmanifest"),
        ("/api/settings/voice", "/api/v1/settings/voice"),
    ):
        legacy_resp = _response(rust_server_url, legacy, signed_auth_cookie)
        canonical_resp = _response(rust_server_url, canonical, signed_auth_cookie)
        assert legacy_resp[0] == canonical_resp[0] == 200
        assert legacy_resp[1].get("content-type") == canonical_resp[1].get(
            "content-type"
        )


def test_static_root_and_manifest_are_served(
    rust_server_url: str, signed_auth_cookie: str
) -> None:
    root_status, root_headers, root_body = _response(
        rust_server_url, "/", signed_auth_cookie
    )
    assert root_status == 200
    assert "text/html" in root_headers.get("content-type", "")
    assert b"<" in root_body

    manifest_status, manifest_headers, manifest_body = _response(
        rust_server_url, "/manifest.webmanifest", signed_auth_cookie
    )
    assert manifest_status == 200
    assert "json" in manifest_headers.get(
        "content-type", ""
    ) or manifest_body.startswith(b"{")


def test_url_prefix_serves_ui_api_and_legacy_alias(
    rust_server_url: str, signed_auth_cookie: str
) -> None:
    if os.environ.get("CODEX_WEB_URL_PREFIX") != "/codoxear":
        return

    for path in (
        "/codoxear/",
        "/codoxear/api/v1/health",
        "/codoxear/api/health",
        "/api/v1/health",
    ):
        status, headers, body = _response(rust_server_url, path, signed_auth_cookie)
        assert status == 200, path
        if path.endswith("/"):
            assert "text/html" in headers.get("content-type", "")
            assert b"./src/main.tsx" not in body
        else:
            assert json.loads(body.decode("utf-8"))["ok"] is True


def test_rust_reads_pre_cutover_sidecar_and_workspace_files(
    rust_server_url: str, signed_auth_cookie: str, shared_app_dir: Path, tmp_path: Path
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    (cwd / "README.md").write_text("# Contract\n", encoding="utf-8")
    _write_contract_session(shared_app_dir, cwd)
    sessions = _json(rust_server_url, "/api/sessions", signed_auth_cookie)
    assert any(row["session_id"] == "sess-contract" for row in sessions["sessions"])

    file_response = _json(
        rust_server_url,
        "/api/sessions/sess-contract/file/read?path=README.md",
        signed_auth_cookie,
    )
    assert file_response["text"] == "# Contract\n"


def test_unknown_route_returns_404_without_internal_path(
    rust_server_url: str, signed_auth_cookie: str
) -> None:
    status, _headers, body = _response(
        rust_server_url, "/api/v2/unknown", signed_auth_cookie
    )
    assert status == 404
    assert b"backend-rs/src" not in body
    assert b"codoxear/server.py" not in body
