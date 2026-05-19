## Why

Phase 2 已让 Rust server 覆盖全部 GET / readonly 路径，但前端一旦执行登录、创建会话、发送消息、队列编辑、文件写入、sidebar 编辑、Pi AskUser 回应等操作，仍必须回到 Python `codoxear/server.py`。Phase 3 要把这些 POST / 写端点补齐到 Rust，并补上 Python sweep 按 env flag 让出的规则，避免双轨期间同一份 `harness.json` / `session_queues.json` / voice ledger 被 Python 与 Rust 同时写。

## What Changes

- 新增 Rust POST router：所有 Phase 0 inventory 中的 POST 路径同时挂在 `/api/v1/...` canonical 与 `/api/...` legacy alias，响应 status、header、JSON 字段和错误体与 Python parity。
- 新增 Rust auth mutation：`POST /api/login` 负责签发与 Python 互认的 `codoxear_auth` cookie；`POST /api/logout` 清除同名 cookie。
- 新增 Rust 状态文件写入：`cwd_groups.json`、`session_aliases.json`、`session_sidebar.json`、`session_files.json`、`session_queues.json`、`harness.json`、`voice_settings.json`、`push_subscriptions.json`、`uploads/<session_id>/<filename>` 等按 `docs/cutover/disk-contracts.md` 原 schema 与 atomic rename 规则写出。
- 新增 Rust broker write-command client：在 Phase 2 read-only `broker_client` 基础上增加 `send`、`keys`、`ui_response`、`shutdown` 等 server-to-broker mutation commands；不实现 broker 本体（Phase 4）。
- 新增 Rust web-owned session create 路径：`POST /api/sessions` 通过现有 Python broker / pi_broker CLI spawn session，等待 sidecar 后返回 Python parity spawn payload；Phase 4 才替换为 Rust broker。
- 新增 Rust queue / harness worker opt-in 实现：`CODOXEAR_ENABLE_QUEUE_SWEEP=1` 与 `CODOXEAR_ENABLE_HARNESS_SWEEP=1` 时 Rust server 启动对应 tokio worker；默认仍关闭。
- 修改 Python `SessionManager` 启动逻辑：当对应 `CODOXEAR_ENABLE_*` flag 为 truthy 时，Python 不启动同类 sweep/scan 线程，满足伞形 spec 的 “Both Rust and Python workers are never co-active”。
- 保留 voice/HLS/WebPush 深度迁移为 Phase 5：Phase 3 只实现 voice 设置、subscription mutation、listener heartbeat，以及对 test/debug 端点给出与 Python 兼容的边界；不实现 TTS/WebPush/HLS worker。
- **无 BREAKING 变更**：现有 Python backend、Python broker、前端 API 路径、cookie 名、磁盘文件名、runtime 目录均保持兼容；Rust 仍是可并行验证的 additive backend。

## Capabilities

### New Capabilities

<!-- 无。Phase 3 继续扩展伞形 change 已创建的 rust-backend-runtime。 -->

### Modified Capabilities

- `rust-backend-runtime`: 增加 Phase 3 写路径 parity、Rust queue/harness worker opt-in、Python worker flag handoff、POST contract parity 和共享磁盘写入规则。

## Impact

- **Affected code**：`backend-rs/src/routes.rs`、`backend-rs/src/broker_client.rs`、`backend-rs/src/session_loader.rs`、`backend-rs/src/runtime.rs`、新增 `backend-rs/src/handlers/{auth,write_sessions,queue,harness,cwd_groups,files_write,voice_write,hooks}.rs` 或等价拆分；新增 worker 模块（如 `queue_worker.rs`、`harness_worker.rs`）；修改 `backend-rs/src/main.rs` 启动 worker；少量修改 `codoxear/server.py` 的 `SessionManager.__init__` 让出 Python sweep；扩展 `tests/contract/` 与 `backend-rs/tests/`。
- **APIs**：迁移所有 POST / 写端点，包括 `/api/login`、`/api/logout`、`/api/sessions`、`/api/sessions/{id}/{send,enqueue,ui_response,heartbeat,queue/delete,queue/update,harness,interrupt,inject_file,inject_image,file/write,edit,rename,delete,takeover/open}`、`/api/cwd_groups/edit`、`/api/files/{read,inspect,blob}`、`/api/settings/voice`、`/api/notifications/subscription[/toggle]`、`/api/audio/listener`、`/api/hooks/notify`。
- **Dependencies**：可能新增 Rust crate：`nix` 或 `libc`（pid / signal / process-group helpers）、`base64` 已存在、`tokio::process`、`fs2` 或 `fcntl` advisory lock helper、`tempfile` dev-dep 扩展；不引入 WebPush/TTS 依赖。
- **Operations / rollback**：默认不切流、不启动 Rust workers；unset Rust server flags 或不运行 Rust binary 即回到 Python。若启用 Rust queue/harness worker，回滚步骤是先停 Rust worker flag、重启 Python server，由 Python 重新接管相同 JSON 文件。
