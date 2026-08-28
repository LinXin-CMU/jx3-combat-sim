# C-001 知识修订增量基线（2026-08-28）

## 结论

语雀作者已修正赴敌距离公式。当前可引用事实为：

```text
实际不触发突进的距离 = MAX（突进保护距离，4尺）
```

本次刷新没有放松版本安全边界，也没有把知识事实伪装成模拟器指标。Boss 距离补偿仍标记为 `target_distance_compensation_not_modeled`。

## 来源与快照身份

| 文档 | 语雀更新时间 | 本地正文 SHA-256 |
| --- | --- | --- |
| [苍云进阶机制（2025）](https://www.yuque.com/sgyxy/cangyun/advanced) | `2026-08-28T06:51:23Z` | `1e92a1d91519a5be0dfe28fd17d1f44d9a1167a36d58790353f44a67f083cc99` |
| [暗影千机·分山劲白皮书](https://www.yuque.com/sgyxy/cangyun/whitepaper-23) | `2026-08-28T06:50:22Z` | `0fed8f5e759d8073034ee88eeb5e80a34db1cb0712168dc635b4c1ebe589b1f6` |

两篇原文中旧 `MIN` 公式计数均为 0，新 `MAX` 公式计数均为 1。规范化文档、原始快照和迁移清单同步更新；迁移审计保持 161 条目录记录、159 篇运行时可用文档、0 个缺失文件与 0 个元数据问题。

## 版本策略

《苍云进阶机制（2025）》标题保留初始年份，但作者持续维护且本次在 2026-08-28 发布修订。清单对这一篇显式声明：

```yaml
version_policy: rolling_current
```

默认策略仍为 `title_bound`。只有显式标记的滚动文档可以越过“标题年份与赛季目录冲突”警告；旧赛季、仅元数据和抓取失败条目的过滤规则不变。该策略也进入 corpus hash，防止旧向量缓存被错误复用。

## 运行时记忆与回归

- 新增来源绑定 Claim：`fs-charge-001`；
- 固定检索题：`current_charge_distance_formula`；
- BM25 命中：Top 1《苍云进阶机制（2025）》；
- 固定集：24/24 Recall@5（100%，门槛 90%）；
- 安全断言：32/32；
- Rust 全量测试：183/183。

## Embedded 与部署验收

- corpus hash：`8e0ffb81856ae575166a8fe3837ce4d9c316b2178ad0bdabb78a5c95e6b01894`；
- 派生向量缓存：`bge-small-zh-v1.5-e787c393a2a7c259.bin`，7,774,318 字节；
- 固定集返回 `requested_mode=embedded`、`active_mode=hybrid_rrf`、`cache_state=hit`、`fallback_code=null`；
- 28 题缓存命中回归耗时 3.50 秒，C-001 题 Top 1；
- release 已部署至本机 `127.0.0.1:3005`，公网未开启；
- HTTP 来源卡闭环通过，C-001 问题返回《苍云进阶机制（2025）》、`current_exact`、`fact_eligible=true`、作者更新时间与语雀原文链接；
- 部署前会话目录有 1606 个文件；部署过程没有清空、覆盖或迁移目录，端到端验收只追加测试会话，部署后为 1639 个文件。
