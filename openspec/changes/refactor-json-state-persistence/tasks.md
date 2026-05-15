## 1. Helper 与测试

- [x] 1.1 为 server JSON 状态原子写入新增 focused 单元测试，覆盖格式、目录创建、排序开关和失败清理。
- [x] 1.2 在 `codoxear/server.py` 新增 `_write_json_state_atomic` helper，最小实现通过新增测试。

## 2. 保存路径替换

- [x] 2.1 将 `SessionManager` 中 harness、alias、sidebar、hidden sessions、file history、queue、recent cwd、cwd group 的保存方法改为复用 helper。
- [x] 2.2 运行相关 Python 单元测试，确认状态 schema 兼容。

## 3. 收尾验证

- [x] 3.1 运行 `openspec validate refactor-json-state-persistence --strict`。
- [x] 3.2 运行可用的前端/后端最小验证，并记录环境性失败（如缺少依赖）。
