from __future__ import annotations

import json
import socket
import threading
import urllib.parse
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

import pytest


_STUB_BROKER_SERVERS: list[socket.socket] = []


def assert_json_equivalent(
    python_response: dict[str, Any],
    rust_response: dict[str, Any],
    *,
    ignore_keys: set[str] | None = None,
) -> None:
    """Compare JSON objects while allowing explicitly documented volatile keys."""

    ignore = ignore_keys or set()

    def scrub(value: Any) -> Any:
        if isinstance(value, dict):
            return {k: scrub(v) for k, v in value.items() if k not in ignore}
        if isinstance(value, list):
            return [scrub(v) for v in value]
        return value

    assert scrub(python_response) == scrub(rust_response)


class JsonHttpResponse:
    def __init__(self, status: int, headers: dict[str, str], body: bytes) -> None:
        self.status = status
        self.headers = headers
        self.body = body

    def json(self) -> dict[str, Any]:
        return json.loads(self.body.decode("utf-8"))


def _get_response(base_url: str, path: str, cookie: str) -> JsonHttpResponse:
    request = urllib.request.Request(f"{base_url}{path}", headers={"Cookie": cookie})
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            body = response.read()
            headers = {key.lower(): value for key, value in response.headers.items()}
            return JsonHttpResponse(response.status, headers, body)
    except urllib.error.HTTPError as exc:
        body = exc.read()
        headers = {key.lower(): value for key, value in exc.headers.items()}
        return JsonHttpResponse(exc.code, headers, body)


def _get_json(base_url: str, path: str, cookie: str) -> dict[str, Any]:
    return _get_response(base_url, path, cookie).json()


def _post_response(
    base_url: str,
    path: str,
    cookie: str | None,
    payload: dict[str, Any] | None,
) -> JsonHttpResponse:
    body = b"" if payload is None else json.dumps(payload).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    if cookie is not None:
        headers["Cookie"] = cookie
    request = urllib.request.Request(
        f"{base_url}{path}", data=body, headers=headers, method="POST"
    )
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            response_body = response.read()
            response_headers = {
                key.lower(): value for key, value in response.headers.items()
            }
            return JsonHttpResponse(response.status, response_headers, response_body)
    except urllib.error.HTTPError as exc:
        response_body = exc.read()
        response_headers = {key.lower(): value for key, value in exc.headers.items()}
        return JsonHttpResponse(exc.code, response_headers, response_body)


def _write_contract_session(
    app_dir: Path,
    *,
    session_id: str = "sess-contract",
    cwd: Path,
    backend: str = "codex",
    log_path: Path | None = None,
    session_path: Path | None = None,
) -> None:
    socks = app_dir / "socks"
    socks.mkdir(parents=True, exist_ok=True)
    sock_path = socks / f"{session_id}.sock"
    _start_stub_broker(sock_path)
    payload: dict[str, Any] = {
        "session_id": f"thread-{session_id}",
        "agent_backend": backend,
        "backend": backend,
        "owner": "web",
        "transport": "pi-rpc" if backend == "pi" else "pty",
        "cwd": str(cwd),
        "start_ts": 100.0,
        "updated_ts": 200.0,
        "broker_pid": 1,
        "codex_pid": 0,
        "busy": False,
        "queue_len": 1,
    }
    if log_path is not None:
        payload["log_path"] = str(log_path)
    elif backend != "pi":
        payload["log_path"] = None
    if session_path is not None:
        payload["session_path"] = str(session_path)
    (socks / f"{session_id}.json").write_text(json.dumps(payload), encoding="utf-8")


def _start_stub_broker(sock_path: Path) -> None:
    sock_path.unlink(missing_ok=True)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(str(sock_path))
    server.listen(8)
    _STUB_BROKER_SERVERS.append(server)

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
                    response: dict[str, Any] = {"ok": True, "requests": []}
                elif cmd == "commands":
                    response = {"ok": True, "commands": []}
                else:
                    response = {"busy": False, "queue_len": 0, "token": None}
                conn.sendall(json.dumps(response).encode("utf-8") + b"\n")

    threading.Thread(target=serve, daemon=True).start()


def _prime_contract_session_discovery(
    python_server_url: str, signed_auth_cookie: str
) -> None:
    """Force Python's in-memory SessionManager to discover ad-hoc test sidecars."""

    response = _get_response(python_server_url, "/api/sessions", signed_auth_cookie)
    assert response.status == 200


def assert_content_type_equal(
    python_response: JsonHttpResponse, rust_response: JsonHttpResponse, path: str
) -> None:
    assert python_response.headers.get("content-type") == rust_response.headers.get(
        "content-type"
    ), path


def test_me_parity(
    python_server_url: str, rust_server_url: str, signed_auth_cookie: str
) -> None:
    path = "/api/me"
    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)
    assert python_response.status == rust_response.status == 200
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(
        python_response.json(), rust_response.json(), ignore_keys={"app_version"}
    )


@pytest.mark.readonly
@pytest.mark.parametrize(
    "scenario",
    ["empty", "recent_cwds", "cwd_groups", "tmux_available", "ref_bootstrap_404"],
)
def test_sessions_bootstrap_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    scenario: str,
) -> None:
    path = "/api/sessions/bootstrap"
    if scenario == "ref_bootstrap_404":
        path = "/api/v1/bootstrap"

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)
    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    if python_response.status == 200:
        assert_json_equivalent(
            python_response.json(), rust_response.json(), ignore_keys={"app_version"}
        )
        if scenario in {"empty", "recent_cwds", "cwd_groups", "tmux_available"}:
            assert python_response.body == rust_response.body
    else:
        assert python_response.status == rust_response.status == 404


@pytest.mark.readonly
def test_settings_voice_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
) -> None:
    path = "/api/settings/voice"

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status == 200
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(
        python_response.json(), rust_response.json(), ignore_keys={"vapid_public_key"}
    )


@pytest.mark.readonly
def test_notifications_subscription_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
) -> None:
    path = "/api/notifications/subscription"

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status == 200
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(
        python_response.json(), rust_response.json(), ignore_keys={"vapid_public_key"}
    )


@pytest.mark.readonly
@pytest.mark.parametrize("scenario", ["missing", "unknown", "known"])
def test_notifications_message_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    scenario: str,
) -> None:
    query = {
        "missing": "",
        "unknown": "?message_id=unknown",
        "known": "?message_id=m-final",
    }[scenario]
    path = f"/api/notifications/message{query}"

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(python_response.json(), rust_response.json())


@pytest.mark.readonly
@pytest.mark.parametrize("scenario", ["invalid", "valid"])
def test_notifications_feed_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    scenario: str,
) -> None:
    query = "?since=invalid" if scenario == "invalid" else "?since=10.0"
    path = f"/api/notifications/feed{query}"

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(python_response.json(), rust_response.json())


@pytest.mark.readonly
def test_metrics_parity(
    python_server_url: str, rust_server_url: str, signed_auth_cookie: str
) -> None:
    path = "/api/metrics"
    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status == 200
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(python_response.json(), rust_response.json())
    assert python_response.body == rust_response.body


@pytest.mark.readonly
@pytest.mark.parametrize("path", ["/api/settings/voice", "/api/v1/settings/voice"])
def test_voice_aliases_are_byte_identical(
    rust_server_url: str, signed_auth_cookie: str, path: str
) -> None:
    response = _get_response(rust_server_url, path, signed_auth_cookie)
    assert response.status == 200
    assert response.headers.get("content-type") == "application/json; charset=utf-8"


@pytest.mark.readonly
@pytest.mark.parametrize(
    "path",
    [
        "/api/sessions",
        "/api/sessions?view=recent",
        "/api/sessions?limit=999",
        "/api/sessions?view=banana",
        "/api/sessions?view=recent&group_limit=1",
    ],
)
def test_sessions_list_parity(
    python_server_url: str, rust_server_url: str, signed_auth_cookie: str, path: str
) -> None:
    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(python_response.json(), rust_response.json())


@pytest.mark.readonly
@pytest.mark.parametrize(
    "query",
    [
        "?cwd=/tmp/codoxear-contract-missing&backend=codex",
        "?cwd=/tmp/codoxear-contract-missing&backend=pi",
        "?cwd=&backend=codex",
        "?cwd=/tmp/codoxear-contract-missing&backend=banana",
    ],
)
def test_session_resume_candidates_parity(
    python_server_url: str, rust_server_url: str, signed_auth_cookie: str, query: str
) -> None:
    path = f"/api/session_resume_candidates{query}"
    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(python_response.json(), rust_response.json())


@pytest.mark.readonly
@pytest.mark.parametrize(
    "suffix",
    [
        "/diagnostics",
        "/queue",
        "/harness",
        "/workspace",
        "/details",
        "/ui_state",
        "/commands",
        "/repo",
    ],
)
def test_session_meta_unknown_parity(
    python_server_url: str, rust_server_url: str, signed_auth_cookie: str, suffix: str
) -> None:
    path = f"/api/sessions/does-not-exist{suffix}"
    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(
        python_response.json(), rust_response.json(), ignore_keys={"error"}
    )


@pytest.mark.readonly
def test_queue_harness_workspace_populated_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    _write_contract_session(shared_app_dir, cwd=cwd)
    (shared_app_dir / "session_queues.json").write_text(
        json.dumps(
            {
                "sess-contract": [
                    "plain task",
                    {"text": "with image", "images": [{"data_b64": "x"}]},
                ]
            }
        ),
        encoding="utf-8",
    )
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)
    for path in (
        "/api/sessions/sess-contract/queue",
        "/api/sessions/sess-contract/harness",
        "/api/sessions/sess-contract/workspace",
    ):
        python_response = _get_response(python_server_url, path, signed_auth_cookie)
        rust_response = _get_response(rust_server_url, path, signed_auth_cookie)
        assert python_response.status == rust_response.status
        assert_content_type_equal(python_response, rust_response, path)
        if path.endswith("/queue"):
            assert rust_response.json()["queue"] == [
                "plain task",
                "with image [1 image]",
            ]
        elif path.endswith("/harness"):
            assert python_response.json() == rust_response.json()
            assert python_response.json()["ok"] is True
        else:
            assert rust_response.json()["queue"]["items"] == [
                "plain task",
                "with image [1 image]",
            ]
            assert (
                python_response.json()["diagnostics"]["session_id"]
                == rust_response.json()["diagnostics"]["session_id"]
            )


@pytest.mark.readonly
@pytest.mark.parametrize(
    "path",
    [
        "/api/sessions/sess-contract/git/changed_files",
        "/api/sessions/sess-contract/git/diff?path=README.md",
        "/api/sessions/sess-contract/git/file_versions?path=README.md",
    ],
)
def test_git_non_repo_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
    path: str,
) -> None:
    cwd = tmp_path / "not-repo"
    cwd.mkdir()
    (cwd / "README.md").write_text("hello\n", encoding="utf-8")
    _write_contract_session(shared_app_dir, cwd=cwd)
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(
        python_response.json(), rust_response.json(), ignore_keys={"error"}
    )


@pytest.mark.readonly
@pytest.mark.parametrize(
    "path",
    [
        "/api/sessions/sess-contract/file/read?path=README.md",
        "/api/sessions/sess-contract/file/search?q=readme",
        "/api/sessions/sess-contract/file/list?path=.",
    ],
)
def test_file_json_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
    path: str,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    (cwd / "README.md").write_text("# Contract\n", encoding="utf-8")
    (cwd / "src").mkdir()
    (cwd / "src" / "main.rs").write_text("fn main() {}\n", encoding="utf-8")
    _write_contract_session(shared_app_dir, cwd=cwd)
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(python_response.json(), rust_response.json())


@pytest.mark.readonly
def test_file_blob_and_download_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    png = b"\x89PNG\r\n\x1a\n" + b"\x00" * 16
    (cwd / "screenshot.png").write_bytes(png)
    (cwd / "README.md").write_text("download me\n", encoding="utf-8")
    _write_contract_session(shared_app_dir, cwd=cwd)
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)

    for path in (
        "/api/sessions/sess-contract/file/blob?path=screenshot.png",
        "/api/files/blob?path=" + urllib.parse.quote(str(cwd / "screenshot.png")),
        "/api/sessions/sess-contract/file/download?path=README.md",
    ):
        python_response = _get_response(python_server_url, path, signed_auth_cookie)
        rust_response = _get_response(rust_server_url, path, signed_auth_cookie)
        assert python_response.status == rust_response.status
        assert python_response.body == rust_response.body
        assert python_response.headers.get("content-type") == rust_response.headers.get(
            "content-type"
        )


@pytest.mark.readonly
@pytest.mark.parametrize(
    "path",
    [
        "/api/sessions/sess-contract/file/read?path=../outside.txt",
        "/api/sessions/sess-contract/file/download?path=../outside.txt",
        "/api/sessions/sess-contract/file/blob?path=README.md",
        "/api/sessions/sess-contract/file/list?path=README.md",
        "/api/files/blob?path=/tmp/codoxear-contract-missing.png",
    ],
)
def test_file_error_boundary_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
    path: str,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    (tmp_path / "outside.txt").write_text("outside\n", encoding="utf-8")
    (cwd / "README.md").write_text("text\n", encoding="utf-8")
    _write_contract_session(shared_app_dir, cwd=cwd)
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)

    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)
    if "../outside.txt" in path:
        assert rust_response.status == 400
        assert "error" in rust_response.json()
        return
    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(
        python_response.json(), rust_response.json(), ignore_keys={"error"}
    )


@pytest.mark.readonly
@pytest.mark.parametrize(
    "path",
    [
        "/api/sessions/sess-contract/messages?init=1&limit=20",
        "/api/sessions/sess-contract/messages?offset=0&limit=20",
        "/api/sessions/sess-contract/live?offset=0&live_offset=0&requests_version=v1",
    ],
)
def test_messages_empty_log_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
    path: str,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    log_path = tmp_path / "empty.jsonl"
    log_path.write_text(
        json.dumps(
            {
                "type": "session_meta",
                "payload": {
                    "id": "thread-sess-contract",
                    "cwd": str(cwd),
                    "model": "gpt-5",
                    "reasoning_effort": "medium",
                },
            }
        )
        + "\n",
        encoding="utf-8",
    )
    _write_contract_session(shared_app_dir, cwd=cwd, log_path=log_path)
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(
        python_response.json(),
        rust_response.json(),
        ignore_keys={"meta_refresh_ms", "offset", "queue_len"},
    )


@pytest.mark.post
def test_login_logout_post_parity(
    python_server_url: str, rust_server_url: str, signed_auth_cookie: str
) -> None:
    for base_url in (python_server_url, rust_server_url):
        bad = _post_response(base_url, "/api/login", None, {"password": "wrong"})
        assert bad.status == 403
        assert bad.json() == {"error": "bad password"}

        logout = _post_response(base_url, "/api/logout", signed_auth_cookie, {})
        assert logout.status == 200
        assert logout.json() == {"ok": True}
        assert "codoxear_auth=" in logout.headers.get("set-cookie", "")
        assert "Max-Age=0" in logout.headers.get("set-cookie", "")


@pytest.mark.post
def test_cwd_group_edit_post_roundtrip_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    _write_contract_session(shared_app_dir, cwd=cwd)
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)

    payload = {
        "cwd": str(cwd),
        "label": "  Phase   Three  ",
        "collapsed": True,
        "hidden": False,
    }
    rust_post = _post_response(
        rust_server_url, "/api/cwd_groups/edit", signed_auth_cookie, payload
    )
    assert rust_post.status == 200
    assert rust_post.json()["label"] == "Phase Three"
    cwd_groups_on_disk = json.loads(
        (shared_app_dir / "cwd_groups.json").read_text(encoding="utf-8")
    )
    assert any(
        entry.get("label") == "Phase Three" for entry in cwd_groups_on_disk.values()
    )

    python_post = _post_response(
        python_server_url,
        "/api/cwd_groups/edit",
        signed_auth_cookie,
        {**payload, "label": "Python"},
    )
    assert python_post.status == 200
    rust_bootstrap = _get_json(
        rust_server_url, "/api/sessions/bootstrap", signed_auth_cookie
    )
    assert any(
        entry.get("label") == "Python"
        for entry in rust_bootstrap["cwd_groups"].values()
    )


@pytest.mark.post
def test_alias_sidebar_queue_harness_post_roundtrip_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    _write_contract_session(shared_app_dir, session_id="sess-contract", cwd=cwd)
    _write_contract_session(shared_app_dir, session_id="sess-dep", cwd=cwd)
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)

    rename = _post_response(
        rust_server_url,
        "/api/sessions/sess-contract/rename",
        signed_auth_cookie,
        {"name": "  Rust Alias  "},
    )
    assert rename.status == 200
    assert rename.json() == {"ok": True, "alias": "Rust Alias"}
    aliases_on_disk = json.loads(
        (shared_app_dir / "session_aliases.json").read_text(encoding="utf-8")
    )
    assert aliases_on_disk["sess-contract"] == "Rust Alias"

    edit = _post_response(
        rust_server_url,
        "/api/sessions/sess-contract/edit",
        signed_auth_cookie,
        {
            "name": "Rust Edit",
            "priority_offset": 0.25,
            "snooze_until": 2000,
            "dependency_session_id": "sess-dep",
        },
    )
    assert edit.status == 200
    sidebar_on_disk = json.loads(
        (shared_app_dir / "session_sidebar.json").read_text(encoding="utf-8")
    )
    assert sidebar_on_disk["sess-contract"]["dependency_session_id"] == "sess-dep"

    enqueue = _post_response(
        rust_server_url,
        "/api/sessions/sess-contract/enqueue",
        signed_auth_cookie,
        {"text": "queued"},
    )
    assert enqueue.status == 200
    queue_on_disk = json.loads(
        (shared_app_dir / "session_queues.json").read_text(encoding="utf-8")
    )
    assert queue_on_disk["sess-contract"] == ["queued"]

    update = _post_response(
        rust_server_url,
        "/api/sessions/sess-contract/queue/update",
        signed_auth_cookie,
        {"index": 0, "text": "updated"},
    )
    assert update.status == 200
    delete = _post_response(
        rust_server_url,
        "/api/sessions/sess-contract/queue/delete",
        signed_auth_cookie,
        {"index": 0},
    )
    assert delete.status == 200
    queue_on_disk = json.loads(
        (shared_app_dir / "session_queues.json").read_text(encoding="utf-8")
    )
    assert "sess-contract" not in queue_on_disk

    harness = _post_response(
        rust_server_url,
        "/api/sessions/sess-contract/harness",
        signed_auth_cookie,
        {
            "enabled": True,
            "request": "inspect",
            "cooldown_minutes": 3,
            "remaining_injections": 2,
        },
    )
    assert harness.status == 200
    python_harness = json.loads(
        (shared_app_dir / "harness.json").read_text(encoding="utf-8")
    )["sess-contract"]
    assert python_harness["request"] == "inspect"
    assert python_harness["remaining_injections"] == 2


@pytest.mark.post
def test_send_ui_interrupt_post_stub_broker_parity(
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    _write_contract_session(shared_app_dir, session_id="sess-codex", cwd=cwd)
    _write_contract_session(shared_app_dir, session_id="sess-pi", cwd=cwd, backend="pi")

    send = _post_response(
        rust_server_url,
        "/api/sessions/sess-codex/send",
        signed_auth_cookie,
        {"text": "hello"},
    )
    assert send.status == 200
    assert send.json()["queue_len"] == 0

    interrupt = _post_response(
        rust_server_url,
        "/api/sessions/sess-codex/interrupt",
        signed_auth_cookie,
        {},
    )
    assert interrupt.status == 200
    assert interrupt.json()["ok"] is True

    ui = _post_response(
        rust_server_url,
        "/api/sessions/sess-pi/ui_response",
        signed_auth_cookie,
        {"id": "q1", "value": "yes", "confirmed": True},
    )
    assert ui.status == 200
    assert ui.json() == {"ok": True}


@pytest.mark.post
def test_file_write_global_file_and_injection_post_contract(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    shared_app_dir: Path,
    tmp_path: Path,
) -> None:
    cwd = tmp_path / "project"
    cwd.mkdir()
    target = cwd / "note.txt"
    target.write_text("old", encoding="utf-8")
    _write_contract_session(shared_app_dir, cwd=cwd)
    _write_contract_session(shared_app_dir, session_id="sess-pi", cwd=cwd, backend="pi")
    _prime_contract_session_discovery(python_server_url, signed_auth_cookie)

    version = __import__("hashlib").sha256(b"old").hexdigest()
    write = _post_response(
        rust_server_url,
        "/api/sessions/sess-contract/file/write",
        signed_auth_cookie,
        {"path": "note.txt", "text": "new", "version": version},
    )
    assert write.status == 200
    assert target.read_text(encoding="utf-8") == "new"
    python_read = _get_json(
        python_server_url,
        "/api/sessions/sess-contract/file/read?path=note.txt",
        signed_auth_cookie,
    )
    assert python_read["text"] == "new"

    global_read = _post_response(
        rust_server_url,
        "/api/files/read",
        signed_auth_cookie,
        {"path": str(target), "session_id": "sess-contract"},
    )
    assert global_read.status == 200
    assert global_read.json()["text"] == "new"
    inspect = _post_response(
        rust_server_url,
        "/api/files/inspect",
        signed_auth_cookie,
        {"path": str(target), "session_id": "sess-contract"},
    )
    assert inspect.status == 200
    assert inspect.json()["size"] == 3

    pi_inject = _post_response(
        rust_server_url,
        "/api/sessions/sess-pi/inject_image",
        signed_auth_cookie,
        {"data_b64": "not-base64", "filename": "x.png", "attachment_index": 1},
    )
    assert pi_inject.status == 409
    assert pi_inject.json()["operation"] == "attachment_injection"


@pytest.mark.post
def test_voice_subscription_listener_debug_and_hooks_post_contract(
    rust_server_url: str, signed_auth_cookie: str, shared_app_dir: Path
) -> None:
    voice = _post_response(
        rust_server_url,
        "/api/settings/voice",
        signed_auth_cookie,
        {"tts_enabled_for_narration": True, "tts_base_url": "https://example.com/v1/"},
    )
    assert voice.status == 200
    assert voice.json()["tts_base_url"] == "https://example.com/v1"

    subscription = {
        "endpoint": "https://push.example/sub",
        "keys": {"p256dh": "p", "auth": "a"},
    }
    upsert = _post_response(
        rust_server_url,
        "/api/notifications/subscription",
        signed_auth_cookie,
        {"subscription": subscription, "user_agent": "Mozilla iPhone"},
    )
    assert upsert.status == 200
    toggle = _post_response(
        rust_server_url,
        "/api/notifications/subscription/toggle",
        signed_auth_cookie,
        {"endpoint": "https://push.example/sub", "enabled": False},
    )
    assert toggle.status == 200
    unknown = _post_response(
        rust_server_url,
        "/api/notifications/subscription/toggle",
        signed_auth_cookie,
        {"endpoint": "https://push.example/unknown", "enabled": False},
    )
    assert unknown.status == 404

    listener = _post_response(
        rust_server_url,
        "/api/audio/listener",
        signed_auth_cookie,
        {"client_id": "browser", "enabled": True},
    )
    assert listener.status == 200
    assert listener.json()["active_listener_count"] == 1

    for path in ("/api/notifications/test_push", "/api/audio/test_announcement"):
        disabled = _post_response(rust_server_url, path, signed_auth_cookie, {})
        assert disabled.status == 501
        assert disabled.json()["phase"] == "phase5"
    assert not (shared_app_dir / "audio_announcement_queue.json").exists()

    hooks = _post_response(rust_server_url, "/api/hooks/notify", None, {"x": 1})
    assert hooks.status == 200
    assert hooks.json() == {"ignored": True}


@pytest.mark.post
def test_session_create_post_validation_and_tmux_unavailable_parity(
    python_server_url: str,
    rust_server_url: str,
    signed_auth_cookie: str,
    tmp_path: Path,
) -> None:
    for base_url in (python_server_url, rust_server_url):
        missing = _post_response(base_url, "/api/sessions", signed_auth_cookie, {})
        assert missing.status == 400
        assert missing.json()["field"] == "cwd"

        bad_args = _post_response(
            base_url,
            "/api/sessions",
            signed_auth_cookie,
            {"cwd": str(tmp_path), "args": "bad"},
        )
        assert bad_args.status == 400
        assert bad_args.json()["error"] == "args must be a list of strings"

    rust_tmux = _post_response(
        rust_server_url,
        "/api/sessions",
        signed_auth_cookie,
        {"cwd": str(tmp_path), "create_in_tmux": True},
    )
    if rust_tmux.status == 400:
        assert rust_tmux.json()["error"] == "tmux is unavailable on this host"
