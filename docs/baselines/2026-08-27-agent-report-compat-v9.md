# Agent 报告兼容性修复验收（2026-08-27）

## 故障

同一基线问题在 DeepSeek V4 Pro 与 V4 Flash 上均完成场景读取和模拟，但最终显示“未形成可靠结论”。持久会话确认共同错误为 `invalid_report_json`：模型最终输出无法按纯 `AgentReportContentV1` JSON 解析，修复轮仍失败。

Token 账本同时表明 Flash 的两轮成文输出接近每轮 2048 token 上限，说明供应商外层 Markdown/说明文字与冗长截断是两个叠加因素，而非模拟器或证据失效。

## 修复

- Prompt 升级为 `agent-system/v9`，要求 1–3 条结论、最多 1 条建议、最多 3 条边界和最多 4 个指标，并限制各文本字段长度。
- 报告解析允许拆除 Markdown 代码块、简短前言及一次额外 JSON 字符串编码。
- 拆包装后仍反序列化到 `deny_unknown_fields` 类型，并继续执行原有 schema、证据 ID、JSON Pointer、数值和来源资格校验。
- 修复轮同样加入 1200 token 紧凑预算；不通过的具体声明继续被删除，不降级证据真实性。

## 自动验证

- Rust：160/160。
- 新增 Markdown 包装、双重 JSON 编码、无关 JSON 拒绝三项测试。
- 前端和 HTTP smoke：通过。

## 同题真实模型对照

问题：`分析当前循环的输出基线，并说明证据边界。`

| Provider | 结果 | 时延 | 输出 tokens | 已发布内容 |
| --- | --- | ---: | ---: | --- |
| DeepSeek V4 Pro | `partially_verified` | 15.7 s | 1080 | 3 条结论、1 条建议 |
| DeepSeek V4 Flash | `partially_verified` | 11.1 s | 1317 | 2 条结论、1 条建议 |

两者均不再进入 `evidence_insufficient / invalid_report_json`。`partially_verified` 表示个别无引用或不匹配声明被逐项删除，保留报告仍可阅读和追溯。
