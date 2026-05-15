## Why

`codoxear/server.py` 已经增长到约 1.1 万行，`SessionManager`、HTTP `Handler`、会话列表投影、文件工作区 API、诊断/队列/通知等职责集中在同一个文件中；后续每个功能都必须修改巨大文件，review 和回归定位成本持续上升。现在仓库已经完成前端 Vite/Preact 化、会话列表 view mode、repo context、lazy file tree 等多轮功能迭代，适合先做一轮“行为不变”的服务端边界重构，为继续演进降低风险。

## What Changes

- 将 `server.py` 中已经相对独立的会话列表投影逻辑抽出为独立模块，保持 `GET /api/sessions` 与 `/api/sessions/bootstrap` 对外契约不变。
- 将文件/工作区相关的路径解析、目录 listing、文件读取/检查、git diff 版本读取等 helper 抽出到独立模块，保持 `/api/sessions/<id>/file/*` 与 `/git/file_versions` 行为不变。
- 将 HTTP 路由处理逐步拆成小的 route handler 函数或模块，优先拆 `GET` 的 sessions、repo、workspace、file、diagnostics、queue 路径，再拆对应 `POST` 路径。
- 将 `SessionManager` 内部的持久化状态读写（recent cwd、cwd groups、aliases、queues、harness、sidebar meta、hidden sessions、session files）按领域抽成 store/helper，保持磁盘文件格式兼容。
- 建立服务端 contract/regression tests，确保重构前后公开 API 响应字段、错误码和关键副作用保持一致。
- 不引入新的外部依赖；不重写 HTTP server 框架；不改变用户可见功能。

## Capabilities

### New Capabilities
- `server-responsibility-boundaries`: 约束服务端模块职责边界、API 行为保持、迁移兼容性与测试门禁。

### Modified Capabilities
- 无：本 change 目标是行为保持型重构，不新增或修改用户可见需求；已有 `session-repo-context` 等能力的接口语义应保持不变。

## Impact

- Backend：`codoxear/server.py` 将变薄；新增 `codoxear/session_list.py`、`codoxear/workspace_files.py`、`codoxear/session_store.py`、`codoxear/http_routes.py` 或等价小模块。
- Tests：补强 `tests/test_pi_server_backend.py` 中 sessions/bootstrap/file/diagnostics/queue 合同测试，必要时新增模块级 unit tests。
- Frontend：原则上不需要改动；仅当测试暴露类型契约缺失时补充已有 TypeScript 类型，不改变 UI 行为。
- APIs：无 breaking change；所有现有 HTTP 路径、响应字段、错误码和鉴权要求必须保持兼容。
- Dependencies：无新增运行时依赖。
