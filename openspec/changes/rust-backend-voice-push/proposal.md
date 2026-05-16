## Why

Phase 4 `rust-backend-broker` 已经通过 reviewer gate 并把 session create / broker 路径推进到 Rust 可灰度状态；Phase 5 需要把最后一组仍由 Python `voice_push.py` 拥有的实时副作用迁到 Rust：voice log scan、HLS audio 输出、WebPush/VAPID 投递、OpenAI-compatible summary/TTS，以及 `voice_settings.json` / `push_subscriptions.json` / `voice_delivery_ledger.json` 的磁盘契约。若不先把这些契约固化，Rust worker 与 Python worker 双写同一 ledger/HLS 目录会导致重复推送、播放列表损坏和不可回滚状态。

## What Changes

- 新增 Rust voice worker（建议 `backend-rs/src/voice_worker/**` 或等价拆分），移植 `codoxear/voice_push.py` 的 message observation、delivery ledger、announcement queue、listener keepalive、HLS playlist/segment 写入、OpenAI-compatible summarization/TTS、WebPush/VAPID 投递。
- 修改 Rust server worker 启动：`CODOXEAR_ENABLE_VOICE_SCAN=1` 启用 Rust log scan；`CODOXEAR_ENABLE_VOICE_WORKER=1` 启用 Rust HLS/TTS/WebPush delivery worker。两个 flag 默认 off；rollback 只需 unset 并重启 Rust server / Python server。
- 保持 Python fallback：`codoxear/voice_push.py`、Python server voice endpoints 和 Python scan thread 不删除；当 Rust voice flag 关闭时仍可接管。
- 补齐 Phase 3 暂留的 side-effect endpoints：Rust `POST /api/notifications/test_push` 和 `POST /api/audio/test_announcement` 从 501 feature-disabled 改为与 Python 等价的真实行为。
- 补齐 Rust HLS byte serving：`GET /api/audio/live.m3u8`、`GET /api/audio/segments/{segment}` 返回与 Python 相同的 content type、no-store headers、404/path traversal 行为。
- 严格保持磁盘文件名与 JSON schema：`voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`；不得迁移为其他草稿名。Rust 读写必须与 Python `json.dumps(..., ensure_ascii=False, indent=2, sort_keys=True) + "\n"` 的语义兼容，并保留 `webpush_vapid_private.pem` 的 PEM key 复用。
- 增加 single-writer guard：Python scan/delivery 与 Rust scan/delivery 不能同时写同一 `voice_delivery_ledger.json` 或 `audio/` HLS 输出；Rust worker 启动前必须持有 voice-owned advisory lock，冲突时 fail closed。
- 新增 Rust dependencies（待实现 spike 决定）：WebPush/VAPID/ECDSA、HTTP client、PEM/P-256 key parsing/signing、可能的 ffmpeg/ffprobe subprocess wrapper；所有依赖必须支持 Linux/macOS。
- 不改变 Phase 4 broker socket protocol；仅说明 Rust voice scan 读取 Rust/Python broker 产出的 `socks/*.json`、session log 和 `delivery_log_off` 等必要集成点。

## Capabilities

### New Capabilities

- `rust-backend-voice-push`: Rust voice push runtime，覆盖 voice settings/subscription/ledger disk contracts、voice log scan、announcement classification/queueing、OpenAI-compatible summary/TTS、HLS playlist/segment artifacts、WebPush/VAPID delivery、worker feature flags、single-writer rollback 行为。

### Modified Capabilities

- `rust-backend-runtime`: Rust server 原有 voice/settings/notification/audio HTTP routes 在 Phase 5 中从 read/write state-file parity 扩展为拥有真实 voice side effects 与 HLS byte serving；API path 与请求/响应字段保持兼容。

## Impact

- **Affected code**：
  - `backend-rs/src/voice_state.rs`、`backend-rs/src/voice_post.rs`、`backend-rs/src/handlers/voice.rs`、`backend-rs/src/routes.rs`、`backend-rs/src/workers.rs`、`backend-rs/src/state_files.rs`
  - 新增 `backend-rs/src/voice_worker/**`（或 `voice_worker.rs` + 子模块）
  - `backend-rs/Cargo.toml` / `Cargo.lock`
  - `codoxear/server.py`（仅限 Python worker handoff / rollback guard 必要调整，不删除 fallback）
  - tests：`backend-rs/tests/voice_*.rs`、`tests/test_voice_push.py`、`tests/test_voice_push_source.py`、`tests/contract/*voice*`
  - docs：`README.md`、`docs/cutover/disk-contracts.md`、`docs/cutover/endpoint-inventory.md`、`docs/cutover/cutover-log.md`（如存在）
- **APIs**：现有 `/api/settings/voice`、`/api/notifications/*`、`/api/audio/*` 路径保持不变，并继续同时支持 `/api/v1/...` 与 legacy `/api/...`；Phase 5 只补齐 side effects 和 HLS bytes。
- **Disk/media contracts**：`voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`、`webpush_vapid_private.pem`、`audio/live.m3u8`、`audio/segments/*.ts` 必须可由 Python/Rust 双向读取；rollback 不需要手工迁移。
- **Ops**：灰度打开 `CODOXEAR_ENABLE_VOICE_SCAN=1` / `CODOXEAR_ENABLE_VOICE_WORKER=1`；回滚为关闭这两个 flag，并确保 Python server 是唯一 voice writer。真实设备验证需要 Tailscale HTTPS + mobile WebPush subscription + OpenAI-compatible TTS key + ffmpeg/ffprobe。
- **Non-breaking**：不删除 Python backend，不移除 Python voice fallback，不规划 Phase 6 Python 删除，不修改 Phase 4 broker 协议。
