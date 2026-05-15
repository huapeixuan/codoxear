## Why

伞形 change `rust-backend-cutover` 已完成 Phase 0：`docs/cutover/endpoint-inventory.md` 与 `docs/cutover/disk-contracts.md` 锁定了目标态契约，`tests/contract/` harness 跑通了 Python fixture 与 Rust fixture 的 placeholder 骨架。现在需要把目标态从"只有契约"推进到"有最小可执行 Rust binary"：建立 `backend-rs/` crate、跑得起来一个 axum server、并对**三个无副作用的只读端点**做 parity。

之所以选这三个端点（`/api/health` 新增 + `/api/me` + `/api/sessions/bootstrap`），是因为它们：

- 不依赖 broker、PTY、tmux、socks sidecar、log scanning、voice push 等任何后台 worker（可以用纯 axum + 文件读取实现）；
- 不写任何磁盘文件（不会触发 D5 byte-level 兼容验证、不会与 Python sweep 竞争）；
- 但已经迫使我们落地：HOME / CODOXEAR_APP_DIR 解析、`hmac_secret` cookie 验证（auth middleware）、`recent_cwds.json` / `cwd_groups.json` / `~/.codex/config.toml` / Pi settings 文件读取——这套基础设施是后续 Phase 2/3 所有路由都要复用的。

完成 Phase 1 后，`tests/contract/test_endpoint_parity.py::test_me_parity` 与 `test_sessions_bootstrap_parity` 从 `pytest.skip` 切换成实际跑双服务器对比；contract harness 不再是"摆设"。Phase 2 开始可以按 endpoint inventory 逐行勾选。

## What Changes

> 本 change 是 Phase 1，**只**实现三个端点；任何其它路由（GET 列表、POST 写、broker、voice、static asset 服务）都明确属于 non-goals。

- **新增** `backend-rs/` Rust crate（package `codoxear-backend-rs`，edition 2021），cargo workspace 独立子目录。`Cargo.toml` 复制 ref 同名 crate 的依赖清单（axum 0.7.9、tokio 1.36 full、tower-http 0.5.2、serde、serde_json、hmac、sha2、base64、tracing、tracing-subscriber 等），但**剔除** Phase 1 用不到的依赖（broker 用的 libc 例外、voice 用的 web-push、reqwest、openssl、ed25519-compact、cpufeatures、bytes/hyper/hyper-util pin 等）。Phase 4/5 时再加回。
- **新增** 二进制 `codoxear-backend-rs`（HTTP server）以及 stub bin `codoxear-broker-rs`（编译通过即可，body 是 `eprintln!("not implemented in phase 1"); std::process::exit(2);`，留给 Phase 4 替换）。
- **新增** Rust 模块骨架：`src/lib.rs`、`src/main.rs`、`src/app_state.rs`、`src/routes.rs`、`src/runtime.rs`、`src/models.rs`、`src/bin/codoxear-broker-rs.rs`。模块文件 layout 1:1 对齐 ref，但 Phase 1 各文件**只**包含三个端点必需的代码；ref 中的 `voice_worker.rs` 和 `broker.rs` 在 Phase 1 不创建源文件（在 Cargo.toml 也不引入）。
- **新增** 三个端点的 axum handler：
  - `GET /api/v1/health` 与 `GET /api/health`：public（无 auth），返回 `{"ok": true, "service": "codoxear-backend-rs"}`。这是 Rust-side 新增端点，Python 没有对等实现；它存在的目的是 deploy / load-balancer / smoke test。
  - `GET /api/v1/me` 与 `GET /api/me`：受 auth 保护，无 cookie 时返回 `401 {"error":"unauthorized"}`；有效 cookie 时返回 `{"ok": true}`（与 Python `_json_response(self, 200, {"ok": True})` byte-level 一致；**不**像 ref 那样多一个 `server_pid` 字段）。
  - `GET /api/v1/sessions/bootstrap` 与 `GET /api/sessions/bootstrap`：受 auth 保护；返回 JSON keys `recent_cwds`、`cwd_groups`、`new_session_defaults`、`tmux_available`，与 Python `codoxear/server.py:8869-8883` 行为一致。`new_session_defaults` 内部的 `backends.codex` / `backends.pi` 子结构亦需 parity。注意 ref 的 `/api/v1/bootstrap` 是不同 shape，**本 phase 不挂 ref 的 `/api/v1/bootstrap` 别名**（与 endpoint inventory "Must be ported in Phase 1" 条目一致）。
- **新增** `backend-rs/src/runtime.rs` 中**仅** Phase 1 所需的子集：
  - `RuntimeConfig { app_dir: PathBuf }` + `from_env()`（读 `CODOXEAR_APP_DIR` / `HOME`，与 ref 一致）。
  - `load_or_create_hmac_secret(app_dir)`（与 ref `routes.rs:1398-1427` 等价：直读 `app_dir/hmac_secret`，长度 ≥ 32 时取前 64 字节；不存在时从 `/dev/urandom` 写 64 字节、`chmod 0600`）。
  - `read_recent_cwds(path)`（解析 Python `recent_cwds.json`，按 ts 倒序、cwd 升序排序，截断到 `RECENT_CWD_MAX = 256`）。
  - `read_cwd_groups(path)`（解析 Python `cwd_groups.json`，原样返回 dict-of-dict；Phase 1 暂不实现 `_reconcile_hidden_cwd_groups` 的清理逻辑——只 read，不 write，故无 reconcile 必要；这一点在 spec scenario 里显式声明）。
  - `read_new_session_defaults()`（解析 `~/.codex/config.toml`、`~/.codex/models_cache.json`、`~/.pi/agent/settings.json`、`~/.pi/agent/models.json`，返回与 Python `_read_new_session_defaults()` 同形 JSON）。`CODEX_HOME` / Pi 路径按 Python 现状实现。
  - `tmux_available()`（`which("tmux").is_some()`，与 Python `_tmux_available()` 一致）。
- **新增** `backend-rs/src/routes.rs` 仅包含 router 注册和 3 个 handler，路由顺序：先 public `health`，再挂带 auth middleware 的 `me` / `bootstrap` 在 canonical `/api/v1/*` 与 legacy `/api/*` 双命名空间。
- **新增** `backend-rs/src/main.rs`：bind host/port 解析 `CODEX_WEB_HOST`/`CODEX_WEB_PORT` 优先、`CODOXEAR_BIND_HOST`/`CODOXEAR_BIND_PORT` fallback，默认 `[::]:8743`（与 D8 决策一致；区别于 ref 默认 `127.0.0.1:8787`）。Phase 1 **不** spawn 任何后台 worker，即使 `CODOXEAR_ENABLE_*` flag 设置为真也直接 ignore（在日志里 warn 一行"worker not implemented in phase 1"）。
- **新增** `backend-rs/tests/health_me_bootstrap.rs`：用 `tower::ServiceExt::oneshot` 对 axum router 直接发请求，覆盖 4 个 case：`/api/health` 200 + 公开访问、`/api/me` 401 无 cookie、`/api/me` + `/api/sessions/bootstrap` 200 with valid cookie（fixture 函数 sign cookie via `hmac_secret`）。
- **新增** `backend-rs/.gitignore`（`/target`），`Cargo.lock` 提交（与 ref 一致，因为是 binary crate）。
- **修改** `tests/contract/conftest.py`：`rust_server_url` fixture 从 `pytest.skip` 切换为 subprocess 启动 `backend-rs/target/release/codoxear-backend-rs`（fixture 失败 fail 而不是 skip，除非显式设置 `CODOXEAR_SKIP_RUST_FIXTURE=1`）；同 fixture 必须 share `HOME` 与 `CODEX_WEB_PASSWORD` 与 Python fixture 一致（cross-server cookie 互通由 `hmac_secret` 文件 share 提供）。
- **修改** `tests/contract/test_endpoint_parity.py`：去掉 `test_me_parity` 与 `test_sessions_bootstrap_parity` 的 `@pytest.mark.skip`；给两个测试加上"先 POST /api/login → 取 cookie → GET 端点"的真实流程（Python 与 Rust 各自跑一份，`assert_json_equivalent` 比对）。`test_sessions_parity` **保留** skip（属于 Phase 2）。
- **新增** `tests/contract/conftest.py` 共用 fixture `shared_app_home`：先建立 tmp HOME、写好 `CODEX_WEB_PASSWORD`，由 `python_server_url` 与 `rust_server_url` 共同使用；保证 `~/.local/share/codoxear/hmac_secret` 是同一份。
- **修改** `tests/contract/README.md`：补充"Phase 1 之后已经可以跑 `pytest tests/contract -k parity`"以及"如何用 `cargo build --release --bins` 构建二进制"。
- **新增** `.github/workflows/backend-rs.yml`（如果当前仓库已经有 CI，则在现有 workflow 里加 job；否则新增 workflow）。job：在 ubuntu-latest 与 macos-latest 上 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --release`、再跑 `pytest tests/contract -k parity`。
- **新增** root `.gitignore` 增加 `backend-rs/target/`。
- **新增** `README.md` 一段说明：开发依赖现包括 rustup stable >=1.78；本地开发可 `(cd backend-rs && cargo run --release)`，Phase 1 默认服务于 8743 端口（与 Python 同端口）。
- **保留** Python `codoxear/server.py` 不变；Phase 1 不动 Python 代码，没有 BREAKING。
- **不变** 用户面 `/api/me` 与 `/api/sessions/bootstrap` 的字段集、字段顺序、cookie 名 `codoxear_auth`、HMAC scheme（base64url payload + "." + base64url sig）。
- **不变** 磁盘契约：Phase 1 仅**读** `recent_cwds.json` / `cwd_groups.json` / `~/.codex/config.toml` / `~/.codex/models_cache.json` / `~/.pi/agent/settings.json` / `~/.pi/agent/models.json`；不写任何文件（除 `hmac_secret` 在文件不存在时 lazy create——这恰好与 Python 行为对齐，且发生在 server 第一次 request 之前）。

## Capabilities

### New Capabilities

<!-- 不引入新 capability。本 change 通过 spec delta 修改伞形 change 已经创建的 `rust-backend-runtime`。 -->

### Modified Capabilities

- `rust-backend-runtime`: 新增 Phase 1 范围的两组 ADDED Requirements：(a) Rust skeleton crate 的最小可运行性与默认 bind/host/port，(b) `/api/health`、`/api/me`、`/api/sessions/bootstrap` 三端点的具体行为契约（含 byte-level parity）。这两组 Requirements 作为伞形 spec 中"目标态"较粗描述的细化，**不** REMOVE 或 MODIFY 任何已有 Requirement。

## Impact

- **Affected code（直接）**：
  - 新增：`backend-rs/Cargo.toml`、`backend-rs/Cargo.lock`、`backend-rs/.gitignore`、`backend-rs/src/{lib.rs,main.rs,app_state.rs,routes.rs,runtime.rs,models.rs}`、`backend-rs/src/bin/codoxear-broker-rs.rs`、`backend-rs/tests/health_me_bootstrap.rs`。
  - 修改：`tests/contract/conftest.py`、`tests/contract/test_endpoint_parity.py`、`tests/contract/README.md`、`README.md`、`.gitignore`、CI workflow（新增或扩展现有）。
  - **未修改**：`codoxear/server.py`、`codoxear/broker.py`、`codoxear/pi_broker.py`、`codoxear/voice_push.py`、其它任何 `codoxear/*.py` 业务模块、`web/` frontend、`pyproject.toml`。
- **APIs**：新增 `/api/health` 与 `/api/v1/health` 两条 public route；新增 Rust 实现的 `/api/me` / `/api/v1/me` / `/api/sessions/bootstrap` / `/api/v1/sessions/bootstrap` 与 Python parity。Python 同名端点继续在 8743（或当前部署端口）由 Python 服务，**Phase 1 不切流**——Rust binary 是独立的、可手动启动的开发工件。
- **依赖**：新增 Rust 工具链要求（rustup stable >= 1.78）。CI 增加约 3-5 分钟 Rust build & test。Frontend / Python 依赖不变。
- **运维**：Phase 1 不改变现有 systemd unit / Tailscale Serve 端口配置。Rust binary 仅用于本地开发与 contract harness 验证；任何打算把 Rust binary 上生产的部署都属于 Phase 2 之后的范围。
- **回滚**：Phase 1 是纯 additive：(a) 不启动 Rust binary 即可完全回到 Phase 0；(b) `git revert` 本 phase 的 commit 即可移除 `backend-rs/` 目录与 contract test 改动，对 Python backend 零影响。
