## 1. Rust 后端现状确认与脚手架

- [ ] 1.1 读取当前分支 Phase 4 Rust 后端结构，确认 Rust crate/sidecar/bridge 的实际目录；若本 checkout 没有 Rust 文件，先同步到包含 Phase 4 的分支再继续
- [ ] 1.2 在现有 Rust 后端中新增 voice push 模块骨架（models/store/hls/webpush/tts/coordinator/bridge），避免新建第二套不受控 runtime
- [ ] 1.3 增加 Rust 依赖并记录选型理由：serde/serde_json、临时文件原子替换、HTTP client、WebPush/VAPID/P-256、测试 mock 工具
- [ ] 1.4 增加环境开关（建议 `CODEX_WEB_RUST_VOICE_PUSH=1`），在启动日志或诊断中输出 active voice backend

## 2. Contract fixtures 与跨语言基线

- [ ] 2.1 新增 voice contract fixture 目录，包含 `voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json` 的正常/脏数据样例
- [ ] 2.2 基于 Python `voice_push.py` 当前行为生成 golden 输出：settings clean、subscription id、ledger clean、VAPID public key、HLS playlist 片段
- [ ] 2.3 新增 Python contract tests，断言现有 Python worker 对 fixture 的 canonical 行为，防止迁移期间误改基线
- [ ] 2.4 新增 Rust contract tests，读取同一 fixtures 并与 Python golden 输出逐字段对齐

## 3. Rust settings / subscriptions / ledger stores

- [ ] 3.1 实现 `voice_settings.json` 读取、默认值、字段清洗、base URL 校验、原子保存，并覆盖缺失文件和非法输入测试
- [ ] 3.2 实现 `push_subscriptions.json` 读取、subscription 校验、endpoint hash 前 24 hex id、device_class 推断、upsert/toggle/drop、原子保存
- [ ] 3.3 实现 `voice_delivery_ledger.json` 读取、坏行过滤、字段默认、状态更新、按时间裁剪到最大行数、原子保存
- [ ] 3.4 增加 Rust 写出文件再由 Python cleaner 回读的 rollback compatibility 测试，必须使用实际文件名

## 4. Rust VAPID / WebPush 等价实现

- [ ] 4.1 实现读取现有 `webpush_vapid_private.pem` 并导出 URL-safe base64 no-padding VAPID public key，测试与 Python 输出完全一致
- [ ] 4.2 实现缺失 PEM 时生成 P-256 ECDSA VAPID 私钥，并增加 Python 可回读测试
- [ ] 4.3 选定并封装 Rust WebPush crate，支持 subscription endpoint、keys.p256dh/auth、payload encryption、VAPID subject、TTL、timeout
- [ ] 4.4 用 mock push server / mock sender 测试 payload JSON 字段、VAPID subject、成功时 subscription last_success 更新、失败时 last_failure/last_error 更新
- [ ] 4.5 覆盖 stale subscription drop：`.invalid` endpoint 或等价 404/410 失败后移除 subscription

## 5. Rust HLS stream 输出

- [ ] 5.1 实现 Rust HLS stream 状态：`audio/live.m3u8`、`audio/segments/`、media sequence、target duration、max segments、last_error
- [ ] 5.2 实现 path-safe segment lookup：只允许 basename `.ts`，拒绝 traversal 和非 `.ts` 名称
- [ ] 5.3 实现音频 append：写入临时 AAC、调用 `ffmpeg` 生成 `.ts`、调用 `ffprobe` 取 duration、重写 playlist、清理旧 segment
- [ ] 5.4 实现 keepalive silence 与 reset，保持 `anullsrc`/静音 segment 语义兼容
- [ ] 5.5 用 fake `ffmpeg`/`ffprobe` 或命令 mock 增加 playlist golden tests：顺序、target duration、retention、reset、缺失工具错误

## 6. Rust TTS / summary client

- [ ] 6.1 实现 OpenAI-compatible `/chat/completions` summary client，保留 final response 约 30 words 与 narration 约 15 words 的 prompt 语义
- [ ] 6.2 实现 OpenAI-compatible `/audio/speech` TTS client，输出 AAC bytes，保留 model/voice/input/response_format 字段
- [ ] 6.3 将 summary/TTS client 抽象为 trait，单元测试中使用 fake client 覆盖成功、空响应、HTTP 错误、缺失 API key

## 7. Rust voice coordinator 状态机

- [ ] 7.1 实现 coordinator public 方法集合，与 Python route 调用所需方法一一对应
- [ ] 7.2 实现 listener heartbeat、listener epoch、no-listener drop、queue/prepared/playing 状态和 condition/async wakeup 机制
- [ ] 7.3 实现 final response observe：ledger 初始化、summary、push send、optional TTS、voice 选择、无 API key fallback
- [ ] 7.4 实现 narration observe：仅在 narration TTS enabled 时入队，dequeue 前 summary，多个 narration 合并
- [ ] 7.5 实现 replacement 语义：同 session + message_class 的较新 final response 替换旧 queued task，ledger 标记 skipped/replaced
- [ ] 7.6 实现 task error 处理：summary/TTS/HLS/push 失败写入 clipped `last_error` 和状态，但 worker 继续处理后续任务
- [ ] 7.7 增加 Rust coordinator 单元测试，对齐 `tests/test_voice_push.py` 中 final response、narration、listener、replacement、ledger、voice mapping 的关键场景

## 8. Python server bridge 与鉴权路径（需二次 review）

- [ ] 8.1 在 `codoxear/server.py` 初始化阶段按 Rust voice env flag 选择 Python 或 Rust coordinator，确保不会双 worker 处理同一 app dir（需二次 review）
- [ ] 8.2 如采用 Rust sidecar/Unix socket/stdio bridge，实现 Python 适配类，使现有 route 继续调用同名 coordinator 方法（需二次 review）
- [ ] 8.3 保持现有 `_require_auth` 鉴权 gate 不变，补充 tests 覆盖 Rust worker active 时 settings/subscription/feed/audio route 的未鉴权拒绝与成功响应（需二次 review）
- [ ] 8.4 补充 server init tests：flag off 使用 Python worker；flag on 使用 Rust worker；bridge 启动失败 fail closed 且错误可诊断

## 9. API / 前端兼容验证

- [ ] 9.1 运行或新增 Python API contract tests，断言 `GET/POST /api/settings/voice` response shape 与现有 frontend types 兼容
- [ ] 9.2 运行或新增 subscription API tests，覆盖 upsert/toggle/test_push/feed/message 在 Rust worker active 时的 JSON 字段
- [ ] 9.3 运行或新增 HLS route tests，断言 `/api/audio/live.m3u8` content type、cache-control、segment content type 与 404 行为
- [ ] 9.4 如 frontend 类型受影响，更新 `web/src/lib/types.ts` / tests；若无变化，记录“不需要 UI 改动”的验证结果

## 10. 文档、迁移与回滚

- [ ] 10.1 更新 `README.md` 或运行配置文档，说明 Rust voice env flag、默认 Python、灰度方式、关闭 flag 回滚方式
- [ ] 10.2 更新 `AGENTS.md` Components/Ops notes，说明 voice push 可由 Python 或 Rust worker 提供，以及不要双 worker 运行
- [ ] 10.3 记录 Rust WebPush/VAPID crate 选型和与 `pywebpush` 等价性验证结果
- [ ] 10.4 明确运行时依赖：`ffmpeg`、`ffprobe`、TTS API key、VAPID subject env、Tailscale subject fallback

## 11. 验证与交接

- [ ] 11.1 运行 `openspec validate rust-backend-voice-push --strict`
- [ ] 11.2 运行 Rust build/test（具体命令以 Phase 4 Rust crate 为准，例如 `cargo test`）
- [ ] 11.3 运行 Python voice/API contract tests：至少覆盖 `tests/test_voice_push.py`、`tests/test_voice_push_source.py` 和新增 Rust bridge tests
- [ ] 11.4 如触及 web 类型或 UI，运行 `cd web && npm run build`，否则说明未触及 web build
- [ ] 11.5 本地 smoke：flag off 启动确认 Python worker；flag on 启动确认 Rust worker；关闭 flag 后 Python worker 能读取 Rust 写出的三个 JSON 文件
- [ ] 11.6 交给 `code-reviewer` 做总体 review；若实现触及 Go 代码则另请 `go-reviewer`（当前预期不触及 Go）