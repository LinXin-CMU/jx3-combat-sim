# Agent 知识检索评测集

这套用例评价本地版本知识检索，不评价模型文风。正例以 Recall@5 计分，安全用例作为硬断言。

覆盖当前赛季机制、循环、配装、宏、实战，历史赛季、体服、跨版本，中文同义表达、技能名与数字混合查询，以及仅元数据、抓取失败、版本冲突和无答案场景。

运行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/agent-knowledge-eval.ps1
```

默认读取 `%USERPROFILE%\Documents\苍云策划知识库\朔风卷雪`，也可用 `-VaultRoot` 指定。评测不会创建 Agent 会话、调用模型、写入 Vault 或访问网络。
