# Agent P2-01/P2-02 Provider 层基线

日期：2026-08-25  
范围：provider 配置、统一模型协议、离线 provider、OpenAI Responses adapter、OpenAI-compatible Chat adapter、设置同步脱敏。  
真实模型调用：0。  
Agent 会话写入：0。

## 本次交付

### 服务端 provider catalog

- 未设置 `JX3_AGENT_CONFIG` 时只加载确定性 `offline` profile，主站不依赖外部模型即可运行；
- 管理员可通过 TOML 定义最多 16 个 profile；浏览器不能提交任意 provider URL；
- profile 将 `api_key_env` 与 key 值分离，配置文件只记录环境变量名；
- `GET /api/agent/providers` 只返回 `id / label / model / available`；
- 网络 profile 缺少 key 时显示不可用，不回传 key 名、base URL 或配置路径；
- 远程 provider 强制 HTTPS，HTTP 只允许 loopback，方便本地 mock 或本地模型服务。

示例配置见 [`../../config/agent.providers.example.toml`](../../config/agent.providers.example.toml)。实际文件应放在仓库外，或使用已忽略的 `agent.providers.toml`。

### 统一模型协议

`backend/src/agent/provider/protocol.rs` 定义 provider 无关的：

- `ModelRequest` 与 user/assistant/tool-result 消息；
- 强类型 `ToolDefinition`；
- `ProviderToolCall`；
- `ModelResponse`、统一 finish reason 与 token usage；
- 消息、工具数量、输出 token 和工具结果体积限制；
- 工具名、call id、重复工具和孤立 tool result 校验。

Orchestrator 后续只依赖该协议，不直接依赖任一上游 JSON 格式。

### Adapter

| Adapter | 上游路径 | 已验证行为 |
| --- | --- | --- |
| `fake` | 无网络 | 第一次确定性发出工具调用，收到工具结果后确定性结束 |
| `openai_responses` | `<base_url>/responses` | 自定义 function tools、`store: false`、串行工具调用、文本/拒绝/函数调用/usage 解析 |
| `openai_compatible_chat` | `<base_url>/chat/completions` | Chat messages、兼容 function tools、文本/拒绝/函数调用/usage 解析 |

OpenAI adapter 的请求设计依据 [OpenAI Responses API](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)。本阶段只通过 loopback mock 检查请求和响应契约，没有访问外部 API。

### 网络与日志安全

- API key 只存在于 adapter 内存，不实现 `Debug`/`Serialize`；
- HTTP client 30 秒超时并关闭自动重定向，避免 Authorization 被转发到另一个 origin；
- 成功响应最大 1 MiB；
- 429、5xx、超时、网络错误、畸形 JSON 和畸形 tool arguments 映射为固定安全错误；
- 错误不包含上游响应正文、请求正文、URL 或 key；
- 尚未实现任何 provider 请求/响应落盘或日志输出。

### 设置同步纵深防御

前端原先会将大部分 `localStorage` 同步到 per-user `settings.json`。现在浏览器保存、同步和启动回灌三处，以及后端保存/读取两处，都会过滤 API key、Authorization、credential、password、secret 和 token 类字段。

这不是凭据存储方案；正确方案仍是 key 从服务端环境变量注入。过滤器只防止未来 UI 代码误把敏感字段带入现有设置同步。

## 验证结果

### 自动测试

| 检查 | 结果 |
| --- | --- |
| `cargo check` | 通过；19 条既有 warning，0 条新增错误 |
| `cargo test` | 88/88 通过；0 失败 |
| Provider 专项测试 | 18/18 通过 |
| 设置敏感字段测试 | 1/1 通过 |
| `node --check frontend/app.js` | 通过 |
| Release 构建 | 通过 |

Provider 专项覆盖：配置边界、安全列表、缺 key 降级、重复 profile、HTTP/HTTPS 边界、fake 确定性、Responses 请求、Chat 请求、tool call/usage 解析、未注册或重复工具调用、畸形参数、超时、取消，以及 429/5xx 正文与 key 不回显。

### 隔离 HTTP 验收

使用 release 可执行文件在 `127.0.0.1:3017` 启动临时实例，`JX3_USERDATA_DIR` 指向 `backend/target` 下的隔离目录，未设置 provider 配置或 key。

结果：

- `/health`：通过；
- `/api/agent/providers`：返回 `agent-provider-list/v1`，仅 `offline` 且 `available=true`；
- 五步 Agent HTTP smoke：通过；
- 20 题无模型评测：20/20；
- `write_attempts=0`；
- `userdata_unchanged=true`；
- `runtime_restored=true`。

验收后 3017 已停止。原 3005 服务仍由原进程运行，真实 `backend/userdata` 仍为 178 个文件、863,482 字节。

## 尚未声称完成

- 没有选择或调用真实模型；
- 没有实现 prompt、Orchestrator、有界工具循环、SSE run manager 或 UI；
- 没有创建 `agent_sessions/v1` 或保存会话；
- 没有对任何真实 provider 的模型兼容性、成本、质量或限额作出结论；
- 若选定的推理模型要求跨工具轮次回传 encrypted reasoning，P2-03 必须先实现仅存在于当前 run 内存的 opaque continuation；该内容不得进入持久会话。

## 下一步

P2-03 将实现版本化 prompt、四工具注册表、有界 Orchestrator、`AgentReportV1` 和数值 claim 的 evidence validator。该阶段仍可完全使用 fake provider 开发；第一次真实模型调用前继续停在模型/profile/费用确认节点。
