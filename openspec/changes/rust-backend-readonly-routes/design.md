## Context

Phase 1（`rust-backend-skeleton`，commit `12917f3`）落地了 `backend-rs/` Rust crate、HMAC cookie auth、`/api/health` / `/api/me` / `/api/sessions/bootstrap` 三个端点，并把 contract harness 从 placeholder 切换到了真实双服务器对比。code-reviewer 给了 WARNING：一处 HIGH（200 路径 Content-Type 缺 `charset=utf-8`，与 Python `_json_response` 不一致），加四处 MEDIUM（`serde_json` 没启 `preserve_order`、`read_cwd_groups` 错误恢复偏离 Python、`normalize_cwd_group_key` 用 `fs::canonicalize` 与 Python `Path.resolve(strict=False)` 不等价、bootstrap populated-state contract test 缺位）。这些都属于"实现 byte-level parity 必须修的基线"，否则 Phase 2 的全量 GET 端点对比测试会因 header 不一致或 dict 顺序不一致而批量失败。

Phase 2 的目标是把 Rust server 推到"GET / readonly 端点全量 parity"。从 `docs/cutover/endpoint-inventory.md` 看，要覆盖 28 条 GET 路由（不含 `/api/audio/*` HLS — Phase 5）：5 条 voice/notification/metrics snapshot、2 条 session list 类、9 条 per-session metadata、3 条 git、6 条 file viewer、3 条 message log。其中 11 条是"huapeixuan-only"，ref `san-tian/codoxear@san-tian-dev` 的 `backend-rs/` 没有等价实现，必须从 Python 源代码 1:1 移植。

所有 readonly 路径会暴露出 5 个跨端点共享的子系统：(1) sock 目录扫描 + sidecar 解析、(2) read-only broker IPC 客户端（`{"cmd": "state"|"ui_state"|"commands"}`）、(3) `rollout_log.py` + `pi_log.py` 日志归一化（771 + 309 行 Python，纯函数）、(4) `git_context.py` PR badge（334 行 Python，含 per-cwd lock + TTL cache）、(5) `voice_settings.json` / `push_subscriptions.json` / `voice_delivery_ledger.json` 三文件的 read-only snapshot。这 5 个子系统在 Phase 2 落地后，Phase 3（POST mutation）只需要在第 3、4 个之外新增 write 路径，Phase 4（broker port）替换 IPC 客户端为 server-side broker，Phase 5（voice push worker）替换 voice snapshot 为 active worker。

约束：
- `backend-rs/` 仍是单 crate，但单文件不能超过 800 行硬上限（CI 已在 Phase 1 钉死 `runtime.rs < 800`）。Phase 2 必须把 `runtime.rs` 拆成 `session_loader` / `log_normalizer` / `git_context` / `voice_state` 等模块，否则会立刻越线。
- Rust server 仍**不**改 Python `codoxear/server.py` 业务代码；contract test 要求两边读同一份 `~/.local/share/codoxear/`，cookie 通过共享 `hmac_secret` 互通。
- Phase 1 contract test 用 `assert_json_equivalent` 只比 dict-eq；Phase 2 要把 Content-Type 与 Cache-Control 等 header 严格相等，并尽量比 raw bytes（在嵌套 dict 启用 `preserve_order` 后）。
- 端点必须同时挂 `/api/v1/*` canonical 与 `/api/*` legacy alias，前端不动。

## Goals / Non-Goals

**Goals:**
- 闭环 Phase 1 review 退回项（Content-Type charset、`serde_json::preserve_order`、`cwd_groups` 错误恢复、`normalize_cwd_group_key` 渐进 canonical、bootstrap populated-state 测试）。
- 把 28 条 readonly GET 端点全部由 Rust server 实现，与 Python parity；输出 dict-eq 必过、header `Content-Type` 严格相等；body byte-eq 在嵌套 dict 顺序对齐后是 stretch goal。
- 落地 5 个共享子系统：read-only broker IPC、session loader、log normalizer、git context、voice state snapshot。
- 把 huapeixuan-only 的 `git_context.py` PR badge、message live/tail/legacy alias、global file viewer、Pi commands / ui_response、`/repo`、`/workspace`、`/details`、`/ui_state`、`/takeover` 全部移植到 Rust。
- 严守模块大小硬上限（800 行/文件）：`runtime.rs` 必须拆分。
- 保持完全 additive：Python backend 可继续作为权威实现，Rust binary 任何时候关掉都能回滚。

**Non-Goals:**
- 任何 POST 写路径 / mutation handler（Phase 3）。
- broker spawn、PTY、tmux、socks sidecar 的 server 侧写入（Phase 4）。
- HLS / WebPush / TTS worker、`/api/audio/live.m3u8`、`/api/audio/segments/*`（Phase 5）。
- Python `codoxear/*.py` 业务模块改动。
- 切流 / 默认部署 Rust binary 替换 Python（Phase 6）。
- 重构 ref Rust 实现的非 huapeixuan 端点（如 ref-only 的 `/api/v1/cwd_suggestions` / `/messages/send` / `/events/stream`）—— 不在 huapeixuan baseline，不需要。

## Decisions

### D-CT (Phase 1 HIGH 修复): 200 路径统一走 `json_response` helper

**Decision**: Phase 2 第一件事是把 `routes.rs` 里所有 `axum::Json` 直接 IntoResponse 的 200 路径改用 `json_response(status, value) -> Response`，让 `Content-Type: application/json; charset=utf-8` 是不变量；同时在 `backend-rs/tests/` 加 helper test 钉死任何新增的 200 handler。

**Why**: `axum::Json` 默认产 `application/json` 不带 charset；Python `_json_response` 写的是 `application/json; charset=utf-8`。这是 byte-level parity 的基础门槛，Phase 1 reviewer 已显式要求；如果不修，Phase 2 全部 25+ 个 200 端点都会被 Content-Type diff 卡住。

**Alternatives**:
- 在每个 handler 单独构造 `Response::builder().header(...)` —— 重复代码、容易漏；reviewer 已经明确 HIGH。
- 用 `axum_extra` / `headers` 中间件覆盖默认 Content-Type —— 影响所有 axum::Json 使用方，副作用面大。

### D-ORDER (Phase 1 MEDIUM 修复): `serde_json` 启用 `preserve_order`

**Decision**: 在 `backend-rs/Cargo.toml` 给 `serde_json` 加 `features = ["preserve_order"]`；改用 `IndexMap`-backed `serde_json::Map` 序列化嵌套对象，并在构造 `backends.codex` / `backends.pi` / 其他动态 dict 时用 **显式 insert 顺序** 与 Python dict 插入序一致。

**Why**: Python dict 是插入序；Phase 2 contract test 想做 byte-eq 必须让嵌套 dict 顺序一致。`preserve_order` 不引入新 transitive crate（依赖图已有 `indexmap`），代价极小。Phase 2 一旦不做这件事，body byte-eq 就不可能；只能停在 dict-eq 层级，丢失大量 regression 信号。

**Alternatives**:
- 在每个 handler 手工构造有序 `Vec<(String, Value)>` —— 重复代码；轻易出错。
- 接受 dict-eq 不做 byte-eq —— 失掉 Phase 1 reviewer 显式要求的"为 byte-level diff 留路"。

### D-CWDG (Phase 1 MEDIUM 修复): `read_cwd_groups` 错误恢复语义对齐 Python

**Decision**: 把 `runtime::read_cwd_groups` 的"`JsonDecodeError` / 顶层非 Object → `Err(...)`"路径改为"`tracing::warn!` + 返回空 `Map`"，与 `codoxear/server.py:5052-5096` 的 `_load_cwd_groups` 一致。

**Why**: Python `_load_cwd_groups` 在 `JSONDecodeError/TypeError/ValueError` 时 `LOG.warning(...)` 并返回 `{}`；server 仍返回 200 + 空 dict。Rust 当前实现在同种损坏场景下返 500，与 Python 行为偏离。Phase 1 contract test 用空 HOME 不触发；但 Phase 2 引入 populated-state 与 PI / repo 等真实 fixture 后会暴露。

**Alternatives**:
- 在 spec 中显式声明此条偏离并加单测 —— 但意味着真实部署上 Rust 5xx 会被前端误以为后端挂掉。
- 让 Python 也走 5xx —— 改动 Python 业务代码，违反 Phase 2 边界。

### D-NORM (Phase 1 MEDIUM 修复): `normalize_cwd_group_key` 渐进 canonical

**Decision**: 把 `fs::canonicalize(...)` 替换为渐进式 canonical：`Path::expanduser` → 从最长前缀往短试 `canonicalize`，把已存在的部分解析、剩余段保留。等价于 Python `Path(trimmed).expanduser().resolve(strict=False)`。

**Why**: `fs::canonicalize` 在任一段不存在时整体 `Err`；当前代码 `unwrap_or(expanded)` 退回未解析路径。对 `/foo/bar` 中 `/foo` 是 symlink、`bar` 不存在时，Python 解析 symlink，Rust 不解析。Phase 3 引入 `cwd_groups/edit` 后会写不一致 key，造成 reconcile 失配。

**Alternatives**:
- 用 `path-clean` crate 做纯字符串清理 —— 不解析 symlink，与 Python 不等价。
- Phase 3 才修 —— 但 Phase 2 sidebar / cwd_groups round-trip parity test 已经会暴露。

### D-IPC: read-only Unix socket broker 客户端

**Decision**: 新增 `backend-rs/src/broker_client.rs`，提供同步 `broker_request(sock_path, request, timeout)` 与三个上层包装 `broker_state` / `broker_ui_state` / `broker_commands`。协议：连接 → 写 `<json> + "\n"` → 读一行 JSON → 关闭。timeout 默认 1.5s（与 Python `get_state` 一致），可由调用方覆盖。socket 不存在或连接拒绝时返回 `Err`，调用方降级到 sidecar 上的最近值（与 Python `get_state` 在 `_pid_alive` 判断后返回 degraded state 一致）。

**Why**: 这是 Phase 2 多个端点的依赖（diagnostics / queue / ui_state / commands）。ref `runtime.rs:7195-7290` 已有等价实现可直接 port，user5510c0eb 协议与 ref 一致（同一 Python broker 写入）。

**Alternatives**:
- 异步 IPC（`tokio::net::UnixStream`）—— 1.5s timeout 下同步阻塞简化得多，且每请求只写一行读一行；改异步增加复杂度无收益。
- 在 server 侧 cache broker state —— 与 Python `get_state` 行为不一致（每 request live 读），且 Phase 2 不应引入 cache 复杂度。

### D-SPLIT: `runtime.rs` 拆分到多 module 防超 800 行

**Decision**: 把 Phase 1 的 `runtime.rs`（799 行）拆为：
- `runtime.rs`（保留 `RuntimeConfig` / `load_or_create_hmac_secret` / 公共 helper），≤ 200 行
- `session_loader.rs`（sock 扫描 + sidecar + 状态文件 merge），≤ 600 行
- `log_normalizer.rs`（rollout/pi log 归一化），可拆 `log_normalizer/mod.rs` + `log_normalizer/codex.rs` + `log_normalizer/pi.rs`，每个 ≤ 600 行
- `git_context.rs`（PR badge），≤ 400 行
- `voice_state.rs`（voice/notification snapshot），≤ 400 行
- `handlers/`（按 wave 分文件：`voice.rs`, `sessions_list.rs`, `session_meta.rs`, `git.rs`, `files.rs`, `messages.rs`, `metrics.rs`），每个 ≤ 400 行

**Why**: CI 钉死的 800 行硬上限不是建议；Phase 2 增加 28 个 handler + 5 个子系统，强行塞进单个 `runtime.rs` 必然超线。模块拆分必须在写 handler 前完成，否则 PR 一次性超限难以收尾。

**Alternatives**:
- 不拆，开 800 行硬上限例外 —— Phase 1 spec scenario 已显式要求 `wc -l < 800`。
- 拆得更细（每个 handler 一个文件）—— 过度碎片化，handler 之间共享 helper 时 import noise 大。

### D-LOG-PORT: log normalizer 用 1:1 算法等价方式 port

**Decision**: `log_normalizer.rs` 的事件 / token / busy / requests 抽取算法与 Python `rollout_log.py` / `pi_log.py` 1:1 等价（行号对齐）：相同字段名、相同短路条件、相同 fallback 顺序。Rust 端同时存在 happy / edge / error 各分支单测，每条对应 Python `tests/test_rollout_log.py` 已有用例（如有）。

**Why**: 这两个文件是"事件归一化"语义的事实标准；任何"我重新设计一下让它更 Rust-y"都意味着 dict-eq 不再过。算法结构不复杂（线性扫描 + 字段抽取），1:1 移植是最低风险路径。

**Alternatives**:
- 在 Rust 重新设计事件模型 —— 不可避免破坏 dict-eq，违反 Phase 2 目标。
- 让 Rust 调用 Python（PyO3 / 子进程）—— 引入 Python runtime 依赖，违反 cutover 目标。

### D-GH: `git_context::pr_summary` 用 `gh pr view --json` 子进程

**Decision**: `git_context.rs::resolve_repo_context(cwd, refresh)` 与 Python `git_context.py:resolve_repo_context` 一一对应：先 `git rev-parse --abbrev-ref HEAD`（cached 30s），再 `gh pr view --json number,title,state,url,isDraft,baseRefName,headRefName`（cached 120s），`gh auth status` 错误时 `availability="no-gh"`、找不到 PR 时 `"no-pr"`、非 git tree `"not-a-repo"`、子进程超时 / IO 错误 `"error"`。timeouts 与 Python 一致：branch 2.0s、PR 4.0s、`gh auth status` cache 300s。锁粒度 per-cwd（用 `parking_lot::Mutex` + `HashMap<PathBuf, Mutex<...>>`）。

**Why**: ref Rust 没有这一层（`san-tian/codoxear` 只做 `current_git_branch`）。前端 PR badge / `/repo` 详情都依赖这条路径；Phase 2 不port 就掉前端功能。

**Alternatives**:
- 让 Rust 调用 Python `python -m codoxear.git_context` —— 引入 Python runtime 依赖。
- 用 `octocrab` / `reqwest` 直接调 GitHub API —— 需要管理 GitHub token，绕过用户已有的 `gh` 认证；与 Python 行为不一致。

### D-MSG: `/messages` / `/tail` / `/live` 保留 huapeixuan legacy 形态

**Decision**: Rust 实现 `/api/sessions/{id}/messages?offset=&limit=&before=&init=` / `/tail` / `/live` 与 Python 完全 parity；**不**实现 ref 的 `/messages/{tail,history,live}` split 形态（前端不需要）。

**Why**: 前端 `web/` 还在用 legacy 形态；Phase 2 切到 split 会破坏前端。Phase 6 cleanup 时再决定是否引入 split 作为 v2 alias。

**Alternatives**:
- 同时挂两套 —— 路由表噪音大，没收益。
- 只挂 split —— 破坏前端。

### D-CT-TEST: contract test header 严格相等 + body byte-eq stretch goal

**Decision**: `tests/contract/test_endpoint_parity.py` 的 readonly 端点用例统一加 `assert response_py.headers["Content-Type"] == response_rs.headers["Content-Type"]`、`Cache-Control`（如 Python 设置了）。body 比对策略：(a) 默认用 `assert_json_equivalent`（dict-eq 经过 scrub）；(b) 启用 `serde_json::preserve_order` 后，对 deterministic 路径（如 bootstrap、settings/voice）追加 `assert response_py.content == response_rs.content` byte-eq 断言。Phase 2 不要求所有端点 byte-eq —— 比如 `/api/sessions` 含 `app_version` / 时间戳浮点等，只能 dict-eq。

**Why**: 显式分级让 reviewer 与 coding-agent 都知道哪条端点必须 byte-eq、哪条只需 dict-eq。Phase 1 contract test 只做 dict-eq；Phase 2 加上 header strict eq 就能在不破坏既有用例的前提下抓回 Phase 1 reviewer 指出的 Content-Type bug，并防止退化。

**Alternatives**:
- 全部 byte-eq —— 不现实（`app_version`、`updated_ts` 等永远不可能 byte-eq）。
- 不强制 header eq —— 失掉 Phase 1 review #1 的修复防退化保障。

### D-FIX: cwd_groups round-trip parity 测试在 Phase 2 落地（不拖到 Phase 3）

**Decision**: 即使 `POST /api/cwd_groups/edit` 是 Phase 3，Phase 2 也必须在 contract test 里跑"预先把 `cwd_groups.json` 写入磁盘 → Python read parity → Rust read parity → assert dict-eq"的 read-only round-trip。这覆盖 Phase 1 review #4 的"populated-state parity"要求。

**Why**: 写路径在 Phase 3，读路径在 Phase 2；Phase 2 read-only 已经能验证 `read_cwd_groups` 在 populated state 下的行为。等到 Phase 3 一并测意味着 Phase 2 reviewer 没法给"populated parity"画勾。

### D-VOICE-SNAPSHOT: voice / notification 读路径走文件 snapshot，**不**唤起 worker

**Decision**: Phase 2 的 `/api/settings/voice`（GET）、`/api/notifications/{subscription,message,feed}`（GET）实现走 `voice_state.rs` 直接读 `voice_settings.json` / `push_subscriptions.json` / `voice_delivery_ledger.json`，不实例化 Phase 5 才有的 `VoicePushCoordinator`。**不**实现 `/api/audio/live.m3u8` 与 `/api/audio/segments/*`（这两个需要 worker active produce HLS bytes，是 Phase 5）。

**Why**: snapshot 路径是纯文件读，与 worker 解耦；ref 已经这么做（`load_voice_settings_response` 仅读文件）。HLS 路径必须有活的 worker 在写 segments，无法独立 read-only 实现。

### D-LIST: `/api/sessions` 用 read-only IPC + 缓存 sidebar / queue / harness / hidden

**Decision**: `/api/sessions` 在 Rust 实现里对每个发现的 session：
1. 先 `_pid_alive` 简单 check（Python `_prune_dead_sessions` 等价）。
2. 调用 `broker_state` 获取 live `busy/queue_len/token`，超时 1.5s；失败时退回 sidecar 最近值（与 Python `get_state` 在 broker 不可用时返 degraded 一致）。
3. 从 `session_sidebar.json` 读 `priority_offset/snooze_until/dependency_session_id`。
4. 从 `git_context.rs` 读 `git_branch/pr_summary`（per-cwd cache，不阻塞）。
5. 从 `log_normalizer.rs` 读 `token snapshot` / `idle` / `todo_snapshot`。
6. 按 Python 排序规则（priority desc, updated desc, start desc, session_id asc）排序 + group。

**Why**: `/api/sessions` 是流量最大的 GET 端点，每秒可能调好几次；必须保证 broker 响应慢时不放大延迟。Python 现状是 broker timeout 退回 degraded，Rust 必须同步。

**Risk**: `/api/sessions` 实现复杂度极高；任何字段遗漏都被前端立刻发现。Mitigation：tasks §4 强制 `_session_list_visible_grouped_rows` 与 `_session_recent_payload` 两个 helper 单元测试 + cross-server diff 测试。

## Risks / Trade-offs

- [Risk] **`/api/sessions` 字段集庞大且部分字段动态生成**（`final_priority` / `time_priority` / `git_branch` / `pr_summary` / `todo_snapshot`）→ Mitigation：spec scenario 显式列出 14 项必须 parity 的字段；contract test 用真实 fixtures（多 backend、多 cwd、有 PR / 无 PR、blocked / snoozed）跑 cross-server diff。
- [Risk] **`rollout_log.py` 与 `pi_log.py` port 算法漂移** → Mitigation：每个公开函数 (`messages_from_codex_log` 等) 必须有 1+ Python parity test，用 `tests/fixtures/rollout/*.jsonl` 真实 log 行做端到端 dict-eq。
- [Risk] **`gh pr view` 在 CI 上跨平台行为差异**（macOS runner 没装 `gh`、token 不存在）→ Mitigation：CI workflow 安装 `gh` 但**不**配置 token；确保 `availability="no-gh"` 路径被覆盖；contract test 用 mock cwd 避免真实网络。
- [Risk] **broker socket 不存在 / 连接拒绝时降级语义** → Mitigation：`broker_client.rs` 测试覆盖 `connect refused`、`timeout`、`empty response`、`malformed JSON` 四种 error path；调用方降级测试用 `nc -lU` 模拟死 broker。
- [Risk] **800 行硬上限二次违反**：Phase 1 `runtime.rs` 已经 799；Phase 2 加 5 个子系统 + 7 个 handler 文件 → Mitigation：第一个 task 就是模块拆分（D-SPLIT），所有 handler PR 在拆分基础上写。CI 在 Phase 1 已有 `wc -l backend-rs/src/runtime.rs` gate；Phase 2 把 gate 扩展到所有 `*.rs`。
- [Risk] **byte-eq 测试在 `app_version` / 时间戳 / fpu 浮点上失败** → Mitigation：D-CT-TEST 显式分级；`assert_json_equivalent` scrub 函数继续覆盖动态字段，byte-eq 仅用于 deterministic 路径。
- [Risk] **`preserve_order` 启用后 ref 已有 dict-eq 测试失败**（如果 ref 实现的 codex/pi 子 dict 顺序与 Python 不同）→ Mitigation：tasks §0.2 在改 `Cargo.toml` 后立即跑 `cargo test --release` 与 `pytest tests/contract -k parity`，把回归挡在子任务级别。
- [Trade-off] **path-clean 不解析 symlink**（D-NORM）→ 选择渐进 canonical 路径，正确但实现稍复杂；保留 `path-clean` 作 fallback。
- [Trade-off] **broker IPC 同步阻塞** vs **异步**（D-IPC）→ 选同步保持简单；Phase 4 broker 实现时 server-side IPC 必须重新评估。
- [Open question] `/api/sessions/{id}/takeover` 与 `/api/sessions/{id}/workspace` 在前端的实际形态：用户是否在生产里依赖每个字段？coding-agent 必须用前端 `web/` 调用方做反向 grep，确认字段集；不能盲信 Python `_session_takeover_payload` 在 server.py 的字段集（可能有 dead code）。
- [Open question] `git_context.rs` 是否需要支持 GitHub Enterprise（`gh` 已认证非 github.com host）？Python 行为：`gh` 认证哪里就调哪里；Rust 应该 1:1 沿用，不引入 host 校验。
- [Open question] Phase 2 是否引入 `mime_guess` 让 `file/blob` 与 `file/read` 的 `content_type` 与 Python `mimetypes` 一致？Python `mimetypes` 在 macOS / Linux / WSL 上结果略有差异（`/etc/mime.types` 优先级）；Rust 应优先 hardcode "正确"映射并允许 fallback。这条留给 coding-agent 在 wave E 决定，但必须在 spec scenario 里写清。

## Migration Plan

Phase 2 不切流，是 additive：
1. 修 Phase 1 follow-up（D-CT, D-ORDER, D-CWDG, D-NORM）+ 跑 `pytest tests/contract -k parity` 确保 Phase 1 retest 全绿。
2. 拆 `runtime.rs`（D-SPLIT），跑全套 Rust + contract test，确保拆分零 regression。
3. 落地 5 个子系统 helper（broker_client / session_loader / log_normalizer / git_context / voice_state），每个 helper 自带单测。
4. 按 wave A → F 顺序加 handler；每个 wave 落地后 contract test 必须全绿才进下一 wave。
5. CI workflow 扩展 macos `gh` 安装、selector `-k readonly`、`wc -l` gate。
6. `docs/cutover/endpoint-inventory.md` 标记每条端点 "implemented by `rust-backend-readonly-routes`"。
7. DoD（tasks §7）9 步全绿 → 交付。

回滚：`git revert` 本 phase commit；Phase 1 follow-up 修复属于兼容性提升，回滚不会破坏 Phase 1 既有测试。

## Open Questions

- 已在 Risks / Trade-offs 列出三条 open question（前端字段反向 grep、GitHub Enterprise、`mime_guess`）；coding-agent 在对应 wave 启动前先回答这三条，再进入 implementation。
