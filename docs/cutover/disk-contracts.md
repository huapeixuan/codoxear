# Codoxear Rust Cutover Disk Contracts

Scope: every persistent file under `~/.local/share/codoxear/` that the Python backend reads or writes at the `rust-backend-cutover` Phase 0 baseline. Rust must parse Python-written files and write files that the Python fallback can read without repair.

Unless a row says otherwise, JSON is encoded as UTF-8. The server response JSON uses compact separators, but persisted JSON mostly uses pretty `indent=2` with a trailing newline.

## Runtime root

- Runtime directory: `~/.local/share/codoxear` from `codoxear.util.default_app_dir()`.
- Legacy `~/.local/share/codex-web` is only warned about and is no longer used.
- Important subdirectories:
  - `socks/`: broker Unix sockets and sidecar JSON.
  - `uploads/`: staged attachment uploads.
  - `audio/` or voice coordinator subdirectories: generated HLS/audio artifacts (owned by `voice_push.py`; file-level media contracts are outside JSON scope, but routes must preserve byte serving semantics).

## JSON / byte contract table

| File / pattern | Owner(s) that write | Readers | Top-level schema | Serialization and key ordering | Atomic-write strategy | Mode | Rust contract notes |
|---|---|---|---|---|---|---|---|
| `hmac_secret` | `codoxear/server.py:_load_or_create_hmac_secret` on first server start | `server.py` auth cookie signing/verification; future Rust auth | Raw random bytes, not JSON. Python accepts existing files with at least 32 bytes and uses first 64 bytes. New file currently writes 64 random bytes. | raw bytes | direct `write_bytes(secret)` then `chmod` (no temp/rename) | `0600` | Rust must preserve exact bytes and HMAC-SHA256 cookie scheme (`base64url(json_payload).base64url(sig)`, payload JSON compact `separators=(",", ":")`, `ensure_ascii=False`). |
| `socks/<id>.json` (Codex broker) | `codoxear/broker.py:Broker._write_meta`; `server.py:_patch_metadata_log_path`, `_patch_metadata_pi_binding` may update best-effort | `server.py:SessionManager._discover_existing`, `refresh_session_meta`; future Rust server | Object with keys: `session_id` string, `backend` string, `owner` string|null, `supports_web_control` bool, `broker_pid` int, `sessiond_pid` int, `codex_pid` int, `cwd` string, `start_ts` number, `log_path` string|null, `sock_path` string, `agent_backend` string, `resume_session_id` string|null, `model_provider` string|null, `preferred_auth_method` string|null, `model` string|null, `reasoning_effort` string|null, `service_tier` string|null, `transport` string|null, `tmux_session` string|null, `tmux_window` string|null, `spawn_nonce` string|null. | `json.dumps(meta)` compact default insertion order, no trailing newline | direct `Path.write_text(...)` then `chmod` (not atomic) | `0600` | Rust broker should improve via temp+rename only if Python reader remains compatible; preserve nullable keys because Python code expects missing vs null in some fields to be benign. |
| `socks/<id>.json` (Pi broker) | `codoxear/pi_broker.py:_write_meta`; `server.py:_patch_metadata_pi_binding` may update | `server.py` discovery; future Rust server | Initial broker write object keys: `session_id`, `backend="pi"`, `transport="pi-rpc"`, `tmux_session`, `tmux_window`, `owner`, `supports_web_control`, `supports_live_ui`, `ui_protocol_version`, `broker_pid`, `agent_pid`, `codex_pid`, `cwd`, `start_ts`, `log_path` (initially `null`), `sock_path`, `resume_session_id`, `spawn_nonce`; optional `session_path`. Server-side patch may add/overwrite `backend="pi"`, add `agent_backend="pi"`, and add/overwrite `session_path`. No persistent `thread_id` key is written by Python. | `json.dumps(meta)` compact default insertion order, no trailing newline | `tempfile.NamedTemporaryFile(..., dir=socks, delete=False)` + `os.replace`, then `chmod` | `0600` | `codex_pid` is still present for compatibility even when backend is Pi; Rust must keep it or Python discovery rejects the sidecar. Rust must parse Pi sidecars without `agent_backend` and infer/patch Pi from `backend="pi"` and/or `transport="pi-rpc"`. |
| `socks/<id>.json` (sessiond) | `codoxear/sessiond.py:_write_meta` | `server.py` discovery | Similar Codex sidecar: `session_id`, `owner`, `supports_web_control`, `broker_pid`, `sessiond_pid`, `codex_pid`, `cwd`, `start_ts`, `log_path`, `sock_path`, `agent_backend`, model/provider fields. | `json.dumps(meta)` compact default insertion order, no trailing newline | direct `Path.write_text(...)` then `chmod` | `0600` | Phase 4 must decide whether `sessiond.py` is retained; until then Rust parser must accept this shape. |
| `harness.json` | `server.py:SessionManager._save_harness`; harness POST and sweep mutate | `server.py:_load_harness`, harness GET/sweep | Object mapping session id string -> object `{enabled: bool, request: string, cooldown_minutes: int, remaining_injections: int}`. Legacy `text` key is invalid and rejected on load/write API. | `json.dumps(obj, ensure_ascii=False, sort_keys=True, indent=2) + "\n"` | write `harness.json.tmp` then `os.replace` | existing umask/default (normally `0644`) | Rust worker must write identical key names and should use advisory lock before sweeping to avoid double injection. |
| `session_aliases.json` | `server.py:_save_aliases` via rename/edit/delete cleanup | `server.py:_load_aliases`, session list/resume candidates | Object mapping session id string -> alias string. Empty/invalid aliases omitted. | pretty JSON, `sort_keys=True`, trailing newline | write `session_aliases.json.tmp` then `os.replace` | default (`0644`) | Must round-trip aliases for active and historical session ids. |
| `session_sidebar.json` | `server.py:_save_sidebar_meta` via edit/sidebar operations | `server.py:_load_sidebar_meta`, session list priority | Object mapping session id -> object with `priority_offset` number; optional `snooze_until` number|null; optional `dependency_session_id` string. Invalid fields are cleaned. | pretty JSON, `sort_keys=True`, trailing newline | write `session_sidebar.json.tmp` then `os.replace` | default (`0644`) | Huapeixuan-only priority/dependency contract; absent from ref in this exact form. |
| `hidden_sessions.json` | `server.py:_save_hidden_sessions` via delete/hide/reconcile | `server.py:_load_hidden_sessions`, session list filtering | Array of either string keys or objects `{id: string, cutoff_ts: number}`. Keys may be raw session id, `session:<id>`, `thread:<backend>:<thread_id>`, `resume:<backend>:<resume_id>`, historical ids. | `json.dumps(obj, ensure_ascii=False, indent=2) + "\n"`; no `sort_keys` for objects in array | write `hidden_sessions.json.tmp` then `os.replace` | default (`0644`) | Rust must preserve mixed string/object array support and cutoff semantics. |
| `session_files.json` | `server.py:_save_files` via file read/write/delete cleanup | `server.py:_load_files`, file history UI | Object mapping `sid:<session_id>` -> array of absolute path strings. Loader also accepts legacy unprefixed session ids and normalizes to `sid:`; `cwd:` keys are ignored. | pretty JSON, `sort_keys=True`, trailing newline | write `session_files.json.tmp` then `os.replace` | default (`0644`) | Keep max history trimming/dedup order compatible. |
| `session_queues.json` | `server.py:_save_queues` via enqueue, queue update/delete, sweep | `server.py:_load_queues`, queue GET/sweep | Object mapping session id -> array. Each item is either a prompt string or object `{text: string, images: [image-input objects]}` when Pi composer images are queued. Invalid items are dropped. | pretty JSON, `sort_keys=True`, trailing newline | write `session_queues.json.tmp` then `os.replace` | default (`0644`) | Byte-level important for Phase 3 worker handoff; Rust and Python must never write concurrently. |
| `recent_cwds.json` | `server.py:_save_recent_cwds` via session discovery/create/backfill/prune | `server.py:_load_recent_cwds`, bootstrap/session list | Object mapping absolute cwd string -> timestamp number. Loader keeps finite positive numbers, dedups normalized cwd, stores top `RECENT_CWD_MAX` sorted by descending ts then path. | pretty JSON, `sort_keys=True`, trailing newline; save order is already sorted before dumping | write `recent_cwds.json.tmp` then `os.replace` | default (`0644`) | Rust should preserve ordering because bootstrap response order is user-visible. |
| `cwd_groups.json` | `server.py:_save_cwd_groups` via `POST /api/cwd_groups/edit` and hidden reconciliation | `server.py:_load_cwd_groups`, bootstrap/session list | Object mapping normalized cwd -> object with `label` string (optional/empty), `collapsed` bool, `hidden` bool, optional `hidden_after_live_start_ts` number. Entries with no label/collapsed/hidden may be omitted. | pretty JSON, `sort_keys=True`, trailing newline | write `cwd_groups.json.tmp` then `os.replace` | default (`0644`) | Huapeixuan-only directory grouping; Phase 2/3 must port before Rust is default. |
| `voice_settings.json` | `voice_push.py:VoicePushCoordinator._save_settings` via `POST /api/settings/voice` | `voice_push.py:_load_settings`, settings GET, voice worker | Object cleaned by `_clean_voice_settings`: booleans such as `tts_enabled_for_narration`, provider/model/base-url/voice fields, and other TTS config accepted by current cleaner. | `json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n"` | `NamedTemporaryFile(dir=parent, prefix="voice_settings.json.", suffix=".tmp")` + `os.replace` | default (`0644`) | Use target file name `voice_settings.json`; do not rename during Rust port. |
| `push_subscriptions.json` | `voice_push.py:_save_subscriptions` via notification subscription APIs | `voice_push.py:_load_subscriptions`, notification GET/worker | Array of subscription records. Each record includes stable `id`, WebPush `endpoint`, subscription key material (`keys` with `p256dh`/`auth`), `user_agent`, `device_label`, `device_class`, `enabled`, and timestamp fields as cleaned by `_clean_subscription_record`. | pretty JSON, `sort_keys=True`, trailing newline | `NamedTemporaryFile(...)` + `os.replace` | default (`0644`) | Rust must use/accept this exact target file name. |
| `voice_delivery_ledger.json` | `voice_push.py:_save_delivery_ledger` via voice delivery | `voice_push.py:_load_delivery_ledger`, notification state/feed | Object mapping message id -> ledger record. Records include message/session identity, observed/delivery status, timestamps, device/subscription delivery data, and error fields as cleaned by `_clean_ledger`. | pretty JSON, `sort_keys=True`, trailing newline | `NamedTemporaryFile(...)` + `os.replace` | default (`0644`) | Rust must use/accept this exact target file name. |
| `webpush_vapid_private.pem` | `voice_push.py:_load_or_create_vapid_key` | `voice_push.py`, future Rust voice worker | PEM private key bytes, not JSON. | PEM bytes | direct `write_bytes(vapid.private_pem())` | current code does not chmod; default umask | Rust voice worker must either reuse this PEM or document a compatible migration. |
| `uploads/<session_id>/<filename>` | `server.py:_stage_uploaded_file` via attachment injection | broker injection routes and user-visible file paths | Raw uploaded bytes, not JSON. Filename is sanitized; max byte limit enforced. | raw bytes | direct `write_bytes(data)` | `0600` | Attachment injection response includes staged absolute `path`; Rust must preserve path safety and mode. |

## Known non-contract / external files read by backend

These live outside `~/.local/share/codoxear` but influence API responses and Rust parity:

- Codex rollout logs: `~/.codex/sessions/rollout-*.jsonl`.
- Pi session logs: `~/.pi/agent/sessions/**/*.jsonl` and target-specific Pi native log discovery.
- Codex/Pi settings and auth files such as `~/.pi/agent/settings.json`, `~/.pi/agent/models.json`, `~/.pi/agent/auth.json`, `~/.codex/models_cache.json`.
- Git working tree files for file viewer/editor and git diff endpoints.

## Byte-level compatibility rules for Rust

1. Parse all current Python shapes, including nullable sidecar fields and legacy `hidden_sessions.json` string entries.
2. Preserve target filenames exactly: `push_subscriptions.json` and `voice_delivery_ledger.json` are the live huapeixuan names.
3. Preserve file modes where Python explicitly sets them: `hmac_secret`, `socks/*.json`, uploaded files are `0600`; other JSON files inherit normal process umask (normally `0644`).
4. Prefer temp-file + `fsync` + POSIX rename in Rust for worker JSON, but do not change schema, key names, or path names.
5. During dual-run phases, a given state file may have only one writer. Feature-flag handoff must disable the corresponding Python sweep/voice writer before enabling the Rust writer.

## Phase 3 worker handoff and rollback

Rust background writers are opt-in during Phase 3 and Python remains the default owner when flags are unset.

- `CODOXEAR_ENABLE_QUEUE_SWEEP=1` makes Rust own queue draining and `session_queues.json`; Python `SessionManager` skips its queue sweep thread while this flag is truthy.
- `CODOXEAR_ENABLE_HARNESS_SWEEP=1` makes Rust own harness injection and `harness.json`; Python skips its harness sweep thread while this flag is truthy.
- `CODOXEAR_ENABLE_VOICE_SCAN=1` is reserved for Phase 5 voice ownership; Python skips its voice scan thread while truthy, but Phase 3 Rust must not send WebPush, synthesize TTS, or write voice delivery ledgers.

Rollback is to stop Rust, unset the corresponding `CODOXEAR_ENABLE_*` flag, and restart the Python `codoxear-server`. Do not leave both servers running with the same writer flag enabled; `session_queues.json`, `harness.json`, and voice state files are single-writer cutover files.

## Phase 3 Rust write strategy

`rust-backend-write-routes` writes Phase 3 JSON state through `backend-rs/src/state_files.rs` helpers: read/modify/write operations take a per-file lock, serialize pretty JSON with sorted keys and trailing newline where Python does, write a temp file in the same directory, `fsync` the temp file, rename it over the target, and best-effort `fsync` the parent directory. This intentionally hardens the atomicity relative to some Python direct-write paths while preserving filenames, schemas, key names, and normal JSON file modes.

## Phase 4 Rust broker rollout strategy

`rust-backend-broker` introduces `CODOXEAR_RUST_BROKER_BIN` as an opt-in broker executable selector for Python and Rust session-create paths. When unset or blank, both servers keep spawning the Python broker fallback. When set, new Codex/Pi web-owned sessions use the selected `codoxear-broker-rs` binary while preserving the same `socks/*.json` filenames, schema keys, nullable fields, socket paths, and `0600` sidecar/socket modes. Rust sidecar helpers use same-directory temp file + `fsync` + rename where they write metadata, which is compatible with the existing Python readers.

### Phase 4 Rust broker validation note

The Phase 4 Rust broker remains opt-in behind `CODOXEAR_RUST_BROKER_BIN`; unsetting the variable keeps Python broker fallback unchanged. Rust writes the same `socks/*.json` sidecar filenames and schema families as Python, with atomic same-directory temp-file replacement and final mode `0600`.

Validation coverage added for the cutover includes:

- Rust-written Codex/Pi sidecars are loaded by the Rust session loader with the same `agent_backend`, `transport`, `session_path`, `supports_live_ui`, and socket-state behavior expected from Python-authored sidecars.
- Python-style Pi sidecars remain accepted by the Rust session loader, preserving rollback/dual-server compatibility.
- macOS log discovery is implemented through injectable `pgrep -P`/`lsof -p ... -F n` command runners and covered by fixture-output tests on non-macOS CI; real macOS validation should run the broker subset on a macOS host or `macos-latest` runner.
- PTY resize/raw-mode behavior is isolated in `backend-rs/src/broker/pty.rs`; unit coverage validates the resize abstraction and invalid-fd failure behavior without requiring a real Codex binary.


### Phase 5 Rust voice ownership notes

- Rust Phase 5 writes only the established voice file names: `voice_settings.json`, `push_subscriptions.json`, `voice_delivery_ledger.json`, `webpush_vapid_private.pem`, and HLS artifacts under `audio/live.m3u8` plus `audio/segments/*.ts`.
- Rust voice workers are opt-in through `CODOXEAR_ENABLE_VOICE_SCAN` and `CODOXEAR_ENABLE_VOICE_WORKER`; Python fallback remains the default writer when those flags are unset.
- Rust creates `voice_worker.lock` in the app dir before owning scan/delivery outputs so a second Rust process fails fast instead of double-writing ledger/HLS/WebPush state.
- Current Phase 5 continuation implements a gated Rust scan loop that writes Python-compatible pending ledger rows from existing Codex/Pi log normalizers, preserves same-slot replacement (`last_error:"replaced by newer message"`), keeps narration disabled semantics, and exposes listener heartbeat state in `/api/settings/voice.audio.*`.
- `POST /api/notifications/test_push` is still no-side-effect while `CODOXEAR_ENABLE_VOICE_WORKER` is off; when the flag is on in this continuation it updates/drop-tests subscription records without contacting real push endpoints. Full encrypted WebPush delivery remains a remaining Phase 5 delivery task.
- `POST /api/audio/test_announcement` remains guarded: it validates worker flag, active listener, and `tts_api_key`, but returns an explicit partial-build error until OpenAI TTS + ffmpeg HLS append are completed.
- Rollback is to stop Rust, unset both voice flags, and restart Python. Do not delete the ledger, subscription file, settings file, or VAPID PEM during rollback.
