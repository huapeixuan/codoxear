import os
import threading
from unittest import mock


def test_env_flag_truthy_matches_rust_worker_handoff_values():
    from codoxear.server import _env_flag_truthy

    with mock.patch.dict(os.environ, {}, clear=False):
        os.environ.pop("CODOXEAR_ENABLE_QUEUE_SWEEP", None)
        assert not _env_flag_truthy("CODOXEAR_ENABLE_QUEUE_SWEEP")
    for value in ("", "0", "false", "False", " FALSE "):
        with mock.patch.dict(
            os.environ, {"CODOXEAR_ENABLE_QUEUE_SWEEP": value}, clear=False
        ):
            assert not _env_flag_truthy("CODOXEAR_ENABLE_QUEUE_SWEEP")
    for value in ("1", "true", "yes", "anything"):
        with mock.patch.dict(
            os.environ, {"CODOXEAR_ENABLE_QUEUE_SWEEP": value}, clear=False
        ):
            assert _env_flag_truthy("CODOXEAR_ENABLE_QUEUE_SWEEP")


def test_truthy_worker_flags_prevent_python_worker_threads(monkeypatch, tmp_path):
    import codoxear.server as server

    started = []

    class FakeThread:
        def __init__(self, *, target, name, daemon):
            self.name = name

        def start(self):
            started.append(self.name)

    monkeypatch.setattr(server, "APP_DIR", tmp_path)
    monkeypatch.setattr(server, "HARNESS_PATH", tmp_path / "harness.json")
    monkeypatch.setattr(server, "ALIAS_PATH", tmp_path / "session_aliases.json")
    monkeypatch.setattr(server, "SIDEBAR_META_PATH", tmp_path / "session_sidebar.json")
    monkeypatch.setattr(
        server, "HIDDEN_SESSIONS_PATH", tmp_path / "hidden_sessions.json"
    )
    monkeypatch.setattr(server, "FILE_HISTORY_PATH", tmp_path / "session_files.json")
    monkeypatch.setattr(server, "QUEUE_PATH", tmp_path / "session_queues.json")
    monkeypatch.setattr(server, "RECENT_CWD_PATH", tmp_path / "recent_cwds.json")
    monkeypatch.setattr(server, "CWD_GROUPS_PATH", tmp_path / "cwd_groups.json")
    monkeypatch.setattr(server, "VOICE_SETTINGS_PATH", tmp_path / "voice_settings.json")
    monkeypatch.setattr(
        server, "PUSH_SUBSCRIPTIONS_PATH", tmp_path / "push_subscriptions.json"
    )
    monkeypatch.setattr(
        server, "DELIVERY_LEDGER_PATH", tmp_path / "voice_delivery_ledger.json"
    )
    monkeypatch.setattr(
        server, "VoicePushCoordinator", mock.Mock(return_value=mock.Mock())
    )
    monkeypatch.setattr(
        server.SessionManager, "_backfill_recent_cwds_from_logs", lambda self: None
    )
    monkeypatch.setattr(
        server.SessionManager, "_discover_existing", lambda self, **kwargs: None
    )
    monkeypatch.setattr(threading, "Thread", FakeThread)
    monkeypatch.setenv("CODOXEAR_ENABLE_HARNESS_SWEEP", "1")
    monkeypatch.setenv("CODOXEAR_ENABLE_QUEUE_SWEEP", "true")
    monkeypatch.setenv("CODOXEAR_ENABLE_VOICE_SCAN", "yes")

    mgr = server.SessionManager()

    assert started == []
    assert mgr._harness_thr is None
    assert mgr._queue_thr is None
    assert mgr._voice_push_scan_thr is None


def test_falsy_worker_flags_preserve_python_worker_threads(monkeypatch, tmp_path):
    import codoxear.server as server

    started = []

    class FakeThread:
        def __init__(self, *, target, name, daemon):
            self.name = name

        def start(self):
            started.append(self.name)

    monkeypatch.setattr(server, "APP_DIR", tmp_path)
    monkeypatch.setattr(server, "HARNESS_PATH", tmp_path / "harness.json")
    monkeypatch.setattr(server, "ALIAS_PATH", tmp_path / "session_aliases.json")
    monkeypatch.setattr(server, "SIDEBAR_META_PATH", tmp_path / "session_sidebar.json")
    monkeypatch.setattr(
        server, "HIDDEN_SESSIONS_PATH", tmp_path / "hidden_sessions.json"
    )
    monkeypatch.setattr(server, "FILE_HISTORY_PATH", tmp_path / "session_files.json")
    monkeypatch.setattr(server, "QUEUE_PATH", tmp_path / "session_queues.json")
    monkeypatch.setattr(server, "RECENT_CWD_PATH", tmp_path / "recent_cwds.json")
    monkeypatch.setattr(server, "CWD_GROUPS_PATH", tmp_path / "cwd_groups.json")
    monkeypatch.setattr(server, "VOICE_SETTINGS_PATH", tmp_path / "voice_settings.json")
    monkeypatch.setattr(
        server, "PUSH_SUBSCRIPTIONS_PATH", tmp_path / "push_subscriptions.json"
    )
    monkeypatch.setattr(
        server, "DELIVERY_LEDGER_PATH", tmp_path / "voice_delivery_ledger.json"
    )
    monkeypatch.setattr(
        server, "VoicePushCoordinator", mock.Mock(return_value=mock.Mock())
    )
    monkeypatch.setattr(
        server.SessionManager, "_backfill_recent_cwds_from_logs", lambda self: None
    )
    monkeypatch.setattr(
        server.SessionManager, "_discover_existing", lambda self, **kwargs: None
    )
    monkeypatch.setattr(threading, "Thread", FakeThread)
    monkeypatch.setenv("CODOXEAR_ENABLE_HARNESS_SWEEP", "0")
    monkeypatch.setenv("CODOXEAR_ENABLE_QUEUE_SWEEP", "false")
    monkeypatch.setenv("CODOXEAR_ENABLE_VOICE_SCAN", "")

    server.SessionManager()

    assert started == ["harness", "queue", "voice-push-scan"]
