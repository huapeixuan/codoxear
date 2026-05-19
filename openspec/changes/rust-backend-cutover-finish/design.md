## Context

Phase 6 的入口事实：

- Phase 1–5 已经落地 `backend-rs/` Rust server、GET/POST route parity、queue/harness/voice worker、Rust broker、OpenAI-compatible TTS/HLS/WebPush，并且 Phase 5 code-reviewer 对 PR #9 / commit `8f489b0` 给出 PASS。
- 当前仓库仍保留 legacy Python runtime surface：`codoxear/server.py`、`broker.py`、`pi_broker.py`、`sessiond.py`、`voice_push.py`、`rollout_log.py`、`pi_log.py`、`pi_rpc.py`、`agent_backend.py`、`git_context.py`、`util.py`、`pty_util.py` 以及大量 `tests/test_*.py` 直接 import Python internals。
- `pyproject.toml` 仍暴露 Python console scripts：`codoxear-server`、`codoxear-broker`、`codoxear-pi-broker`，并保留 `Pillow` / `py-vapid` / `pywebpush` 等 Python runtime dependencies。
- Rust server 已经提供 `/api/v1/*` canonical 与 `/api/*` legacy alias；Phase 6 不需要改变 frontend API，也不需要删除 `/api/*` alias。
- 伞形 `rust-backend-cutover` 明确 Phase 6 是不可通过 env flag 回滚的阶段；回滚方式是 git revert。

关键约束：

- 必须避免“删 Python 但文档/entry point/CI 仍指向 Python”的半切状态。
- 必须保留用户已有 `~/.local/share/codoxear` 状态可读，包括 Python 历史 sidecar、harness/queue/sidebar、voice ledger/subscriptions/VAPID/HLS 文件。
- 必须保留 `web/` 与 `codoxear/static/` 静态资产路径；本阶段不是 frontend 重构。
- 真实 iOS/Tailscale WebPush/HLS smoke 在 Phase 5 是 reviewer-known manual pending；Phase 6 不把它变成代码删除阻塞，但交接必须继续标明。

## Goals / Non-Goals

**Goals:**

- 删除或 fail-closed legacy Python backend / broker / voice / sessiond runtime surface。
- 将 Rust binaries 设为唯一 server/broker 入口，并让 README/AGENTS/CI/安装说明反映 Rust-only 目标态。
- 移除 Python backend console scripts 和 runtime dependencies，或将 Python packaging 明确收窄为静态资产/非 runtime tooling。
- 清理/迁移依赖 Python backend internals 的测试，最终 verification 以 Rust build/test、contract tests、web build/test、OpenSpec validation 为准。
- 补齐 cutover log 与 umbrella tasks，给 code-reviewer 一个可审查的 Phase 6 final state。

**Non-Goals:**

- 不删除或弃用 `/api/*` legacy alias；兼容 alias 仍由 Rust server 提供。
- 不重构 `web/` UI，不改变用户可见功能、字段、样式。
- 不在 Phase 6 继续大规模重构 Rust module layout；`runtime.rs`/各 handler 结构的整理留给 follow-up `rust-backend-modularize`。
- 不引入新的发布系统或预编译 binary distribution；如需要 Homebrew/GitHub Releases，另开 change。
- 不以 env flag 维持 Python fallback；Phase 6 后 fallback 是 git revert。

## Decisions

### D1：删除 Python runtime 文件优先于保留兼容 stub

**选择**：主要 Python backend 文件应直接删除；只有当 packaging/import 约束强制需要时，才允许保留 fail-closed stub，且 stub 只能输出“Rust-only”迁移信息，不可继续提供 runtime 行为。

**原因**：issue 目标是“删除 legacy Python backend surface”。保留可运行 stub 会继续制造误用空间，也会让 grep/测试无法判断是否真的 cutover。

**替代方案**：保留 Python wrappers 调用 Rust binary。否决：这仍是 Python runtime surface，且会隐藏部署错误；如果用户要 shell wrapper，应在 README 中写 shell function 或 systemd unit，而不是 Python module。

### D2：`pyproject.toml` 不再暴露 Python server/broker scripts

**选择**：删除 `[project.scripts]` 中 `codoxear-server`、`codoxear-broker`、`codoxear-pi-broker`；Python package 若保留，只用于 `codoxear/static` package data 或开发辅助，不作为 backend 入口。

**原因**：console scripts 是用户最容易继续运行 legacy backend 的路径。Phase 6 必须从安装面消除旧入口。

**替代方案**：保留 scripts 但打印 deprecated。否决：和 D1 一样，容易让 ops 误以为 Python 仍是支持入口；而且 scripts import 已删除模块会变成不受控错误。

### D3：保留 `/api/*` legacy alias，删除的是 Python legacy surface，不是 HTTP alias

**选择**：Phase 6 不移除 `/api/*` legacy alias；`/api/v1/*` canonical 与 `/api/*` alias 继续并存。

**原因**：issue 范围要求“确认 `/api/v1/*` canonical 与 `/api/*` legacy alias 在最终态下仍满足兼容要求，或明确归档/弃用策略”。当前 frontend 和 contract tests 仍依赖 alias；删除 alias 是外部 API breaking change，不是 Python cutover 必需条件。

**替代方案**：同时删除 `/api/*` alias。否决：扩大 scope，会把 backend implementation cutover 变成 API deprecation，需要额外用户迁移和版本策略。

### D4：`sessiond.py` 在 Phase 6 删除，不迁移为单独 Rust daemon

**选择**：删除 `codoxear/sessiond.py`，不保留 headless Python launcher。Web-owned/headless session 创建统一通过 Rust server `POST /api/sessions` + `codoxear-broker-rs`；terminal-owned session 统一通过 `codoxear-broker-rs` wrapper。

**原因**：Phase 4/5 已经覆盖 Rust broker 和 server create paths；`sessiond.py` 的 sidecar 读取兼容可以保留在 Rust parser 中，但无需继续提供 Python launcher。

**替代方案**：先迁移 `sessiond` 到 Rust binary。否决：没有当前强需求，且会新增第三个 runtime entry point；如后续确有 headless daemon 需求，可基于 Rust broker另开 change。

### D5：测试清理以“行为覆盖迁移”而不是“一刀切删所有 Python tests”为准

**选择**：删除 import Python backend internals 的 tests；若某些 tests 覆盖的是仍然重要的用户行为（static assets、URL prefix、contract parity、voice disk compatibility），应迁移到 Rust integration/contract tests 或保留为不 import deleted modules 的黑盒 tests。

**原因**：直接删所有 Python tests 会丢行为覆盖；但保留 direct import 会阻止 Python removal。标准是是否测试目标态行为，而不是测试语言。

**替代方案**：保留 Python tests 并改成跳过。否决：跳过会掩盖覆盖空洞；Phase 6 需要干净、可执行的 final suite。

### D6：CI 最终以 Rust + contract + web 为主，Python 只用于 pytest harness/packaging时显式安装

**选择**：更新 `.github/workflows/backend-rs.yml` 和本地验证命令：Rust fmt/clippy/test/build 是核心；`tests/contract` 若仍使用 pytest，只把 Python 当测试 runner，不当 backend runtime；web 仍跑 `npm run build`。

**原因**：Phase 6 之后 Python 不是产品 runtime，但 pytest 仍可作为测试工具存在。CI 命名与安装步骤要避免“install Python backend dependencies”这类过时说法。

**替代方案**：完全移除 pytest。否决：contract harness 已经存在且有价值；是否迁移到纯 Rust 可后续处理。

## Risks / Trade-offs

- **[Risk] 删除 helper 过度，Rust 仍有测试/脚本间接依赖 Python fixtures** → **Mitigation**：先跑 `rg 'codoxear\.(server|broker|pi_broker|voice_push|rollout_log|pi_log|pi_rpc|agent_backend|git_context|util|pty_util)' tests backend-rs scripts docs`，逐项迁移或删除；最终 `python -m pytest` 只跑仍合法的 harness。
- **[Risk] `pyproject.toml` 收窄后 `pip install -e .` 不再提供前端静态文件测试所需 package metadata** → **Mitigation**：若需要保留 wheel，只保留 `codoxear/static` package data 并确保 static asset tests/README 以此为准；不保留 runtime scripts。
- **[Risk] 用户 shell rc 中仍有 `codox` / `piox` 指向 `codoxear-broker`** → **Mitigation**：README/AGENTS 明确更新 wrappers 到 `codoxear-broker-rs --`; release notes 标为 breaking。
- **[Risk] Phase 6 删除 Python fallback 后现场回滚心理模型错误** → **Mitigation**：文档与 cutover log 显式写“rollback = git revert Phase 6 commit(s) to Phase 5 PASS baseline `8f489b0` or descendant”。
- **[Risk] `docs/superpowers/plans/*` 历史计划大量引用 Python 文件，导致 grep 误判** → **Mitigation**：不要求改写历史计划；final grep gate 只针对 runtime docs (`README.md`, `AGENTS.md`, `docs/cutover`, active OpenSpec) 和 active code/tests。历史 docs 可以保留为归档事实，但不能作为 quick start。
- **[Trade-off] 删除 Python package scripts 可能让 `pip install .` 用户失去入口** → 接受；Rust binary 是目标态。后续可提供 cargo install / release artifact。

## Migration Plan

1. **Pre-flight**：基于 Phase 5 PASS commit `8f489b0` 或其 descendant；运行 `openspec validate rust-backend-cutover --strict`、`openspec validate rust-backend-voice-push --strict`、Rust focused tests，确认不是从失败基线切入。
2. **Inventory active Python surface**：列出 `codoxear/*.py`、`tests/test_*.py` import graph、`pyproject.toml` scripts/dependencies、README/AGENTS/scripts/CI 对 Python runtime 的引用。
3. **Delete runtime modules**：删除 server/broker/pi/sessiond/voice and owned helpers；保留 `codoxear/static/`、`__init__.py`、必要 package metadata；若 helper 已被 Rust tests/fixtures 取代则同步删除。
4. **Entry point and docs cutover**：更新 `pyproject.toml`、README、AGENTS、docs/cutover/cutover-log.md（不存在则创建）、umbrella tasks Phase 6 row。
5. **Test migration/cleanup**：删除 Python-internal unit tests；迁移关键 behavior 到 Rust tests/contract tests；更新 CI selectors。
6. **Verification**：运行 OpenSpec validation、`cargo fmt/clippy/test/build --release --bins`、contract tests、web tests/build、static asset checks、grep gates、smoke。
7. **Review handoff**：切 `in_review` 给 code-reviewer；明确真实 iOS/Tailscale WebPush/HLS smoke 仍是人工 pending（若未补跑），但 Phase 6 代码不再扩大该风险。

Rollback：Phase 6 无 env flag rollback。若发现 regression，执行 git revert Phase 6 commit(s)，回到 Phase 5 PASS baseline；然后重新运行 Phase 5 verification，必要时重新启用 Python fallback。

## Open Questions

1. Python package 是否彻底删除还是保留 static-assets-only wheel？建议 coding-agent 以最小可运行发布路径决定：如果当前安装/测试仍依赖 package data，则保留 static-assets-only；否则文档转为 cargo build/run。
2. 是否在 Phase 6 同时建立 `cargo install --path backend-rs` 或 workspace-level Cargo manifest？建议只在不扩大 scope 的情况下做；发布工程化可后续。
3. 历史 `docs/superpowers/plans/*` 是否批量改写？建议不改，避免破坏历史记录；只确保 README/AGENTS/docs/cutover/active OpenSpec 不再指导运行 Python backend。
