## Context

Phase 5 的入口事实：

- Phase 4 `rust-backend-broker` 已在 commit `92d2a0f` 后 reviewer PASS，Rust server / Python server 都能通过 `CODOXEAR_RUST_BROKER_BIN` 灰度选择 broker。Phase 5 不需要、也不应该改 broker socket protocol。
- Python voice 能力集中在 `codoxear/voice_push.py`（~1.8k 行）和 `codoxear/server.py` 的 voice routes / `_voice_push_scan_loop`：
  - `VoicePushCoordinator` 拥有 settings/subscription/ledger、listener heartbeat、message observation、queue/prepared/playing state、OpenAI-compatible summary/TTS、WebPush、HLS keepalive。
  - `MergedHLSStream` 在 `~/.local/share/codoxear/audio/` 下写 `live.m3u8` 和 `segments/*.ts`，segment 名形如 `000001-<prefix>.ts`。
  - Python scan thread 每 `CODEX_WEB_VOICE_PUSH_SWEEP_SECONDS`（默认 1.0s）扫描 session logs 的增量，并调用 `_rollout_log._extract_delivery_messages` 生成 `ClassifiedAssistantMessage`。
- Phase 2/3 已经在 Rust 中落地了部分 voice HTTP parity：`voice_state.rs` 读 `voice_settings.json` / `push_subscriptions.json` / `voice_delivery_ledger.json`，`voice_post.rs` 写 settings/subscription/listener；但两个 debug side-effect routes 仍是 501：`POST /api/notifications/test_push`、`POST /api/audio/test_announcement`。Rust 目前还没有 `voice_worker`。
- Phase 0 `docs/cutover/disk-contracts.md` 明确活跃文件名是：`voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`；`webpush_vapid_private.pem` 是 VAPID PEM；`audio/` 是 HLS/audio artifact 目录。触发评论特别要求不要误用其他草稿文件名。
- Python server 已有 `if not _env_flag_truthy("CODOXEAR_ENABLE_VOICE_SCAN")` 才启动 Python scan thread 的 handoff；但 Python `VoicePushCoordinator` 内部 worker / keepalive 线程在初始化时总会启动，因此 Phase 5 必须进一步避免 Python worker 与 Rust worker 同时写 HLS/ledger。

Stakeholders：codoxear maintainer、真实手机端 WebPush/HLS 用户、自托管 ops。主要验证环境：Linux/macOS Rust tests、本地或 Tailscale HTTPS 的 iOS/移动浏览器实际订阅、OpenAI-compatible TTS endpoint、ffmpeg/ffprobe。

## Goals / Non-Goals

**Goals:**

- 把 `voice_push.py` 的 runtime side effects 移植到 Rust：log scan、message classification/dedupe、ledger 更新、announcement queue、OpenAI-compatible summary/TTS、HLS append/keepalive、WebPush/VAPID 投递。
- 保持所有用户面 HTTP API 和 response shape 与 Python 兼容，尤其补齐 `POST /api/notifications/test_push` 与 `POST /api/audio/test_announcement`。
- 保持磁盘 byte-level 契约：文件名、JSON key、status string、timestamp 字段、pretty JSON + sort keys + trailing newline、HLS playlist/segment 命名与 cleanup 语义。
- 确保同一时间只有一个 voice writer（Python 或 Rust）写 `voice_delivery_ledger.json` 与 `audio/`，worker 冲突 fail closed。
- 验证 `pywebpush` → Rust WebPush/VAPID/ECDSA 的等价性：同一 `webpush_vapid_private.pem` 能生成同一 public key bytes（base64url uncompressed P-256 point），并能对真实 browser push service 成功投递。
- 提供一秒回滚路径：unset `CODOXEAR_ENABLE_VOICE_SCAN` / `CODOXEAR_ENABLE_VOICE_WORKER` 后 Python fallback 能从现有 ledger/HLS/subscription 状态继续工作。

**Non-Goals:**

- 不删除 `codoxear/voice_push.py`、Python routes 或 Python fallback；Phase 6 才处理 Python backend 删除。
- 不改变 Phase 4 broker 协议、sidecar schema 或 socket command set；仅复用其 session discovery/log path。
- 不重构 frontend，不新增用户可见 voice 功能，只做 Rust parity。
- 不引入新的 notification payload schema；service worker / mobile client 继续消费现有 payload 字段。
- 不把 Rust voice 默认启用；默认仍 off，必须由 env flag 显式打开。

## Decisions

### D1：把 Rust voice 拆成小模块，而不是单个 1.8k 行文件

**选择**：新增 `backend-rs/src/voice_worker/`，建议模块：`mod.rs`（coordinator）、`settings.rs`、`subscriptions.rs`、`ledger.rs`、`messages.rs`、`hls.rs`、`openai.rs`、`webpush.rs`、`locks.rs`。`voice_state.rs` / `voice_post.rs` 中已存在的 cleaner 可迁移或复用，但所有 `backend-rs/src/**/*.rs` 继续保持 ≤800 行。

**原因**：Phase 4 已建立 line-count gate；voice 同时涉及 crypto、HTTP、ffmpeg、HLS、ledger 和 worker loop，单文件会不可 review。

**替代方案**：按 Python `voice_push.py` 1:1 放进 `voice_worker.rs`。否决：违反仓库 line-count 约束，也不利于单元测试隔离。

### D2：两个 env flag 分别表达 scan ownership 与 delivery ownership

**选择**：

- `CODOXEAR_ENABLE_VOICE_SCAN=1`：Rust server 启动 log scan worker，观察 session logs 并写/更新 `voice_delivery_ledger.json`、入队 announcements / push final response。
- `CODOXEAR_ENABLE_VOICE_WORKER=1`：Rust server 启动 delivery worker / HLS keepalive / WebPush debug side effects，拥有 `audio/` 和 delivery queue。

两个 flag 默认 off。生产灰度建议先在测试 app_dir 开 `VOICE_SCAN=1` 跑 dry/log parity，再同时打开 `VOICE_WORKER=1` 做真实设备验证；不允许 Rust worker 只在无法持锁时降级为“部分写入”。

**原因**：Phase 0 umbrella 已把 scan/worker 拆成两个 flag；scan 和 HLS/WebPush 副作用风险不同，独立回滚粒度更安全。

**替代方案**：一个 `CODOXEAR_ENABLE_VOICE=1` 控制全部。否决：无法隔离“只观察 ledger parity”和“真实推送/音频副作用”。

### D3：single-writer 用 app-dir 级 voice lock + Python handoff 双保险

**选择**：Rust voice owner 启动时创建并持有 `~/.local/share/codoxear/voice_worker.lock`（或 `audio/.voice-worker.lock`，实现需在 docs 中固定），用 `create_new`/`O_EXCL` 写入 pid、role、timestamp、process kind；退出时删除。Python `SessionManager` 在 Rust voice flag truthy 时不启动 scan thread；同时 `VoicePushCoordinator` 需要新增“worker disabled / HTTP-only”模式，避免仅因 Python server 被启动用于 fallback endpoints 就产生 HLS keepalive/worker 写入。

**原因**：仅靠 env flag 不足以防止两个进程误配置；仅靠 lock 也不能阻止 Python 已启动线程继续写。双保险可满足“Python worker 与 Rust worker 不能同时写同一 ledger/HLS 输出”。

**替代方案**：文件级 lock 每次写 ledger/HLS 时短持有。否决：无法避免两个 worker 都观察到同一消息后先后发送 WebPush/TTS，只能避免 JSON 物理损坏，不能避免重复副作用。

### D4：HLS artifact 与 Python 保持文件级兼容，不引入 Rust-native muxer

**选择**：Rust HLS 继续 shell out 到 `ffmpeg`/`ffprobe`，生成与 Python 相同的 AAC→MPEG-TS segment 结果：

- root：`<app_dir>/audio/`
- playlist：`audio/live.m3u8`
- segments dir：`audio/segments/`
- audio input temp：`segments/<message_id[:12]>.aac`
- temp chunks：`segments/<message_id[:12]>-part-%03d.ts`
- final segment：`segments/%06d-<prefix[:12]>.ts`
- target duration：`max(12, ceil(max(segment durations)))`
- max segments：18，超出删除最旧 segment
- silence keepalive：listener active 且 idle 时每 6s 追加 `anullsrc=r=24000:cl=mono` silence segment

**原因**：HLS player 看到的是 byte artifact；用同一个 ffmpeg/ffprobe pipeline 最容易保持播放兼容和 duration 行为。

**替代方案**：使用 Rust AAC/HLS muxer crate。否决：会改变 segment bytes/duration/codec 细节，Phase 5 风险过大。

### D5：WebPush/VAPID 以“同 PEM、同 public key、真实 endpoint 可投递”为等价标准

**选择**：Rust 复用 `webpush_vapid_private.pem`。实现前 spike Rust crate（候选如 `web-push` 或维护状态更好的等价 crate）必须证明：

1. 能读取 py_vapid 生成的 PEM private key；
2. 导出的 public key 是 uncompressed P-256 point，base64url(no padding) 后与 Python `Vapid.public_key.public_bytes(X962, UncompressedPoint)` 一致；
3. JWT `sub`、`aud`、`exp`、TTL=300、ECDSA P-256/SHA-256 签名能被 browser push service 接受；
4. 对 404/410/stale endpoint 的错误分类与 Python `_should_drop_push_subscription` 等价，能 drop stale subscriptions 并写 `last_failure_ts`/`last_error`。

**原因**：WebPush 失败可能只在真实 push service 暴露；单元测试签名不够。Public key 等价保证前端现有 subscription 不需要重新注册。

**替代方案**：生成新的 Rust VAPID key 并要求用户重新订阅。否决：破坏 rollback 和现有 mobile subscription。

### D6：OpenAI-compatible client 只做当前 Python API surface

**选择**：Rust HTTP client 实现两个 POST：

- `{base_url}/chat/completions`，JSON body 与 Python 完全一致（system prompt、temperature 0.2、max_completion_tokens 90、messages），解析 `choices[0].message.content`，支持 string 或 `[{type:text|output_text,text}]`。
- `{base_url}/audio/speech`，JSON body `{model, voice, input, response_format:"aac"}`，Accept `application/octet-stream`，timeout 默认 30s。

错误语义保持 Python：missing key 是 `tts_api_key is required`；HTTP error 包含 `/<route> failed with <code>: <detail>`；empty audio 是 `audio/speech returned empty body`。不在 Phase 5 引入流式 TTS、Responses API 或重试新策略；只允许对网络超时做 bounded single-attempt error propagation 并写 ledger。

**原因**：当前 ledger/status tests 依赖这些错误与状态；扩展 API 会扩大范围。

**替代方案**：迁移到 OpenAI Responses / speech streaming。否决：非目标且会改变用户可见延迟与错误。

### D7：ledger status machine 以 Python tests 为 oracle

**选择**：Rust 必须覆盖并通过 Python `tests/test_voice_push.py` 中的语义：

- final_response：先写 ledger pending；summary 成功则 `summary_status=sent`、`summary_text`、`notification_text`，push 发送固定 `DEFAULT_PUSH_NOTIFICATION_TEXT`；TTS disabled 则 `narrated_status=skipped`。
- narration：仅在 `tts_enabled_for_narration` true 且有 active listener 时入队；同 session narration 可 merge；同 slot 新 final_response 可 replace 旧 queued task。
- no listener / listener epoch drift：mark skipped，不生成/append stale audio。
- summary failure：mobile push 仍使用固定 text；ledger `summary_status=error`、`last_error` clipped；不产生 TTS final_response。
- ledger trim：`DELIVERY_LEDGER_MAX` 上限与 Python 一致，按 `updated_ts/created_ts` 删除最旧。

**原因**：这是 voice UX 的核心；Rust 实现不应按“更合理”的状态机重写语义。

**替代方案**：简化为 final_response-only push，无 narration/HLS queue parity。否决：不满足 Phase 5 范围和已有 tests。

### D8：Rust scan 读取现有 session/log metadata，不新增 broker dependency

**选择**：Rust voice scan worker 复用 `session_loader::load_session_rows` 与 log normalizer/delivery extraction；读取 `log_path`、`session_id`、alias/display name、resume muted 标记、`delivery_log_off`。若 Rust 当前没有持久化 `delivery_log_off` 等价字段，实现需先在 Rust session state 中补齐，确保重启后不从 0 重复扫描导致重复 ledger/push。

**原因**：Phase 5 与 broker 只通过 disk/log contract 集成，避免再次修改 broker protocol。

**替代方案**：voice worker 通过 broker socket 实时订阅消息。否决：需要新 broker command，违反非目标。

## Risks / Trade-offs

- **[Risk] Rust WebPush crate 与 pywebpush/VAPID 不完全等价** → Mitigation：先做 spike + public key equivalence unit test + real push endpoint smoke；若候选 crate 不满足，改用低层 `p256`/`ecdsa` + HTTP 手写，不能要求用户重新订阅。
- **[Risk] Python `VoicePushCoordinator` 初始化即启动 worker，导致 Rust flag 开启时仍有 Python HLS/ledger writer** → Mitigation：实现 Python HTTP-only/disabled worker 模式，并加测试证明 `CODOXEAR_ENABLE_VOICE_WORKER=1` 时不会启动 Python voice worker/keepalive 线程。
- **[Risk] HLS playlist 被两个 worker 或旧 segment 污染** → Mitigation：Rust worker 拿 voice lock 后可 reset/rewrite playlist，但不得删除不属于 current segment window 的未知文件；cleanup 只删除自己 tracked 的 stale segments 和 temp chunks。
- **[Risk] Rust scan 重启后重复推送历史 final response** → Mitigation：ledger dedupe by `message_id` + persisted `delivery_log_off` parity + resume sessions mute；contract tests 覆盖 truncated log resets。
- **[Risk] ffmpeg/ffprobe 缺失导致 worker反复失败** → Mitigation：startup health/status 在 `settings_snapshot.audio.last_error` 暴露 `ffmpeg and ffprobe are required for merged HLS output`；任务 mark error/skipped 与 Python 一致；不 crash 整个 server。
- **[Risk] OpenAI-compatible endpoint latency阻塞 worker** → Mitigation：TTS/summary 在 dedicated tokio blocking/async task 中执行；HTTP timeout 30s；worker loop 不阻塞 HTTP routes。
- **[Trade-off] 继续 shell out ffmpeg/ffprobe** → 接受；这是与 Python artifact parity 的代价，后续可单独优化。
- **[Trade-off] 保留 Python fallback 造成两套 voice code 并存** → 接受；这是 Phase 6 前可回滚 cutover 的必要成本。

## Migration Plan

1. 代码默认落地但不启用：`CODOXEAR_ENABLE_VOICE_SCAN` / `CODOXEAR_ENABLE_VOICE_WORKER` unset 时 Rust server 行为保持 Phase 3（无 voice side effects），Python fallback 仍可运行。
2. 在临时 app_dir 运行 Rust unit/contract tests，验证 settings/subscriptions/ledger/HLS artifact 与 Python fixtures 双向兼容。
3. 打开 `CODOXEAR_ENABLE_VOICE_SCAN=1`（不打开 worker）运行 log scan parity：观察 ledger 新增记录、notification feed/message 与 Python oracle 一致；无 WebPush/TTS/HLS 副作用。
4. 打开 `CODOXEAR_ENABLE_VOICE_SCAN=1 CODOXEAR_ENABLE_VOICE_WORKER=1`，用测试 subscription / fake OpenAI / fake ffmpeg 或 fixtures 跑自动化；再接真实 OpenAI-compatible endpoint + Tailscale HTTPS + iOS/mobile subscription 做 smoke。
5. 验证 rollback：停止 Rust server，unset 两个 voice flag，启动 Python server；Python 读取 Rust 写的 `voice_delivery_ledger.json` / `push_subscriptions.json`，HLS playlist endpoint 仍可返回或重建，不重复推送已 sent 的 message。
6. 在 `docs/cutover/cutover-log.md`（如存在）追加 Phase 5 行，列出已验证 env flags、rollback 结果和真实设备验证摘要。

## Open Questions

1. **WebPush crate 最终选择**：需要 coding-agent 在 task 1 中 spike；推荐以 PEM/public-key/真实 push 成功为准，而不是 crate 流行度。
2. **Python `VoicePushCoordinator` disabled 模式最小改动点**：是构造参数 `enable_worker=False`，还是延迟初始化 worker threads？coding-agent 可选，但必须保留 HTTP settings/subscription endpoints 可用。
3. **`delivery_log_off` 在 Rust 侧的持久化位置**：若 Phase 4/当前 Rust session rows 没有等价 mutable field，需要先调查现有 `SessionRow` / sidecar patch 机制，选择与 Python兼容的方式记录 offset。
4. **真实设备验证责任人**：自动化能覆盖 WebPush signing/fake endpoint，但 iOS/mobile push 需要有人提供实际 subscription 环境；coding-agent 完成实现后必须在 handoff 中明确是否已跑真实设备 smoke。
