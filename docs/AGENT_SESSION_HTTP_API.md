# Agent Session API v1

Agent 会话保存在当前 worker 的 `JX3_USERDATA_DIR/agent_sessions/v1`。Router 为每个登录用户分配独立 worker 和独立 userdata，因此接口无需接收用户 id，也不会跨账号扫描目录。

## 数据模型

```text
agent_sessions/
  v1/
    <session_id>/
      meta.json
      events/
        000001.json
        000002.json
```

`meta.json` 使用 `agent-session-meta/v1`，创建后不覆盖。状态变化使用 `agent-session-event/v1` 编号事件；事件先写入同目录临时文件、同步落盘，再原子重命名为最终编号。第一版没有自动删除、归档或覆盖接口。

持久化内容包括用户可见问题、run/provider/model 身份、scenario/prompt hash、规范化工具步骤、evidence id、用量和最终 `AgentReportV1`。永不持久化 API key、Authorization、cookie、隐藏推理、原始 provider payload、完整时间轴或绝对路径。高置信凭据文本会在进入 run 前拒绝，写盘边界还会二次脱敏所有字符串。

## 创建或继续会话

`POST /api/agent/runs` 不传 `session_id` 时创建新会话：

```json
{
  "question": "分析当前循环。",
  "provider_profile": "offline",
  "simulation": {}
}
```

传入已有 id 时追加一轮：

```json
{
  "question": "继续验证上一个结论。",
  "provider_profile": "offline",
  "session_id": "session-...",
  "simulation": {}
}
```

创建响应除 Run URL 外还包含：

```json
{
  "session_id": "session-...",
  "session_url": "/api/agent/sessions/session-..."
}
```

后续 `run_started` 会记录上一轮 `parent_run_id`。损坏会话不会被覆盖，继续运行返回 `409 agent_session_corrupted`，用户可以新建会话。

## 会话列表

```http
GET /api/agent/sessions
```

返回 `agent-session-list/v1`，最多列出最近更新的 200 个会话。摘要包含标题、状态、最后 run、scenario hash、事件数和损坏事件数。列表由目录与事件派生，不依赖可损坏的全局索引。

## 会话详情

```http
GET /api/agent/sessions/:session_id
```

返回 `agent-session-detail/v1` 的 meta、派生摘要和有效事件。遗留 `.tmp-*` 文件会被忽略；无法解析或 sequence 不匹配的 JSON 计入 `corrupted_event_count`，原文件保持不变。

## 重启恢复

worker 启动时扫描会话。若最后一轮只有 `run_started`、没有 `run_result`，系统追加：

```json
{
  "kind": "run_interrupted",
  "code": "worker_restarted"
}
```

它不会自动重发模型请求。旧瞬态 Run API 仍返回 `404 agent_run_not_found`；用户从会话详情看到中断事实，再显式发起新 run。

## 固定错误

| HTTP | code | 含义 |
| ---: | --- | --- |
| 400 | `sensitive_input_rejected` | 问题疑似包含凭据 |
| 404 | `agent_session_not_found` | id 不存在或不安全 |
| 409 | `agent_session_corrupted` | 会话含损坏事件，禁止继续覆盖 |
| 503 | `agent_session_store_unavailable` | 会话目录不可写；主站其他功能继续运行 |
