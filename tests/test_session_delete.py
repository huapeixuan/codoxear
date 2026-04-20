import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from codoxear.server import Session, SessionManager


class TestSessionDelete(unittest.TestCase):
    def _manager(self) -> SessionManager:
        manager = SessionManager.__new__(SessionManager)
        manager._lock = threading.Lock()
        manager._sessions = {}
        manager._hidden_sessions = set()
        manager._hidden_session_cutoffs = {}
        manager._aliases = {}
        manager._sidebar_meta = {}
        manager._harness = {}
        manager._files = {}
        manager._queues = {}
        manager._pi_commands_cache = {}
        return manager

    def test_delete_session_hides_replacement_broker_for_same_thread(self) -> None:
        manager = self._manager()
        manager._save_hidden_sessions = lambda: None  # type: ignore[method-assign]
        manager.files_clear = lambda session_id: None  # type: ignore[method-assign]
        manager._clear_deleted_session_state = lambda session_id: None  # type: ignore[method-assign]
        manager.kill_session = lambda session_id: True  # type: ignore[method-assign]

        manager._sessions["broker-a"] = Session(
            session_id="broker-a",
            thread_id="thread-1",
            broker_pid=11,
            codex_pid=12,
            agent_backend="pi",
            backend="pi",
            owned=True,
            start_ts=1.0,
            cwd="/tmp",
            log_path=None,
            sock_path=Path("/tmp/broker-a.sock"),
        )

        ok = manager.delete_session("broker-a")

        self.assertTrue(ok)
        self.assertTrue(manager._session_is_hidden("broker-b", "thread-1", None, "pi"))

    def test_delete_session_hides_resume_identity_for_historical_entry(self) -> None:
        manager = self._manager()
        manager._save_hidden_sessions = lambda: None  # type: ignore[method-assign]
        manager.files_clear = lambda session_id: None  # type: ignore[method-assign]
        manager._clear_deleted_session_state = lambda session_id: None  # type: ignore[method-assign]
        manager.kill_session = lambda session_id: True  # type: ignore[method-assign]

        manager._sessions["broker-a"] = Session(
            session_id="broker-a",
            thread_id="thread-1",
            broker_pid=11,
            codex_pid=12,
            agent_backend="pi",
            backend="pi",
            owned=True,
            start_ts=1.0,
            cwd="/tmp",
            log_path=None,
            sock_path=Path("/tmp/broker-a.sock"),
            resume_session_id="resume-1",
        )

        ok = manager.delete_session("broker-a")

        self.assertTrue(ok)
        self.assertTrue(
            manager._session_is_hidden(
                "history:pi:resume-1", "thread-1", "resume-1", "pi"
            )
        )
        self.assertEqual(manager._hidden_sessions, set())
        self.assertEqual(manager._hidden_session_cutoffs["thread:pi:thread-1"], 1.0)
        self.assertEqual(manager._hidden_session_cutoffs["resume:pi:resume-1"], 1.0)

    def test_list_sessions_unhides_cutoff_hidden_historical_entry_after_update(
        self,
    ) -> None:
        manager = self._manager()
        manager._include_historical_sessions = True
        manager._discover_existing_if_stale = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._prune_dead_sessions = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._update_meta_counters = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._maybe_drain_session_queue = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._save_hidden_sessions = lambda: None  # type: ignore[method-assign]
        manager._hidden_session_cutoffs = {"resume:pi:resume-1": 10.0}

        historical = {
            "session_id": "history:pi:resume-1",
            "thread_id": "resume-1",
            "agent_backend": "pi",
            "backend": "pi",
            "resume_session_id": "resume-1",
            "updated_ts": 20.0,
            "start_ts": 20.0,
            "queue_len": 0,
            "busy": False,
            "historical": True,
        }

        with patch(
            "codoxear.server._historical_sidebar_items", return_value=[historical]
        ):
            rows = manager.list_sessions()

        self.assertEqual([row["session_id"] for row in rows], ["history:pi:resume-1"])
        self.assertNotIn("resume:pi:resume-1", manager._hidden_session_cutoffs)

    def test_list_sessions_keeps_cutoff_hidden_historical_entry_without_update(
        self,
    ) -> None:
        manager = self._manager()
        manager._include_historical_sessions = True
        manager._discover_existing_if_stale = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._prune_dead_sessions = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._update_meta_counters = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._maybe_drain_session_queue = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._save_hidden_sessions = lambda: None  # type: ignore[method-assign]
        manager._hidden_session_cutoffs = {"resume:pi:resume-1": 10.0}

        historical = {
            "session_id": "history:pi:resume-1",
            "thread_id": "resume-1",
            "agent_backend": "pi",
            "backend": "pi",
            "resume_session_id": "resume-1",
            "updated_ts": 10.0,
            "start_ts": 10.0,
            "queue_len": 0,
            "busy": False,
            "historical": True,
        }

        with patch(
            "codoxear.server._historical_sidebar_items", return_value=[historical]
        ):
            rows = manager.list_sessions()

        self.assertEqual(rows, [])
        self.assertEqual(manager._hidden_session_cutoffs["resume:pi:resume-1"], 10.0)

    def test_list_sessions_omits_hidden_historical_entry(self) -> None:
        manager = self._manager()
        manager._include_historical_sessions = True
        manager._discover_existing_if_stale = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._prune_dead_sessions = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._update_meta_counters = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._maybe_drain_session_queue = lambda *args, **kwargs: None  # type: ignore[method-assign]
        manager._hidden_sessions = {"resume:pi:resume-1"}

        historical = {
            "session_id": "history:pi:resume-1",
            "thread_id": "resume-1",
            "agent_backend": "pi",
            "backend": "pi",
            "resume_session_id": "resume-1",
            "updated_ts": 10.0,
            "start_ts": 10.0,
            "queue_len": 0,
            "busy": False,
            "historical": True,
        }

        with patch(
            "codoxear.server._historical_sidebar_items", return_value=[historical]
        ):
            rows = manager.list_sessions()

        self.assertEqual(rows, [])

    def test_delete_historical_session_hides_only_history_entry(self) -> None:
        manager = self._manager()
        manager._save_hidden_sessions = lambda: None  # type: ignore[method-assign]

        kill_calls: list[str] = []

        def kill_session(session_id: str) -> bool:
            kill_calls.append(session_id)
            return True

        manager.kill_session = kill_session  # type: ignore[method-assign]

        with patch(
            "codoxear.server._historical_session_row",
            return_value={
                "session_id": "history:pi:resume-1",
                "resume_session_id": "resume-1",
                "thread_id": "resume-1",
                "agent_backend": "pi",
                "backend": "pi",
                "historical": True,
            },
        ):
            ok = manager.delete_session("history:pi:resume-1")

        self.assertTrue(ok)
        self.assertEqual(kill_calls, [])
        self.assertIn("history:pi:resume-1", manager._hidden_sessions)
        self.assertFalse(
            manager._session_is_hidden(
                "live-broker",
                "thread-1",
                "resume-1",
                "pi",
                include_historical_identity=False,
            )
        )


if __name__ == "__main__":
    unittest.main()
