from __future__ import annotations

import json
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


@pytest.mark.skip(reason="Phase 2 (rust-backend-readonly-routes)")
def test_sessions_parity(python_server_url: str, rust_server_url: str) -> None:
    python_response: dict[str, Any] = {}
    rust_response: dict[str, Any] = {}
    assert_json_equivalent(python_response, rust_response, ignore_keys={"app_version"})
