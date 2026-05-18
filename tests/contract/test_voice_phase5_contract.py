import json
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
