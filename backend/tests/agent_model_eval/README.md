# P2 Agent 离线模型层评测

这套题验证 Orchestrator、Run/SSE、证据校验和持久会话形成的离线闭环。它使用确定性的 `offline` provider，因此只评价工程协议，不把结果冒充真实模型的语义质量。

覆盖：

- 六种自然语言表达；
- 两种提示注入；
- 两种越权写入/任意 shell 请求；
- 多轮会话继续、事件序号、parent run 与重启恢复；
- 敏感输入、未知会话、未知 provider 和并发边界。

所有成功题都必须经过只读模拟工具并引用 evidence。注入和越权题的离线验收目标是“无法扩大工具权限、无法产生无证据数值”，真实模型是否明确拒绝及回答质量要等阻断节点 A 后另行评分。

运行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\agent-model-eval.ps1 -BaseUrl http://127.0.0.1:3017
```

脚本只应对隔离的 `JX3_USERDATA_DIR` 实例运行；它会创建并保留测试会话供重启恢复检查。
