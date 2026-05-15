## Why

`codoxear.server` 里多处运行时 JSON 状态文件使用相同的“清洗 → 临时文件 → `os.replace`”模式，但保存逻辑分散在多个方法中。继续新增会放大重复代码和不一致的原子写入行为，因此先把这一类状态持久化收敛为小而明确的内部边界。

## What Changes

- 新增内部 JSON 状态持久化辅助函数，统一创建 app 目录、缩进/排序编码、临时文件写入和原子替换。
- 将 harness、alias、sidebar、hidden sessions、file history、queue、recent cwd、cwd group 等保存路径改为复用该辅助函数。
- 保持现有磁盘 JSON 结构、API 行为和加载/清洗规则不变。
- 不引入新依赖，不改变前端或后端公开接口。

## Capabilities

### New Capabilities
- `json-state-persistence`: codoxear 服务端内部运行时 JSON 状态必须通过一致的原子写入语义持久化。

### Modified Capabilities

## Impact

- 影响代码：`codoxear/server.py` 及相关单元测试。
- API：无公开 API 变更。
- 依赖：无新增依赖。
- 风险：若 helper 语义与现有格式不一致，可能影响用户已有运行时状态文件；需要用针对性测试锁定输出格式与异常清理行为。
