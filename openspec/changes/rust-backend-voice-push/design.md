## Context

Phase 1–4 已经把 Rust backend skeleton、GET/read-only parity、POST/write routes、queue/harness worker、Python worker handoff、以及 Rust broker opt-in 路径落地。当前 Phase 5 的事实边界如下：

- Python `codoxear/voice_push.py` 仍然是完整 voice side-effect owner：它维护 `VoicePushCoordinator`、后台 `voice-push` worker、`voice-push-keepalive`、HLS `MergedHLSStream`、WebPush/VAPID、OpenAI-compatible summary/TTS、listener state 和 `voice_delivery_ledger.json`。
- Rust 已有轻量 voice 文件读写：`backend-rs/src/voice_state.rs` 读取 `voice_settings.json` / `push_subscriptions.json` / `voice_delivery_ledger.json`，`voice_post.rs` 写 settings/subscription/listener，并让 `/api/notifications/test_push`、`/api/audio/test_announcement` 返回 Phase 3 feature-disabled 501。
- Python worker handoff 已在 `codoxear/server.py` 中实现：`CODOXEAR_ENABLE_VOICE_SCAN` truthy 时 Python 不启动 `voice-push-scan` 线程；但 Python `VoicePushCoordinator` 初始化仍会创建 worker/HLS/VAPID state，作为 fallback 保留。
- `docs/cutover/disk-contracts.md` 明确 live 文件名必须是 `voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`，VAPID 文件是 `webpush_vapid_private.pem`。
- `docs/cutover/endpoint-inventory.md` 把 `POST /api/notifications/test_push` 与 `POST /api/audio/test_announcement` 标为 Phase 5 owner；HLS GET routes 已在 inventory 中要求保持 `/api/audio/live.m3u8` 与 `/api/audio/segments/*`。

最危险的不是单个 HTTP route，而是后台副作用所有权：Python 和 Rust 不能同时扫描同一日志、写同一 ledger、投递同一 push 或 append 同一 HLS playlist。

## Goals / Non-Goals

**Goals:**

- 将 `voice_push.py` 对用户可见的 voice side effects port 到 Rust：voice scan、ledger lifecycle、OpenAI summary/TTS、HLS live stream、WebPush/VAPID、listener/test endpoints。
- 保持现有前端与 API contract：路径、auth、JSON 字段、状态码、content type 与 Python 兼容。
- 保持磁盘兼容：Rust 写出的 `voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json`、`webpush_vapid_private.pem` 必须被 Python fallback 读取。
- 实现可灰度、可回滚：Rust voice scan / worker 默认 off，开启时 Python 让出；关闭 Rust flags 后 Python 能从同一文件恢复。
- 提供自动化验证：HLS artifact、ledger/subscription contract、OpenAI mock、WebPush/VAPID mock、Rust/Python cross-read、worker flag gating。

**Non-Goals:**

- 不删除 `codoxear/voice_push.py`、Python `VoicePushCoordinator` 或 Python server fallback；这是 Phase 6 以后才考虑。
- 不改变 Phase 4 broker protocol，不要求 broker 主动推送 voice events；Rust voice scan 从已落地的 `session_loader` / `log_normalizer` / session metadata 读取。
- 不改变前端音频/通知交互，不新增 API 字段作为前端依赖。
- 不把 `voice_settings.json` / subscription schema 迁移到新文件或数据库。
- 不把真实第三方 WebPush/iOS smoke 作为自动 CI 硬依赖；自动化用 mock，真实设备 smoke 作为 handoff 记录。

## Decisions

### D1：拆分 `CODOXEAR_ENABLE_VOICE_SCAN` 与 `CODOXEAR_ENABLE_VOICE_WORKER`

**选择**：Rust voice scan（发现 assistant messages 并写 ledger）由 `CODOXEAR_ENABLE_VOICE_SCAN` 控制；Rust delivery worker（summary/TTS/HLS/WebPush/debug endpoints）由 `CODOXEAR_ENABLE_VOICE_WORKER` 控制。worker 默认 off；`VOICE_WORKER=1` 必须要求 scan ownership 已明确，或者启动时报错。

**原因**：扫描 ledger 与实际发送 WebPush/TTS 是不同风险面。scan 可以先灰度验证 ledger 互读；delivery worker 涉及外部网络、HLS 文件和用户设备通知，必须单独开关。

**替代方案**：一个 `CODOXEAR_ENABLE_VOICE=1` 总开关。否决：无法分阶段定位问题；一旦 WebPush/TTS 出错只能整体回滚。

### D2：实现 `voice_worker` 独立模块，不把逻辑塞进 `workers.rs`

**选择**：新增 `backend-rs/src/voice_worker/` 目录或 `voice_worker.rs` + 子模块：`state.rs`（in-memory queue/listener）、`ledger.rs`、`scan.rs`、`openai.rs`、`hls.rs`、`webpush.rs`、`vapid.rs`、`tasks.rs`。保留每个 `backend-rs/src/**/*.rs` ≤ 800 行。

**原因**：Python `voice_push.py` 已经 ~1800 行，直接 port 到单 Rust 文件会重复 Phase 0 的单文件问题。voice worker 还涉及外部依赖和状态机，拆模块便于 reviewer 按边界检查。

**替代方案**：在现有 `voice_state.rs` / `voice_post.rs` 内继续加逻辑。否决：这些模块当前只负责 file snapshot 和 lightweight POST；混入后台状态机会让 read-only paths 难以保持无副作用。

### D3：ledger 是跨进程事实源，队列/playing/listener 是进程内运行态

**选择**：`voice_delivery_ledger.json` 继续作为跨 Python/Rust 的 durable delivery state；Rust 内部维护 queue、prepared、generating、playing、listener_epoch、active listeners、last_append_ts 等运行态。重启后 Rust 通过 ledger status 与 logs 重新构造未完成工作，不持久化新 queue 文件。

**原因**：Python 没有单独 `voice_queue.json`，新增持久队列会扩大磁盘 contract。ledger 已经记录 pending/sent/error/skipped，足以恢复扫描与避免重复投递。

**替代方案**：新增 Rust-only queue 文件。否决：Python fallback 不能理解，回滚时会丢 pending tasks 或重复投递。

### D4：Rust scan 复用现有 session loader + log normalizer，而不是读取 HTTP messages endpoint

**选择**：voice scan 周期调用 `session_loader::load_session_rows`，对有 `log_path` 的 session 使用 Rust `log_normalizer` / final-turn helpers 获取 assistant messages，构造 `ClassifiedAssistantMessage` 等价结构。

**原因**：Rust 已经在 Phase 2 port 了 Codex/Pi log normalizers；直接复用可避免 HTTP 自调用和 auth/cookie 问题，也能用现有 fixtures 做 parity tests。

**替代方案**：让前端或 HTTP route 触发 `observe_messages`。否决：voice 是后台能力，不能依赖浏览器打开页面；Python 当前也是 server scan。

### D5：HLS 继续使用 ffmpeg/ffprobe subprocess，先保持 artifact parity

**选择**：Rust HLS append 仍调用 `ffmpeg` 分段和 `ffprobe` 获取 duration，生成与 Python `MergedHLSStream` 相同 playlist/segment artifact；后续若要换纯 Rust media pipeline，另开 change。

**原因**：HLS byte/artifact contract 是 Phase 5 重点。复用 ffmpeg 行为可最大化与 Python 一致，减少 audio codec 差异。

**替代方案**：使用 Rust media crate 直接 mux MPEG-TS/HLS。否决：依赖与兼容风险高，且不是 cutover 的目标。

### D6：WebPush crate 先做等价 spike，再固化依赖

**选择**：coding-agent 先评估并选择 Rust WebPush/VAPID crate（候选 `web-push` 或维护状态更好的替代），以以下硬指标为准：能加载/生成 P-256 VAPID PEM、公开 uncompressed point、设置 TTL、payload、VAPID `sub`、兼容浏览器 push subscription `p256dh/auth`。选择结果写入 design 的 Implementation Notes。

**原因**：伞形 design 已把 `web-push` 0.10.2 是否覆盖 `pywebpush` 行为列为 open question。Phase 5 必须用测试证明，而不是凭包名假设。

**替代方案**：调用 Python `pywebpush` helper。否决：这会保留 Python runtime dependency，不能完成 Rust cutover。

### D7：VAPID PEM 兼容优先于 Rust-native key format

**选择**：Rust 必须直接读写 `webpush_vapid_private.pem`，格式以 Python `py_vapid.Vapid.private_pem()` 为 contract。`vapid_public_key` 计算为 X9.62 uncompressed public point的 base64url 无 padding。

**原因**：这是现有浏览器 subscription 与 service worker 公开 key 的兼容基础。如果 Rust 生成不同 key 或不同 public key 编码，现有设备需要重新订阅，违背 cutover 目标。

**替代方案**：迁移到 JWK 或 Rust crate 自己的 key store。否决：破坏 Python fallback 和现有 subscription。

### D8：WebPush payload 与 Python“默认推送文案”语义保持，不顺手修正

**选择**：Rust final-response push payload 保持 Python `_send_push_notifications` 当前语义：payload `notification_text` 使用传入的 `DEFAULT_PUSH_NOTIFICATION_TEXT`（当前是“回复完成”），而 ledger `notification_text` 仍可存 summary/preview。若产品要改变推送文案，另开 change。

**原因**：Phase 5 目标是兼容，不是产品行为调整。这里容易被实现者误以为应该发送 summary 文本，但 Python 当前不是这样。

**替代方案**：推送 summary 文案。否决：改变用户可见通知语义，且与 issue “推送语义兼容”冲突。

### D9：debug endpoints 在 worker disabled 时继续显式 disabled；enabled 时真实 side effect

**选择**：`POST /api/notifications/test_push` 和 `POST /api/audio/test_announcement` 在 `VOICE_WORKER` falsy 时返回明确 disabled error；truthy 且 prerequisites 满足时执行真实 Rust WebPush/TTS/HLS 路径。

**原因**：Phase 3 已经建立“不假成功”的安全语义；Phase 5 只在明确启用时改变为真实成功。

**替代方案**：worker disabled 时 fallback 调 Python。否决：会引入同进程跨语言调用和双 owner 复杂性；回滚应通过进程/flag，不是在 Rust route 内调用 Python。

### D10：contract tests 以 Python oracle + mock external services 双层验证

**选择**：自动化测试分两层：
1. Rust unit/integration 用 mock OpenAI server、mock WebPush receiver、fake ffmpeg/ffprobe 或 controlled subprocess 验证状态机和 artifacts。
2. Cross-language tests 用 Python loaders / `tests/test_voice_push.py` oracle 验证 disk schema、VAPID public key、HLS playlist invariants。

**原因**：真实 OpenAI/WebPush/iOS 不适合 CI；但完全不测外部 protocol 又会错过 VAPID/ECDSA 等价风险。

## Risks / Trade-offs

- **[Risk] Rust WebPush crate 与 `pywebpush` 的 VAPID/ECDSA/header 加密细节不一致** → **Mitigation**：先做 spike test：同一 PEM + 同一 subscription + 同一 payload，比较 public key、TTL/sub claims，并通过 mock/fixture verifier；不通过则换 crate，不降级调用 Python。
- **[Risk] Python 与 Rust 双写 ledger/HLS 造成重复通知或 playlist corrupt** → **Mitigation**：worker flags 默认 off；启动时检测 voice lock（例如 `<app_dir>/voice_delivery_ledger.json.lock` 或 `<app_dir>/audio/.voice-owner.lock`）并 fail-fast；Python 已按 `CODOXEAR_ENABLE_VOICE_SCAN` 让出 scan。
- **[Risk] HLS duration/segment naming 与 Python 有细小差异导致 Safari/iOS 播放问题** → **Mitigation**：照 Python ffmpeg/ffprobe 参数实现；新增 playlist byte/invariant tests；真实设备 smoke 列为 handoff。
- **[Risk] 重启 Rust 后 pending ledger rows 被重复推送** → **Mitigation**：扫描时不重置已有 row；delivery 只处理 pending 状态，并在发送前后原子更新 status；WebPush success/failure timestamps与 status 一起写。
- **[Risk] OpenAI API key 泄漏到 logs/tests** → **Mitigation**：错误日志只记录 endpoint/status/裁剪 detail，不打印 Authorization header 或完整 request；tests 使用 fake key。
- **[Risk] active listener 是内存态，切换 Python/Rust 时 test announcement 行为不同** → **Mitigation**：这是现有 Python 行为；rollback 不承诺迁移 active listeners，用户浏览器 heartbeat 会在 45 秒内恢复。
- **[Trade-off] 保持 ffmpeg/ffprobe 外部依赖** → 接受；Python 已有同样依赖，cutover 后运维要求不增加新类别，只是 Rust 也检查并报告。

## Migration Plan

1. **Pre-flight**：确认 Phase 4 基线在 `92d2a0f` 或包含它的分支；运行 `openspec validate rust-backend-cutover --strict` 与现有 Rust tests。
2. **Dependency spike**：选择 WebPush/VAPID crate，证明同一 PEM 的 public key 与 Python 一致；确定 fake/mocked WebPush test 方案。
3. **Module scaffold**：新增 `voice_worker` 模块、HLS/VAPID/OpenAI/WebPush clients 的 trait 化接口，先写无副作用 unit tests。
4. **Disk + ledger parity**：补齐 ledger writer、status transitions、trim、Rust/Python cross-read tests。
5. **Scan worker**：实现 `CODOXEAR_ENABLE_VOICE_SCAN` gated scan loop；先只写 ledger，不 delivery；验证关闭 flag 回 Python 不重复。
6. **Delivery worker**：实现 listener state、queue/merge/replace、summary/TTS、HLS append、keepalive silence；接入 `CODOXEAR_ENABLE_VOICE_WORKER`。
7. **WebPush + debug endpoints**：实现 VAPID load/create、test push、final-response push、subscription failure/drop；将 Phase 3 501 endpoints 改成 enabled-path 真实 side effects。
8. **HTTP/HLS serving**：确保 `/api/audio/live.m3u8` 和 segments 从 Rust HLS state/disk 服务，content type 与 404 行为兼容。
9. **Docs and umbrella update**：更新 disk contracts Phase 5 notes、endpoint inventory Phase 5 rows、umbrella tasks Phase 5 row、README flags/rollback。
10. **Verification**：运行 OpenSpec validate、Rust fmt/clippy/test/build、Python voice tests、contract voice selectors；记录 real-device smoke 是否完成。

Rollback：停止 Rust server，unset `CODOXEAR_ENABLE_VOICE_SCAN` 和 `CODOXEAR_ENABLE_VOICE_WORKER`，重启 Python `codoxear-server`。不删除 `voice_delivery_ledger.json` / subscription / settings / VAPID PEM。若 HLS playlist 因实验损坏，可让 Python coordinator reset/rewrite `audio/live.m3u8`；ledger/subscription 文件不需要迁移。

## Open Questions

1. Rust WebPush/VAPID crate 最终选型：`web-push` 是否足够，还是需要替代 crate / 低层 ECE + JWT 组合？coding-agent 必须先用 spike test 决定。
2. Rust voice worker 是否应在启动时创建跨进程 lock 文件并让 Python 检测？当前 Python 只按 env flag 让出 scan；建议 Phase 5 至少 Rust 自身 fail-fast，是否补 Python lock 检测由实现复杂度决定。
3. HLS tests 是否使用真实 ffmpeg/ffprobe 还是 fake binary？建议 unit tests fake，integration/CI 在可用时跑真实 ffmpeg selector，缺失时明确 skip 并保留 playlist pure tests。
4. Real iOS/Tailscale HTTPS push smoke 是否由 coding-agent 完成还是交给人类？自动化可证明协议大部分行为，但真实 APNs/WebPush 仍建议人工 smoke。
## Implementation Notes

- 2026-05-18 coding-agent Phase 5 partial implementation selected Rust `web-push` 0.11.0 with `default-features = false` for VAPID PEM parsing/public-key derivation and future request construction. The spike test loads a Python `py_vapid`-generated `webpush_vapid_private.pem` fixture and verifies the base64url uncompressed public key matches Python exactly. Rust-created keys are emitted as PKCS#8 PEM because `web-push` can reload that format reliably; Python readability is left in the remaining Phase 5 cross-language gate.
- The first implementation slice added `backend-rs/src/voice_worker/` scaffolding, trait boundaries for OpenAI/WebPush/HLS/clock, a Rust voice owner lock (`voice_worker.lock`), VAPID helpers, ledger trim helpers, HLS playlist/segment serving helpers, and WebPush payload/drop semantics tests. It does **not** yet implement the full scan loop, OpenAI HTTP calls, HLS ffmpeg append, or enabled debug side-effect paths.
- Python fallback remains present. `CODOXEAR_ENABLE_VOICE_WORKER` now prevents Python `voice-push` / `voice-push-keepalive` delivery threads from starting, while `CODOXEAR_ENABLE_VOICE_SCAN` continues to prevent the Python scan thread. This keeps rollback simple: unset Rust flags and restart Python.
- 2026-05-18 continuation adds a gated Rust scan loop (`CODEX_WEB_VOICE_SCAN_SECONDS`, default 2.5s) that reuses `session_loader::load_session_rows` and `log_normalizer::codex::read_chat_events_from_tail` for Codex/Pi logs, writes Python-compatible ledger rows with sorted pretty JSON + trailing newline, and preserves same-slot pending replacement semantics. `CODOXEAR_ENABLE_VOICE_WORKER=1` without scan now enters explicit `worker-drain-only` ownership with a warning rather than silently co-writing scan state.
- Listener heartbeat state is now a Rust runtime registry used by `/api/audio/listener` and `/api/settings/voice.audio.*`; last listener drop clears queued/prepared/generating/playing state, resets `audio/live.m3u8`, and marks affected ledger rows skipped with `last_error:"no active listener"`. The enabled debug endpoints are still partial: test push can exercise subscription update/drop semantics under the worker flag, while full encrypted WebPush send, OpenAI TTS, ffmpeg append, and the full worker loop remain outstanding.
- 2026-05-19 continuation adds a concrete OpenAI-compatible HTTP client for `/chat/completions` and `/audio/speech` (currently direct `http://` support plus trait mocks for tests), a Rust `MergedHlsStream` ffmpeg/ffprobe implementation with silence keepalive and rolling 18-segment cleanup, and a real WebPush message builder/sender backed by `web-push`’s `isahc-client` feature for HTTPS-capable encrypted sends. `POST /api/notifications/test_push` now attempts real Rust WebPush delivery under `CODOXEAR_ENABLE_VOICE_WORKER=1` and updates/drops subscription records; `POST /api/audio/test_announcement` now creates a Python-compatible `test-...` ledger row and enqueues it when an active listener and API key exist. The remaining Phase 5 gap is still the continuous delivery worker state machine that drains queued ledger tasks through OpenAI → HLS → final ledger status; the enabled test announcement endpoint queues work but does not yet synthesize/append it without that worker loop.
- 2026-05-20 closeout completes the remaining compatibility gates: Rust-created PKCS#8 `webpush_vapid_private.pem` is verified by Python `py_vapid.Vapid.from_file` with identical public-key output; VAPID subject selection now mirrors Python env → Tailscale `Self.DNSName` → `https://localhost`; Codex and Pi log fixture scans cover final responses, enabled/disabled narration, duplicate restart scans, replacement, and malformed lines; and v1/legacy voice snapshot, notification feed/message, HLS, auth, and disabled debug endpoint contracts are covered. Real iOS/Tailscale device smoke remains a manual pending item because this agent environment has no real iOS Home Screen app, Tailscale HTTPS device, or browser push endpoint credentials.
