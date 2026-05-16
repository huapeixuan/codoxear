## Context

现有 voice push 能力集中在 `codoxear/voice_push.py`：

- `VoicePushCoordinator` 由 `codoxear/server.py` 在 server manager 初始化时启动，维护 voice worker、keepalive worker、listener heartbeat、delivery ledger、subscription、VAPID key、HLS playlist/segment。
- 磁盘状态位于 Codoxear app dir，关键文件名已经被用户和测试依赖：`voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`，VAPID private key 当前为 `webpush_vapid_private.pem`。
- HTTP API route 由 `server.py` 提供，均在 `_require_auth` 之后访问 coordinator。
- HLS 输出为 `audio/live.m3u8` 与 `audio/segments/*.ts`，segment 生成依赖 `ffmpeg` / `ffprobe`。
- WebPush 由 Python `py_vapid` + `pywebpush` 提供；推送 payload 使用 `notification_text` 字段，final response 的 push 通知文本当前保持 `DEFAULT_PUSH_NOTIFICATION_TEXT`，实际摘要文本写入 ledger 和 feed。

本 change 不是“把 UI 改成 Rust”，也不是删除 Python server；它是在 Rust 后端 cutover 阶段把 voice worker 的状态机与外部副作用迁移到 Rust 路径，并通过 env flag 与 Python worker 并存。

## Goals / Non-Goals

**Goals:**

- Rust voice worker 在启用 flag 时接管现有 voice settings、push subscriptions、delivery ledger、HLS、WebPush/VAPID 能力。
- 保持现有 HTTP API、JSON 字段、磁盘文件名、HLS 路径和 service worker payload 兼容。
- 对 `pywebpush` → Rust WebPush/VAPID 的等价性做可验证设计：同一 VAPID 私钥导出相同 public key；同一 subscription/payload/subject 生成可被浏览器 push service 接受的请求。
- 保留 Python worker 作为默认和回滚路径；严禁两个 worker 同时处理同一 app dir 的 voice 状态。
- 为 coding-agent 提供可按模块拆分、每项 ≤2 小时的实现任务与 contract tests。

**Non-Goals:**

- 不删除 `codoxear/voice_push.py` 或 Python server。
- 不改变 Web UI 的用户交互、现有 route 命名、service worker 展示语义。
- 不新增 openapi/proto/事件系统。
- 不把 TTS/summarization 服务换成新的供应商；继续使用 OpenAI-compatible `/chat/completions` 与 `/audio/speech` 契约。
- 不实现无 `ffmpeg` 的纯 Rust 音频转封装；缺失 `ffmpeg` / `ffprobe` 仍作为可诊断错误处理。

## Decisions

### Decision 1: Rust worker behind a single env flag, default Python

- **What**：新增类似 `CODEX_WEB_RUST_VOICE_PUSH=1` 的开关。未开启时，`server.py` 继续构造 Python `VoicePushCoordinator`；开启时只构造 Rust voice bridge/coordinator。初始化必须保证同一进程只启动一个 voice worker。
- **Why**：本 change 涉及推送、HLS、磁盘状态和外部 API，直接切换风险高。默认 Python 能保持当前稳定行为；env flag 能让开发/灰度/回滚明确。
- **Alternatives considered**：
  - 直接替换 Python worker：回滚困难，且 VAPID/HLS 差异一旦出现会影响所有用户。
  - Python 与 Rust 双写 ledger：会造成队列、listener、push delivery 重复发送，违反推送语义。

### Decision 2: Rust coordinator 实现同一抽象接口，server route 不分叉

- **What**：抽象出 Python/Rust 两个 coordinator 都要实现的最小方法集合：`settings_snapshot`、`set_settings`、`subscriptions_snapshot`、`upsert_subscription`、`toggle_subscription`、`send_test_push_notification`、`listener_heartbeat`、`enqueue_test_announcement`、`observe_messages`、`playlist_bytes`、`segment_path`、`notification_text_for_message`、`notification_state_for_message`、`notification_feed_since`。`server.py` route 继续调用 `MANAGER._voice_push.<method>`。
- **Why**：HTTP API 是稳定边界；把分支限制在构造阶段，能减少鉴权路径和 route 行为分叉。
- **Alternatives considered**：新增一套 `/api/rust/...` route；会让前端和 service worker 同时承担迁移复杂度，不利于兼容。

### Decision 3: 磁盘 JSON schema 以 Python cleaner 为 canonical contract

- **What**：Rust 侧按现有 `_clean_voice_settings`、`_clean_subscription_record`、`_clean_ledger` 的输入容错和输出字段实现 serde 模型；读入未知/坏数据时采用 Python 当前清洗语义，写出时保持 pretty JSON、稳定字段名、原子替换。
- **Why**：回滚要求 Rust 写出的文件能被 Python worker 继续读取；Python 写出的历史文件也必须被 Rust 接管。
- **Alternatives considered**：Rust 引入新 schema 版本字段或新文件名；会破坏“关闭 flag 回到 Python worker”的简单回滚。

### Decision 4: HLS 仍通过 ffmpeg/ffprobe 生成 `.ts` segment，Rust 只负责编排与 playlist

- **What**：Rust worker 调用 `ffmpeg` 将 AAC/TTS bytes 转成 `.ts` segment，调用 `ffprobe` 读取 duration，维护 `#EXTM3U`、`#EXT-X-VERSION:3`、`#EXT-X-TARGETDURATION`、`#EXT-X-MEDIA-SEQUENCE`、`#EXTINF` 与 `segments/<name>` 条目；保留 max segments、keepalive silence、reset 行为。
- **Why**：当前浏览器/HLS 契约依赖 artifact，而不是内部实现。使用同一外部工具链最容易保持播放兼容。
- **Alternatives considered**：使用 Rust muxer crate 生成 TS；需要额外验证 iOS/Safari HLS 兼容，超出本阶段目标。

### Decision 5: WebPush/VAPID 使用同一 PEM 私钥与 P-256 public key URL-safe base64

- **What**：Rust 读取/生成 `webpush_vapid_private.pem`，使用 P-256 ECDSA VAPID；`vapid_public_key` 必须等于未压缩 X9.62 public point 的 URL-safe base64 no padding，与 Python `_b64u(public_bytes)` 一致。推送 payload JSON 字段保持 `session_id`、`session_display_name`、`message_id`、`notification_text`、`timestamp`。
- **Why**：浏览器 subscription 绑定的是 VAPID public key；public key 编码不一致会导致现有订阅失效。
- **Alternatives considered**：Rust 使用独立 VAPID key 文件；会导致所有客户端重新订阅，且与回滚不兼容。

### Decision 6: TTS/summarization client 与 worker 状态机分层

- **What**：Rust 模块拆为：settings store、subscription store、ledger store、VAPID/webpush client、HLS stream、OpenAI-compatible client、voice coordinator/state machine。状态机负责 queue/prepared/playing/listener epoch；外部 client 通过 trait/mock 注入。
- **Why**：voice 行为需要大量 contract tests。trait 注入让 tests 不依赖真实 OpenAI、push service 或 ffmpeg。
- **Alternatives considered**：把 HTTP/TTS/WebPush/HLS 都写在一个 coordinator 文件中；初期快，但难以验证和 review。

### Decision 7: 最小跨语言边界优先本地进程内调用/子进程，而不是网络服务

- **What**：若 Rust 后端已经以同进程模块/二进制方式存在，则优先提供本地 bridge；若当前 repo 尚未有 Rust crate，则先建立 `rust-backend` crate，并让 Python server 在 flag 启用时启动/调用 Rust voice sidecar，sidecar 通过 stdio/Unix socket 暴露上述 coordinator 方法。具体采用现有 Phase 4 Rust 后端结构；不得另起不受控常驻服务。
- **Why**：本仓库当前 checkout 中尚未看到 Rust 文件，但 issue 前置条件说明 Phase 4 已存在。coding-agent 需要先对齐实际 Phase 4 结构，再把 voice worker 放入现有 Rust 后端，而不是凭空创造第二套 runtime。
- **Alternatives considered**：直接重写整个 `server.py` 为 Rust HTTP server；范围过大，也违背“不删除 Python 后端”。

## Risks / Trade-offs

- **Risk: Rust/Python ledger 清洗差异导致回滚后状态丢失** → Mitigation：建立 JSON golden fixtures，Python 写/Rust 读/Rust 写/Python 读 round-trip contract tests；测试实际文件名。
- **Risk: VAPID public key 编码差异导致已订阅设备失效** → Mitigation：用同一 PEM fixture 分别跑 Python 与 Rust，断言 public key 完全一致；对 subscription endpoint、payload、subject 做 mock push request 断言。
- **Risk: Rust worker 和 Python worker 同时运行导致重复推送/重复 HLS segment** → Mitigation：启动期互斥，只根据 env flag 构造一个 coordinator；日志输出当前 active voice backend；测试 server init 只启动一个 worker。
- **Risk: HLS playlist 细节差异导致 iOS/Safari 播放中断** → Mitigation：playlist golden tests 覆盖 media sequence、target duration、segment retention、reset、keepalive silence；必要时保留 ffmpeg 参数完全等价。
- **Risk: 外部 crate 的 WebPush 行为与 `pywebpush` 不完全一致** → Mitigation：先写 VAPID/public key 和 payload tests，再接入真实 crate；若 crate 不满足，则封装底层请求签名逻辑或记录阻塞，不静默上线。
- **Risk: Rust sidecar 通讯失败影响现有 API** → Mitigation：flag 默认关闭；开启 flag 后 bridge 失败要 fail closed 并在日志中说明；关闭 flag 即回 Python。
- **Risk: 实现范围过大** → Mitigation：按 store/HLS/VAPID/coordinator/API bridge 分阶段，每个阶段都有 contract tests；不做 UI 改造。

## Migration Plan

1. **准备**：确认 Phase 4 Rust 后端实际 crate/sidecar 结构；新增 `CODEX_WEB_RUST_VOICE_PUSH`（具体命名由实现保持文档一致）。
2. **Contract tests 先行**：为 settings/subscriptions/ledger/HLS/VAPID 建立 Python 当前行为 fixtures 与断言。
3. **Rust stores 与 models**：实现 JSON clean/load/save/atomic replace，先通过 round-trip tests。
4. **Rust HLS stream**：实现 playlist/segment/silence/reset，先通过 fake ffmpeg/ffprobe 与 artifact tests。
5. **Rust WebPush/VAPID**：实现 PEM 读取/生成、public key 导出、mock push send、stale subscription drop 语义。
6. **Rust coordinator 状态机**：迁移 queue/listener/final response/narration/ledger 状态转换，用 fake client/fake HLS/fake push 测试。
7. **Server bridge**：在 `server.py` 初始化阶段按 flag 选择 Python 或 Rust coordinator；保持 route 调用不变。
8. **灰度**：本地开启 flag 跑 Rust build/test 与 API smoke；确认磁盘文件能被 Python 回读。
9. **回滚**：关闭 flag 并重启服务；Python worker 使用相同 `voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json` 继续工作。

## Open Questions

- Phase 4 Rust 后端在目标分支中的实际目录/进程模型是什么？coding-agent 应先读取当前分支的 Rust crate，再决定 bridge 细节。
- Rust WebPush crate 选型需要以实际可用性验证为准；候选 crate 必须支持 VAPID P-256 PEM、TTL、payload encryption、subscription keys.p256dh/auth。
- 是否需要对 Rust voice worker 增加结构化日志/metrics？本 change 至少要求错误写入 ledger/HLS last_error；更细 metrics 可作为后续增强。