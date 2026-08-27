# Agent Embedded 混合检索基线（2026-08-27）

## 结论

知识检索已从纯 BM25 演进为本地 `BM25 + BAAI/bge-small-zh-v1.5 + weighted RRF`。运行时不需要 Qdrant、外部向量库或 embedding API；Markdown 仍是唯一资料源，模型与向量均为 userdata 下可重建缓存。

## 架构与边界

- 语料：159 篇文档、3796 个确定性分块、corpus `8af06e3bf05c0eb90cf9934655f12700499b86ff221d6e5c238d225c727a579e`；
- Dense 模型：`BAAI/bge-small-zh-v1.5`，FastEmbed 运行的 ONNX 文件来自 `Xenova/bge-small-zh-v1.5`；
- 模型文件：94,851,877 bytes，SHA-256 `69a0b846f4f116b5e6aabf9546ea6754d02264f3211a13a1bd69b31b8040749a`；
- 向量：512 维、L2 归一化，缓存文件 7,774,318 bytes；
- 融合：BM25 与 Dense Top 100 加权 RRF，Dense 权重 0.7；
- 拒答：Dense 最低相似度 0.45；没有词法命中的纯 Dense 候选至少 0.55 才能进入融合池；
- 版本安全：当前/历史/体服/分类过滤先于两个召回通道，Dense 无权扩大版本范围；
- 降级：模型、缓存、索引或查询失败均回退 BM25，并在结构化响应中返回固定诊断码。

## 评测结果

| 检查项 | 结果 |
| --- | ---: |
| Rust 自动测试 | 164/164 |
| BM25 固定集 Recall@5 | 23/23（100%） |
| Hybrid 固定集 Recall@5 | 23/23（100%） |
| Hybrid 版本/来源/拒答断言 | 27/27 |
| Hybrid 当前模式 | `hybrid_rrf` |
| 向量缓存 | `hit` |
| 缓存命中 + 27 条查询 | 3.10s |
| 首次 3796 块索引构建 | 171.17s |

首次真 Dense 运行没有通过：乱码负例返回五个无词法命中的结果，最高相似度 0.429806。阈值从 0.36 校准到 0.45 后，负例重新稳定为空；正例 Recall@5 不变。该失败样本被保留为检索安全门槛的依据。

## 可验证增益

对照问题：`怎样减少战斗中的空转`

- 纯 BM25 Top 5 未出现《英雄及挑战阆风悬城_ 分山实战技巧》；
- Hybrid 以 Dense 相似度 0.571121 将该文档召回 Top 5；
- 对照由 Rust 测试 `configured_vault_hybrid_recovers_a_semantic_rotation_query` 固化，不依赖模型生成答案。

## 复现

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/agent-knowledge-eval.ps1 -Retrieval bm25
powershell -NoProfile -ExecutionPolicy Bypass -File tools/agent-knowledge-eval.ps1 -Retrieval embedded
```

若首次下载模型需要代理，只向启动或评测子进程设置标准 `HTTP_PROXY/HTTPS_PROXY`；凭据与代理配置不进入仓库。模型下载或索引失败时，评测摘要会显示 `active_mode: bm25` 和 `fallback_code`，不能视为 Embedded 验收通过。
