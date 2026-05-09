from __future__ import annotations

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


@pytest.mark.skip(reason="Phase 1 will enable once rust_server_url launches Rust")
def test_me_parity(python_server_url: str, rust_server_url: str) -> None:
    python_response: dict[str, Any] = {}
    rust_response: dict[str, Any] = {}
    assert_json_equivalent(python_response, rust_response, ignore_keys={"app_version"})


@pytest.mark.skip(reason="Phase 1 will enable once rust_server_url launches Rust")
def test_sessions_bootstrap_parity(
    python_server_url: str, rust_server_url: str
) -> None:
    python_response: dict[str, Any] = {}
    rust_response: dict[str, Any] = {}
    assert_json_equivalent(python_response, rust_response, ignore_keys={"app_version"})


@pytest.mark.skip(reason="Phase 1 will enable once rust_server_url launches Rust")
def test_sessions_parity(python_server_url: str, rust_server_url: str) -> None:
    python_response: dict[str, Any] = {}
    rust_response: dict[str, Any] = {}
    assert_json_equivalent(python_response, rust_response, ignore_keys={"app_version"})
