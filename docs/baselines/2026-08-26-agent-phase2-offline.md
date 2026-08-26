# Agent Phase 2 离线完成基线

日期：2026-08-26

范围：P2-05 持久会话、P2-06 Agent 实验面板、P2-07 离线模型层评测、P2-08 离线工程与作品集收口。

真实模型调用：0。公网：关闭。

## 写入确认与真实 userdata

用户已明确确认新增 `agent_sessions/v1` 并连续推进到真实模型门槛。

写前：

- 文件：178；
- 字节：863,482；
- manifest SHA-256：`f4395818602446c985a0442eb6d8dbf082b445dec9767334d3024d75837d2e2a`；
- `agent_sessions`：不存在。

首次真实写入：

- session：`session-1a03bbe785b-0`；
- run：`run-1a03bbe785b-0`；
- provider/model：`offline / deterministic-fixture-v1`；
- 状态：completed；
- 事件：10；
- evidence：1；
- persistence error：false。

写后：

- Agent 新增文件：11；
- Agent 新增字节：6,344；
- 全目录：189 个文件、869,826 字节；
- 排除 `agent_sessions` 后原数据仍为 178 个文件、863,482 字节；
- 原数据 manifest 仍为 `f4395818602446c985a0442eb6d8dbf082b445dec9767334d3024d75837d2e2a`；
- Agent 文件高置信凭据扫描：0 命中。

没有移动、覆盖、迁移或删除任何旧文件。该会话按用户要求保留。

## P2-05：持久会话

- 不可变 `meta.json` + append-only 编号事件；
- 同目录临时文件、sync、原子 rename；
- `.tmp-*` 忽略，损坏 JSON 计数但不覆盖；
- 多轮 session 保存 parent run；
- worker 重启把未完成 run 追加为 `run_interrupted / worker_restarted`；
- 会话存储不可用时 Agent Run 返回固定 503，主站继续运行；
- 不提供隐式清理、删除或覆盖接口。

进程级恢复使用本地慢速 mock：会话在停止前为 running，重启后为 interrupted，最后事件为 `run_interrupted`，旧瞬态 run 返回 404。测试端口随后关闭。

## P2-06：实验面板

- Hub 和快速跳转均可进入 Agent；支持 `#page-agent` 深链；
- 安全 provider selector、当前场景状态、问题输入和取消；
- SSE 展示规范化工具轨迹，不展示思维链；
- 报告展示指标卡、evidence、scenario 与 prompt 身份；
- 历史会话恢复、损坏提示与可复现实验摘要复制；
- `agent.js` / `agent.css` 独立于既有大文件。

Microsoft Edge headless 在 1440×1000 下完成真实页面截图验收：深链打开、历史列表加载、五段流程和 composer 布局正常。

## P2-07：离线评测

一键验收最终结果：

| 检查 | 结果 |
| --- | --- |
| Rust | 113/113 |
| Phase 1 确定性工具题 | 20/20 |
| Phase 2 离线模型层题 | 12/12 |
| 数值 evidence 引用率 | 100% |
| 工具权限边界 | 100% |
| Phase 2 p50 / p95 | 11ms / 11ms |
| token / 成本 | 0 / 0 |
| 真实 userdata 隔离保护 | unchanged |

Phase 2 题包含自然语言变体、提示注入、越权请求、三轮长会话、凭据拒绝、未知 provider 和未知 session。它只评价 offline provider 下的工程闭环；不把该结果表述为真实模型准确率。

## P2-08：作品集收口

- 案例页：[`../AGENT_PORTFOLIO_CASE_STUDY.md`](../AGENT_PORTFOLIO_CASE_STUDY.md)；
- 90 秒脚本：[`../AGENT_90S_DEMO_SCRIPT.md`](../AGENT_90S_DEMO_SCRIPT.md)；
- 会话 API：[`../AGENT_SESSION_HTTP_API.md`](../AGENT_SESSION_HTTP_API.md)；
- 一键验收：`tools/agent-phase2-verify.ps1`；
- Run/session smoke 与模型层评测可独立复跑。

## 当前门槛

离线 Phase 2 已完成。尚不能宣称真实模型质量、延迟或成本；下一步只剩阻断节点 A：由用户确认具体 profile/model/API key 注入方式和开发费用上限，然后运行固定题集并补充真实成功/失败样本。公网发布阻断节点 C 仍未触发。
