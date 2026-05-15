## Context

当前 `SessionManager` 中多个 `_save_*` 方法都手写同一套 JSON 保存步骤：复制内存态、`os.makedirs(APP_DIR, exist_ok=True)`、写入 `*.json.tmp`、`os.replace`。这些方法保存的文件是 codoxear 的运行时 UI/队列状态，属于本地状态，不需要迁移格式。

## Goals / Non-Goals

**Goals:**
- 让 JSON 状态保存共享一个小型内部函数，减少重复和语义漂移。
- 保持现有 JSON 文件内容结构、格式和原子替换行为。
- 增加测试覆盖 helper 的格式和失败清理语义。

**Non-Goals:**
- 不重构 `SessionManager` 的状态模型或锁粒度。
- 不改变任何 `/api/*` contract。
- 不迁移已有 runtime state 文件。
- 不处理 voice push 相关持久化；该模块已有独立 coordinator 边界，可后续单独整理。

## Decisions

1. 在 `codoxear/server.py` 内新增 `_write_json_state_atomic(path, value, *, sort_keys=True)`。
   - 理由：当前重复点只服务 server runtime state，放在 server 内能避免过早扩大公共 util API。
   - 替代方案：放入 `util.py`。本轮暂不采用，因为 util 已承担跨 broker/session/log 的职责，server UI 状态写入不必暴露给其他模块。
   - 实现约束：临时文件使用固定的 `path.with_suffix(".json.tmp")`，以保持现有路径形态和可观测行为；本轮不引入随机 tmp 文件名。

2. helper 负责 app 目录创建、JSON 编码、临时文件写入、`os.replace` 和失败时清理临时文件。
   - 理由：这些是所有 `_save_*` 方法真正共享的不变量。
   - 替代方案：只抽 JSON 编码。收益不足，仍会保留大部分重复。

3. 各 `_save_*` 方法继续负责锁内快照与业务清洗。
   - 理由：锁粒度和清洗规则是业务语义；本轮重构只收敛持久化机制。

## Risks / Trade-offs

- [Risk] helper 默认排序可能改变原本不排序的列表文件输出顺序。→ Mitigation：保留调用点对 `sort_keys` 的控制；hidden sessions 继续不排序对象 key。
- [Risk] 临时文件残留。→ Mitigation：helper 在异常路径尝试删除 tmp，并用测试覆盖。
- [Risk] 误改 runtime state schema。→ Mitigation：只替换写入机制，不碰 `_load_*` 清洗逻辑；运行现有相关 Python tests。
