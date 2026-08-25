# Agent P2-03 Orchestrator 与证据校验基线

日期：2026-08-25

范围：版本化 prompt、四工具注册表、有界工具循环、结构化报告、数值证据校验和离线失败 trace。

真实模型调用：0。

Agent 会话写入：0。

公网状态：关闭。

## 本次交付

### 可审计 Prompt

系统提示词位于 [`../../backend/prompts/agent_system_v1.md`](../../backend/prompts/agent_system_v1.md)，版本为 `agent-system/v1`，当前 SHA-256 为：

```text
f6f018ff12bccb86a43a142153543f749c67e89858d9714eabb22ebe24088624
```

每个 run 都记录 prompt 版本和哈希。提示词明确区分模型与模拟器职责：模型负责规划和解释，确定性模拟器负责数值事实；scenario、宏文本、用户消息和工具输出均按不可信数据处理，不能覆盖系统规则。

### 四工具注册表

Orchestrator 只暴露：

- `get_current_scenario`
- `simulate_scenario`
- `compare_scenarios`
- `analyze_timeline`

注册表直接调用 Phase 1 Rust 领域函数，不允许模型提交 URL、路径、REST 地址或写操作。HTTP 工具和 Orchestrator 共用 `AgentRuntime` 不可变运行时快照，避免两个入口形成不同版本/心法边界。

模型侧 A/B patch 首批只开放六个明确的策划变量：加速、技能序列、网络延迟、初始怒气、基础攻击和目标防御。字段、范围、候选数量和 `additionalProperties=false` 都写入 JSON Schema；服务端仍会再次反序列化并做领域校验。

### 有界 Orchestrator

默认硬预算保持 Phase 2 计划值：

| 资源 | 上限 |
| --- | ---: |
| 模型轮次 | 6 |
| 工具调用 | 8 |
| 模拟次数 | 8 |
| A/B 候选 | 3 |
| 单轮输出 | 2048 tokens |
| 墙钟时间 | 60 秒 |

预算由服务端计数。`get_current_scenario` 必须先成功；取消、超时、provider 故障、协议错误、工具预算耗尽和证据不足都有固定终止状态。trace 只保存规范化事件和 evidence id，不包含隐藏推理、原始 provider payload 或凭据。

### 结构化报告与双层校验

模型输出 `agent-report-content/v1`，Orchestrator 再补入问题、scenario hash、provider/model、prompt 身份、用量、耗时和终止状态，形成 `agent-report/v1`。

第一层由 provider adapter 请求严格 JSON Schema 输出。OpenAI Responses 使用 `text.format` JSON Schema，Chat-compatible adapter 使用 `response_format.json_schema`；同时保持自定义 function tools、串行工具调用和 `store: false`。设计依据 [OpenAI Responses API create reference](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)。当前仍只用本地 mock 检查请求形状，没有连接外部 API。

第二层由本地 claim validator 执行：

- finding 和 recommendation 引用的 evidence id 必须来自本次 run；
- 数值只能位于结构化 `metrics`，不能混入自然语言字段；
- 每个 metric 必须给出 evidence id 和 `/result/...` JSON Pointer；
- Pointer 指向的源值必须是数值，并与模型提交值在浮点容差内相等；
- 报告校验失败只允许一次结构化修复；再次失败后输出“证据不足”报告，未经验证的 finding 被清空。

这保证的是“展示出来的数值可解析回模拟证据”，不是声称模型的自然语言推理永远正确。

### 离线 Fake Provider

fake provider 现在会确定性完成：读取场景 → 模拟基线 → 生成带 evidence id 与 JSON Pointer 的报告。它用于验证编排、工具、报告和失败状态，不代表真实模型质量。

## 自动验证结果

| 检查 | 结果 |
| --- | --- |
| `cargo check` | 通过；19 条既有 warning |
| `cargo test` | 106/106 通过；0 失败 |
| Release 构建 | 通过；19 条既有 warning |
| Orchestrator 成功闭环 | 通过；3 个模型轮次、2 个工具调用、1 次模拟 |
| 明确拒绝 | 通过；结构化 refusal 被保留 |
| 预取消 | 通过；0 模型轮次、0 工具调用 |
| 模拟预算耗尽 | 通过；超预算 A/B 在执行前被拒绝 |
| Provider 429 | 通过；固定 `provider_failed` 终止状态 |
| 伪造数值 | 通过；一次修复后降级，verified findings 为空 |
| Provider 结构化输出映射 | Responses 与 Chat 两种请求形状均通过 |
| `node --check frontend/app.js` | 通过 |

### 隔离 HTTP 与 Phase 1 回归

使用 release 可执行文件在 `127.0.0.1:3017` 启动临时实例，测试数据目录位于 `backend/target`，未设置真实 provider key。

- 五步 Agent HTTP smoke：通过；
- Phase 1 无模型评测：20/20；
- `write_attempts=0`；
- `userdata_unchanged=true`；
- `runtime_restored=true`；
- scenario `ad2cb2a2e686e60966e72bb8850a170d22107bca9d90d5b23467a4580e687160`；
- fingerprint `610e3ce3fd7851a0`。

验收后隔离端口 3017 已停止。原端口 3005 仍由 PID 15792 运行。真实 `backend/userdata` 仍为 178 个文件、863,482 字节。

## 阶段边界

- 没有真实模型调用、费用或外部网络请求；
- 没有 Run HTTP API、SSE manager 或聊天 UI；
- `AgentCancellation` 已建立协作式取消契约，活动 run 管理与断线取消属于 P2-04；
- 没有创建或写入 `agent_sessions/v1`；
- 真实 provider/model 的兼容性、质量、延迟和成本仍未验证；
- 如果选定模型要求跨工具轮次回传 opaque/encrypted reasoning，首次真实调用前仍需增加仅限当前 run 内存的 continuation 支持。

下一步是 P2-04：为这套编排器增加 Run API、规范化 SSE、单用户并发限制和取消生命周期。首次真实模型调用仍是阻断节点 A，必须先确认 profile/model 与费用上限。
