# Agent 可观测性基础基线（2026-08-27）

## 目标

把原先的扁平状态标签升级为可重放、可验收的阶段轨迹，同时严格区分：

- 对用户公开的执行概述；
- 服务端保存的规范化事件与脱敏诊断；
- 不记录、不展示的模型隐藏推理和供应商原始响应。

设计参考现代 Agent tracing 的 trace / span 思路：一次 run 是端到端 trace，规划、工具、证据、校验和终止状态是可观察阶段。OpenAI Agents SDK 的 tracing 同样把模型生成、工具调用、handoff 与 guardrail 记录为不同 span；Anthropic 对 agent loop 的公开描述则采用 plan → act → observe → adjust 的循环。这里没有照搬第三方 SDK，而是把相同的可观测性原则适配到项目既有的 Rust 编排器、SSE 和会话事件协议。

参考：

- [OpenAI Agents SDK — Tracing](https://openai.github.io/openai-agents-python/tracing/)
- [Anthropic — Trustworthy agents in practice](https://www.anthropic.com/research/trustworthy-agents)

## 公开阶段模型

| 阶段 | 用户看到的内容 | 可验证数据 |
| --- | --- | --- |
| 规划 | 问题类型、工具范围与最小验证路径概述 | `planning`、客户端范围事件 |
| 动作 | 正在调用的只读工具及其目标 | `tool_started`、`tool_name` |
| 观察 | 工具是否完成、产生多少份证据 | `tool_finished`、`evidence_ids`、稳定状态码 |
| 校验与恢复 | 结构、数值、单位、引用校验，以及有界修复/降级 | `validating`、`report_*`、`provider_empty_*` |
| 终止 | 完成、部分通过、拒绝、预算耗尽、供应商故障或超时 | 终止事件与 `AgentRunResultV1.status` |

每个阶段只展示固定、可解释的概述。概述由规范化事件类型、工具名、证据数量和稳定代码确定，不由模型自由生成，因此不会把不可审计的推理伪装成系统事实。

## UI 行为

- 独立 AI 分析页与循环模拟弹窗使用同一套阶段语义；
- `tool_started` 与对应的 `tool_finished` 合并为一条生命周期记录，避免“调用工具 / 取得证据”重复刷屏；
- 当前阶段带“当前”标记、金色呼吸点和横向扫光；下一事件到达后才转为完成态；
- 每次供应商等待都有成对的 `model_started` / `model_finished` 生命周期事件，因此模型生成期间不会出现“请求仍在运行、轨迹却没有活动节点”的空档；
- 运行中、完成、受限三种状态使用主题变量中的金色、绿色和警告色；
- 历史会话直接从既有 `run_trace` 重建阶段概述，不修改或迁移原会话数据；
- 页面和会话栏保持内部滚动，阶段文本换行不会撑宽或撑高整页。

## 失败诊断

结果卡新增可折叠诊断区。硬失败默认展开，正常或部分通过默认折叠，包含：

- 终止阶段与最后成功工具；
- 模型轮次、工具次数、模拟次数、知识检索次数；
- 证据数量、报告修复次数、空响应重试次数；
- 输入/输出 token、总耗时、prompt 版本和 run ID；
- 稳定诊断码及面向用户的排查建议。

诊断区明确不保存 API Key、隐藏推理或供应商原始响应。这样既能在作品集中展示可观测性和故障工程，也不会扩大敏感数据面。

## 本次验收

- `node --check frontend/agent.js` 通过；
- `git diff --check` 通过；
- 3005 静态资源已命中 `20260827-trace1` cachebuster；
- 真实浏览器以 1440 × 900 viewport 恢复历史会话：15 条阶段记录、每条均有概述；文档宽度 1440 / 1440，无横向溢出；会话栏和对话区均保持内部滚动；
- 历史结果卡成功生成运行诊断区，旧会话数据未改写。
