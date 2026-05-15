## ADDED Requirements

### Requirement: 服务端模块拆分保持 HTTP API 兼容
服务端重构 SHALL 保持现有 HTTP endpoint 的路径、方法、状态码、JSON 字段和鉴权要求不变，除非另有 OpenSpec change 明确修改 API contract。

#### Scenario: session 列表 endpoint 兼容
- **WHEN** 客户端调用 `GET /api/sessions` 及其现有 query 参数组合
- **THEN** 响应 MUST 保持现有分页、分组、`git_branch`、`pr_summary`、busy、queue、priority 等字段语义不变

#### Scenario: session 详情与消息 endpoint 兼容
- **WHEN** 客户端调用现有 session detail、live、messages、repo、workspace、queue、diagnostics 相关 endpoint
- **THEN** 响应 MUST 与重构前 contract 等价，且未知 session、非法参数、未授权请求的错误状态码 MUST 不变

#### Scenario: 静态资源 endpoint 兼容
- **WHEN** 浏览器请求 `/`、`/favicon.ico`、`/manifest.webmanifest`、`/service-worker.js`、`/assets/*`、`/static/*`
- **THEN** 服务端 MUST 按现有静态资源查找、content-type、cache-control 和 URL prefix 规则响应

### Requirement: 模块边界表达单一职责
服务端重构 SHALL 将可独立理解和测试的领域逻辑移出 `server.py`，并为每个新模块定义清晰职责，避免循环依赖和跨模块共享可变全局状态。

#### Scenario: 认证逻辑独立
- **WHEN** cookie 签名、cookie 校验、密码 gate 或 auth response 逻辑被修改
- **THEN** 相关实现 MUST 位于认证边界模块或清晰命名的 auth helper 中，并由现有 auth 测试或新增测试覆盖

#### Scenario: 文件 workspace 逻辑独立
- **WHEN** 文件读取、写入、路径解析、gitignore 过滤、文件搜索或上传 staging 逻辑被修改
- **THEN** 相关实现 MUST 位于文件/workspace 边界模块中，且 MUST 不依赖 HTTP handler 实例状态

#### Scenario: session payload 逻辑独立
- **WHEN** session list、recent list、details、diagnostics 或 workspace payload 结构被修改
- **THEN** payload 构造 SHOULD 位于 session view/payload 边界模块中，且 MUST 可通过单元测试验证输入 session 状态到 JSON payload 的映射

### Requirement: 重构分阶段可回滚
服务端重构 SHALL 以可验证的小阶段推进，每一阶段只移动一个清晰边界，并保留兼容 shim 直到调用点完全迁移。

#### Scenario: 阶段性抽取
- **WHEN** coding-agent 完成一个模块抽取阶段
- **THEN** 对应 tests MUST 通过，且 `server.py` 中原 helper 要么被删除，要么仅作为兼容转发 shim 存在

#### Scenario: 回滚安全
- **WHEN** 某个抽取阶段引入测试失败或行为不一致
- **THEN** 该阶段 MUST 能通过恢复单个新模块及其调用点来回滚，而不要求回滚全部重构工作

### Requirement: 不引入新运行时依赖
本次服务端模块化 SHALL 不引入新的 Web framework、异步运行时或外部服务依赖。

#### Scenario: 依赖检查
- **WHEN** 重构完成后检查 `pyproject.toml`
- **THEN** 除非有单独批准，`dependencies` MUST 不因本 change 增加 Web framework、router framework 或新的长期运行时依赖

### Requirement: 验证覆盖核心行为
重构完成 SHALL 通过自动化测试覆盖被移动边界的关键行为，并保留前端构建作为端到端 contract smoke。

#### Scenario: Python 验证
- **WHEN** 服务端重构任务完成
- **THEN** `python3 -m pytest` MUST 通过，或交接中 MUST 明确列出与本重构无关的既有失败

#### Scenario: 前端 contract 验证
- **WHEN** 服务端 API contract 或静态资源服务相关代码被移动
- **THEN** `cd web && npm run build` MUST 通过，且相关前端 contract 测试 MUST 通过
