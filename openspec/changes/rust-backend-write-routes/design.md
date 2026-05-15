## Context

Phase 1 已落地 `backend-rs/` skeleton、HMAC cookie auth 和三条基础 GET；Phase 2 已实现全部 GET / readonly parity，并抽出了 `broker_client`、`session_loader`、`log_normalizer`、`git_context`、`voice_state` 等共享子系统。当前 Rust server 仍不能处理任何 POST，所以前端真正使用时仍依赖 Python `codoxear/server.py` 作为唯一写端。

Phase 3 的写端点会触及三类共享资源：

1. **broker socket mutation**：`send`、`keys`/interrupt、`ui_response`、`shutdown` 等命令会改变正在运行的 CLI session。
2. **runtime JSON 文件**：`cwd_groups.json`、`session_aliases.json`、`session_sidebar.json`、`session_files.json`、`session_queues.json`、`harness.json`、`voice_settings.json`、`push_subscriptions.json` 等必须继续被 Python fallback 读取。
3. **后台 sweep ownership**：Python 现状在 `SessionManager.__init__` 无条件启动 harness loop、queue loop、voice-push scan loop。Rust 若也启动 worker，就会出现同一文件双写。伞形 spec 已要求 “Both Rust and Python workers are never co-active”。

约束：Phase 3 不迁移 broker 本体（Phase 4），不迁移 voice/HLS/WebPush/TTS worker（Phase 5），不删除 Python backend（Phase 6）。所有 endpoint 仍必须 `/api/v1/*` + `/api/*` 双挂，前端不改。

## Goals / Non-Goals

**Goals:**

- 覆盖 Phase 0 inventory 中除 voice debug/HLS 深层能力外的 POST / 写路径 parity，使 Rust server 可完成前端常规交互：登录、创建 session、发送/排队 prompt、队列编辑、文件读写、sidebar rename/edit、harness 配置、Pi AskUser 回应、attachment staging、hooks no-op。
- 使用同一套磁盘 schema 和 atomic write helper 写出 Python 可读文件，严格遵循 `docs/cutover/disk-contracts.md`。
- 在 Rust 侧实现 queue/harness worker opt-in，并在 Python 侧实现按同名 env flag 让出，防止 `session_queues.json` / `harness.json` 双写。
- 扩展 contract parity tests：每个 POST endpoint 至少覆盖 happy path 与主要错误路径，且验证写入磁盘 round-trip（Rust 写 → Python GET/POST 后续读取、Python 写 → Rust GET/POST 后续读取）。
- 保持 additive：默认 Rust worker off；Python 仍可单独运行；回滚不需要修复文件。

**Non-Goals:**

- 不实现 Rust broker / PTY / sidecar writer（Phase 4）。`POST /api/sessions` 仍可 spawn Python broker / pi_broker。
- 不实现 voice delivery worker、WebPush 实际发送、OpenAI TTS、HLS segment 生产，`POST /api/notifications/test_push` 与 `POST /api/audio/test_announcement` 若无法做到 parity，应在 Phase 3 spec 中明确保留到 Phase 5 并提供可测试的 501/feature-disabled 行为，不能静默假成功。
- 不修改 frontend。
- 不引入新的 API shape 或新用户功能。
- 不清理 Python legacy surface。

## Decisions

### D1：POST handler 按 ownership wave 实现，而不是按文件顺序照搬 `do_POST`

**选择**：将 Phase 3 分成 Auth/CWD groups、session metadata JSON、broker command mutation、queue/harness、file writes/uploads、voice/subscription lightweight writes、session create/delete、hooks 八个 wave。

**原因**：`do_POST` 文件顺序混合了无状态 auth、磁盘写、broker write、进程 spawn、voice debug。按 ownership 分组能让每个 wave 的 tests 明确验证一个资源锁/文件 schema，不会把高风险 spawn 与简单 JSON 写混在一起。

**替代**：逐行 port `do_POST`。否决：容易遗漏共享 helper，且 review 难以判断双写风险。

### D2：集中式 atomic JSON writer + per-file advisory lock

**选择**：新增 `runtime::write_json_atomic(path, value, sort_keys, mode)` 与 `runtime::with_state_file_lock(path, f)`。所有会被 Python fallback 读写的 JSON 文件都先获取 `<path>.lock` advisory lock，再写 temp + fsync file + rename + best-effort fsync parent。key ordering 以 Python writer 为准：大多数 `sort_keys=True, indent=2, trailing newline`；sidecar/非本 phase 文件不新增写入。

**原因**：Rust 与 Python 在过渡期可能不同时间写同一个文件；即使通过 worker flag 避免同时 sweep，用户请求仍可能打到两个 server。advisory lock 不能约束旧 Python 当前未加锁的所有路径，但可以让 Rust 自身不会并发覆盖，并为后续给 Python 加锁留接口。

**替代**：只用 atomic rename，不加 lock。否决：同一 Rust process 内多请求并发写 queue/harness 时会 lost update。

### D3：broker write commands 仍通过现有 Python broker socket 协议

**选择**：扩展 `broker_client.rs`，新增 `broker_send`、`broker_keys`、`broker_ui_response`、`broker_shutdown`，协议与 `codoxear/broker.py` / `pi_broker.py` 当前 `_handle_conn` 一致：单 JSON line request，单 JSON line response，timeout 2–3s。

**原因**：Phase 4 才 port broker；Phase 3 server 只需要调用现有 broker socket，即可让 Rust POST `/send`、`/interrupt`、`/ui_response` 与 Python 行为一致。

**替代**：Rust 直接操作 PTY 或 Pi RPC。否决：这就是 Phase 4/更深的工作，会扩大范围。

### D4：`POST /api/sessions` 先 spawn Python broker，Phase 4 再替换 Rust broker

**选择**：Rust `session_create` 复刻 Python `spawn_web_session` 的请求解析、cwd/worktree/tmux 校验、env 构造和 sidecar wait，但子进程仍是 `python -m codoxear.broker` 或 `python -m codoxear.pi_broker`。如果 `CODOXEAR_RUST_BROKER_BIN` 已显式设置，可按伞形 Phase 4 的 env 约定走该 binary，但 Phase 3 不要求该 binary 可用。

**原因**：用户需要在 Rust server 上创建 web-owned session 才能完成写路径 parity；但 broker 本体迁移是独立高风险阶段。spawn Python broker 是最小可用方案。

**替代**：Phase 3 不实现 session create。否决：前端常规写路径不完整；inventory 明确把 `POST /api/sessions` 放入 Phase 3。

### D5：queue mutation 写本地 JSON，queue drain worker 单独 flag 控制

**选择**：`POST /enqueue`、`queue/delete`、`queue/update` 只改 `session_queues.json` 并返回 Python parity `queued/queue_len`；实际自动 drain 由 Rust `CODOXEAR_ENABLE_QUEUE_SWEEP` worker 处理，默认 off。`POST /send` 优先 broker `send`；broker unavailable 但 pid alive 时 fallback enqueue，与 Python `MANAGER.send` 一致。

**原因**：queue JSON 是持久化用户意图，必须先 port。drain worker 是后台副作用，必须由 env flag 单独管控以便回滚。

**替代**：enqueue 同时立即尝试 drain。否决：会改变 Python 的 grace/idle 语义。

### D6：harness POST 与 harness worker 分离

**选择**：`POST /harness` 只更新 `harness.json`；Rust `CODOXEAR_ENABLE_HARNESS_SWEEP` worker 复刻 Python `_harness_sweep` 的 cooldown、remaining、assistant-last-message、broker idle 检查和 send 行为。

**原因**：配置写入与自动注入是两个不同风险面。配置 POST 必须始终可用；自动注入必须可 flag 回滚。

### D7：Python worker handoff 以同名 env flag 在 `SessionManager.__init__` 做启动判断

**选择**：Python 新增小 helper `_env_flag_truthy(name)`；`CODOXEAR_ENABLE_HARNESS_SWEEP` truthy 时不 start `_harness_thr`，`CODOXEAR_ENABLE_QUEUE_SWEEP` truthy 时不 start `_queue_thr`，`CODOXEAR_ENABLE_VOICE_SCAN` truthy 时不 start `_voice_push_scan_thr`。Phase 3 不启用 `CODOXEAR_ENABLE_VOICE_WORKER`，但 helper 保留日志说明。

**原因**：这是伞形 spec 的硬要求，也是最小 Python 改动；不触碰 Python handler 语义。

**替代**：部署文档要求不要同时启动 Python。否决：人为约定无法由测试保证，且用户可能双开。

### D8：voice debug/test endpoints 不在 Phase 3 假实现

**选择**：轻量 voice writes（settings、subscription upsert/toggle、audio listener heartbeat）在 Phase 3 port；真正发送 WebPush / enqueue TTS announcement 的 debug endpoints 如果不能复用 worker，则返回明确 feature-disabled / 501，并在 endpoint inventory 保持 Phase 5 owner，或者由 spec 明确将它们延后。

**原因**：`send_test_push_notification` 依赖 VAPID/WebPush，`enqueue_test_announcement` 依赖 active listener + TTS/HLS queue；强行在 Phase 3 实现会把 Phase 5 拉进来。比假返回 `ok` 更安全。

**替代**：照搬 Python voice worker 大段逻辑。否决：违反 Phase 3 非目标。

### D9：contract tests 以“磁盘 round-trip + next GET parity”验证写路径

**选择**：每个写文件 endpoint 的测试不仅比 POST 响应，还要在 POST 后调用相关 GET（如 `/api/sessions/bootstrap`、`/api/sessions/{id}/queue`、`/api/sessions/{id}/harness`、`/api/files/read`）确认 Rust 写出的文件能被 Python 读取，反向也成立。

**原因**：POST 响应可能正确但磁盘 schema 错误；cutover 风险主要在 fallback 互读。

## Risks / Trade-offs

- [Risk] `POST /api/sessions` 复刻 Python spawn 逻辑很长且跨 tmux/worktree/Pi/Codex → Mitigation：先覆盖 non-tmux codex/pi happy path与错误路径；tmux/worktree 单独 task 和 contract test，必要时可标为 Phase 3.1 follow-up，但不能影响已有路径。
- [Risk] Rust 与 Python server 同时接受用户 POST 导致 alias/queue/harness lost update → Mitigation：Rust 内部 per-file lock + atomic read-modify-write；部署上同一端口只切一个 server；contract tests 覆盖并发 Rust 请求。
- [Risk] queue/harness worker 误判 idle 后重复发送 prompt → Mitigation：默认 off；worker tests 使用 stub broker + log fixture，先验证 cooldown/remaining/queue_len/assistant-last-message边界，再允许 production flag。
- [Risk] Pi AskUser fallback 逻辑复杂（live ui_response unsupported 时转 ESC/text） → Mitigation：按 `submit_ui_response` 原逻辑逐分支测试：live ok、unknown cmd + cancelled、unknown cmd + value、non-Pi 502。
- [Risk] file write path traversal/symlink 行为与 Phase 2 file read helper 不一致 → Mitigation：复用 Phase 2 `resolve_session_path` / `_resolve_under` 等价 helper，新增 symlink escape、version conflict、create conflict tests。
- [Risk] voice debug endpoint 延后可能让“全部 POST”听起来不完整 → Mitigation：proposal/tasks 明确列为 Phase 5 owner，不静默成功；交接时提醒 coding-agent 与 reviewer。

## Migration Plan

1. 新增 write helper 与 broker write-command client，先跑 unit tests。
2. 按 Auth/CWD groups → metadata JSON → queue/harness config → broker send/ui/interrupt → files write/upload → session create/delete → lightweight voice writes → hooks 的顺序实现 POST handlers，每个 wave 后跑对应 contract selector。
3. 实现 Rust queue/harness worker，但默认 off；用 stub broker + log fixtures 测试。
4. 修改 Python `SessionManager.__init__` worker handoff，并增加 pytest 验证 truthy flag 下线程不会启动/不会 touch 对应文件。
5. 更新 endpoint inventory、disk contract notes、contract README、umbrella tasks log。
6. 验证：Rust fmt/clippy/test、pytest contract POST parity、OpenSpec validate 当前 change 与伞形 change。

Rollback：默认无需回滚（Rust 不运行即无影响）。若启用了 Rust queue/harness worker，先 unset `CODOXEAR_ENABLE_QUEUE_SWEEP` / `CODOXEAR_ENABLE_HARNESS_SWEEP`，重启 Rust/Python server；Python 将重新启动对应 thread 并继续读取同一 JSON 文件。若 POST handler 产生错误 JSON，可停止 Rust server 切回 Python；因 schema 按 disk contracts 写出，Python 可继续读。

## Open Questions

- `POST /api/notifications/test_push` 与 `POST /api/audio/test_announcement` 在 Phase 3 是返回 501 feature-disabled，还是保留为 Python-only 到 Phase 5？建议：标记 Phase 5 owner，不在 Rust Phase 3 注册成功路径。
- `POST /api/sessions` tmux create path 是否必须在 Phase 3 DoD 覆盖真实 tmux？建议：至少 Linux 本地/CI 有 tmux 时覆盖；无 tmux 环境覆盖 400 `tmux is unavailable` parity。
- 是否给 Python JSON writers 同步加 advisory lock？建议本 phase 只加 worker handoff，避免大范围改 Python；若 contract 并发测试发现 lost update，再开单独 hardening change。
