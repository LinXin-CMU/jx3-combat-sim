# Agent 私有 Replay 与续接记忆基线（2026-08-28）

## 最近会话诊断

审计对象：`session-1a047998262-23`，共 4 轮。

| 用户问题 | 服务端路由 | 结果 |
| --- | --- | --- |
| 分析当前循环的输出基线，并说明证据边界 | `current_rotation_baseline` | 取得知识、基线与时间轴证据，但报告发生 `metric_value_mismatch` 后部分发布 |
| 循环伤害构成合理吗 | `current_rotation_baseline` | 仍为部分发布，结论偏概括 |
| 有没有可以调优的地方 | `general_grounded_analysis` | 未检索知识，只模拟一次；发生 `missing_metric_source` |
| 如何调优 | `general_grounded_analysis` | 未检索知识，只模拟一次；发生 `metric_value_mismatch`，只剩泛化建议 |

失配不是单点模型质量问题：短续接在路由前没有继承上轮任务；模型上下文只有最近两份裁剪后的公开报告；校验失败后，旧存储又没有原始模型响应、工具参数和 Evidence，因此无法准确重放被删除的字段。

## 双层会话结构

公开投影保持原路径与 schema：

```text
agent_sessions/v1/<session-id>/
├── meta.json
└── events/*.json
```

它只包含前端需要的用户问题、阶段概述、来源卡、已校验报告和稳定错误码。现有 `/api/agent/sessions` 与 `/api/agent/sessions/:id` 不读取私有目录。

所有新 run 默认增量保存私有重放日志：

```text
agent_sessions/v1/<session-id>/_private/replay/v1/<run-id>/events/*.json
```

事件覆盖：

- 完整冻结场景、当前问题、注入给模型的有界会话上下文；
- provider profile、model、预算、Prompt 正文、版本与哈希；
- 服务端选择的 `AnalysisPlanV1`；
- 每次规范化 `ModelRequest` 与 `ModelResponse`；
- 每次工具调用的 call ID、参数、完整输出、Evidence ID 与预算状态；
- 报告接受、修复、salvage 或拒绝的校验码和对应内容；
- 最终 `AgentRunResultV1`。

每个 payload 在凭据清除后计算 SHA-256，并使用 create-new + 临时文件原子改名落盘。记录只追加，不覆盖旧事件。

## 安全与可复现边界

- API key 从未进入标准化 provider 协议；问题入口仍拒绝疑似凭据；所有 replay 字符串落盘前再次执行凭据清除。
- 不保存供应商隐藏思维链，也不把它伪装成可复现材料；保存的是本系统实际接收的规范化响应。
- Replay 可以确定性重放本地工具、Evidence、报告解析和校验过程；重新请求外部模型仍可能因供应商随机性得到不同回答。
- 历史 run 缺失的原始模型响应无法倒推；私有 replay 从本版本部署后的新 run 开始完整记录。

## 续接修复

Session store 会从上一轮公开轨迹提取最后一个服务端 Playbook ID。当前问题若已有明确任务信号则独立路由；只有“继续”等短续接落入通用任务时才继承上轮 Playbook。`调优/优化` 现在直接识别为当前循环分析，因此会恢复知识、时间轴和模拟的专业证据路径，而不是退化为单次通用模拟。

## 验证

- 私有 replay 追加性、凭据清除、payload hash 与公开 API 隔离均有单元测试；
- RunManager 端到端测试确认 `run_input → analysis_plan → tool_dispatch → model_request → model_response → report_validation → run_result` 全链落盘；
- 旧会话读取与 interrupted 恢复行为保持兼容；
- Rust 全量测试：185/185。

3005 实机使用 `session-1a047f6bb72-0` 连续执行“分析当前循环的输出基线”与“继续”：第二轮继承 `current_rotation_baseline`，私有目录按序生成 8 个 replay 事件；公开 session JSON 不包含 `model_request`、`prompt_instructions` 或 `tool_dispatch`。部署后唯一监听地址为 `127.0.0.1:3005`。
