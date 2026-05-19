## Context

`codoxear` 当前有三块 Python 进程：

1. **`codoxear-server`**（`codoxear/server.py`，11.1k LOC，单一 `Handler` + 线程池）—— 提供 UI 静态资源 + `/api/*`，并且通过 `SessionManager` 内部线程跑 harness sweep / queue sweep / voice push scan。
2. **`codoxear-broker`**（`codoxear/broker.py`，1.7k LOC）—— PTY wrapper，spawn `codex`，写 `~/.local/share/codoxear/socks/<id>.{sock,json}`。
3. **`codoxear-pi-broker`**（`codoxear/pi_broker.py`，1.0k LOC）—— Pi 后端的对等实现。

由 `git log --oneline` 可以看到，`san-tian/codoxear@san-tian-dev` 在过去若干个月里已经把 backend 整个迁到 Rust：

```
a619ce0 Add opt-in Rust broker
c3c016e Move daemon voice scanning to Rust
3287abb Move no-worker voice delivery to Rust ledger
a061fca Move voice delivery worker to Rust
8e39885 Default Rust server sessions to Rust broker
425bb05 Default standalone sessions to Rust broker
0676084 Remove Python voice fallback
0241005 Remove Python broker fallback
9bb2704 Remove legacy Python surface
```

最终形态（`backend-rs/`）：单 crate `codoxear-backend-rs`，`axum 0.7` + `tokio 1.36` + `tower-http`，`src/main.rs` 启动 server，`routes.rs` 注册 `/api/v1/...` 路由 + 与现有 `/api/...` 兼容路由（`legacy_router` + `public_api_router`），`runtime.rs` 是 ~10.6k LOC 的"业务逻辑层"——会话发现、消息历史/实时拉取、文件读写、harness 注入、git diff、voice push 等都内联在这个文件。`broker.rs` 2.2k LOC 同时支撑 `codex` 和 `pi`。`models.rs` 是 serde 类型层。`voice_worker.rs` 1.7k LOC 是独立的 voice 投递 worker。`app_state.rs` 持有 `RuntimeConfig`、`SharedStore`（`Arc<RwLock<Store>>`）、`broadcast::Sender<LiveEvent>`。

当前 `huapeixuan/codoxear` 完全没有 Rust 代码（`git log --all --grep=Rust` 仅匹配到一个无关 commit "Trust web-created session cwd on launch"）。最近主线在快速演进 Python feat：composer image input、per-session drafts、context usage 等。这意味着：

- 我们不能"先停止 Python 主线开发，集中三个月做迁移"——Python 路径必须在 cutover 完成前持续可工作。
- 我们也不能"开新仓库重写然后丢老仓"——issue 明确要求"重构当前仓库"。

约束：

- 用户面 API、cookie、磁盘契约（`socks/*.{sock,json}`、`session_aliases.json`、`harness.json`、`session_queues.json`、`session_sidebar.json`、`session_files.json`、`hidden_sessions.json`、`recent_cwds.json`、voice 子树）必须保持 byte-level 兼容，否则双轨运行不可能。
- 单进程 Python server 内部不少状态在内存（如 `SessionManager._broker_states`），这些**不需要**在 Rust 中跨进程持久化，只需保证 Rust 重启后能从磁盘 sidecar 重建（参考 `san-tian` 的 `load_sessions_response`）。
- macOS 与 Linux 都要支持；不依赖 Linux-only 的 `/proc` 接口（broker 已在 Python 侧切到 `lsof/pgrep` 做 macOS 兼容，Rust port 必须沿用，不能回退）。

stakeholders: maintainer (huapeixuan)、所有 web 用户（仅观感影响：UI URL 不变，但 `/nova-preview/` 是 Rust-served 的并行入口）、自托管 ops（systemd unit / Tailscale Serve 端口需保持 `8743`）。

## Goals / Non-Goals

**Goals:**

- 把 Python backend 的全部行为以 Rust 重写到 `backend-rs/`，axum + tokio。
- 为每一个迁移阶段提供独立 feature flag（env var），保证可逐 endpoint 灰度，可一秒回滚到 Python。
- 复用 `san-tian-dev` 的模块拆分骨架（`app_state.rs` / `routes.rs` / `runtime.rs` / `models.rs` / `voice_worker.rs` / `broker.rs`），避免再发明一种"第二种 Rust 架构"。
- 完全保持磁盘元数据契约（`socks/*.json` schema、harness.json、queue.json、voice ledger）与现有 Python 写出格式 byte-level 一致。
- frontend 完全不动，`web/dist` 继续被 Rust server 服务（`/static/dist/*`），也通过 `/nova-preview/` 别名暴露（与 ref 一致）。
- 最终阶段移除 `codoxear/server.py`、`codoxear/broker.py`、`codoxear/pi_broker.py`、`codoxear/voice_push.py` 等 Python backend 文件；保留 `codoxear/static/` 与 `pyproject.toml`（变成 build-time 静态打包来源，或者删除发布 Python wheel 的入口，由 phase 6 决定）。

**Non-Goals:**

- 不重写 frontend（`web/` 不改）。即使 `san-tian` ref 的 `frontend/` 与本仓 `web/` 目录命名不一致，也以本仓 `web/` 为准。
- 不引入新的用户面功能。本次只做 backend 重构。
- 不在本伞形 change 中真正落地 Phase 1 之外的代码改动；后续每个 phase 单独开 OpenSpec change。
- 不在本次重构中清理 san-tian-dev 没有的 huapeixuan 独有功能（如 PR badge、image composer、context usage 显示）——这些在 git log 中是 huapeixuan 后加的，必须保留。会在 Phase 1/2 spec parity 检查时显式列出补差项。
- 不引入新的语言（Go、Node 之类）；不引入新的框架（actix、warp 等）。

## Decisions

### D1：直接复制 `san-tian-dev` 的 Rust 模块结构，而不是从零设计

**选择**：把 `backend-rs/Cargo.toml`、`src/{main.rs,lib.rs,app_state.rs,routes.rs,runtime.rs,models.rs,voice_worker.rs,broker.rs,bin/codoxear-broker-rs.rs}` 的 layout 1:1 落地。

**为什么**：

- ref 已经过几十个 commit 的 PR-by-PR 演化验证，骨架是已 ship 过的设计。
- 让"是否对齐 ref"在 review 时可机械判断：`diff -ruN huapeixuan/backend-rs san-tian/backend-rs` 应该越来越短。
- 如果重新设计（比如把 routes / handlers / services 分三层），就要重新评估 ref 的所有决策（异步 broadcast、Arc<RwLock<Store>>、env-flagged worker），徒增风险。

**替代方案**：分层架构（routes → services → repositories）。**否决**理由：ref 是单 `runtime.rs` 大文件，业务逻辑函数是顶层 free function（不是 service struct）。这与 Rust 习惯里"按 domain 模块分层"的偏好略有冲突，但保持与 ref 一致更重要——本 change 不是发明新架构，是 cutover。等 cutover 完成再做架构清理是单独的 change。

### D2：API 双命名空间（`/api/v1/*` canonical + `/api/*` legacy）

**选择**：所有路由在 Rust 上同时挂 `/api/v1/<path>` 和 `/api/<path>`，body 完全相同。

**为什么**：

- 当前 Python server 仅暴露 `/api/<path>`，所有现存 frontend / 自动化都按这个调用。
- 切换到 Rust 时 frontend 不动，所以 `/api/<path>` 必须继续工作。
- `/api/v1/<path>` 是为以后的 frontend / 第三方 client 留的清理点；同时让 ref 的 `routes.rs` 几乎可以照抄过来（ref 自身就是双挂的）。

**替代方案**：只 `/api/<path>`。**否决**理由：将来想做 v2 时无 namespace；ref 已用双命名空间，照抄成本最低。

### D3：以 env 变量 feature flag 控制 worker 启停

**选择**：

- `CODOXEAR_ENABLE_HARNESS_SWEEP` (默认 0)
- `CODOXEAR_ENABLE_QUEUE_SWEEP` (默认 0)
- `CODOXEAR_ENABLE_VOICE_SCAN` (默认 0)
- `CODOXEAR_ENABLE_VOICE_WORKER` (默认 0)
- `CODOXEAR_RUST_BROKER_BIN` (默认 unset；set 后 Python `Session._spawn_*` 走 Rust broker 二进制)

每个 flag 控制一个能力是否在 Rust 进程内启动；同时 Python 侧的同名能力**仅在所有对应 flag 都 off 时**自启动，避免双写竞争（Python 现有 `SessionManager` 启动时不读 env，需要在 phase 2 改成读这些 env，做"如果 Rust 启用则不要启 Python 线程"判断）。

**为什么**：

- ref 已采用同名 flag (`scripts/codoxear-local` 中可见)，照抄即可。
- 单 flag 单 toggle，回滚粒度细。
- Python 侧根据 flag 让出，能力同一时刻只在一个进程跑，不会 double-write `voice_delivery_ledger.json`。

**替代方案**：通过 `pyproject.toml` 删除 entry point + 完全替换。**否决**理由：缺乏灰度，回滚需要回 git。

### D4：Broker 由 Python `Session` 启动子进程时通过环境变量切换

**选择**：Python 的 `SessionManager._spawn_broker` 检查 `CODOXEAR_RUST_BROKER_BIN`：

```python
broker_bin = os.environ.get("CODOXEAR_RUST_BROKER_BIN", "").strip()
if broker_bin:
    cmd = [broker_bin, *broker_args]
else:
    cmd = [sys.executable, "-m", "codoxear.broker", *broker_args]
```

Rust broker 必须接受**完全相同的 CLI 参数 + 环境变量**（包括 `CODEX_WEB_OWNER`、`CODEX_WEB_AGENT_BACKEND`、`CODEX_WEB_TMUX_*`），并写出**完全相同 schema** 的 `socks/<id>.{sock,json}`。

**为什么**：

- ref `0241005 Remove Python broker fallback` 之前的状态正是这种"双轨"，等同于把双方 contract 锁死在 broker JSON sidecar schema 上。
- 比"Python server 直接调 Rust HTTP API"更简单：broker 只是文件 + Unix socket 通信。

### D5：保持磁盘契约 byte-level 兼容

**选择**：所有 `~/.local/share/codoxear/*.json` 文件、`socks/*.{sock,json}` 元数据、voice ledger 文件、HMAC cookie 签名格式（`HMAC-SHA256(b64u(payload) + "." + b64u(now))`），都不变。

**测试方法**：在 Phase 1 写一个跨语言契约测试 `tests/contract/test_disk_schema_parity.py`，让 Python broker 写一份元数据，再让 Rust 解析；反之亦然。所有字段往返必须一致。

**为什么**：

- 双轨运行期间，同一台机器上可能同时存在 Python 写的 `socks/sess-A.json` 和 Rust 写的 `socks/sess-B.json`，两个 server 都要能读对方的。
- ref 没明确写出 contract test，但他们的 commit 顺序"Move ... to Rust"暗示了对等行为已被反复验证过；我们要主动用 contract test 把这个验证显式化，避免 phase 间漂移。

### D6：单 `runtime.rs` 大文件 vs. 拆多个模块

**选择**：先 1:1 复制 ref 的 `runtime.rs`（~10.6k LOC），保持顶层 free function 风格；不在 cutover 期间做架构清理。

**为什么**：

- 全局共享的 helper（path 解析、`run_git_capture`、`safe_filename` 等等）在 ref 中是 file-private free function。把它们拆到子模块需要决定 `pub(crate)` / `pub(super)` 的边界，这是另一个维度的设计。
- cutover 完成（phase 6）以后再开 follow-up change `rust-backend-modularize` 做拆分，那时已经有了 Rust 单元测试覆盖网，重构是安全的。

**风险**：单文件 10k 违反 `coding-style.md` 800 行硬上限。**接受**：在伞形 change 的 design 中明确标注此为已知 debt，phase 6 完成后立即开 follow-up。

### D7：测试矩阵

- **Rust 单元测试**：`backend-rs/src/**/#[cfg(test)] mod tests`，覆盖纯函数（path 解析、git numstat 解析、harness 文本渲染、queue item 序列化）。目标 80%+ line coverage。
- **Rust 集成测试**：`backend-rs/tests/*.rs`，使用 `tower::ServiceExt::oneshot` 对 axum router 直接发请求，参考 ref 的 `tests/bootstrap_tmux.rs`。
- **跨语言契约测试**：在 `tests/contract/` 下用 pytest，启动 Rust 二进制 + Python server，两边发同样的请求，比对 JSON 响应。CI 中跑（要求 Rust 工具链）。
- **保留全部 Python 现有 pytest**：cutover 期间，Python server 测试就是验证 fallback 路径还在跑。

### D8：默认绑定与端口

ref `main.rs` 默认 `127.0.0.1:8787`；本仓 Python 默认 `:: : 8743`。**保留** 8743 默认值（避免破坏现有用户）。Rust `main.rs` 改为：

```rust
let port = env::var("CODEX_WEB_PORT").or_else(|_| env::var("CODOXEAR_BIND_PORT"))
    .ok().and_then(|s| s.parse().ok()).unwrap_or(8743);
let host = env::var("CODEX_WEB_HOST").or_else(|_| env::var("CODOXEAR_BIND_HOST"))
    .ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| IpAddr::from([0,0,0,0,0,0,0,0]));
```

`CODEX_WEB_*` 变量是 Python 现状，必须优先；`CODOXEAR_BIND_*` 是 ref 的，作为回退别名。

## Risks / Trade-offs

- **[Risk] 11k LOC `server.py` 中藏的隐式行为没被 ref 完全覆盖**（huapeixuan 在 ref 之后加的功能：PR badge、image composer、per-session drafts、context usage 显示等）→ **Mitigation**：Phase 1 Tasks 第一项是写出**完整 endpoint inventory**（grep `path == "/api/...` + `path.startswith("/api/sessions/")`），逐一比对 ref 的 `routes.rs`，新增的部分作为 phase-specific tasks 显式列出。
- **[Risk] Python 与 Rust 同时写 `voice_delivery_ledger.json` 等共享 JSON 文件造成竞态** → **Mitigation**：Phase 切换时遵循"先把 Python 那一面关停 → 再启 Rust 那一面"的 deploy 顺序，且在 design 中要求 worker 启动前用 `O_EXCL` 创建 `<file>.lock` advisory lock；启动失败立刻 abort，不"先 sleep 再 retry"。
- **[Risk] macOS 上 PTY/lsof 行为差异** → **Mitigation**：Phase 4（broker cutover）的入收测试必须包含 macOS GitHub Actions runner，不能只 Linux 通过就 ship。
- **[Risk] 用户的 `~/.zshrc` / `~/.bashrc` 已经写死了 `codoxear-broker --` 这个二进制名** → **Mitigation**：Rust broker 装出 `codoxear-broker-rs` 二进制，但同时在 `pyproject.toml` 保留 `codoxear-broker = codoxear.broker:main` 入口（直到 phase 6 才删），并在 README 显式说明用户什么时候应该把 alias 切到 `codoxear-broker-rs`。
- **[Trade-off] Rust 编译时间** → 用户安装 codoxear 从 `pip install` 一步变成"装 rustup → cargo build --release"。可接受，因为 ref 已经做了同样的取舍；后续可考虑提供 GitHub Releases 预编译 artifact。
- **[Trade-off] `runtime.rs` 单文件 10k LOC 违反 coding-style** → 如 D6 所述，接受为已知 debt，phase 6 之后立刻 follow-up。

## Migration Plan / Phase 拆分

每个 phase 是一个独立的、未来要单独创建的 OpenSpec change。本 umbrella change 只在 specs/ 中固化"目标态"；phase change 各自固化"增量"。每个 phase 都要可独立回滚（关 env flag 即可）。

| Phase | OpenSpec change name | 范围 | 完成定义 |
|---|---|---|---|
| 0 | （本 change，`rust-backend-cutover`） | 写 proposal/design/spec/tasks，建立目标态契约。**不写 Rust 代码**。 | OpenSpec validate 通过；coding-agent 可进入 phase 1。 |
| 1 | `rust-backend-skeleton` | 新建 `backend-rs/` crate；`main.rs` + `app_state.rs` + `models.rs` + 空 `routes.rs` + 空 `runtime.rs`；只挂 `/api/v1/health` 和 `/api/v1/me`、`/api/v1/sessions/bootstrap`（read-only，无副作用）。Python server 不变。 | `cargo test --release` 通过；手动 curl 三个端点对比 Python 响应，JSON 字段一致；CI 增加 `backend-rs/` 的 build & test job。 |
| 2 | `rust-backend-readonly-routes` | 实现所有 GET 路由（sessions list、messages tail/history/live、diagnostics、queue read、harness GET、git/diff、git/changed_files、git/file_versions、file/read、file/search、file/blob、cwd_suggestions、settings/voice GET、notifications GET、metrics）。仍只 `/api/v1`。 | 双轨：Python 8743 + Rust 8744 同时跑，frontend 切到 Rust 端口对照测试 30 分钟无差异。 |
| 3 | `rust-backend-write-routes` | 实现 POST 路由（session create/edit/rename/delete、send、enqueue、queue mutate、harness POST、ui_response、heartbeat、interrupt、login/logout、settings save、notifications subscribe、audio listener、file write、files inspect/blob、hooks notify、cwd_groups edit）。 | Python `SessionManager` 检测 `CODOXEAR_ENABLE_*` flag，若 Rust 启用则不再启 Python 线程；contract test 通过。 |
| 4 | `rust-backend-broker` | port `broker.py` + `pi_broker.py` 到 `backend-rs/src/broker.rs` + bin `codoxear-broker-rs`；`pyproject.toml` 中 `codoxear-broker` 入口仍保留。Python `Session._spawn_broker` 检查 `CODOXEAR_RUST_BROKER_BIN`。 | 在 macOS + Linux runner 上验证 codex / pi 两个 backend 都能用 Rust broker 启动并被 server 列出。 |
| 5 | `rust-backend-voice-push` | port `voice_push.py` 到 `voice_worker.rs`，包括 HLS 服务、WebPush 投递、OpenAI TTS client、ledger 写入。 | voice push 通过 Rust 路径在真实手机上收到通知；Python `voice_push.py` 仅在 `CODOXEAR_ENABLE_VOICE_*` 全部为 0 时自启动。 |
| 6 | `rust-backend-cutover-finish` | 删除 `codoxear/server.py`、`codoxear/broker.py`、`codoxear/pi_broker.py`、`codoxear/voice_push.py` 及伴随测试；`pyproject.toml` 仅保留 `codoxear/static/` 静态资源；`README.md` / `AGENTS.md` 反映 Rust-only。 | `pip install` 只装静态资源 wheel 或彻底废弃 wheel；Rust 二进制是唯一入口。 |
| 7 (follow-up) | `rust-backend-modularize` | 把 `runtime.rs` 拆分为 `domain/{sessions, queue, harness, files, git, voice}/...`，恢复 800 行硬上限。 | 不在 cutover 阻塞路径上。 |

## Rollback strategy

- Phase 1–3 部分上线：在 `~/.local/share/codoxear/.env` 把 `CODOXEAR_ENABLE_*` 全设为 0，重启 systemd unit；Python 自启动，Rust server 仅作为静态资源 fallback。
- Phase 4 broker：`unset CODOXEAR_RUST_BROKER_BIN`，所有新 session 重新落到 Python broker。已经在跑的 Rust broker 不影响（它独立持有 PTY）。
- Phase 5 voice：把 `CODOXEAR_ENABLE_VOICE_SCAN/WORKER` 设为 0，Python `voice_push.py` 重新接管。
- Phase 6 不可回滚（删了 Python 文件），所以入收前必须 phase 1–5 全部 GA 至少 1 周。

## Open Questions

1. 是否需要在 cutover 中保留 `codoxear/sessiond.py`（headless launcher）？ref 没有对等模块。**建议**：phase 4 评估，若使用率极低则一并移除。**Owner**：在 phase 4 spec 中决定。
2. `web-push` 0.10.2 是否能覆盖 `pywebpush` 的全部 ECDSA / VAPID 行为？**建议**：phase 5 之前先用 `cargo run --example` 跑通真实 endpoint，再排进 phase。
3. CI runner 的 Rust 缓存策略？**建议**：使用 `Swatinem/rust-cache@v2` action，本 change 不固化具体 CI yaml，留给 phase 1。
4. 是否需要支持 Windows？目前 Python 已经声明 not supported，Rust 沿用即可——不在 open question 之内。
