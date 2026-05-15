## Context

当前仓库已有一次成功的前端责任拆分先例：`docs/superpowers/specs/2026-04-10-app-shell-responsibility-split-design.md` 将 `AppShell` 从“混合控制文件”改造成 orchestrator + hooks + UI subcomponents。服务端现在面临类似但更高风险的问题：`codoxear/server.py` 约 1.1 万行，既包含大量纯 helper，也包含 `SessionManager`、`Handler`、静态资源、auth、文件 workspace、git/worktree、voice/push、queue/harness 等逻辑。

关键约束：

- 项目是轻量本地工具，当前没有引入 Web framework；保持简单部署比架构“现代化”更重要。
- README 与 AGENTS 均强调本地状态、session 日志、broker/socket 元数据的兼容性，重启 server 不应影响 session 内容。
- 当前测试覆盖面较广，适合用“先移动、再验证”的机械式重构，而不是重写。
- 已有 active change `session-show-branch-pr` 已完成 20/21 task，仍应保留其 repo context endpoint 与 payload 语义。

## Goals / Non-Goals

**Goals:**

- 让 `server.py` 收敛为：配置常量 + `Session`/`SessionManager` 组装 + HTTP server entrypoint + 少量兼容导出。
- 将低耦合 helper 先拆到独立模块，再拆高耦合 payload/manager 逻辑。
- 建立一个后续 coding-agent 可按顺序执行、每步 ≤ 2 小时、每步可验证的任务序列。
- 保持现有 API、状态文件、环境变量、CLI 命令、测试语义不变。

**Non-Goals:**

- 不重写为 FastAPI/Flask/aiohttp。
- 不改前端 UI/UX，不新增功能。
- 不改变 Codex/Pi backend 语义、broker 协议、socket metadata 格式。
- 不处理所有大文件；第一批聚焦服务端 `server.py`。`ConversationPane.tsx`、`pi_messages.py`、`broker.py` 可作为后续 change。
- 不归档或强行完成 `session-show-branch-pr` 的 manual smoke；仅确保本重构不破坏它。

## Decisions

### 1. 采用“strangler fig”式模块抽取，而不是一次性重写

先从纯函数边界开始抽取，保留原函数名或兼容 import，逐步迁移调用点。每完成一个边界运行对应测试。

备选方案：一次性重写 `server.py` 为多包结构。拒绝原因：当前 handler 与 manager 状态交织严重，一次性改动难以 review，也难以定位回归。

### 2. 第一批模块边界按职责而不是按 endpoint 切分

推荐新增或整理为以下边界（最终命名可由 coding-agent 根据实际导入冲突微调）：

- `codoxear/http_auth.py`：env password gate、cookie parse/sign/verify、auth JSON response helper 可用的纯逻辑。
- `codoxear/static_assets.py`：Vite/legacy dist 查找、manifest version、content-type/cache-control、index rewrite。
- `codoxear/workspace_files.py`：路径解析、文本/图片/PDF inspect、gitignore 过滤、文件搜索、读写与上传 staging。
- `codoxear/git_worktree.py`：`_run_git`、repo root、worktree path/branch、numstat、当前分支兼容入口；注意保留现有 `git_context.py` 用于 PR context。
- `codoxear/session_payloads.py`：session list/recent/details/diagnostics/workspace payload 的纯映射逻辑；必要时先只移动 stateless helpers。
- `codoxear/http_routes.py` 或 `server_routes.py`：把 `Handler` 的长条件链分派成可读的 route table / route functions；仍使用 `BaseHTTPRequestHandler`。

备选方案：按 `GET/POST` 或 URL 前缀切分 route 文件。拒绝原因：这能缩短 handler，但不会解决文件 workspace、session payload、auth 等逻辑边界混乱。

### 3. 先移动“叶子 helper”，再移动 `SessionManager`

`SessionManager` 当前直接持有 voice coordinator、queue/harness 线程、session discovery、metadata persistence 等状态。第一批不要拆 manager 内部线程模型，先抽出 manager 调用的纯 helper 和 payload 构造。待 server.py 缩小且测试稳定后，再为 manager 拆分 store/queue/harness 子服务。

### 4. HTTP route 层保持同步、标准库实现

继续使用 `http.server.BaseHTTPRequestHandler`，route 层只做轻量匹配：解析 URL prefix、鉴权、参数校验、调用 manager/helper、返回 JSON/bytes。不引入 decorator-heavy framework。

### 5. 每步以测试保护兼容性

重构任务应按“移动代码 → 更新 import → 运行相关测试 → 全量 smoke”的节奏推进。若测试暴露隐式依赖，优先增加 characterization test，再移动实现。

## Proposed Phase Order

1. **Characterization baseline**：运行 `python3 -m pytest`、`cd web && npm run test`、`cd web && npm run build`，记录既有失败；先不要改行为。
2. **Auth/static 叶子模块**：抽取认证 cookie/env/password 与静态资源 helpers。这两块依赖较少，能快速降低 handler 噪音。
3. **Workspace/files 模块**：抽取路径解析、文件 read/write/inspect/search/upload staging。该块函数多但多为纯逻辑，测试较集中。
4. **Git/worktree 模块**：抽取 git helper 与 worktree 创建逻辑，保留 `git_context.py` PR 逻辑不变。
5. **Session payload 模块**：先移动 list/recent/group/details payload 纯函数，再评估 diagnostics/workspace payload 是否需要 manager adapter。
6. **Route 分派瘦身**：引入 route functions/table，把 `do_GET/do_POST` 的 endpoint 分支搬出 handler；`Handler` 保留 request/response primitive。
7. **收尾**：删除无用 shim、更新 AGENTS/README 架构说明、运行全量验证。

## Risks / Trade-offs

- **隐式测试依赖私有函数名** → 迁移前用 `rg "_function_name" tests codoxear web` 找引用；必要时保留兼容 shim 并逐步更新测试。
- **循环 import** → 新模块不得 import `MANAGER` 或 `Handler`；需要状态时通过参数传入。`server.py` 可 import 子模块，子模块不反向 import `server.py`。
- **行为无意改变** → 优先机械移动，不顺手“优化”；任何语义修复另开 change。
- **route table 过度设计** → 只做项目需要的最小分派，不引入框架或复杂 middleware。
- **manual smoke 成本高** → 自动化测试为主；手动 smoke 聚焦启动 server、登录、session list、messages、file viewer、new session dialog 静态 contract。

## Migration Plan

- 在同一分支内分多 commit/阶段推进；每阶段可独立回滚。
- 保留 `codoxear.server:main` 与 `codoxear-server` entrypoint 不变。
- 保留 runtime state 文件路径和 JSON schema 不变。
- 完成后更新 `AGENTS.md` Components section，说明新的服务端模块边界。

## Open Questions

- 是否要在第一批内拆 `SessionManager` 内部 queue/harness/voice 线程？建议否，等 route/helper 边界稳定后另开 change。
- 是否要同步拆 `ConversationPane.tsx` 或 `pi_messages.py`？建议否，避免把多条重构线混在一个 change 中。
