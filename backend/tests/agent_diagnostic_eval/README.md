# Agent 动态诊断真实问题集

这 12 题来自实际使用中的提问方式，用同一份 300 秒分山劲宏冻结场景检查四件事：是否理解循环、是否抓住主问题、是否执行必要实验、回答是否自然可用。

自动结果只检查过程与明显缺失，不代替玩法质量评审。Runner 会保留每题完整脱敏回复、工具轨迹、诊断状态、耗时和 token，供第一个人工确认节点逐题查看。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tools\agent-diagnostic-real-eval.ps1 -BaseUrl http://127.0.0.1:3005 -ProviderProfile deepseek-v4-flash
```
