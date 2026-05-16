## Why

Phase 4 已经把 Rust broker 灰度路径落地，Rust backend 仍然把 voice scan、OpenAI TTS、HLS 输出和 WebPush/VAPID 投递留在 Python `codoxear/voice_push.py`。只要这些后台副作用仍由 Python 独占，Rust cutover 就无法进入“server + broker + worker”默认路径，也无法验证 `voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json` 的真实双向兼容。

本 change 规划 Phase 5：把 voice push / HLS / WebPush 能力迁移到 Rust，同时保留 Python fallback，确保关闭 Rust voice env flag 后可无迁移回滚到 Python worker。

## What Changes

- 新增 Rust voice worker 能力，port `codoxear/voice_push.py` 的后台 scan、消息分类、delivery ledger、announcement queue、OpenAI-compatible summarization / TTS、HLS live playlist/segment 输出、WebPush 投递与订阅清理。
- 复用并补强 Phase 2/3 已有 `backend-rs/src/voice_state.rs`、`voice_post.rs`、`handlers/voice.rs`；将 Phase 3 的 `501 feature disabled` debug endpoints 变为真实 Rust side-effect endpoints。
- Rust 必须读写实际文件名：`voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`，并继续使用 `webpush_vapid_private.pem` 或兼容迁移该 PEM。
- 新增 `CODOXEAR_ENABLE_VOICE_SCAN` / `CODOXEAR_ENABLE_VOICE_WORKER` 语义：Rust voice scan 与 Rust voice delivery/HLS/WebPush worker 独立 opt-in；Python `voice-push-scan` 已按 `CODOXEAR_ENABLE_VOICE_SCAN` 让出，但 Python voice fallback 不删除。
- 实现 HLS artifact contract：`audio/live.m3u8`、`audio/segments/<seq>-<prefix>.ts`、segment 命名、playlist headers、rolling cleanup、keepalive silence、segment path traversal 防护与 content type 保持兼容。
- 实现 WebPush/VAPID 等价验证：Rust crate 必须复用 Python 生成的 VAPID ECDSA P-256 PEM，公开 key base64url 与 Python `_ensure_vapid_keys()` 一致；真实或可控 mock endpoint 证明 payload、TTL、`sub` claim、410/404 stale subscription 处理与 `pywebpush` 语义一致。
- 不删除 Python backend，不移除 Python voice fallback，不改变 Phase 4 broker protocol，不改变前端 API shape。

## Capabilities

### New Capabilities

- `rust-voice-push`: Rust voice scan、voice delivery worker、OpenAI TTS/HLS/WebPush side effects、voice disk contracts、worker flags、rollback 与验证契约。

### Modified Capabilities

- `rust-backend-runtime`: Phase 5 将伞形 runtime 的 voice scan / voice delivery worker 从“保留给未来阶段”推进为 Rust 可拥有的 opt-in worker；HTTP route shape 不变。

## Impact

- **Affected code**：
  - 新增/修改 `backend-rs/src/voice_worker.rs` 或 `backend-rs/src/voice_worker/**`。
  - 修改 `backend-rs/src/main.rs` / `workers.rs` 接入 `CODOXEAR_ENABLE_VOICE_SCAN` 与 `CODOXEAR_ENABLE_VOICE_WORKER`。
  - 修改 `backend-rs/src/voice_state.rs`、`voice_post.rs`、`handlers/voice.rs`、`routes.rs`，把 debug endpoints 和 HLS GET endpoints 接到真实 Rust state。
  - 修改 `backend-rs/Cargo.toml` 增加 OpenAI HTTP、HLS/ffmpeg subprocess、VAPID/WebPush/ECDSA 所需 crate（具体选择需在 design 中验证）。
  - 可能小幅修改 `codoxear/server.py`：只允许补足 worker flag 日志/guard，不删除 Python fallback。
  - 新增/扩展 tests：`backend-rs/tests/voice_worker*.rs`、`backend-rs/tests/voice_hls*.rs`、`backend-rs/tests/voice_webpush*.rs`、`tests/contract/*voice*`，复用 `tests/test_voice_push.py` 作为 Python oracle。
- **APIs**：现有 `/api/settings/voice`、`/api/notifications/*`、`/api/audio/*` 路径、请求字段、响应字段、状态码与 auth 不变；Phase 5 只改变 Rust backend 是否真正执行 side effects。
- **Disk contracts**：必须保持 `voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`、`webpush_vapid_private.pem`、`audio/live.m3u8`、`audio/segments/*.ts` 与 Python 兼容。
- **Dependencies / runtime tools**：需要 `ffmpeg` / `ffprobe`，需要 OpenAI-compatible HTTP client，可能新增 WebPush/VAPID crate。依赖选择必须说明 Linux/macOS 行为。
- **Ops / rollback**：开启灰度：先确保 Python server 不作为 voice writer 运行，再设置 Rust `CODOXEAR_ENABLE_VOICE_SCAN=1` 和/或 `CODOXEAR_ENABLE_VOICE_WORKER=1`。回滚：unset Rust voice flags，停止 Rust worker，重启 Python `codoxear-server`；Python 从同一 ledger/subscription/HLS 目录恢复。