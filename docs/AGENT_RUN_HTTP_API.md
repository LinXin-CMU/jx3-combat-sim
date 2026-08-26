# Agent Run API v1

本接口把只读 Orchestrator 暴露为有界任务。当前默认使用 `offline` fake provider；真实模型仍需单独确认。Run 的活动状态保存在当前 worker 内存中，同时创建或续接一个持久 Agent 会话。worker 重启后旧 run 返回稳定的 `agent_run_not_found`，但会话和规范化事件仍可通过 Session API 恢复；未完成 run 会被标记为 `interrupted`，不会自动重发外部请求。

## 创建 Run

```http
POST /api/agent/runs
Content-Type: application/json; charset=utf-8
```

```json
{
  "question": "分析当前循环的确定性输出，并说明证据边界。",
  "provider_profile": "offline",
  "session_id": "session-...（续接时可选）",
  "simulation": {
    "haste_level": 42087,
    "sequence": ["盾击", "盾压"],
    "network_delay": 0,
    "attributes": {
      "base_attack": 38466.0,
      "weapon_damage": 10986.0,
      "crit_level": 54841.0,
      "crit_effect_level": 0.0,
      "overcome_level": 29480.0,
      "strain_level": 66031.0,
      "haste_level": 42087.0
    },
    "target": {
      "level": 134,
      "defense_bonus": 0.0,
      "damage_cof": 0.0
    }
  }
}
```

服务端把 simulation 与当前 worker 的版本/心法绑定为不可变 `ScenarioSnapshotV1`，返回 `202 Accepted`：

```json
{
  "schema_version": "agent-run-created/v1",
  "run_id": "run-...",
  "session_id": "session-...",
  "scenario_hash": "...",
  "status": "accepted",
  "session_url": "/api/agent/sessions/session-...",
  "stream_url": "/api/agent/runs/run-.../stream",
  "status_url": "/api/agent/runs/run-...",
  "cancel_url": "/api/agent/runs/run-.../cancel"
}
```

同一 worker 同时只能有一个活动 run；第二个请求返回 `409 agent_run_conflict`。Router 模式下每个登录用户拥有独立 worker，因此该限制天然按用户隔离。

## 状态与结果

```http
GET /api/agent/runs/:run_id
```

运行中返回 `running=true`。终止后 `result` 是完整 `AgentRunResultV1`，包括报告、证据索引、预算用量和固定终止状态。响应同时包含 `session_id` 与 `persistence_error`；后者只在会话落盘失败时出现，Agent 会降级但主站仍可用。

## SSE

```http
GET /api/agent/runs/:run_id/stream
Accept: text/event-stream
```

事件只包含规范化字段：

```text
planning
tool_started
tool_finished
validating
report_repair_requested
completed / refused / cancelled / ...
run_result
```

每条事件都有单调递增的 `sequence`。新订阅者先重放当前 run 的内存事件，再接收实时事件；`run_result` 携带最终结构化结果并关闭流。事件不包含 provider 原始请求/响应、隐藏推理、API key、Authorization 或完整工具大对象。

SSE 断开不会让后台任务失去边界：run 仍受六十秒墙钟预算约束，可通过状态接口恢复查看。显式取消使用下面的接口。

## 取消

```http
POST /api/agent/runs/:run_id/cancel
```

取消是幂等操作。活动 run 会收到协作式取消信号；正在等待的 provider future 会被立即丢弃。已经终止的 run 返回 `accepted=false, already_terminal=true`，不会改写原结果。

## 固定错误

| HTTP | code | 含义 |
| ---: | --- | --- |
| 400 | `invalid_json` | 请求 JSON 不符合 schema |
| 400 | `invalid_question` | 问题为空、过长或包含非法控制字符 |
| 400 | `sensitive_input_rejected` | 问题疑似包含 API key、Authorization 或其他凭据 |
| 400 | `provider_not_found` / `provider_disabled` | profile 不在服务端允许列表 |
| 400 | `invalid_session_id` | session id 不符合安全标识格式 |
| 404 | `agent_session_not_found` | 指定续接的会话不存在 |
| 409 | `agent_run_conflict` | 当前 worker 已有活动 run |
| 422 | `invalid_scenario` | simulation 无法冻结为合法场景 |
| 503 | `provider_key_unavailable` | 网络 profile 的服务端凭据不可用 |
| 404 | `agent_run_not_found` | run 不存在、已被瞬态缓存淘汰或 worker 已重启 |

所有错误均使用 `agent-run-error/v1`，不回显请求正文、凭据、URL 或本地路径。

## 本地验收

后端启动后执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\agent-run-smoke.ps1
```

会话列表、详情、事件格式与落盘边界见 [`AGENT_SESSION_HTTP_API.md`](AGENT_SESSION_HTTP_API.md)。
