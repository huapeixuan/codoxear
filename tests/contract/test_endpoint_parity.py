from __future__ import annotations

import json
import urllib.parse
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

import pytest


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
    (socks / f"{session_id}.sock").write_text("", encoding="utf-8")
    payload: dict[str, Any] = {
        "session_id": f"thread-{session_id}",
        "agent_backend": backend,
        "backend": backend,
        "owner": "web",
        "transport": "pi-rpc" if backend == "pi" else "pty",
        "cwd": str(cwd),
        "start_ts": 100.0,
        "updated_ts": 200.0,
        "broker_pid": 0,
        "codex_pid": 0,
        "busy": False,
        "queue_len": 1,
    }
    if log_path is not None:
        payload["log_path"] = str(log_path)
    if session_path is not None:
        payload["session_path"] = str(session_path)
    (socks / f"{session_id}.json").write_text(json.dumps(payload), encoding="utf-8")


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
        "/api/sessions/sess-contract/messages?init=1&limit=20",
        "/api/sessions/sess-contract/messages?offset=0&limit=20",
        "/api/sessions/sess-contract/tail",
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
    log_path.write_text("", encoding="utf-8")
    _write_contract_session(shared_app_dir, cwd=cwd, log_path=log_path)

    python_response = _get_response(python_server_url, path, signed_auth_cookie)
    rust_response = _get_response(rust_server_url, path, signed_auth_cookie)

    assert python_response.status == rust_response.status
    assert_content_type_equal(python_response, rust_response, path)
    assert_json_equivalent(
        python_response.json(), rust_response.json(), ignore_keys={"meta_refresh_ms"}
    )
