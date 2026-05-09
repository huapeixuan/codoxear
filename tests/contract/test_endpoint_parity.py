from __future__ import annotations

import json
from typing import Any
import urllib.request

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


def _get_json(base_url: str, path: str, cookie: str) -> dict[str, Any]:
    request = urllib.request.Request(f"{base_url}{path}", headers={"Cookie": cookie})
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.loads(response.read().decode("utf-8"))


def test_me_parity(
    python_server_url: str, rust_server_url: str, signed_auth_cookie: str
) -> None:
    python_response = _get_json(python_server_url, "/api/me", signed_auth_cookie)
    rust_response = _get_json(rust_server_url, "/api/me", signed_auth_cookie)
    assert_json_equivalent(python_response, rust_response, ignore_keys={"app_version"})


def test_sessions_bootstrap_parity(
    python_server_url: str, rust_server_url: str, signed_auth_cookie: str
) -> None:
    python_response = _get_json(
        python_server_url, "/api/sessions/bootstrap", signed_auth_cookie
    )
    rust_response = _get_json(
        rust_server_url, "/api/sessions/bootstrap", signed_auth_cookie
    )
    assert_json_equivalent(python_response, rust_response, ignore_keys={"app_version"})


@pytest.mark.skip(reason="Phase 2 (rust-backend-readonly-routes)")
def test_sessions_parity(python_server_url: str, rust_server_url: str) -> None:
    python_response: dict[str, Any] = {}
    rust_response: dict[str, Any] = {}
    assert_json_equivalent(python_response, rust_response, ignore_keys={"app_version"})
