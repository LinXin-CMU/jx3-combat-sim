# Agent P2-04 Run API 与 SSE 生命周期基线

日期：2026-08-26

范围：瞬态 RunManager、创建/状态/流/取消 API、实时与重放 SSE、单 worker 并发限制、Router 流式代理回归。

真实模型调用：0。

Agent 会话写入：0。

公网状态：关闭。

## 本次交付

### 每用户单活动 Run

`AgentRunManager` 属于 `SharedState`，每个 worker 同时只允许一个活动 Agent run。Router 原有架构会为每个已认证用户分配独立 worker，因此并发限制和内存事件天然按用户隔离。

Run 创建时：

1. 只接受服务端 provider catalog 中的 profile id；
2. 把完整 simulation 与当前 worker 的版本/心法冻结为 `ScenarioSnapshotV1`；
3. 立即返回 `202 Accepted`、run id、scenario hash 和三个后续 URL；
4. 后台执行 P2-03 的有界 Orchestrator。

瞬态内存最多保留十六个最近 run。它们不是会话数据，进程重启后不会恢复；P2-05 才会在人工确认后增加 append-only 会话事件。

### Run API

实现接口：

```text
POST /api/agent/runs
GET  /api/agent/runs/:run_id
GET  /api/agent/runs/:run_id/stream
POST /api/agent/runs/:run_id/cancel
```

请求、响应和固定错误见 [`../AGENT_RUN_HTTP_API.md`](../AGENT_RUN_HTTP_API.md)。一键验收脚本为 [`../../tools/agent-run-smoke.ps1`](../../tools/agent-run-smoke.ps1)。

### 规范化 SSE

Orchestrator 增加同步事件观察边界，RunManager 只发布：

- planning；
- tool_started / tool_finished；
- validating / report_repair_requested；
- 固定终止状态；
- 携带最终 `AgentRunResultV1` 的 run_result。

SSE 不透传 provider 原始 payload、隐藏推理、工具大对象、请求正文或凭据。每个事件拥有 RunManager 分配的单调 sequence。订阅者先重放当前 run 已产生的内存事件，再进入实时流；即使页面在 run 完成后连接，也能取得完整规范化事件和最终报告。run_result 发出后流正常结束。

事件 channel 容量为六十四，而单次 run 的硬预算使规范化事件总量低于该容量；实现仍在 broadcast lag 时从内存快照补齐缺失 sequence。

### 取消、断线与超时

- 取消为幂等操作；同一 run 只产生一个 cancel_requested 事件；
- `AgentCancellation` 从原子标志升级为原子标志加异步通知；
- Orchestrator 用 `tokio::select!` 同时等待 provider、墙钟超时和取消，取消会立即丢弃正在等待的 provider future；
- SSE 客户端断线只释放订阅者，不持有或泄漏后台任务；run 仍受六十秒硬超时约束；
- worker 重启后瞬态 run 明确返回 `404 agent_run_not_found`，不会假装已经恢复；P2-05 将用持久事件把未完成 run 标记为 interrupted。

## 自动验证结果

| 检查 | 结果 |
| --- | --- |
| `cargo check` | 通过；19 条既有 warning |
| `cargo test` | 109/109 通过；0 失败 |
| Release 构建 | 通过；19 条既有 warning |
| Fake RunManager 闭环 | 通过；planning 到 run_result sequence 连续 |
| 同 worker 并发限制 | 通过；第二个活动 run 返回 conflict |
| Provider 等待中取消 | 通过；一秒内终止为 cancelled |
| 重复取消 | 通过；只产生一个 cancel_requested |
| PowerShell smoke 语法 | 通过；兼容 Windows PowerShell 5 |

### 隔离 HTTP 验收

在 `127.0.0.1:3017` 使用隔离 target 数据目录启动最终 release：

- Run 创建、状态轮询、完成后 SSE 重放、终态取消和未知 id：全部通过；
- 离线 run 状态：completed；
- 工具调用：二次；
- 报告 evidence id：一个；
- scenario `4aafafbb3da97fda40c62911d34ed7ac48c60d23c39a80b22bf96ced313e8f08`；
- 原五步 Agent HTTP smoke：通过；
- Phase 1 二十题：20/20；
- `write_attempts=0`、`userdata_unchanged=true`、`runtime_restored=true`。

停止并用相同隔离目录重启 worker 后，旧瞬态 run 稳定返回 404。验收后端口 3017 已关闭。

### Router SSE 代理回归

在隔离端口 3018 启动 Router，通过测试账号进入独立 worker，在 Router 外层完成：登录 → 创建 offline run → 接收 run_result SSE。流式代理通过。随后核对可执行文件路径并停止测试 worker 与 Router；端口 3018 和临时 worker 端口均已关闭。

原本地服务端口 3005 仍由 PID 15792 运行。真实 `backend/userdata` 仍为 178 个文件、863,482 字节。

## 阶段边界与下一阻断点

- 没有调用真实模型或产生费用；
- 没有创建 `agent_sessions/v1`，也没有向真实 userdata 写 Agent 文件；
- 没有前端 Agent 面板；
- 瞬态 run 不承诺跨进程恢复，完整恢复属于 P2-05；
- 真实 provider 的延迟、成本和质量仍未验证。

下一步是 P2-05 持久会话。它将首次新增 `JX3_USERDATA_DIR/agent_sessions/v1`，因此必须先重新核对真实 userdata 哈希、事件脱敏字段、原子写入和“不自动删除”策略，并由用户确认后才能实施。
