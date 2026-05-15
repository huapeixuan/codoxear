## 1. Baseline 与边界确认

- [ ] 1.1 运行并记录 baseline：`python3 -m pytest`、`cd web && npm run test`、`cd web && npm run build`；若有既有失败，写入 change notes 或 issue comment
- [ ] 1.2 用 `rg` 检查 tests 和模块内对 `codoxear.server` 私有 helper 的直接引用，列出需要 shim 或更新的引用点
- [ ] 1.3 为 `GET /api/sessions`、`GET /api/sessions/<id>/repo`、文件 read/write/inspect、auth 失败路径确认已有 characterization tests；缺口处先补测试，不改实现

## 2. Auth 与静态资源叶子模块

- [ ] 2.1 新增 `codoxear/http_auth.py`，机械移动 env/password、cookie parse/sign/verify、auth 判断相关纯函数；保留 `CODEX_WEB_PASSWORD` 等 env 语义
- [ ] 2.2 更新 `server.py` 调用 auth 模块；如 tests 直接 import 旧私有函数，先保留兼容 shim 并标注后续删除
- [ ] 2.3 新增/更新 auth tests，覆盖未登录、cookie 有效、cookie 无效、密码缺失 fail-closed
- [ ] 2.4 新增 `codoxear/static_assets.py`，移动 Vite dist 查找、manifest version、index rewrite、content-type/cache-control helpers
- [ ] 2.5 运行相关验证：`python3 -m pytest tests/test_url_prefix.py tests/test_vite_dist_serving.py tests/test_vite_asset_versioning.py` 与 `cd web && npm run build`

## 3. Workspace / 文件边界

- [ ] 3.1 新增 `codoxear/workspace_files.py`，移动路径解析、safe expand/resolve、文本解码、图片/PDF kind、strict read/write atomic helpers
- [ ] 3.2 移动目录 listing、gitignore 匹配、文件搜索、upload staging、attachment injection 相关 helper；保持函数输入显式，不依赖 `Handler` 或 `MANAGER`
- [ ] 3.3 更新 server route/manager 调用点与 tests import；保留必要 shim 以降低单步风险
- [ ] 3.4 运行相关验证：`python3 -m pytest tests/test_file_inspect.py tests/test_file_list.py tests/test_file_upload.py tests/test_file_viewer_source.py tests/test_path_resolution.py`

## 4. Git / worktree 边界

- [ ] 4.1 新增 `codoxear/git_worktree.py`，移动 `_run_git`、repo root、current branch 兼容、numstat、worktree slug/path/create helpers
- [ ] 4.2 保持 `codoxear/git_context.py` 为 PR context 专属模块；必要时让它复用新的 git subprocess helper，但不得改变 `session-show-branch-pr` spec 语义
- [ ] 4.3 更新 server 调用点，确认 `/api/sessions/<id>/repo` 与 session list 中 `git_branch`/`pr_summary` 不回归
- [ ] 4.4 运行相关验证：`python3 -m pytest tests/test_git_context.py tests/test_session_repo_endpoint.py tests/test_recent_cwds.py`

## 5. Session payload 与 sidebar 分组边界

- [ ] 5.1 新增 `codoxear/session_payloads.py`，先移动 `_frontend_session_list_row`、session group sort/filter、recent payload、details payload 等 stateless helpers
- [ ] 5.2 为需要 `SessionManager` 的 diagnostics/workspace payload 引入显式 adapter 参数，避免新模块 import `MANAGER` 或 `server.Handler`
- [ ] 5.3 更新 `SessionManager.list_sessions()` / route 调用点，确保 hidden sessions、cwd group、priority、todo snapshot、busy 状态语义不变
- [ ] 5.4 运行相关验证：`python3 -m pytest tests/test_session_sidebar_priority.py tests/test_hidden_sessions_startup.py tests/test_sidebar_update_ts.py tests/test_sessions_pending_log_idle.py tests/test_launch_defaults.py`

## 6. HTTP route 分派瘦身

- [ ] 6.1 新增 `codoxear/http_routes.py` 或 `codoxear/server_routes.py`，定义最小 route function/table；route function 接收 handler、manager、parsed URL，不持有全局状态
- [ ] 6.2 将 `Handler.do_GET` 中静态资源、settings/notifications/audio、sessions、files/workspace/git endpoint 分批迁移到 route functions
- [ ] 6.3 将 `do_POST` / `do_DELETE` 等 mutation endpoint 分批迁移；每批只移动同一 URL 前缀，避免混合 queue/session/file 改动
- [ ] 6.4 保留 URL prefix、auth、JSON error、bytes response 行为；更新 route-level tests 或增加 characterization tests

## 7. 收尾验证与文档

- [ ] 7.1 删除确认不再被 tests 或内部调用引用的兼容 shim；保留的 shim 必须有注释说明删除条件
- [ ] 7.2 更新 `AGENTS.md` Components section，说明 `server.py` 已拆出的模块职责；如 README 架构描述过时，同步更新
- [ ] 7.3 运行全量验证：`python3 -m pytest`、`cd web && npm run test`、`cd web && npm run build`
- [ ] 7.4 手动 smoke：启动 `CODEX_WEB_PASSWORD=... python3 -m codoxear.server`，验证登录、session list、repo endpoint、messages、file viewer、new session bootstrap 不报错
- [ ] 7.5 交付时说明未纳入第一批的后续候选：`SessionManager` 内部服务拆分、`ConversationPane.tsx` markdown/render 拆分、`pi_messages.py` normalizer 拆分、`broker.py` PTY/control channel 拆分
