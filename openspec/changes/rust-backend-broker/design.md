## Context

Phase 4 的入口事实：

- `backend-rs/src/bin/codoxear-broker-rs.rs` 当前只打印“not implemented”并退出 2；Rust server Phase 3 的 `session_create.rs` 仍通过 `python -m codoxear.broker` 或 `python -m codoxear.pi_broker` 创建 live session。
- Python Codex broker (`codoxear/broker.py`) 的关键职责包括：PTY spawn/terminal bridge、Unix socket JSON-line command server、rollout log discovery、busy/token state、`socks/*.json` sidecar 写入、shutdown/process-group cleanup。它还在 `CODEX_WEB_AGENT_BACKEND=pi` 时转交给 `PiBroker` foreground mode。
- Python Pi broker (`codoxear/pi_broker.py`) 的关键职责包括：Pi RPC session 启动、AskUser bridge extension、pending UI request state、`live_messages`/`ui_state`/`commands` socket commands、`session_path` sidecar、SIGINT/foreground stdin/stdout bridge。
- `docs/cutover/disk-contracts.md` 已经把 Codex/Pi/sessiond sidecar schema、mode `0600`、nullable 字段、`session_path`、`agent_backend` patch 兼容规则列为 Phase 0 byte-level contract。
- Phase 3 已实现 Rust server 通过 broker socket 做 `/send`、`/ui_response`、`/interrupt`、`/delete` 等 mutation，因此 Phase 4 的核心不是改 HTTP API，而是替换 socket server 的实现者。
- issue 明确要求通过 `CODOXEAR_RUST_BROKER_BIN` 在 Python `Session._spawn_broker` 路径上灰度转发到 Rust binary；本仓实际函数名是 `SessionManager.spawn_web_session`，Rust server 同样有 `backend-rs/src/session_create.rs` 的 create path，也要同一 flag。

## Goals / Non-Goals

**Goals:**

- 实现可运行的 `codoxear-broker-rs`，覆盖 Codex 与 Pi 两个 backend 的 live broker 能力。
- 保持当前 Python broker CLI、env、sidecar、socket JSON-line protocol、process cleanup、owner/tmux/spawn_nonce/model metadata 兼容。
- 为 Python server 与 Rust server 的 web-owned session create 增加同一个 rollout selector：`CODOXEAR_RUST_BROKER_BIN`。
- 保证 rollback 只需要 unset `CODOXEAR_RUST_BROKER_BIN` 并重启 server；不迁移磁盘文件，不删除 Python fallback。
- 显式验证 Linux 与 macOS：Linux `/proc` log discovery；macOS `lsof`/`pgrep` + PTY 行为。

**Non-Goals:**

- 不删除 `codoxear/broker.py`、`codoxear/pi_broker.py`、`codoxear/sessiond.py` 或 Python console scripts；这是 Phase 6。
- 不改变 `/api/sessions` 请求/响应字段，不改 frontend。
- 不实现 Phase 5 voice/HLS/WebPush/TTS worker，也不把 voice side effects 挂到 broker。
- 不把 `sessiond.py` 迁移为 Rust；本阶段仅保证 Rust/Python server 能继续读取 sessiond sidecar。是否移除/迁移 sessiond 留给 Phase 6。
- 不把 Rust broker 默认启用；默认仍 Python broker，只有 env flag set 时才走 Rust。

## Decisions

### D1：一个 broker binary，同一代码内按 backend 分派

**选择**：`codoxear-broker-rs` 解析公共 CLI/env 后，按 `CODEX_WEB_AGENT_BACKEND`（默认 `codex`）进入 Codex broker 或 Pi broker。Pi 专用 `--session-file` 是同一 binary 的可选参数。

**原因**：Python 对用户暴露的是一个 `codoxear-broker` wrapper；`broker.py` 也会在 Pi backend 下委托 `PiBroker`。保持单 binary 可以让 Python/Rust server 的 `CODOXEAR_RUST_BROKER_BIN` 指向一个路径，不需要再引入 `CODOXEAR_RUST_PI_BROKER_BIN`。

**替代方案**：拆成 `codoxear-broker-rs` 与 `codoxear-pi-broker-rs`。否决：增加 rollout/env/README/CI 矩阵复杂度，且和当前 Python wrapper 语义不一致。

### D2：先做 contract-first broker modules，再接真实 process

**选择**：实现顺序为 `meta.rs`（sidecar）、`ipc.rs`（socket protocol）、`log_discovery.rs`（proc/lsof）、`pty.rs`（PTY abstraction）、`codex.rs`、`pi.rs`。先用 fake backend/RPC/PTY 在 Rust tests 中锁定 JSON 与状态，再接真实 Codex/Pi child process。

**原因**：broker 最危险的是 byte-level contract 与 macOS behavior；先写真实 PTY 再回补测试容易把隐式行为遗漏。Phase 3 已经有 broker client tests，可复用其 stub 思路反向测试 broker server。

**替代方案**：直接照 Python 逻辑一口气移植。否决：难以 review，也容易突破 800 行限制。

### D3：sidecar 写入使用兼容强化策略

**选择**：Rust 写 `socks/*.json` 时使用同目录 temp file + fsync + rename + chmod `0600`，但 JSON schema、字段名、null/missing 规则与 Python 保持一致。Codex sidecar 包含 Python Codex 的所有 nullable keys；Pi sidecar 包含 Python Pi keys，并额外写 `agent_backend:"pi"` 作为 Rust 明确字段（Python discovery已接受 server-side patch 添加该字段）。

**原因**：`disk-contracts.md` 允许 Rust 在 Python reader 兼容前提下改进 atomicity。broker sidecar 是 server discovery 的关键文件，rename 更安全；但不能改变字段 contract。

**替代方案**：完全复制 Python direct write。否决：没有必要保留非 atomic 风险；Phase 3 state writer 已采用 Rust atomic strategy。

### D4：socket command response 以 Python error strings 为 contract

**选择**：所有 socket command validation error（如 `text required`、`seq required`、`unknown cmd`、Pi `unknown or expired request`、`request already resolved`）按 Python 字符串返回。

**原因**：Phase 3 Rust/Python server 的 mutation endpoints 会把 broker error 映射成 HTTP error；若 broker error 漂移，会造成上层 contract tests 失败或 UI 行为变化。

**替代方案**：Rust 使用结构化错误码。否决：这是后续 protocol v2 的事；Phase 4 只做兼容 port。

### D5：Codex PTY 采用 crate 封装，但必须暴露 Unix process id 和 resize/IO primitives

**选择**：可选 `portable-pty` 或 `nix`/`libc` 自行 forkpty；最终必须满足：拿到 managed child pid（用于 `codex_pid`、process group cleanup、log discovery root pid）、可设置 window size、可读写 PTY master、支持 terminal raw mode / restore。

**原因**：Python 使用 `pty.fork()`，功能面广；Rust 标准库没有 PTY。crate 选择必须服务 contract，而不是只追求跨平台抽象。

**替代方案**：用普通 `Command` pipe spawn Codex。否决：会破坏 TUI/terminal-owned session 行为，无法模拟真实 terminal。

### D6：log discovery 平台分支显式实现并测试

**选择**：Linux 保留 `/proc/<pid>/fd` + descendant traversal；macOS 保留 `pgrep -P` + `lsof -p <pids> -F n`。实现层通过 trait/command runner 注入，使 tests 可以在 Linux 上模拟 macOS `lsof` 输出。

**原因**：issue 特别点名 macOS PTY/lsof；只依赖 Linux `/proc` 会回退平台支持。Python `util.py` 已经有对应行为，可直接作为 oracle。

**替代方案**：轮询 `~/.codex/sessions` 最新文件。否决：容易绑定错误 session/subagent，且无法区分同 cwd 多 broker 并发。

### D7：Pi broker 优先复用 Pi RPC protocol 行为，不退回纯 PTY Pi

**选择**：Rust Pi broker 等价移植 `pi_broker.py` 的 Pi RPC path：session file、`PiRpcClient` 等能力需在 Rust 中实现或通过最小 subprocess/RPC bridge 实现，但对 socket clients 暴露同一 Pi broker command surface。

**原因**：当前 README 说明 Pi path 是 RPC-backed live UI，不是普通 PTY attach；`ui_response`、`commands`、`live_messages` 依赖 RPC event stream。退回 PTY 会丢失 AskUser/live UI 能力。

**替代方案**：先让 Pi 走 Codex-style PTY，只支持 send/keys。否决：不满足 issue 的 Codex broker / Pi broker 关键能力范围，也会破坏 Phase 3 Pi UI response endpoint。

### D8：`CODOXEAR_RUST_BROKER_BIN` 同时作用于 Python server 与 Rust server

**选择**：Python `SessionManager.spawn_web_session` 和 Rust `spawn_web_session` 都读该 env；非空时替换 broker executable，空/blank 时不改变 Phase 3 fallback。tmux create path 的 inline command 也必须使用同一选择结果。

**原因**：用户可能在 Phase 4 仍运行 Python server，也可能运行 Rust server；灰度变量必须覆盖两条 create path，否则无法验证“Python fallback 可回滚”。

**替代方案**：只改 Python server。否决：Rust server Phase 3 已有 create endpoint；如果不改，Rust server 仍永久依赖 Python broker。

## Risks / Trade-offs

- **[Risk] PTY crate 无法提供 Python `pty.fork()` 等价 child pid / process group 语义** → Mitigation：先做 spike task，证明 pid、resize、read/write、cleanup 可用；若 crate 不满足，改用 `nix::pty::forkpty` + Unix-specific code。
- **[Risk] macOS `lsof` 输出或权限差异导致 log_path 长期为 null** → Mitigation：tests 注入 lsof 输出；Phase 4 verification 必须包含 macOS runner 或本地 transcript；broker 启动后没有 log_path 仍可先写 sidecar，但发现失败必须有 debug 日志。
- **[Risk] Codex subagent log 被误绑定** → Mitigation：复用 Phase 2/3 Rust log normalizer 的 session_meta parser，专门加 subagent fixture test。
- **[Risk] Pi RPC Rust port 超出预期** → Mitigation：先界定 Pi RPC 最小命令集：prompt、abort、get_state、get_commands、send_ui_response、drain_events/stderr、close；如需桥接 TypeScript extension，保持 command arg 与 Python 完全一致；不要扩展新 UI protocol。
- **[Risk] 两个 broker 同时写同一 sidecar/socket** → Mitigation：socket path 使用 broker pid 或 token，启动前 unlink only own target；sidecar rename 到 own socket stem；不复用 Python broker stem。
- **[Risk] 800 行限制与 broker 复杂度冲突** → Mitigation：从一开始拆模块；line-count gate 放进 tasks。
- **[Trade-off] Phase 4 仍保留 Python fallback，因此代码重复一段时间** → 接受；这是 rollback 的代价，Phase 6 再删除。

## Migration Plan

1. Rust broker binary landed but default不启用：`CODOXEAR_RUST_BROKER_BIN` unset 时所有 session create 仍走 Python broker。
2. 本地/CI contract tests 通过后，在开发机上设置 `CODOXEAR_RUST_BROKER_BIN=$(pwd)/backend-rs/target/release/codoxear-broker-rs`，先用 Rust server 创建 Codex/Pi web-owned session。
3. 再用 Python server + 同一 env 创建 Codex/Pi web-owned session，验证 Python discover/read/send/delete Rust sidecar/socket。
4. 终端手动验证：`codoxear-broker-rs -- <codex args>` 与 `CODEX_WEB_AGENT_BACKEND=pi codoxear-broker-rs -- <pi args>` 的 terminal-owned UX。
5. macOS 验证 PTY + lsof：在 macOS runner 或本地执行 broker subset，保存命令输出到 handoff。
6. 若出现问题：unset `CODOXEAR_RUST_BROKER_BIN`，重启 server；新 session 回到 Python broker。已有 Rust broker session 可通过 UI delete/shutdown 或进程 kill 停止；磁盘 sidecar schema 不需要迁移。

## Open Questions

1. **Rust Pi RPC 实现方式**：直接 Rust 实现 Pi RPC protocol，还是用很小的 Python/Node helper bridge？建议 coding-agent 先调查 `codoxear/pi_rpc.py` 和 Pi CLI protocol 后选择；但最终 socket/sidecar contract 不得变化。
2. **PTY crate 选择**：`portable-pty` vs `nix::pty::forkpty`。建议以 pid/process-group/resize 能力为准，而不是 API 易用性。
3. **CI macOS 成本**：如果 GitHub Actions quota 不适合每次跑完整 suite，至少 broker smoke + log discovery subset 必须跑；完整真实 Codex/Pi 可作为手动验证。
