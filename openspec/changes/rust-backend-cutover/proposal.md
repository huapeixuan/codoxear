## Why

`codoxear/server.py` 已经增长到 ~11,099 行单文件、单 `Handler` 类，所有 HTTP 路由通过 `do_GET` / `do_POST` 内部 if/elif 链 + `path.endswith(...)` 后缀匹配派发；后台 sweep（harness、queue、voice push）以 Python 线程方式跑在 `SessionManager` 内部，跨模块共享可变状态。代码已经触达 `coding-style.md` 中"single file 800 lines max" 的硬上限十多倍，新增能力（如 PR badge、composer image input、per-session drafts）每次都迫使 server.py 进一步膨胀。

参考实现 `san-tian/codoxear@san-tian-dev` 已经把整个后端切换到 Rust（axum + tokio），并按照"opt-in Rust broker → 把 daemon-side 的 voice scan / 投递 / session 切换到 Rust → 删除 Python fallback → Remove legacy Python surface"的顺序，渐进式地完成了 cutover。我们要在 `huapeixuan/codoxear` 上复刻这条路径：保留同一份用户体验和 API 形状，但把 backend 实现替换为 Rust，并且按相同的可灰度顺序推进，避免一次大爆炸。

## What Changes

> 这是一个**伞形**(umbrella) change，覆盖整个 cutover 的契约和路线图。每个 phase 之后都会以独立的、范围更小的 OpenSpec change 落地（命名见 design.md「Phase 拆分」）。本 change 完成后，`coding-agent` 可以直接开始 Phase 1。

- **新增** `backend-rs/` Rust crate（`codoxear-backend-rs`），二进制 `codoxear-backend-rs`（HTTP server）和 `codoxear-broker-rs`（broker），axum 0.7 + tokio + tower-http。
- **新增** 路由层：路由统一以 `/api/v1/...` 命名空间为 canonical；`/api/...` 老路径保留为 legacy alias，行为与 Python 当前 `do_GET`/`do_POST` 严格对齐，方便同 build 内并行验证。
- **新增** 三个进程内后台 worker（harness sweep、queue sweep、voice scan、voice delivery），各自由独立的环境变量 feature flag 控制启停（`CODOXEAR_ENABLE_HARNESS_SWEEP` / `..._QUEUE_SWEEP` / `..._VOICE_SCAN` / `..._VOICE_WORKER`），用于灰度切换。
- **新增** `scripts/codoxear-local`（start / stop / restart / status / logs），生命周期围绕 Rust 二进制管理，并要求 `frontend/dist` 已构建。
- **修改** broker 启动入口：保留 `codoxear-broker`（Python）作为兼容入口，但通过环境变量 `CODOXEAR_RUST_BROKER_BIN` 在 Python `Session` 创建路径上转发到 Rust broker（与 ref 仓库 `Default Rust server sessions to Rust broker` 对齐）。
- **修改** 静态资源服务：保留 `codoxear/static/dist` 走 Python 时的现有路径不动；Rust server 增加 `/nova-preview/` 作为 Rust-served 入口，便于 A/B 路由对比；不修改 frontend 源码（不属于本次 backend 重构范围）。
- **保留** 现有 Python 模块（`broker.py`、`pi_broker.py`、`server.py`、`voice_push.py` 等）作为 fallback，直到所有 Phase 全部 GA 之后再删除。**BREAKING** 出现在最后一个 phase（"Remove legacy Python surface"）。
- **不变** 用户可见 API contract（请求/响应字段、状态码、cookie 名 `codoxear_auth`、HMAC cookie 签名格式、运行时目录 `~/.local/share/codoxear`、log 路径 `~/.codex/sessions/rollout-*.jsonl` 与 `~/.pi/agent/sessions/*.jsonl` 等等）。Rust 实现以"行为级 parity"为入收标准，详见 specs/rust-backend-runtime/spec.md。

## Capabilities

### New Capabilities

- `rust-backend-runtime`: 由 Rust 提供的 HTTP server、后台 sweep workers、broker 二进制，以及它们与 `~/.local/share/codoxear` 中持久化文件、`socks/*.sock|*.json` 元数据、backend session log 的契约。覆盖会话发现、消息历史/实时拉取、队列、harness 调度、voice push、文件读写、git diff、认证 cookie、WebPush 订阅等 backend 行为。

### Modified Capabilities

<!-- 当前 openspec/specs/ 为空（仅有 changes/session-show-branch-pr 是 in-flight），所以本 change 不会修改既有 spec，只会新增 rust-backend-runtime。 -->

## Impact

- **Affected code（直接）**：
  - 新增：`backend-rs/` 全部源码与测试（`Cargo.toml`、`src/{main.rs, lib.rs, app_state.rs, routes.rs, runtime.rs, models.rs, voice_worker.rs, broker.rs}`、`src/bin/codoxear-broker-rs.rs`、`tests/`）。
  - 新增：`scripts/codoxear-local`、`.tmp/`（runtime artifacts，需 gitignore）、`backend-rs/target/` 已经在根目录 `.gitignore` 默认 cargo 模板。
  - 修改：`README.md`、`AGENTS.md`、`pyproject.toml`（脚本入口标记为 deprecated 但保留）。
  - **未修改** `web/`：本 change 不重构 frontend；所有契约保持 backwards-compatible，frontend build 不需要改动。
- **APIs**：所有现有 `/api/...` 路径在 Rust 后端继续可用（legacy router）；同时在 `/api/v1/...` 暴露同一组路由，便于以后清理。
- **依赖**：新增 Rust 工具链要求（rustup stable，最低 1.78），CI 增加 Rust build & test。Frontend 不增加依赖。
- **运维**：进程模型从 1 个 Python server + N 个 Python broker，演变为 1 个 Rust server + N 个 Rust broker（Phase 4 之前 broker 仍可双轨运行，由 `CODOXEAR_RUST_BROKER_BIN` 控制）。systemd unit / Tailscale Serve 端口 `8743` 保持不变。
- **回滚**：每个 phase 都通过 env flag 关停回 Python 路径；最终 phase 删除 Python 之后才需要 git revert 回滚。
