## Why

Phase 1–3 已经把 Rust HTTP server、read-only 路由、POST mutation 路由、queue/harness worker 与 Python worker handoff 落地；但 Rust server 目前创建会话时仍依赖 `python -m codoxear.broker` / `python -m codoxear.pi_broker`，`backend-rs/src/bin/codoxear-broker-rs.rs` 仍是未实现占位。只要 broker 还在 Python，Rust cutover 就无法进入“server + broker 双 Rust 默认路径”，也无法在 Phase 5/6 前验证最关键的 PTY、Unix socket 和 sidecar 契约。

本 change 要把 Codex broker 与 Pi broker 的关键能力移植到 Rust，并通过 `CODOXEAR_RUST_BROKER_BIN` 在 Python 与 Rust server 的 session create 路径上灰度使用 Rust broker；unset 该变量即可回滚到 Python broker。

## What Changes

- 新增 `backend-rs/src/broker.rs`（可按 ≤800 行限制拆为 `broker/{mod.rs,codex.rs,pi.rs,meta.rs,ipc.rs,log_discovery.rs,pty.rs}`）实现 `codoxear-broker-rs`。
- Rust broker 支持当前 Python broker CLI：`codoxear-broker-rs --cwd <path> -- <agent args...>`，并支持 Pi 专用兼容入口：`--cwd <path> --session-file <path> -- <pi args...>`。
- Rust Codex broker 负责 spawn Codex CLI、PTY 输入输出桥接、Unix socket command server、Codex rollout log discovery、busy/token state、sidecar 写入和 shutdown。
- Rust Pi broker 负责启动/桥接 Pi RPC session、AskUser extension 参数、live UI request state、`ui_response`/`commands`/`live_messages` socket command、Pi `session_path` sidecar 和 shutdown。
- 修改 Python `SessionManager.spawn_web_session`：当 `CODOXEAR_RUST_BROKER_BIN` 非空时，Codex 与 Pi web-owned session create 改为执行该 Rust broker 二进制；为空时保留现有 Python broker 路径。
- 修改 Rust server `backend-rs/src/session_create*.rs`：同样优先使用 `CODOXEAR_RUST_BROKER_BIN`，为空时继续使用 Python broker fallback。
- 保持 `codoxear-broker` Python entry point、`codoxear.broker`、`codoxear.pi_broker` 不删除；本阶段不做 Phase 6 legacy removal。
- 新增 broker contract / integration tests，覆盖 sidecar schema、socket command protocol、spawn env/args、macOS `lsof`/`pgrep` log discovery 分支和 Linux `/proc` 分支。

## Capabilities

### New Capabilities
- `rust-backend-broker`: Rust broker 二进制、Codex/Pi backend process bridge、socket control protocol、sidecar 元数据契约、灰度切换和 rollback 行为。

### Modified Capabilities
- `rust-backend-runtime`: Rust/Python session create 路径新增 `CODOXEAR_RUST_BROKER_BIN` broker selection 规则，但 API 请求/响应 contract 不变。

## Impact

- **Affected code**：
  - `backend-rs/src/bin/codoxear-broker-rs.rs`
  - 新增/修改 `backend-rs/src/broker*.rs` 或 `backend-rs/src/broker/**`
  - `backend-rs/src/session_create.rs`、`backend-rs/src/session_create_support.rs`
  - `codoxear/server.py`
  - broker 相关 tests：`backend-rs/tests/*broker*.rs`、`tests/test_*broker*.py`、`tests/contract/*broker*` 或现有 contract suite
  - docs：`README.md`、`docs/cutover/disk-contracts.md`、`openspec/changes/rust-backend-cutover/tasks.md`
- **APIs**：`POST /api/sessions` 与 `/api/v1/sessions` 请求/响应不变；只改变内部 broker executable selection。
- **Disk/socket contracts**：必须严格遵守 `docs/cutover/disk-contracts.md` 的 `socks/*.json` schema、mode `0600`、socket path、`session_path`、`agent_backend` patch 兼容规则，以及现有 socket JSON line protocol。
- **Dependencies**：可能新增 Rust crate（例如 PTY、signal、nix、portable-pty、notify/regex 等）；新增依赖必须说明 macOS/Linux 行为并进入 CI。
- **Ops/rollback**：开启灰度：`CODOXEAR_RUST_BROKER_BIN=/path/to/codoxear-broker-rs`；回滚：unset 该变量并重启 Python/Rust server，新 session 回到 Python broker。已运行的 Rust broker session 可继续运行或通过 UI delete/shutdown 停止。
