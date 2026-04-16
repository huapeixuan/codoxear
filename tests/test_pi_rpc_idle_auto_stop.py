import io
import json
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from codoxear.server import Handler
from codoxear.server import Session
from codoxear.server import SessionManager
from codoxear.server import _session_supports_idle_auto_stop


def _make_manager() -> SessionManager:
    mgr = SessionManager.__new__(SessionManager)
    mgr._lock = threading.Lock()
    mgr._sessions = {}
    mgr._queues = {}
    mgr._hidden_sessions = set()
    mgr._aliases = {}
    mgr._sidebar_meta = {}
    mgr._harness = {}
    mgr._files = {}
    mgr._pi_commands_cache = {}
    mgr._recent_cwds = {}
    mgr._discover_existing_if_stale = lambda *args, **kwargs: None  # type: ignore[method-assign]
    mgr._prune_dead_sessions = lambda *args, **kwargs: None  # type: ignore[method-assign]
    mgr.refresh_session_meta = lambda *args, **kwargs: None  # type: ignore[method-assign]
    mgr.delete_session = lambda session_id: session_id == "pi-live"  # type: ignore[method-assign]
    return mgr


class _HandlerHarness:
    def __init__(self, path: str, body: bytes = b"") -> None:
        self.path = path
        self.headers = {"Content-Length": str(len(body))}
        self.rfile = io.BytesIO(body)
        self.wfile = io.BytesIO()
        self.status: int | None = None
        self.sent_headers: list[tuple[str, str]] = []

    def send_response(self, status: int) -> None:
        self.status = status

    def send_header(self, key: str, value: str) -> None:
        self.sent_headers.append((key, value))

    def end_headers(self) -> None:
        return


class TestPiRpcIdleAutoStop(unittest.TestCase):
    def test_session_supports_idle_auto_stop_only_for_web_owned_pi_rpc(self) -> None:
        eligible = Session(
            session_id="pi-live",
            thread_id="thread-1",
            broker_pid=10,
            codex_pid=11,
            agent_backend="pi",
            backend="pi",
            owned=True,
            start_ts=1.0,
            cwd="/tmp",
            log_path=None,
            sock_path=Path("/tmp/pi-live.sock"),
            transport="pi-rpc",
            auto_stop_on_idle=True,
            idle_timeout_seconds=1800,
        )
        self.assertTrue(_session_supports_idle_auto_stop(eligible))

        terminal_owned = Session(
            session_id="pi-term",
            thread_id="thread-2",
            broker_pid=10,
            codex_pid=11,
            agent_backend="pi",
            backend="pi",
            owned=False,
            start_ts=1.0,
            cwd="/tmp",
            log_path=None,
            sock_path=Path("/tmp/pi-term.sock"),
            transport="pi-rpc",
            auto_stop_on_idle=True,
            idle_timeout_seconds=1800,
        )
        self.assertFalse(_session_supports_idle_auto_stop(terminal_owned))

    def test_heartbeat_session_refreshes_last_web_activity(self) -> None:
        mgr = _make_manager()
        mgr._sessions["pi-live"] = Session(
            session_id="pi-live",
            thread_id="thread-1",
            broker_pid=10,
            codex_pid=11,
            agent_backend="pi",
            backend="pi",
            owned=True,
            start_ts=1.0,
            cwd="/tmp",
            log_path=None,
            sock_path=Path("/tmp/pi-live.sock"),
            transport="pi-rpc",
            auto_stop_on_idle=True,
            idle_timeout_seconds=1800,
            last_web_activity_ts=50.0,
        )

        with patch("codoxear.server.time.time", return_value=125.0):
            payload = mgr.heartbeat_session("pi-live")

        self.assertEqual(payload["session_id"], "pi-live")
        self.assertEqual(payload["idle_timeout_seconds"], 1800)
        self.assertEqual(payload["last_web_activity_ts"], 125.0)
        self.assertEqual(mgr._sessions["pi-live"].last_web_activity_ts, 125.0)

    def test_idle_auto_stop_sweep_deletes_expired_pi_rpc_session(self) -> None:
        mgr = _make_manager()
        mgr._sessions["pi-live"] = Session(
            session_id="pi-live",
            thread_id="thread-1",
            broker_pid=10,
            codex_pid=11,
            agent_backend="pi",
            backend="pi",
            owned=True,
            start_ts=1.0,
            cwd="/tmp",
            log_path=None,
            sock_path=Path("/tmp/pi-live.sock"),
            transport="pi-rpc",
            auto_stop_on_idle=True,
            idle_timeout_seconds=1800,
            last_web_activity_ts=100.0,
        )
        mgr._sessions["codex-live"] = Session(
            session_id="codex-live",
            thread_id="thread-2",
            broker_pid=12,
            codex_pid=13,
            agent_backend="codex",
            backend="codex",
            owned=True,
            start_ts=1.0,
            cwd="/tmp",
            log_path=None,
            sock_path=Path("/tmp/codex-live.sock"),
            auto_stop_on_idle=True,
            idle_timeout_seconds=1800,
            last_web_activity_ts=100.0,
        )

        deleted: list[str] = []
        mgr.delete_session = lambda session_id: deleted.append(session_id) or True  # type: ignore[method-assign]

        with patch("codoxear.server.time.time", return_value=2000.0):
            mgr._idle_auto_stop_sweep()

        self.assertEqual(deleted, ["pi-live"])
        self.assertEqual(mgr._sessions["pi-live"].idle_stop_reason, "web_idle_timeout")

    def test_heartbeat_route_returns_conflict_for_non_pi_rpc_session(self) -> None:
        handler = _HandlerHarness("/api/sessions/codex-live/heartbeat", body=b"{}")
        mgr = _make_manager()
        mgr._sessions["codex-live"] = Session(
            session_id="codex-live",
            thread_id="thread-2",
            broker_pid=12,
            codex_pid=13,
            agent_backend="codex",
            backend="codex",
            owned=True,
            start_ts=1.0,
            cwd="/tmp",
            log_path=None,
            sock_path=Path("/tmp/codex-live.sock"),
        )

        with (
            patch("codoxear.server.MANAGER", mgr),
            patch("codoxear.server._require_auth", return_value=True),
        ):
            Handler.do_POST(handler)  # type: ignore[arg-type]

        self.assertEqual(handler.status, 409)
        self.assertEqual(
            json.loads(handler.wfile.getvalue().decode("utf-8")),
            {
                "error": "idle auto-stop heartbeat is only supported for web-owned pi-rpc sessions"
            },
        )
