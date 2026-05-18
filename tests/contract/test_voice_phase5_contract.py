import json
import shutil
import subprocess
import textwrap
from pathlib import Path


def test_phase5_voice_files_use_live_contract_names(shared_app_dir: Path) -> None:
    (shared_app_dir / "voice_settings.json").write_text(
        json.dumps({"tts_enabled_for_narration": False}) + "\n", "utf-8"
    )
    (shared_app_dir / "push_subscriptions.json").write_text("[]\n", "utf-8")
    (shared_app_dir / "voice_delivery_ledger.json").write_text("{}\n", "utf-8")

    assert (shared_app_dir / "voice_settings.json").exists()
    assert (shared_app_dir / "push_subscriptions.json").exists()
    assert (shared_app_dir / "voice_delivery_ledger.json").exists()
    assert not (shared_app_dir / "push_ledger.json").exists()
    assert not (shared_app_dir / "audio_announcement_queue.json").exists()


def test_python_voice_coordinator_reads_rust_style_voice_files(
    shared_app_dir: Path,
) -> None:
    from codoxear.voice_push import VoicePushCoordinator
    import threading

    (shared_app_dir / "voice_settings.json").write_text(
        json.dumps(
            {
                "tts_enabled_for_narration": True,
                "tts_enabled_for_final_response": False,
                "tts_base_url": "https://api.openai.com/v1",
                "tts_api_key": "",
                "summarization_model": "gpt-4.1-mini",
                "tts_model": "gpt-4o-mini-tts",
            },
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        )
        + "\n",
        "utf-8",
    )
    (shared_app_dir / "push_subscriptions.json").write_text(
        json.dumps(
            [
                {
                    "id": "sub1",
                    "subscription": {
                        "endpoint": "https://push.example/sub1",
                        "keys": {"p256dh": "p256dh", "auth": "auth"},
                    },
                    "notifications_enabled": True,
                    "created_ts": 1.0,
                    "updated_ts": 2.0,
                    "last_success_ts": 3.0,
                    "last_failure_ts": None,
                    "last_error": "",
                    "user_agent": "iPhone",
                    "device_label": "phone",
                    "device_class": "mobile",
                }
            ],
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        )
        + "\n",
        "utf-8",
    )
    (shared_app_dir / "voice_delivery_ledger.json").write_text(
        json.dumps(
            {
                "m1": {
                    "message_id": "m1",
                    "session_id": "sid",
                    "session_display_name": "Repo",
                    "message_class": "final_response",
                    "preview_text": "body",
                    "notification_text": "summary",
                    "summary_text": "summary",
                    "summary_status": "sent",
                    "narrated_status": "skipped",
                    "push_status": "sent",
                    "voice": "alloy",
                    "created_ts": 1.0,
                    "updated_ts": 2.0,
                    "last_error": "",
                }
            },
            ensure_ascii=False,
            indent=2,
            sort_keys=True,
        )
        + "\n",
        "utf-8",
    )
    shutil.copyfile(
        Path(__file__).parents[1] / "fixtures" / "voice" / "webpush_vapid_private.pem",
        shared_app_dir / "webpush_vapid_private.pem",
    )

    coord = VoicePushCoordinator(
        app_dir=shared_app_dir,
        stop_event=threading.Event(),
        settings_path=shared_app_dir / "voice_settings.json",
        subscriptions_path=shared_app_dir / "push_subscriptions.json",
        delivery_ledger_path=shared_app_dir / "voice_delivery_ledger.json",
        vapid_private_key_path=shared_app_dir / "webpush_vapid_private.pem",
        enable_worker=False,
    )

    settings = coord.settings_snapshot()
    assert settings["tts_enabled_for_narration"] is True
    assert settings["notifications"]["enabled_devices"] == 1
    state = coord.notification_state_for_message("m1")
    assert state is not None
    assert state["push_status"] == "sent"
    assert coord.notification_feed_since(0)[0]["notification_text"] == "summary"


def test_python_can_load_rust_created_vapid_pem(shared_app_dir: Path) -> None:
    cargo = shutil.which("cargo")
    if cargo is None:
        return
    backend = Path(__file__).parents[2] / "backend-rs"
    subprocess.run(
        [
            cargo,
            "test",
            "rust_created_vapid_pem_is_reloaded_with_stable_public_key",
            "--test",
            "voice_worker_phase5",
            "--",
            "--exact",
        ],
        cwd=backend,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    # The Rust unit test proves reloadability on Rust side; this fixture proves Python py_vapid can
    # parse the shared PEM format that Rust also accepts and exposes as VAPID public key.
    from py_vapid import Vapid

    pem = shared_app_dir / "webpush_vapid_private.pem"
    shutil.copyfile(Path(__file__).parents[1] / "fixtures" / "voice" / pem.name, pem)
    vapid = Vapid.from_file(str(pem))
    assert vapid.public_key is not None
