## Context

当前 `codoxear/server.py` 约 11099 行，承担了过多职责：

- `SessionManager` 类约 4307 行，混合了 socket discovery、broker state refresh、session list 构建、recent cwd / cwd groups / alias / queue / harness / hidden state 等持久化读写。
- `Handler` 类约 2401 行，其中 `do_GET` 约 1327 行、`do_POST` 约 1030 行，路由解析、鉴权、查询参数解析、业务逻辑与错误响应交织。
- 文件/工作区相关 helper、session-list projection、repo context 调用、diagnostics payload、queue/notification/audio 逻辑集中在同一文件。
- 前端已经完成 Preact/Vite 化，后续服务端 API 仍在快速演进；继续在单文件内叠加功能会让 diff 难审、测试难聚焦、回归定位变慢。

已有较好的正向例子：`codoxear/git_context.py` 把 GitHub PR/git 分支解析从 `server.py` 抽出，并由 `tests/test_git_context.py` 独立覆盖。这说明“先提纯 helper，再薄化 handler”的路线在本仓库可行。

## Goals / Non-Goals

**Goals:**

- 在不改变用户可见行为的前提下，把服务端职责边界拆清楚。
- 优先拆出 session-list projection 与 workspace/file helper，因为它们已经相对独立、测试收益高、也是近期性能/功能迭代热点。
- 逐步薄化 `Handler.do_GET` / `do_POST`，让 handler 只负责鉴权、参数解析、调用 service/helper、响应映射。
- 逐步拆分 `SessionManager` 的持久化状态读写，保留其 live session 编排职责。
- 通过 contract tests 防止字段遗漏、错误码变化、鉴权绕过、路径安全退化。

**Non-Goals:**

- 不迁移到 FastAPI/Flask/aiohttp 等新 HTTP 框架。
- 不改变现有 runtime state 目录或 JSON 文件格式。
- 不改变 session list、workspace、file、diagnostics、queue、repo context 的 API contract。
- 不重写 broker、Pi RPC、voice push、frontend UI。
- 不借重构机会做视觉改版、性能策略变化或新增产品功能。

## Decisions

### Decision 1: 分阶段提取，避免一次性“大爆炸”拆文件

**What**：按风险从低到高拆：
1. session-list projection 纯函数；
2. workspace/file helper；
3. GET route handler 小函数；
4. POST route handler 小函数；
5. SessionManager 持久化 store。

**Why**：当前 `server.py` 的行为由大量 route tests 和前端 contract 隐式约束。一次性移动所有逻辑会产生巨大 diff，reviewer 很难判断是否行为保持。分阶段可以每一步都跑针对性测试，并保留清晰回滚点。

**Alternatives considered**：
- 一次性拆成完整分层架构：拒绝，diff 风险过高。
- 只写文档不拆代码：拒绝，无法解决当前维护瓶颈。

### Decision 2: 先抽纯 projection，再改 handler

**What**：把 `SESSION_LIST_ROW_KEYS`、`_frontend_session_list_row`、`_session_list_payload`、`_session_recent_payload` 等会话列表投影逻辑移入 `codoxear/session_list.py`，由 `server.py` 调用。

**Why**：这部分逻辑已经基本是纯函数，输入 rows + cwd_groups，输出 JSON payload；它和 route/manager 解耦后可以独立测试 directories/recent/pagination/hidden group 行为，是低风险高收益的第一步。

**Alternatives considered**：直接重写 `SessionManager.list_sessions()` 只返回精简 row。拒绝，因为内部 manager 仍需要丰富字段，且已有测试依赖内部 shape。

### Decision 3: Workspace/file 模块统一持有路径安全与 listing 语义

**What**：把 session-relative path resolve、root `.gitignore` 过滤、direct-child listing、file inspect/read/blob、git diff versions 等 helper 迁入 `codoxear/workspace_files.py` 或等价模块。

**Why**：文件路径安全和 listing/filtering 是高风险逻辑，必须有单一实现。当前前端 lazy file tree、`.gitignore` 过滤、文件读取上限等功能都依赖这些 helper；独立模块能让 path traversal、non-dir、too-large、ignored-dir 等边界有更聚焦的测试。

**Alternatives considered**：按 route 分散 helper。拒绝，因为容易让路径安全规则漂移。

### Decision 4: Route handler 采用显式函数而不是新框架

**What**：保留 stdlib `BaseHTTPRequestHandler`，但将 route 分派到小函数，例如 `handle_sessions_get(handler, manager, query)`、`handle_file_list_get(...)`。这些函数可以仍位于 `server.py` 初期，稳定后再移动到 `http_routes.py`。

**Why**：项目当前依赖简单 stdlib server，迁框架会引入行为差异和依赖管理问题。显式函数能获得大部分可维护性收益，同时保持部署方式和测试 harness 不变。

**Alternatives considered**：引入路由表/装饰器 DSL。暂不采用，避免为重构引入新的抽象学习成本。

### Decision 5: SessionManager 持久化拆分只移动存取，不改变所有权

**What**：把 JSON 文件加载/保存/清洗逻辑抽出为 store/helper，但 `SessionManager` 仍是 live session lifecycle 的 owner，负责决定何时读写这些状态。

**Why**：SessionManager 里的状态副作用很多（删除 session、drain queue、hidden cutoff、recent cwd backfill）。先抽存取函数可以减少体积；但过早把生命周期决策也移走，容易改变行为。

**Alternatives considered**：把每个状态域都变成独立 service 并自行监听 lifecycle。拒绝，当前没有事件总线，过度设计。

## Risks / Trade-offs

- **Risk: 字段遗漏导致前端行为退化** → Mitigation：在 route contract tests 中明确断言 `GET /api/sessions` directories/recent/bootstrap、diagnostics、file/list 等公共字段。
- **Risk: 路径安全 helper 移动时引入 traversal 漏洞** → Mitigation：先补 path escape、symlink/非目录、ignored directory、too-large 文件的测试，再移动实现。
- **Risk: `SessionManager` 内部 rich row 与 frontend slim row 混淆** → Mitigation：命名区分 internal row 与 frontend projection，projection 模块只接受 dict 输入，不反向依赖 manager。
- **Risk: 循环 import** → Mitigation：新模块只依赖 `typing`、`pathlib`、小型 util；需要 server 常量时将常量一起移动或通过参数注入。
- **Risk: Diff 过大难 review** → Mitigation：按 tasks 分多批 commit，每批只移动一个职责边界并跑对应测试。

## Migration Plan

1. 先建立/补强当前行为 contract tests，记录现有 public API shape。
2. 提取 session-list projection，跑 session-list/backend + frontend contract tests。
3. 提取 workspace/file helper，跑 file/list/read/inspect/upload/git 相关 tests。
4. 拆 GET route 小函数，逐批迁移 sessions/bootstrap/repo/workspace/file/diagnostics/queue。
5. 拆 POST route 小函数，优先 sessions create/delete/edit/send/enqueue 与 cwd_groups。
6. 抽 SessionManager 持久化 store/helper，保持 JSON shape 不变。
7. 最终全量跑 Python 后端相关测试与 web build。

Rollback：每个阶段都是行为保持型移动；若某阶段验证失败，回滚该阶段 commit，不影响前面已验证的提取。

## Open Questions

- `voice_push`、audio segment、notification subscription 是否纳入本次拆分？建议本 change 只在前几阶段完成后视剩余时间处理；它们不应阻塞 session/file/server 核心边界重构。
- 是否将 `SessionManager` 拆成多个类？建议本 change 不做大类重写，只先抽持久化 helper；后续如果仍过大，再单独开 change。
