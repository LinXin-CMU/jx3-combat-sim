# v48：接入旧工作流 A 算法调优

## 阅读依据

`frontend/app.js` 的 Step 6：pruneRedundant、castDiffFeedback、swapSearch、tightenSearch、autoIterate；`backend/src/macro_prune.rs` 的候选枚举；现有 batch_simulate 的 Lite + timeline 路径。

主线：旧生成 → 算法调优 → Agent 结合上下文微调 → 测试/再次算法调优 → 一份最终宏与说明。

## 已接入

- `distill_macro` 默认生成后自动调优，支持传入 macro_text 对模型修订版再次调优。
- 完整继承冻结模拟环境，以原轴实际时长跑宏；调用同一个 simulate_core，使用 lite_keep_timeline 统计释放数。
- 直接复用 macro_prune 的剪枝、同页相邻 swap、数值条件候选枚举，未修改旧生成器和旧前端。
- 释放数统计沿用工作流的主动技能/连段别名及雾海特殊名称口径。
- 自适应顺序：原始次数偏差 >5 时先 cast-diff，然后 prune、swap、tighten。
- 保留工作流阈值：敏感技能权重5；cast-diff 改善且 DPS 跌幅<=0.5%；剪枝跌幅<=0.2%、敏感/普通漂移1/2；swap 提升>=0.2%；tighten 跌幅<=0.2%。
- 新适配器对 tighten 额外落实旧注释里的“次数偏差不恶化”；剪枝检查新增技能在内的集合；各阶段不增加宏字数溢出。
- 不移植旧前端剪枝后未经重新模拟就接受的浮点比较符归一化。交付候选必须确实被评估。

## 资源和记录

每次最多96次候选模拟（参数可设1..128），每轮Agent默认模拟总额度128（配置校验最大256），实际逐次计入 simulations。阶段按份额分配搜索机会，预留2次后续验证；每次算法调用20秒软时间检查，单次 simulate_core 本身不支持中途抢占。

算法缓存相同文本结果，避免循环回到已接受版本。达到额度时返回已测候选和 evaluation_limit/time_limit，不能称为找到最优解。完整候选试验记录在 evidence.tuning.trials；模型接收初稿/最终指标和接受步骤摘要，正常用户报告只输出最终版本。

## 验证

334 Rust 测试通过，含接受准则、预算计数、冻结场景不变、最终候选独立 Full 复算与 Lite 调优 DPS 一致、非法宏/额度拒绝。宏质量及模型进一步微调收益需真实任务验证。

隔离发布回归：HTTP 5/5、工具20/20、模型离线12/12。历史 golden 仍为已知 data_sha256 漂移，未覆盖。已部署 127.0.0.1:3005，SHA256 `6A0174140CEDB0275053EB9360056116180CC30B752F971EDB7737F36EC561EA`；30460 个已有文件改变0个。重复 debug 和隔离测试服务已按路径核对后停止。
