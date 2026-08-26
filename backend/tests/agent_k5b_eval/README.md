# K5B 真实模型分层评测

同一组 6 题分别运行 DeepSeek V4 Pro 与 V4 Flash，不重试。它把结果拆成四层：

- `planning`：是否选择了与任务相符的最小工具路径；
- `version`：来源赛季和版本标记是否正确；
- `evidence`：资料与模拟证据是否各自承担正确职责；
- `expression_proxy`：结论、层级、边界、可读性和篇幅五项结构代理分，不等同于人工专业度评分。

运行会产生真实模型费用。必须显式确认后执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/agent-k5b-real-eval.ps1 -MaxCostUsd 0.20
```

脚本使用本地隔离端口与临时 userdata，单题不重试，结果写入被 Git 忽略的 `backend/runs/`。费用是按返回 usage 和公开单价估算，不冒充供应商账单。
