## 1. 建立行为基线与拆分边界

- [ ] 1.1 运行当前基线测试：`python3 -m pytest tests/test_pi_server_backend.py tests/test_file_list.py tests/test_file_viewer_source.py tests/test_frontend_contract_source.py -q`，记录失败项；改动目录：tests；无需迁移/回滚。
- [ ] 1.2 补齐 session-list contract tests，覆盖 `GET /api/sessions` 默认/directories、`view=recent`、`/api/sessions/bootstrap` 字段与错误参数；改动：`tests/test_pi_server_backend.py`；无需迁移/回滚。
- [ ] 1.3 补齐 workspace/file contract tests，覆盖 `/file/list?path=...` direct-child、path escape、非目录、`.gitignore`、ignored dir、文件 read/blob/inspect 关键错误码；改动：`tests/test_file_list.py`、`tests/test_file_inspect.py` 或 `tests/test_pi_server_backend.py`；无需迁移/回滚。
- [ ] 1.4 确认 `openspec validate server-responsibility-split --strict` 通过后再开始实现；改动：OpenSpec；无需迁移/回滚。

## 2. 提取 session-list projection

- [ ] 2.1 新建 `codoxear/session_list.py`，移动 `SESSION_LIST_*` 常量、frontend row projection、directories payload、recent payload、group filtering/sorting纯函数；改动：`codoxear/session_list.py`、`codoxear/server.py`；无需迁移/回滚。
- [ ] 2.2 新增 `tests/test_session_list.py`，以纯 dict rows 覆盖 grouped pagination、hidden cwd group、recent ordering、frontend row slimming；改动：tests；无需迁移/回滚。
- [ ] 2.3 修改 `server.py` 的 `/api/sessions` route 只负责 query validation、调用 `SessionManager.list_sessions()` / `cwd_groups_get()` 和 `session_list` payload 函数；改动：`codoxear/server.py`；无需迁移/回滚。
- [ ] 2.4 跑验证：`python3 -m pytest tests/test_session_list.py tests/test_pi_server_backend.py tests/test_frontend_contract_source.py -q`；如失败回滚本 section 或补测试揭示的兼容差异。

## 3. 提取 workspace/file helper

- [ ] 3.1 新建 `codoxear/workspace_files.py`，移动 session cwd 下安全路径解析、direct-child listing、root `.gitignore` 过滤、排序与内置忽略规则；改动：`codoxear/workspace_files.py`、`codoxear/server.py`；无需迁移/回滚。
- [ ] 3.2 将文件 inspect/read/blob 的大小限制、文本/图片元数据、MIME/下载判断 helper 迁入同一模块或清晰的子模块，保留异常类型/消息兼容；改动：`codoxear/workspace_files.py`、`codoxear/server.py`；无需迁移/回滚。
- [ ] 3.3 将 git file versions helper 迁入 workspace/file 边界，保留 `GIT_DIFF_*` 配置和错误映射；改动：`codoxear/workspace_files.py`、`codoxear/server.py`；无需迁移/回滚。
- [ ] 3.4 新增/更新 `tests/test_workspace_files.py` 与现有 file tests，确保路径安全、忽略规则、too-large、blob/read/inspect 行为不变；改动：tests；无需迁移/回滚。
- [ ] 3.5 跑验证：`python3 -m pytest tests/test_workspace_files.py tests/test_file_list.py tests/test_file_inspect.py tests/test_file_upload.py tests/test_pi_server_backend.py -q`。

## 4. 薄化 GET route handler

- [ ] 4.1 为 sessions/bootstrap/repo/details/live/workspace/diagnostics/queue/commands 新建显式 `handle_*_get` 函数，初期可放在 `server.py` 顶层，稳定后再移入 `codoxear/http_routes.py`；改动：`codoxear/server.py` 或 `codoxear/http_routes.py`；无需迁移/回滚。
- [ ] 4.2 将 `/api/sessions/<id>/file/list|read|blob|inspect` 和 `/git/file_versions` GET route 改为调用 workspace/file helper，handler 仅做参数解析与错误响应；改动：`codoxear/server.py`；无需迁移/回滚。
- [ ] 4.3 保持所有 GET route 的 `_require_auth` 调用位置和 401/404/400/502 映射不变，新增 tests 覆盖至少一个 moved route 的未授权与未知 session；改动：tests；无需迁移/回滚。
- [ ] 4.4 跑验证：`python3 -m pytest tests/test_pi_server_backend.py tests/test_git_context.py tests/test_file_list.py tests/test_file_inspect.py -q`。

## 5. 薄化 POST route handler

- [ ] 5.1 为 session create/delete/rename/edit/heartbeat/send/enqueue/interrupt/ui_response/harness route 新建显式 `handle_*_post` 函数，保持请求体解析、错误码与返回 payload 不变；改动：`codoxear/server.py` 或 `codoxear/http_routes.py`；无需迁移/回滚。
- [ ] 5.2 为 `/api/cwd_groups/edit`、notification、audio/settings 等 POST route 抽出小 handler，确保副作用仍通过 `SessionManager` 或现有 coordinator 触发；改动：`codoxear/server.py`；无需迁移/回滚。
- [ ] 5.3 跑验证：`python3 -m pytest tests/test_pi_server_backend.py tests/test_server_queue_persistence.py tests/test_send_ack.py tests/test_voice_push.py -q`。

## 6. 抽取 SessionManager 持久化 store/helper

- [ ] 6.1 新建 `codoxear/session_state_store.py`，移动 JSON load/save/clean helpers：recent cwd、cwd groups、aliases、queues、harness、sidebar meta、hidden sessions、session files；改动：新模块与 `server.py`；迁移/回滚：必须保持原文件名与 JSON shape。
- [ ] 6.2 将 `SessionManager` 的 `_load_*` / `_save_*` 方法改为薄封装或直接委托 store/helper，保留 lifecycle 决策在 `SessionManager` 内；改动：`codoxear/server.py`；无需数据迁移。
- [ ] 6.3 新增 `tests/test_session_state_store.py`，用临时目录验证旧 JSON shape 可读、坏 JSON recovery、保存 shape 兼容；改动：tests；回滚：删除新模块并恢复原方法。
- [ ] 6.4 跑验证：`python3 -m pytest tests/test_session_state_store.py tests/test_server_queue_persistence.py tests/test_session_sidebar_priority.py tests/test_recent_cwds.py tests/test_session_file_history.py -q`。

## 7. 收尾验证与文档

- [ ] 7.1 更新 `AGENTS.md` Components 或 Development reminders，说明新增服务端模块职责与不要把新业务逻辑继续堆回 `server.py`；改动：`AGENTS.md`；无需迁移/回滚。
- [ ] 7.2 跑后端聚焦全量：`python3 -m pytest tests -q`；如耗时过长，至少跑本 change touched suites 并在交接中说明未跑项。
- [ ] 7.3 如触及 web 类型或前端 contract，运行 `cd web && npm run test -- src/lib/api.test.ts src/domains/sessions/store.test.ts src/components/workspace/FileViewerDialog.test.tsx`；改动：web tests；无需迁移/回滚。
- [ ] 7.4 运行 `cd web && npm run build`，确认静态包仍可构建；改动：`codoxear/static/dist` 如构建产物被仓库跟踪则同步。
- [ ] 7.5 运行 `openspec validate server-responsibility-split --strict`，确认 OpenSpec 仍有效。
- [ ] 7.6 提交后交给 `code-reviewer`；重点审查 API 兼容、路径安全、鉴权未绕过、循环 import、测试覆盖是否足以证明行为保持。
