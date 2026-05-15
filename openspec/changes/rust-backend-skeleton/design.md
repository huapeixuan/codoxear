## Context

伞形 change `rust-backend-cutover` 已经在 design.md 里固定了 cutover 的 7 阶段路线、`backend-rs/` 模块 layout、env flag 命名、disk byte-level 兼容、cookie/HMAC 对称、port 8743 默认值等高层决策（D1–D8）。本 phase 是把"目标态契约"落到一段最小可执行 Rust 代码上：第一行 `cargo run`、第一次 axum 接到请求、第一份 `hmac_secret` 被 Rust 创建、第一次 contract test 看到 Python 与 Rust 的 JSON 一致。

为什么"最小"≠"最少思考"？这一 phase 必须一次性把后续所有 phase 都要复用的基建打通：

1. **Cargo.toml 依赖清单**——选错版本会让 Phase 4/5 改 web-push、reqwest 时被迫整体升级。本 phase 的 Cargo.toml 应当是 ref 当前 Cargo.toml 的**严格子集**（行数更少，但每个保留行的版本号 byte-level 相同），这样 Phase 2/3/4/5 可以无脑加行。
2. **HMAC cookie 解析**——`/api/me` 必须能读 Python 已发出的 cookie；这要求 Rust 用与 Python 完全相同的 secret 文件、相同的 base64url 实现、相同的 HMAC-SHA256 流程。这是 Phase 1 第一个 cross-language byte-level 兼容点；做错的话整套 contract harness 就跑不起来。
3. **`new_session_defaults` 解析**——Python 实现里有 `tomllib` 解析 `~/.codex/config.toml`、`json` 解析 `models_cache.json` / Pi 文件，且对 `provider_choices`、`reasoning_efforts`、`supports_fast` 等字段的拼装顺序有讲究（直接影响 frontend 渲染顺序）。这一段是 Phase 1 最容易"看起来对、实际不一致"的地方，必须有 contract test 兜底。
4. **contract harness 的 fixture 设计**——Phase 1 之后，`tests/contract/test_*.py` 会快速增长到 60+ 测试。如果 Phase 1 把 fixture 设计错（比如让 Python 与 Rust 跑不同 HOME），后续每个 phase 都要重写 fixture。

不在本 phase 处理但需要"为以后留好钩子"的事情：

- broker 启动（Phase 4）：Phase 1 的 `codoxear-broker-rs` bin 是 stub，但 binary name / Cargo target name / `pyproject.toml` 入口名都已经是最终值。
- 后台 worker（Phase 2/3）：Phase 1 的 `main.rs` 已经在 startup 时检查 `CODOXEAR_ENABLE_*` flag 并 log warn；只是 spawn 函数体是 `unimplemented!`-style noop。
- broker / voice / sweep modules（Phase 4/5）：Phase 1 **不**在 `src/` 创建 `broker.rs` / `voice_worker.rs`，避免在 Phase 1 引入死代码；但 `lib.rs` 留下注释 `// pub mod broker; // added in Phase 4` 让后续添加是 1 行 diff。

## Goals / Non-Goals

**Goals:**

- 建立 `backend-rs/` Rust crate 并能 `cargo build --release --bins` 通过；产出二进制 `codoxear-backend-rs`（可启动 axum server）和 `codoxear-broker-rs`（stub，运行即退出）。
- 实现三个端点 `/api/health`、`/api/me`、`/api/sessions/bootstrap` 的 axum handler，挂在 canonical `/api/v1/*` 与 legacy `/api/*` 双命名空间。
- 实现 cookie 验证 middleware（与 Python `_require_auth` byte-level 对齐），让 Rust 能识别 Python 写的 cookie，反之亦然。
- 实现 `hmac_secret` 文件 lazy-create 与读取，与 Python `_load_or_create_hmac_secret` 行为一致（包括 64-byte 长度、`/dev/urandom` 来源、`0o600` 权限）。
- 实现 `recent_cwds.json` / `cwd_groups.json` / `~/.codex/config.toml` / `~/.codex/models_cache.json` / `~/.pi/agent/settings.json` / `~/.pi/agent/models.json` 文件的 read-only 解析，得出与 Python 完全一致的 `bootstrap` 响应。
- 让 contract harness 中 `test_me_parity` 与 `test_sessions_bootstrap_parity` 真正跑起来并 PASS（之前是 `pytest.skip`）。
- CI 增加 `backend-rs` 的 `cargo build` + `cargo test --release` + `pytest tests/contract -k parity` job，覆盖 ubuntu-latest 与 macos-latest。

**Non-Goals:**

- 不实现 ref 的 `/api/v1/bootstrap`（不同 shape）；只实现 target 的 `/api/sessions/bootstrap`。
- 不实现任何 GET 列表（sessions、queue、harness、git、file、messages…）——这是 Phase 2。
- 不实现任何 POST 写（含 `/api/login`/`/api/logout`，**虽然 contract test 需要它**，见 Decision D-AUTH）。
- 不实现 broker、voice、sweep、HLS、WebPush。
- 不修改 Python 任何 `codoxear/*.py`。
- 不切流：Rust binary 不会替代 Python systemd unit；现有部署完全不动。
- 不引入 Cargo workspace 多 crate 拆分；只是单 binary crate。
- 不实现 `runtime.rs` 的 11k LOC 全部内容；只实现 Phase 1 必需的 ~5 个公开函数。
- 不实现 `voice_worker.rs` / `broker.rs` 源文件（`Cargo.toml` 也不引入相应依赖）。
- 不实现 frontend 静态资源服务（`/`、`/static/*`、`/assets/*`、`/manifest.webmanifest`、`/service-worker.js` 等）。Phase 6 之前 Python 才是 frontend 服务的来源。
- 不引入 `CODOXEAR_USE_LEGACY_WEB` 等 env 行为开关。

## Decisions

### D-AUTH：Phase 1 不实现 `/api/login`，但 contract test 需要 cookie——用"测试侧手工 sign cookie"绕过

**问题**：`test_me_parity` 与 `test_sessions_bootstrap_parity` 的端点都受 auth 保护。Python fixture 启动后，测试会 `POST /api/login`（password 来自 `CODEX_WEB_PASSWORD` fixture）拿到 cookie。Rust 这边 Phase 1 还**没有** `/api/login` handler。

**选择**：在 `tests/contract/conftest.py` 增加一个 fixture `signed_auth_cookie(shared_app_home)`：

- 读取 `<shared_app_home>/.local/share/codoxear/hmac_secret`（让 Python fixture 启动时把它创建好；或者由 fixture 自己用纯 Python 复现 `_load_or_create_hmac_secret`）；
- 用 `hmac.new(secret, json.dumps({"exp": ...}, separators=(",", ":"), ensure_ascii=False).encode(), hashlib.sha256)` 生成 cookie；
- 测试时直接把这个 cookie 设到 request 上，**不**经过 `/api/login`。

这样 Phase 1 的 Rust 不需要实现 login，也能通过 contract test。Phase 3 实现 `/api/login` 之后，再把 fixture 切到"通过 login 流程取 cookie"作为更接近真实用户行为的测试。

**为什么**：

- Phase 1 范围必须严格"3 个 read-only 端点"。如果把 login 也塞进 Phase 1，那么 Phase 3 的 OpenSpec 范围就要重新切（login 已经实现了），契约 test 重复。
- "test 自己 sign cookie" 是 Python 与 Rust **同一份 hmac_secret 必然产生同一份 cookie** 的 byte-level 验证；如果 Rust 不能解析 Python signed cookie，contract test 直接红——这其实是更强的 cross-language compatibility check。
- ref 的 routes test (`bootstrap_tmux.rs`) 也是直接构造 request header；不走 login。

**替代方案**：在 Phase 1 顺手实现 `POST /api/login` + `/api/logout`。**否决**理由：扩大 Phase 1 范围；Phase 3 的 OpenSpec 已经把这些归类为 write-routes，重新切分会引发 spec churn。

### D-BIND：默认 bind `[::]:8743`（Python 现状）而非 ref 的 `127.0.0.1:8787`

**选择**：

```rust
let host: IpAddr = env::var("CODEX_WEB_HOST")
    .or_else(|_| env::var("CODOXEAR_BIND_HOST"))
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or_else(|| IpAddr::from([0, 0, 0, 0, 0, 0, 0, 0]));
let port: u16 = env::var("CODEX_WEB_PORT")
    .or_else(|_| env::var("CODOXEAR_BIND_PORT"))
    .ok()
    .and_then(|v| v.parse().ok())
    .unwrap_or(8743);
```

`CODEX_WEB_HOST` / `CODEX_WEB_PORT` 优先（Python 现状），`CODOXEAR_BIND_HOST` / `CODOXEAR_BIND_PORT` fallback（与 ref 兼容）。默认 `[::]:8743`。

**为什么**：与伞形 design D8 一致；任何把 Rust binary 部署到现有 systemd unit 上的用户都不需要改 env。Phase 1 在本地开发时，开发者通常会把 Python stop 掉、用 Rust binary 占用 8743；contract harness 跑随机端口、不依赖默认值。

**替代方案**：保持 ref 默认 `127.0.0.1:8787`。**否决**：与伞形决策冲突。

**陷阱**：`CODEX_WEB_HOST=::` 是合法的 Python 现状；Rust 解析 IPv6 字面量必须支持。fixture 一律用 `127.0.0.1` 避免 IPv6/IPv4 双栈问题。

### D-DEPS：Cargo.toml 是 ref Cargo.toml 的**严格子集**，按版本号 1:1 锁定

**选择**：本 phase Cargo.toml 仅引入：

```toml
[dependencies]
axum = { version = "0.7.9", features = ["macros", "json"] }
base64 = "0.22.1"
hmac = "0.12.1"
indexmap = "=2.2.6"
serde = { version = "1.0.228", features = ["derive"] }
serde_json = "1.0.145"
sha2 = "0.10.9"
tokio = { version = "1.36.0", features = ["full"] }
toml = "=0.8.8"
tower = { version = "0.5.2", features = ["util"] }
tower-http = { version = "0.5.2", features = ["cors", "trace"] }
tracing = "0.1.41"
tracing-subscriber = { version = "0.3.20", features = ["env-filter"] }

[dev-dependencies]
http-body-util = "=0.1.3"
hyper = { version = "=1.9.0" }
```

**剔除**：`web-push` (Phase 5)、`reqwest` (Phase 4 broker / Phase 5 voice 才用)、`openssl`/`ed25519-compact` (Phase 5)、`bytes`/`hyper-util`/`base64ct`/`cpufeatures`/`idna_adapter`/`futures-util`/`tokio-stream`/`libc` (Phase 2/3/4 才用)。`hyper` 仅作 dev-dependency（用 `http-body-util` 测试 axum router）。

**为什么**：

- Phase 1 引入的每个依赖都增加 cargo build 时间和 supply chain 风险；只引入"现在用得上"的。
- 加依赖永远比删依赖容易；Phase 2/3/4/5 需要时再加，相应 phase 的 OpenSpec 会显式说明。
- 锁定具体 patch 版本（与 ref Cargo.toml 一致），让 reviewer 可以 `diff backend-rs/Cargo.toml /tmp/san-tian/Cargo.toml` 一眼看出"只缺这些行"。

**陷阱**：`indexmap = "=2.2.6"` 与 `toml = "=0.8.8"` 是 ref 锁的精确版本（`=` 前缀）；如果不锁可能被 cargo 解到不兼容的更新版（`toml` 0.9.x 不向后兼容 0.8.x 的 `Value` API）。

**Trade-off**：Phase 1 的 `Cargo.lock` 也会比 ref 短（因为依赖少）；Phase 2/3/4/5 加依赖时 lockfile 会膨胀。可接受。

### D-ROUTE：路由注册顺序与 ref 完全一致，便于后续 phase 拷贝

**选择**：`src/routes.rs` 的 `pub fn router(state: AppState) -> Router` 内部即使 Phase 1 只挂 3 个端点，也按 ref `routes.rs:49-164` 的相对顺序注册。具体：

```rust
pub fn router(state: AppState) -> Router {
    Router::new()
        // Static / nova-preview routes — added in Phase 6 / removed entirely;
        // Phase 1 leaves no placeholder.
        .route("/api/v1/health", get(health))
        // .route("/api/v1/bootstrap", get(bootstrap)) // ref-only path; not in target
        .route("/api/v1/me", get(me))
        .route("/api/v1/sessions/bootstrap", get(sessions_bootstrap))
        .nest("/api", public_api_router(state.clone()))
        .with_state(state)
}

fn public_api_router(state: AppState) -> Router<AppState> {
    let protected = Router::new()
        .route("/me", get(me))
        .route("/sessions/bootstrap", get(sessions_bootstrap))
        .route_layer(middleware::from_fn_with_state(state, require_public_api_auth));
    Router::new()
        .route("/health", get(health))
        .merge(protected)
}
```

**为什么**：

- Phase 2/3/4 的 spec 直接对照 endpoint inventory 添加新行，diff 是"线性追加"。
- ref 的 `nest("/api", ...)` + 把 `/health` 放在 protected 外面的模式正好匹配 Python `_require_auth` 的"login/logout/health 公开，其余都需要 auth"语义。
- legacy `/api/*` 与 canonical `/api/v1/*` 的双挂载策略与伞形 D2 一致。

**替代方案**：所有路由都直接 `.route("/api/v1/...")` 与 `.route("/api/...")` 双行注册（不用 `nest`）。**否决**：Phase 2 添加 GET 列表时会扩到 ~60 行的"两段对称",维护成本高。`nest` 的写法是 ref 已经验证的。

**陷阱**：axum 的 `route_layer` 与 `layer` 不同：`route_layer` 仅对当前 Router 中的 route 生效，merge 之后不传染。Phase 1 必须用 `route_layer`，否则 `/api/health` 会被 auth 拦住。

### D-DATA：`new_session_defaults` 实现策略——直读 + 容错，但**不**做 Python 的"raise ValueError"行为

**问题**：Python `_read_codex_launch_defaults()` 在 `models_cache.json` 不存在时返回 `defaults["reasoning_effort"] = None`，但在 `models_cache` 存在但 schema 错误时 raise `ValueError`。raise 会冒泡到 `do_GET`，由 `BaseHTTPRequestHandler` 默认 500 响应。

**选择**：Rust `read_new_session_defaults()` 把 raise 改为 `Result<Value, String>`，handler 把 `Err` 转换成 `(StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e})))`。**JSON body 与 Python 500 不一致**——Python 500 是 plain text "Internal Server Error"，Rust 是 `{"error": "..."}`。

**接受这个不一致**，理由：

- 在干净的 contract test HOME 下，所有 source 文件都不存在，走的是 happy path（returns defaults，无 error），parity 100%。
- 真实生产只有 `models_cache.json` 损坏才触发；Python 当前的"500 plain text"是 bug，Rust 的"500 JSON"更好，frontend 也已经 fault-tolerant 处理任意 5xx。
- 把这个不一致显式写进 spec scenario "Bootstrap 错误响应：JSON body 而非 plain text"，让 reviewer 看到。

**为什么**：

- 严格 byte-level parity 要求 Rust 模拟 Python `BaseHTTPRequestHandler.send_error()` 的 HTML body，太重。
- contract test 的 happy path（空 HOME）已经 cover Phase 1 的实际验证目标。

### D-FIXTURE：contract harness 用 `shared_app_home` fixture 让 Python + Rust 共享磁盘状态

**问题**：之前的 `python_server_url` fixture 自己 mktemp HOME，Rust fixture 再 mktemp 一份，两个 server 看到不同的 `recent_cwds.json`，bootstrap 输出会发散。同时 Rust 与 Python 的 cookie 互通需要共享 `hmac_secret`。

**选择**：

```python
@pytest.fixture
def shared_app_home() -> Iterator[Path]:
    with tempfile.TemporaryDirectory(prefix="codoxear-contract-home-") as home:
        yield Path(home)

@pytest.fixture
def python_server_url(shared_app_home: Path) -> Iterator[str]:
    # spawn Python server with HOME=shared_app_home
    ...

@pytest.fixture
def rust_server_url(shared_app_home: Path) -> Iterator[str]:
    # spawn Rust binary with HOME=shared_app_home
    ...
```

两个 fixture 都使用同一个 `shared_app_home`。函数内部各自启动子进程。

**为什么**：

- 同一个 `~/.local/share/codoxear/` 让 cookie / recent_cwds / cwd_groups / hmac_secret 都是同一份；testing 真实接近"用户先用 Python，再用 Rust 看到一致状态"的场景。
- pytest 的 fixture 依赖图天然支持"两个 fixture 共享上游 fixture"。
- Phase 1 已经测出"shared HOME 但两个进程"模式可不可行；如果有问题（端口冲突、Python startup 占用 cookie 文件 lock），Phase 1 解决，后续 phase 不再踩坑。

**陷阱**：Python `_load_or_create_hmac_secret()` 在 import time 就跑（`HMAC_SECRET = _load_or_create_hmac_secret()` at module top）；测试启动 Python 之前，Rust 不能先创建 `hmac_secret`，否则两边可能竞争（实际上 lazy-create 是幂等的，先 create 的赢；后 read 的拿到同一份 secret——所以反而没问题）。spec scenario 显式声明"任一 server 第一次 request 时 lazy-create hmac_secret，第二个 server 必须 read 而不是 overwrite"。

### D-TEST-CONTRACT：测试 fixture 不强制构建 Rust binary，但 fail-fast 提示

**问题**：Phase 1 之后，开发者在没装 rustup 的机器上跑 `pytest tests/contract` 会 fail。是 skip 还是 fail？

**选择**：默认 fail。如果 binary 不存在，fixture raises `RuntimeError("backend-rs binary not found at <path>; run (cd backend-rs && cargo build --release --bins) first")`。除非环境变量 `CODOXEAR_SKIP_RUST_FIXTURE=1` 显式 set，才 `pytest.skip()`。

**为什么**：

- 默认 fail 强迫 CI 把 Rust 工具链装好；防止"contract test 默默 skip 几个月"导致 cutover phase 之间漂移。
- 提供 escape hatch（env var），让本地只想跑 Python 单元测试的开发者不被 contract test 阻塞。

### D-CI：用 `Swatinem/rust-cache@v2` action 缓存 cargo registry 与 target

**选择**：CI workflow（如 `.github/workflows/backend-rs.yml`）：

```yaml
jobs:
  rust:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy, rustfmt
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: backend-rs
      - name: cargo fmt
        run: cd backend-rs && cargo fmt --all -- --check
      - name: cargo clippy
        run: cd backend-rs && cargo clippy --all-targets -- -D warnings
      - name: cargo test
        run: cd backend-rs && cargo test --release
      - uses: actions/setup-python@v5
        with: { python-version: "3.12" }
      - name: pip install
        run: pip install -e .[dev]   # adjust per pyproject.toml extras
      - name: contract tests
        run: pytest tests/contract -k parity
```

**为什么**：与伞形 design Open Question 3 给出的建议（`Swatinem/rust-cache@v2`）一致；macos-latest 必须包含因为 broker phase 4 测试要在 macOS 跑（与伞形 risk 一致），Phase 1 提前打通 macOS runner 配置可以暴露 toolchain 问题。

**Trade-off**：matrix 翻倍 CI 时长。可接受。

### D-WORKER-NOOP：Phase 1 main.rs 检测 `CODOXEAR_ENABLE_*` flag 并 log warn，不实现 spawn

**选择**：

```rust
if env::var("CODOXEAR_ENABLE_HARNESS_SWEEP").map(|v| !is_falsy(&v)).unwrap_or(false) {
    tracing::warn!("CODOXEAR_ENABLE_HARNESS_SWEEP=1 but harness sweep is not implemented in Phase 1; ignoring");
}
// same for CODOXEAR_ENABLE_QUEUE_SWEEP, CODOXEAR_ENABLE_VOICE_SCAN, CODOXEAR_ENABLE_VOICE_WORKER
```

`is_falsy(s)` 与 ref `runtime.rs:1684-` 一致：empty / "0" / "false"（case-insensitive）为 falsy。

**为什么**：

- 让 Phase 1 binary 在生产意外被启动（带着 sweep flag）也不会出现"Rust 假装跑了 sweep 实则没有，Python 看到 flag 让出，结果谁都不跑"的双盲。
- log warn 让运维立刻发现配置不对。
- spec scenario 显式声明这个行为。

### D-MODELS：models.rs 只放 BootstrapResponse 等 Phase 1 用到的 DTO

**选择**：`backend-rs/src/models.rs` 只定义 Phase 1 端点用到的 serde 类型：

- `BootstrapResponse { recent_cwds: Vec<String>, cwd_groups: Map<String, CwdGroupEntry>, new_session_defaults: NewSessionDefaults, tmux_available: bool }`
- `CwdGroupEntry { label: Option<String>, collapsed: bool, hidden: bool, hidden_after_live_start_ts: Option<f64> }`
- `NewSessionDefaults { default_backend: String, backends: Backends }`
- `Backends { codex: CodexDefaults, pi: PiDefaults }`
- `CodexDefaults { ... }` 与 `PiDefaults { ... }`：与 Python 实际输出 1:1。

**陷阱**：Python `cwd_groups` value 用 dict[str, dict[str, Any]]，serde 不可直接 derive 出 `IndexMap` 保持插入顺序——必须用 `serde_json::Map`。fields 用 `#[serde(skip_serializing_if = "Option::is_none")]` 保证缺失字段不写 `null`，与 Python `_cwd_group_entry()` 行为一致。

**为什么**：

- 强类型让 Phase 1 的 contract test 失败时 stack 指向 deserialize 错误，定位快。
- Phase 2 加 sessions list 等 DTO 时，每加一个就在 models.rs 增量加一段；ref `models.rs` 到 313 行，Phase 1 应在 ~80 行。

## Risks / Trade-offs

- **[Risk] `_read_codex_launch_defaults()` 与 `_read_pi_launch_defaults()` 的细节行为复杂**——Python 实现里有"如果 model 在 cache 里找到则用其 default_reasoning_level；找不到则按 priority 排序取第一个"等隐式逻辑。Rust 必须 1:1 复刻，否则 bootstrap parity test 会发散。**Mitigation**：
  - tasks §3 强制要求实现前先把 Python 行为写成"伪代码 + 单元测试输入/输出对"，再翻译成 Rust。
  - contract test 的 `shared_app_home` 默认是空，走 happy path（无 codex config / no models cache → 全 None default）。
  - 增加一个 `backend-rs/tests/launch_defaults.rs` unit test，用 fixture toml/json 文件覆盖 5 个场景（无 config、有 model 在 cache、有 model 不在 cache、Pi 有 settings、Pi 无 settings）。
- **[Risk] Cookie 互通**——Python `json.dumps({"exp": ...}, separators=(",", ":"), ensure_ascii=False)` 与 Rust `serde_json::to_vec(&json!({"exp": ...}))` 字节序列必须完全一致。`serde_json` 默认 compact、无空格、key 顺序按 insertion——需要确认 `json!` 宏在 single-key map 上确实输出 `{"exp":N}` 不带空格。**Mitigation**：增加 `backend-rs/tests/auth_cookie_parity.rs`，construct cookie via Rust，verify via Python pytest（或反之），覆盖 Python ↔ Rust 双向验证。
- **[Risk] macOS runner 上 cargo build 慢**——首次缓存填充约 5-10 分钟。**Mitigation**：D-CI 已用 `Swatinem/rust-cache@v2`；后续 phase 加依赖时缓存失效成本可控。
- **[Risk] Rust binary 启动时 import-time `HMAC_SECRET = _load_or_create_hmac_secret()` 风格的 panic** ——Rust 的 lazy-create 必须在第一个 request 来之前完成，否则首个 request 会触发文件 IO 并 race。**Mitigation**：`build_state()` 在 `main.rs` 里调用，里面预 create app_dir / hmac_secret；request handler 只 verify。
- **[Trade-off] `tests/contract` 需要 Python 与 Rust 同时跑**——CI 时长增加。可接受。
- **[Trade-off] Phase 1 的 Rust binary 实现的功能 < 5%**——但建好的 hmac/cookie/runtime/models/routes/test 框架是 100% 复用的；后续 phase 只填业务逻辑，不重复造轮子。
- **[Risk] `tomllib` 在 Rust 端的等价物——`toml` crate 0.8.8 与 Python tomllib 对个别 edge case（数值精度、datetime、key 命名）解析不一致**。**Mitigation**：Phase 1 实际使用的字段（`model`、`model_reasoning_effort`、`preferred_auth_method`、`model_provider`、`model_provider_id`、`service_tier`）都是 string；不涉及 toml 高级特性。

## Migration Plan

Phase 1 是 additive，没有数据迁移。

- 部署：不部署到生产。本 phase 仅产出 binary 与 contract test。
- 启用条件：开发者 / CI 手动 `(cd backend-rs && cargo run --release)`。生产 systemd unit 不动。
- 灰度策略：n/a。
- 撤销：删 `backend-rs/` 目录、revert `tests/contract/conftest.py` 与 `test_endpoint_parity.py` 的本 phase 改动。Python backend 完全不受影响。

## Definition of Done

完成标准（与 tasks.md 末尾 §6 验证步骤一致）：

1. `cd backend-rs && cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings` 全绿。
2. `cd backend-rs && cargo test --release` 全绿（包括 `health_me_bootstrap.rs`、`auth_cookie_parity.rs`、`launch_defaults.rs` 三个 integration test）。
3. `cd backend-rs && cargo build --release --bins` 产出 `target/release/codoxear-backend-rs` 与 `target/release/codoxear-broker-rs`。
4. `pytest tests/contract -q -k parity` 跑 `test_me_parity` 与 `test_sessions_bootstrap_parity` 都 PASS（`test_sessions_parity` 仍 skipped）。
5. `openspec validate rust-backend-skeleton --strict` PASS。
6. CI workflow（ubuntu-latest + macos-latest）双跑全绿。
7. README 更新；endpoint inventory 的 `Ref-only routes not present in huapeixuan Python baseline` 顶部 `/api/health` 行后追加注释 "implemented by `rust-backend-skeleton` Phase 1"。
8. `backend-rs/src/runtime.rs` 单文件 < 800 行（Phase 1 实际预期 ~250-400 行；伞形 D6 的 10k LOC 例外仅适用于 cutover-finish 之后的 `runtime.rs`）。

## Open Questions

1. **Phase 1 是否在 `pyproject.toml` 加任何 entry point？**
   **建议**：不加。Phase 1 的 Rust binary 仅供本地开发与 contract test，不是 user-facing CLI；进 PATH 的方式由用户手动 `cargo install --path backend-rs` 或者直接用 `target/release/...`。Phase 4 broker port 时再决定 `pyproject.toml`。

2. **`backend-rs/Cargo.lock` 的 supply chain check（`cargo audit` / `cargo deny`）放在 Phase 1 还是 Phase 2？**
   **建议**：Phase 2。Phase 1 依赖少（< 20 个 transitive crate），audit 价值低；Phase 2 加 sessions 列表后才有真实 attack surface。但 CI 现在加 `cargo audit` 也不会报错，作为可选 task 列出。

3. **是否需要 `axum::serve` 之外的 graceful shutdown handling（SIGTERM 接到后等待 in-flight request）？**
   **建议**：Phase 1 不做。3 个端点都是 < 100ms 响应，没有 long-poll；systemd kill -TERM → axum panic → 进程退出，对可见行为无影响。Phase 2 引入 messages/live SSE 时再考虑。
