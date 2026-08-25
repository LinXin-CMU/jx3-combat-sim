# Agent 只读工具 HTTP API v1

状态：P1-05 已实现（2026-08-25）  
边界：本协议只连接确定性工具层，不连接语言模型，不持久化 trace，不读写用户配置。

## 共同约束

- 四个端点均为 `POST`，由现有认证/router 转发到用户独立 worker；
- 客户端先提交完整 `SimulateRequest` 获取不可变场景快照，后续调用必须携带该快照；
- 场景以 canonical SHA-256 标识，版本或心法切换后旧快照会返回 `runtime_mismatch`，不会静默混算；
- `trace_id` 只允许 1–64 字节的 ASCII 字母、数字、`_`、`-`；服务端不保存 trace；
- `data_hash` 在 worker 启动或显式重载时计算，单次工具调用不遍历文件系统；
- 所有模拟预算均有端点硬上限，客户端只能收紧预算，不能提高上限。

## 端点

### `POST /api/agent/tools/scenario`

请求：

```json
{
  "trace_id": "analysis-001",
  "simulation": { "完整 SimulateRequest": "..." }
}
```

响应包含：

- `scenario`：绑定当前 `game_version`、`mount`、完整模拟输入和 `scenario_hash`；
- `evidence`：`get_current_scenario` 的紧凑配置摘要。

### `POST /api/agent/tools/simulate`

```json
{
  "trace_id": "analysis-001-sim",
  "scenario": { "scenario 端点返回的完整快照": "..." },
  "max_simulations": 1
}
```

响应只返回强类型模拟证据摘要：DPS、总伤害、战斗时长、技能数、fingerprint 和技能伤害占比。硬上限为 1 次模拟。

### `POST /api/agent/tools/compare`

```json
{
  "trace_id": "analysis-001-ab",
  "baseline": { "scenario 端点返回的完整快照": "..." },
  "candidates": [
    {
      "label": "network-25ms",
      "patch": { "network_delay": 25 }
    }
  ],
  "max_simulations": 4
}
```

一次允许 1–3 个候选。响应包含 baseline、显式字段 diff、候选指标和 DPS delta；硬上限为 baseline + 3 个候选共 4 次模拟。

### `POST /api/agent/tools/timeline`

```json
{
  "trace_id": "analysis-001-timeline",
  "scenario": { "scenario 端点返回的完整快照": "..." },
  "max_simulations": 1
}
```

服务端重放快照并返回两个 envelope：

- `simulation`：本次确定性模拟证据；
- `timeline`：引用 `simulation.evidence_id` 的时间线分析证据。

客户端不能上传一份自称可信的 timeline 结果。第一版不持久化服务端执行结果，因此由服务端重放场景比接收客户端计算产物更容易审计。

## 成功证据

成功工具响应使用 `agent-evidence/v1`，至少包含：

```text
schema_version, trace_id, evidence_id, tool_name, scenario_hash,
engine_version, engine_commit, data_hash, args, result, warnings, duration_ms
```

`engine_commit` 未在构建时注入时明确为 `unknown` 并带 warning；作品集正式构建必须由 CI 注入提交号。

## 稳定错误 envelope

错误响应使用 `agent-tool-error/v1`，只返回安全 trace、合法场景哈希、工具名、稳定 code 和脱敏消息。主要错误码：

| HTTP | code | 含义 |
|---:|---|---|
| 400 | `invalid_json` | JSON 或顶层工具 schema 不合法 |
| 400 | `invalid_trace_id` | trace ID 不符合安全字符约束 |
| 400 | `invalid_scenario` | 场景缺字段或字段值非法 |
| 400 | `invalid_budget_limit` | 客户端预算超过端点硬上限 |
| 400 | `invalid_candidate_count` | 候选数不在 1–3 |
| 400 | `invalid_candidate_label` | 候选标签非法 |
| 400 | `duplicate_candidate_label` | 候选标签重复 |
| 409 | `scenario_hash_mismatch` | 场景内容与声明哈希不一致 |
| 409 | `runtime_mismatch` | worker 当前版本/心法与快照不一致 |
| 422 | `no_scenario_changes` | typed patch 没有产生真实变化 |
| 422 | `timeline_details_unavailable` | 时间线缺少确定性分析所需字段 |
| 429 | `budget_exceeded` | 调用所需模拟次数超过客户端预算 |
| 500 | `internal_error` | 内部序列化失败，响应不暴露内部细节 |

## 本地验证

```powershell
.\tools\agent-smoke.ps1 -BaseUrl http://127.0.0.1:3005
```

脚本覆盖四个成功端点、跨工具 evidence chain、重复模拟证据一致性和超预算错误码。
