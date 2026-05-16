## Why

Codoxear 的语音播报、HLS 音频输出、WebPush / VAPID 推送目前集中在 Python `voice_push.py` 与 `server.py` 线程中。随着项目进入 Rust 后端 cutover，继续让这些状态ful、长生命周期、涉及磁盘契约和外部加密签名的能力留在 Python，会让后续切换出现双写/双 worker 语义不清、回滚边界不清、以及 VAPID 等价性难以验证的问题。

本 change 的目标是在不删除 Python 后端的前提下，把 voice worker、HLS 输出、WebPush/VAPID 相关能力迁移到 Rust 后端路径，并保持现有浏览器 API、磁盘文件名和推送语义兼容，使用户可以通过环境开关安全灰度与回滚。

## What Changes

- 新增 Rust 侧 voice push 服务，覆盖现有 `voice_push.py` 的核心能力：
  - 读取、清洗、保存 `voice_settings.json`。
  - 读取、清洗、保存 `push_subscriptions.json`。
  - 读取、更新、裁剪 `voice_delivery_ledger.json`。
  - 根据 assistant narration / final response 生成 summary、TTS 音频、通知文本和 delivery ledger 状态。
  - 维护 listener heartbeat、queue/prepared/playing 状态和 no-listener replacement 语义。
  - 输出兼容现有 `/api/audio/live.m3u8` 与 `/api/audio/segments/*.ts` 的 HLS playlist / segment artifact。
  - 使用 Rust WebPush/VAPID crate 发送与 Python `pywebpush` 等价的推送 payload。
- 在后端启动时通过 Rust voice env flag 选择 Rust voice worker；默认保留 Python worker 路径，不删除 `voice_push.py`。
- 保持现有 HTTP API 对外契约不变：
  - `GET/POST /api/settings/voice`
  - `GET/POST /api/notifications/subscription`
  - `POST /api/notifications/subscription/toggle`
  - `POST /api/notifications/test_push`
  - `GET /api/notifications/message`
  - `GET /api/notifications/feed`
  - `GET /api/audio/live.m3u8`
  - `GET /api/audio/segments/<name>`
  - `POST /api/audio/listener`
  - `POST /api/audio/test_announcement`
- 增加 Rust/Python contract tests，覆盖磁盘 JSON、ledger 状态转换、subscription 清理、VAPID public key 编码、HLS playlist/segment 输出和推送 payload。
- 增加明确回滚路径：关闭 Rust voice env flag 后，服务回到 Python worker，继续使用同一组磁盘文件。
- **BREAKING**：无；该 change 不删除 Python 后端，不改变现有 API route、JSON 字段或磁盘文件名。

## Capabilities

### New Capabilities
- `rust-voice-push`: Rust 后端提供与现有 Python voice push/HLS/WebPush 语义兼容的 voice worker 能力，并通过环境开关灰度启用。

### Modified Capabilities
<!-- None: 当前仓库没有已归档 openspec/specs；本 change 通过新 capability 固化现有 voice push 契约。 -->

## Impact

- 影响子包 / 模块：
  - Python 后端：`codoxear/server.py` 需要增加 Rust voice worker 适配层与 env flag 选择逻辑；`codoxear/voice_push.py` 保留为默认与回滚路径。
  - Rust 后端：新增或扩展 Rust crate/module，用于 voice settings、subscription、delivery ledger、HLS、WebPush/VAPID、TTS/OpenAI-compatible client。
  - 前端：原则上不改 UI 行为；仅在需要时补充类型/测试以证明现有 API response 兼容。
  - 测试：新增 Rust 单元/集成测试、Python contract tests、可能的 fixture golden files。
- 对外接口：不新增 openapi/proto/事件；现有 HTTP JSON API 和 HLS 文件路径保持兼容。
- Admin / 鉴权路径：触及已鉴权的 Codoxear HTTP API route，必须继续复用 `_require_auth` / cookie gate；不引入新的未鉴权入口。
- 数据 / 磁盘契约：必须继续使用实际文件名 `voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`，并保持 JSON 字段向后兼容。
- 依赖：Rust 侧需要选择并验证 WebPush/VAPID/ECDSA、HTTP client、JSON、HLS/ffmpeg 调用、临时文件原子替换相关 crate；Python `py-vapid` / `pywebpush` 在回滚路径仍保留。
- 运行环境：HLS segment 生成继续依赖 `ffmpeg`/`ffprobe`；Rust worker 必须与 Python worker 在缺失依赖时表现为可诊断错误而非破坏 playlist/ledger。
- 回滚：关闭 Rust voice env flag，重启服务后使用 Python worker 读取同一组文件继续工作。