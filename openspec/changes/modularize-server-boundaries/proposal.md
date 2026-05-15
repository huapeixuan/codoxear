## Why

`codoxear/server.py` 已增长到约 1.1 万行，集中承担静态资源、认证、文件浏览/写入、git/worktree、session discovery、session lifecycle、queue、harness、voice/push、Pi live UI、HTTP routing 等多种职责。继续在单文件内追加功能会让 review、测试定位和语义边界维护越来越困难。

这次重构的目的不是改变产品行为，而是先建立可持续演进的服务端模块边界，让后续功能（Pi RPC、workspace、通知、session 编排）能在清晰职责内迭代。

## What Changes

- 将 `codoxear/server.py` 中已相对独立的纯函数与领域逻辑逐步抽取到小模块，优先抽取低风险、测试覆盖明确的边界。
- 为 HTTP API 引入轻量路由/handler 分层，使 `Handler.do_GET/do_POST/...` 从长条件链收敛为“鉴权 + 路由分派 + 调用领域服务”。
- 将 session 列表/详情 payload、文件 workspace helpers、git/worktree helpers、认证/cookie helpers、静态资源 helpers 等职责从根 server 模块中拆出。
- 保持现有 API、运行时状态目录、cookie 名称、环境变量、CLI entrypoint、前端行为和测试语义不变。
- 每一阶段都通过现有 pytest/前端测试锁定行为，避免“大爆炸式”改写。
- 不引入新的 Web framework 或外部运行时依赖；继续使用当前 `http.server` + Python 标准库为主的结构。

## Capabilities

### New Capabilities
- `server-module-boundaries`: 定义服务端重构后的职责边界、兼容性约束和验证要求，确保模块拆分不改变用户可见行为。

### Modified Capabilities
- `session-repo-context`: 不改变需求；后续移动相关调用时必须保留现有 git branch / PR endpoint 语义。

## Impact

- 主要影响：`codoxear/server.py`、新增 `codoxear/server_*.py` 或领域模块、相关 pytest。
- 可能影响：已有 `codoxear/git_context.py` 调用点、`tests/test_*server*`、`tests/test_file_*`、`tests/test_session_*`、`tests/test_voice_push*` 等依赖 server helpers 的测试。
- 不影响：前端 API contract、`codoxear-broker`/`codoxear-sessiond` CLI 语义、runtime state 文件格式、OpenSpec 已有 `session-show-branch-pr` change 的需求。
- 依赖：无新增第三方依赖。
