## Why

Phase 1–5 已经把 Rust server、读写路由、worker handoff、broker、voice/HLS/WebPush/TTS 路径实现并通过 code-reviewer；继续保留 Python backend surface 会让两套入口、依赖和文档并存，削弱最终目标态并增加运维误用风险。本 change 是不可通过 env flag 回滚的最终 cutover：把运行入口与文档切到 Rust-only，并删除 legacy Python backend / broker / voice fallback。

## What Changes

- **BREAKING** 删除 legacy Python backend surface：`codoxear/server.py`、`codoxear/broker.py`、`codoxear/pi_broker.py`、`codoxear/sessiond.py`、`codoxear/voice_push.py` 以及只服务这些模块的 Python helper / tests 不再作为运行入口存在。
- 将 `codoxear-backend-rs` 与 `codoxear-broker-rs` 设为唯一受支持的 server / broker 入口；README、AGENTS、安装与启动说明全部改为 Rust-first / Rust-only。
- 停用 Phase 1–5 的 Python fallback 文档与入口：不再承诺通过 unset `CODOXEAR_RUST_BROKER_BIN` 或关闭 `CODOXEAR_ENABLE_*` 回到 Python backend；Phase 6 后的回滚方式是 git revert 到 Phase 5 PASS commit。
- 保留前端和运行时磁盘契约：`web/`、`codoxear/static/`、`~/.local/share/codoxear` 下 session sidecar、queue/harness/sidebar、voice settings/subscriptions/ledger/HLS/VAPID 文件继续按 Rust 实现读写。
- 保留用户可见 HTTP API compatibility：`/api/v1/*` canonical 和 `/api/*` legacy alias 都继续由 Rust server 提供；本阶段不弃用 `/api/*` alias。
- 收口 CI / verification：最终验证只跑 Rust build/test、contract tests、前端 build/test、OpenSpec validation 和 smoke；删除或迁移依赖 Python backend internals 的测试。

## Capabilities

### New Capabilities
<!-- None: this is the finalization phase of the existing Rust backend runtime. -->

### Modified Capabilities
- `rust-backend-runtime`: 从“双轨 Rust + Python fallback 可回滚”修改为“Rust-only runtime”；删除 Python backend / broker / voice fallback，但保持 HTTP API、磁盘状态和前端资产契约。

## Impact

- Affected code:
  - Remove/deprecate Python runtime modules under `codoxear/` except static packaging and any explicitly retained non-backend metadata files.
  - Update `pyproject.toml` so Python wheel is either removed from runtime docs or limited to static assets / metadata; no `codoxear-server` / `codoxear-broker` / `codoxear-pi-broker` console scripts remain.
  - Update `README.md`, `AGENTS.md`, `docs/cutover/*`, OpenSpec umbrella tasks, and CI workflows to describe Rust-only operation.
  - Update/remove Python tests that import deleted backend modules; retain contract tests only when they target Rust endpoints or static assets.
- APIs: no intentional HTTP path/field removal; `/api/v1/*` remains canonical and `/api/*` remains compatibility alias.
- Dependencies: Rust toolchain / cargo release build becomes mandatory. Python runtime dependencies `py-vapid`, `pywebpush`, `Pillow` are no longer required for backend execution unless kept for packaging/tooling outside runtime.
- Ops: rollback is git revert, not env flag. Operators must not expect Python fallback after this change.
