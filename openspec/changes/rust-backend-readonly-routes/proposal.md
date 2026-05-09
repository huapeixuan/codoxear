## Why

Phase 1 (`rust-backend-skeleton`) 落地了 `backend-rs/` Rust crate、HMAC cookie auth、以及 3 个无副作用端点（`/api/health`、`/api/me`、`/api/sessions/bootstrap`）。Phase 2 的目标是把"骨架 + 三端点"推进到"全部 GET / 只读端点 parity"，让前端可以**只用 Rust server**完成所有不写状态的请求路径，而 Python `codoxear/server.py` 仍然作为权威实现并行运行用于回滚与对比验证。

之所以单独切一个 phase，是因为只读路径已经覆盖了全部"通用基础设施"：sock 目录扫描 + sidecar 解析、`session_aliases.json` / `session_queues.json` / `harness.json` / `session_sidebar.json` / `session_files.json` / `hidden_sessions.json` 等 JSON 状态文件读取、broker unix-socket 客户端（read-only `state` / `ui_state` / `commands` 命令）、`rollout_log.py` + `pi_log.py` 日志归一化、`git_context.py` PR badge、`gh pr view` 子进程编排、HLS 之外的 voice/notification 文件 snapshot 读取。所有这些设施一旦在 Phase 2 落地，Phase 3（POST 写）就只需要在它们之上加 mutation handler，Phase 4（broker port）和 Phase 5（voice push worker）也基于同一组 model / runtime helper。

Phase 1 review 还留下了一处 HIGH（`/api/me`、`/api/sessions/bootstrap` Content-Type 缺 `charset=utf-8`，与 Python 不一致）和若干 MEDIUM（`serde_json` 没启 `preserve_order`、`read_cwd_groups` 错误恢复语义偏离、`normalize_cwd_group_key` 渐进 canonical、bootstrap populated-state contract test 缺位）。这些都属于"实现 readonly parity 必须修的基线"，不能拖到 Phase 3，否则 Phase 2 的 byte-level 对比测试无法启用。

## What Changes

> 本 change 是 Phase 2，**只**实现 GET / readonly 端点 parity；POST 写路径属于 Phase 3，broker port 属于 Phase 4，voice/HLS/WebPush worker 属于 Phase 5。

### 0. Phase 1 follow-up baseline（必须先修，作为 Phase 2 的实现基线）

- **修改** `backend-rs/src/routes.rs`：把 `me()` 与 `sessions_bootstrap()` 改用统一 `json_response(...)` helper，让所有 200 路径输出 `Content-Type: application/json; charset=utf-8`，与 Python `_json_response` byte-level 一致；同时新增 helper test，确保后续 phase 不再出现裸 `axum::Json` 200 路径。
- **修改** `backend-rs/Cargo.toml`：给 `serde_json` 启用 `preserve_order` feature（依赖图里已有 `indexmap`），并把 bootstrap 嵌套 `Map<String, Value>`（`backends.codex` / `backends.pi` 子 dict 等）改用显式 insert 顺序，确保与 Python 的 dict 插入序一致，为 Phase 2+ 的 byte-level diff 留路。
- **修改** `backend-rs/src/runtime.rs::read_cwd_groups`：把 parse 失败 / 顶层非 Object 的路径从"`Err(...)` → handler 500"改为"`tracing::warn!` + 返回空 Map"，与 Python `_load_cwd_groups`（`codoxear/server.py:5052-5096`）的恢复语义一致。
- **修改** `backend-rs/src/runtime.rs::normalize_cwd_group_key`：把 `fs::canonicalize(...)` 替换为渐进式 canonical（解析所有存在的父级、保留剩余段），等价于 Python `Path(...).expanduser().resolve(strict=False)`，避免 Phase 3 `cwd_groups/edit` reconcile 失配。
- **新增** contract harness `tests/contract/test_endpoint_parity.py::test_sessions_bootstrap_parity` 的 populated-state 用例：(b) 用预置 `recent_cwds.json` 比对排序与截断；(c) `cwd_groups.json` round-trip parity；(d) `tmux_available` 反映宿主 `which tmux`；保持 (a) empty parity 与 (e) `/api/v1/bootstrap` 不存在两条已实现的 scenario。

### 1. 共享基础设施（infrastructure）

- **新增** `backend-rs/src/broker_client.rs`：read-only Unix socket JSON-line 客户端，提供 `broker_state(sock_path)`、`broker_ui_state(sock_path)`、`broker_commands(sock_path)`，行为与 Python `SessionManager._sock_call`（`codoxear/server.py:7568-7587`）等价：超时默认 1.5s、协议 `<json>\n` → 单行 JSON 响应；socket 不存在 / 连接拒绝时返回 `Err`，由调用方决定如何降级。**不**写任何命令到 broker（Phase 4 才加）。
- **新增** `backend-rs/src/session_loader.rs`：把 ref `runtime::load_sessions_response`（参考 `backend-rs/src/runtime.rs:277-348`）按 huapeixuan 字段集重写——读 `socks/*.{sock,json}`、merge `session_aliases.json`、`session_queues.json`、`harness.json`、`session_sidebar.json`、`session_files.json`、`hidden_sessions.json`，输出 `Vec<SessionRow>`。每行包含 frontend 需要的全部字段（`session_id, agent_backend, backend, owner, transport, cwd, log_path, start_ts, updated_ts, broker_pid, codex_pid, busy, queue_len, harness_enabled, alias, files, priority_offset, snooze_until, dependency_session_id, final_priority, base_priority, time_priority, git_branch, pr_summary, todo_snapshot` 等）。`busy` / `queue_len` 调用 `broker_state` 得到 live 值，broker socket 不可用时降级为 sidecar 上的最近值。
- **新增** `backend-rs/src/log_normalizer.rs`：port `codoxear/rollout_log.py`（771 行）与 `codoxear/pi_log.py`（309 行）核心函数，覆盖 `_messages_from_codex_log`、`_messages_from_pi_log`、`_idle_from_log`、`_token_snapshot_from_log`、`_run_settings_from_log`、`_todo_snapshot_payload_for_session` 所需的事件 / token / busy / requests 归一化。Phase 2 只需要 read-only 路径，序列化 / write-back / live tail 的 advisory lock 不需要。
- **新增** `backend-rs/src/git_context.rs`：port `codoxear/git_context.py`（334 行）：`resolve_repo_context(cwd, refresh)`，包含 per-cwd TTL cache、per-cwd lock（用 `parking_lot::Mutex` 或 `tokio::sync::Mutex`）、`gh pr view --json` 子进程调用与降级（`no-gh` / `no-pr` / `not-a-repo` / `error`）；timeouts 与 Python 同步：branch 2.0s、PR 4.0s、`gh auth status` cache 300s。
- **新增** `backend-rs/src/voice_state.rs`：read-only snapshot 读取，覆盖 `voice_settings.json`、`push_subscriptions.json`、`voice_delivery_ledger.json` 三个文件以及 `notification_message` / `notification_feed` 派生逻辑。**不**启动 HLS / WebPush / TTS worker（Phase 5）；只把当前文件状态序列化为 JSON snapshot 返回。

### 2. GET 端点 parity（依据 `docs/cutover/endpoint-inventory.md`）

按"实现复杂度 + 依赖关系"组成 6 个 wave；每 wave 内的端点共享 helper，wave 之间存在依赖（后面的 wave 用前面 wave 的 helper）。

**Wave A — Voice / notification snapshot + metrics**（依赖：`voice_state.rs`、`json_response` 修复）
- `GET /api/settings/voice` + `/api/v1/settings/voice` — `_voice_push.settings_snapshot()` 等价。
- `GET /api/notifications/subscription` + `/api/v1/notifications/subscription` — `subscriptions_snapshot()` 等价。
- `GET /api/notifications/message?message_id=` + `/api/v1/...` — 校验 `message_id` 非空（400）、不存在（404）、命中（200）。
- `GET /api/notifications/feed?since=` + `/api/v1/...` — `since` 解析失败（400）、`items` 列表（200）。
- `GET /api/metrics` + `/api/v1/metrics` — `_metrics_snapshot()`，huapeixuan-only。

**Wave B — Session list & resume candidates**（依赖：`session_loader.rs`、`git_context.rs`、`broker_client.rs`）
- `GET /api/sessions?view=&group_key=&offset=&limit=&group_offset=&group_limit=` + `/api/v1/...` — full parity of `_session_list_payload` / `_session_recent_payload`，包括 `view ∈ {directories, recent}` 校验、`group_limit` 上限 20 / `limit` 上限 50、`remaining_by_group`、`omitted_group_count`。
- `GET /api/session_resume_candidates?cwd=&backend=&agent_backend=` + `/api/v1/...` — port `_session_resume_candidates_for_cwd`：返回 `ok`、`exists`、`is_git_repo`、`backend`、`sessions`，包含同 cwd 的可恢复 session 描述。

**Wave C — Per-session metadata（broker IPC + 文件读）**
- `GET /api/sessions/{id}/diagnostics` + `/api/v1/...` — full parity of `codoxear/server.py:9100-9231`：调用 `broker_state` 获取 `busy/queue_len/token`；从 sidebar 读 `priority_offset/snooze_until/dependency_session_id`；从 git_context 读 `git_branch/pr_summary`；用 `log_normalizer` 读 token & idle；输出全部 30+ 字段。
- `GET /api/sessions/{id}/queue` + `/api/v1/...` — 直接读 `session_queues.json` 的本地 list（与 Python `_queue_list_local` 一致），不调用 broker。
- `GET /api/sessions/{id}/harness` + `/api/v1/...` — 读 `harness.json` 该 session 的 `enabled/cooldown_minutes/remaining_injections/request`，缺失字段用 Python defaults。
- `GET /api/sessions/{id}/workspace` + `/api/v1/...` — port `_session_workspace_payload`（huapeixuan-only）。
- `GET /api/sessions/{id}/details` + `/api/v1/...` — port `_session_details_payload`（huapeixuan-only）。
- `GET /api/sessions/{id}/ui_state` + `/api/v1/...` — Pi-only：`broker_ui_state` 调用 → 返回；非 Pi 直接读 sidebar `priority_offset/snooze_until/dependency_session_id`（huapeixuan-only）。
- `GET /api/sessions/{id}/commands` + `/api/v1/...` — Pi command snapshot：`broker_commands` 调用（huapeixuan-only）。
- `GET /api/sessions/{id}/takeover` + `/api/v1/...` — terminal takeover descriptor，port `_session_takeover_payload`（huapeixuan-only）。
- `GET /api/sessions/{id}/repo` + `/api/v1/...` — `git_context::resolve_repo_context(cwd, refresh=qs)`，返回 `to_detail_dict()`（huapeixuan-only）。

**Wave D — Git endpoints**（依赖：`run_git_capture`，可复用 ref `backend-rs/src/runtime.rs:4256` 的实现）
- `GET /api/sessions/{id}/git/changed_files` + `/api/v1/...` — `git diff --name-only` + `--cached` + 各自 `--numstat`，合并 + 截断到 `GIT_CHANGED_FILES_MAX`，输出 `ok/cwd/files/entries/staged/unstaged`；`409` for non-repo。
- `GET /api/sessions/{id}/git/diff?path=&staged=` + `/api/v1/...` — `git diff` / `git diff --cached`；输出 `ok/cwd/path/staged/diff`。
- `GET /api/sessions/{id}/git/file_versions?path=` + `/api/v1/...` — 输出 `ok/cwd/path/abs_path/base_exists/base_text/current_exists/current_text/current_size`。

**Wave E — File viewer**（依赖：`fs` + `mime_guess` + ref `runtime::load_file_read_response` 等价）
- `GET /api/sessions/{id}/file/read?path=` + `/api/v1/...` — 输出 `ok/kind/path/rel/size/text?/editable?/version?/content_type?/image_url?/pdf_url?/download_only?/reason?/viewer_max_bytes?`。
- `GET /api/sessions/{id}/file/search?q=&limit=` + `/api/v1/...` — 输出 `ok/query/cwd/mode/matches/scanned/truncated`。
- `GET /api/sessions/{id}/file/list?path=` + `/api/v1/...` — 输出 `ok/cwd/path/entries`（huapeixuan-only）。
- `GET /api/sessions/{id}/file/blob?path=` + `/api/v1/...` — 内联 image/pdf 字节，错误 `error` JSON。
- `GET /api/sessions/{id}/file/download?path=` + `/api/v1/...` — `Content-Disposition: attachment` 字节（huapeixuan-only）。
- `GET /api/files/blob?path=` + `/api/v1/files/blob` — global 全局文件 viewer 内联字节（huapeixuan-only）。

**Wave F — Message log readers**（依赖：`log_normalizer.rs`）
- `GET /api/sessions/{id}/messages?offset=&limit=&before=&init=` + `/api/v1/...` — port `MANAGER.get_messages_page`：`offset≥0`、`limit ∈ [20,200]`、`before≥0`、`init=1` 启用 init payload；输出消息历史 + `diag` 字段（含 `meta_refresh_ms` for 非 Pi）。**注意**：ref 使用 `/messages/{tail,history,live}` split，target 仍用 legacy `/messages`，必须保留 huapeixuan 形态。
- `GET /api/sessions/{id}/tail` + `/api/v1/...` — `MANAGER.get_tail`，返回 `{"tail": ...}`（huapeixuan-only alias）。
- `GET /api/sessions/{id}/live?offset=&live_offset=&requests_version=` + `/api/v1/...` — port `_session_live_payload`（huapeixuan-only alias，与 ref 的 `/messages/live` 不同）。

### 3. 端点显式 non-goals

以下虽然是 GET，但**不在** Phase 2 范围：
- `GET /api/audio/live.m3u8` 与 `GET /api/audio/segments/{seg}` — 依赖 voice push worker 产 HLS 段，属于 Phase 5。
- 任何 POST handler、broker spawn、HLS / WebPush / TTS worker、Python `codoxear/*.py` 业务模块改动 — 不在 Phase 2。

### 4. 测试与验证

- **修改** `tests/contract/test_endpoint_parity.py`：解锁本 phase 范围内的全部 readonly parity 测试，使用 `assert_json_equivalent` 比对 Python 与 Rust 的 JSON 形状与值；header 比对加上 `Content-Type` 严格相等断言；`backend-rs/tests/` 同时新增 200 路径 Content-Type 单测，钉死 `application/json; charset=utf-8`。
- **新增** `backend-rs/tests/log_normalizer_parity.rs`：用 `tests/fixtures/rollout/*.jsonl` 与 `tests/fixtures/pi/*.jsonl`（Phase 2 引入的 fixture）跑端到端规整，与 Python `python -c "from codoxear.rollout_log import _messages_from_codex_log; ..."` 输出做 dict-eq。
- **新增** `backend-rs/tests/git_context_parity.rs`：在 `tempfile::tempdir()` 里 `git init` + 模拟 commit，跑 `current_branch` / `pr_summary` 与 Python 等价。`gh` 不在路径上时 `availability="no-gh"`。
- **CI**：扩展 `.github/workflows/backend-rs.yml`，在 `pytest tests/contract -k parity` 之外增加 `-k readonly` 用例 selector；`macos-latest` runner 必须包含 `gh` CLI（`brew install gh`）。
- **DoD**（Definition of Done）：tasks §7 的 9 步 gate 全绿才视为 Phase 2 完成。

## Capabilities

### New Capabilities

<!-- 不引入新 capability。本 change 通过 spec delta 修改伞形 change 已经创建的 `rust-backend-runtime`。 -->

### Modified Capabilities

- `rust-backend-runtime`: 新增 Phase 2 范围 ADDED Requirements，覆盖 (a) Phase 1 follow-up 修复（Content-Type charset、`serde_json` preserve_order、`cwd_groups` 错误恢复、`normalize_cwd_group_key`）、(b) read-only broker IPC 客户端、(c) 6 个 wave 的全部 GET 端点 parity 契约（含 byte-level / structural-eq scenario）。**不** REMOVE / MODIFY 任何已有 Requirement。

## Impact

- **Affected code（直接）**：
  - 新增：`backend-rs/src/broker_client.rs`、`backend-rs/src/session_loader.rs`、`backend-rs/src/log_normalizer.rs`、`backend-rs/src/git_context.rs`、`backend-rs/src/voice_state.rs`、`backend-rs/src/handlers/{voice.rs,sessions.rs,session_meta.rs,git.rs,files.rs,messages.rs,metrics.rs}` 或对应等价文件分割（避免 `runtime.rs` 单文件 >800 行）；`backend-rs/tests/{log_normalizer_parity.rs,git_context_parity.rs,sessions_list_parity.rs,diagnostics_parity.rs,messages_parity.rs,file_read_parity.rs,git_diff_parity.rs}`。
  - 修改：`backend-rs/Cargo.toml`（加 `serde_json` features = ["preserve_order"]、可能加 `mime_guess`、`tempfile` dev-dep）、`backend-rs/src/routes.rs`（注册全部新 handler 至 canonical + legacy 双命名空间，统一走 `json_response` helper）、`backend-rs/src/runtime.rs`（修 `read_cwd_groups` / `normalize_cwd_group_key`；如已超 800 行则拆分到 helper 模块）、`backend-rs/src/models.rs`（新增 response 结构体）、`tests/contract/conftest.py`（无大改；仅在 fixture 中提供 git/voice/log fixtures 时新增 helper）、`tests/contract/test_endpoint_parity.py`（解锁 readonly 用例）、`docs/cutover/endpoint-inventory.md`（每行勾选状态从 "Phase 2 port" → "implemented by `rust-backend-readonly-routes`"）、`README.md` / `tests/contract/README.md`（更新运行说明）、`.github/workflows/backend-rs.yml`（macOS gh 安装 + 新 selector）。
  - **未修改**：`codoxear/server.py`、`codoxear/broker.py`、`codoxear/pi_broker.py`、`codoxear/voice_push.py`、其它 `codoxear/*.py` 业务模块、`web/` frontend、`pyproject.toml`。
- **APIs**：新增 Rust 实现的 25+ GET 端点（每个挂 `/api/v1/*` canonical + `/api/*` legacy alias）；Python 同名端点继续在原端口由 Python 服务，**Phase 2 不切流**——Rust binary 仍是独立的、可手动启动的开发 / 验证工件。
- **依赖**：可能新增 `mime_guess`（文件 viewer 用）、`humansize`、`urlencoding`、`parking_lot`（git_context lock）等小 crate；不引入 `web-push` / `reqwest` / `openssl` / `hyper` pin（保留给 Phase 5）；rustup stable 仍 ≥ 1.78。
- **运维**：Phase 2 不改变现有 systemd unit / Tailscale Serve 端口配置。Rust binary 仍只用于本地开发与 contract harness 验证。
- **回滚**：Phase 2 仍然纯 additive：(a) 不启动 Rust binary 即可完全回到 Phase 1；(b) `git revert` 本 phase commit 即可移除 `backend-rs/` 新增模块与 contract test 改动，对 Python backend 零影响；(c) Phase 1 follow-up 修复（Content-Type、`serde_json` 顺序、`cwd_groups` 恢复、canonical）属于 byte-level 兼容性提升，不引入语义破坏，回滚不会破坏现有 Phase 1 测试。
