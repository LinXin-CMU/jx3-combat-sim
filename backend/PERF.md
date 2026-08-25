# 模拟器性能优化与正确性防线

记录当前模拟器的优化架构、不变量、以及修改时的注意事项。

## 当前状态（2026-05-01，第二轮优化后）

### 性能（release build，绝云宏 300s 模拟）

| 指标 | 第一轮后 | 第二轮后 | 总 Δ |
|------|---------|---------|------|
| **Full core** | 12.1ms | **8.2ms** | -32% |
| **Lite core** | 10.6ms | **6.0ms** | -43% |
| **Full response（含 serde）** | ~14.5ms | **~10.7ms** | -26% |
| **Lite response** | ~10.6ms | **~6.5ms** | -39% |
| macro_phase1 ms | 4.05 | **1.18** | -71% |
| macro_phase2 ms | 1.55 | **0.62** | -60% |
| macro_advance calls | 1598 | 1598 | — |
| snapshot calls (Lite) | 700 | **0** | -100% |

### 已落地的优化

1. **Lite 模式（serialize-time + build-time 双层短路）**
   - serialize-time：响应体跳过 timeline 详情等字段（serde skip）
   - build-time：Lite=true 时不构造 `state_before` / `state_after` / `runtime_stats` / `runtime_recipes`、不写 `buff_events` 时间轴。**真正的"轻"，不再为不会被消费的字段干活**
2. **`next_decision_time`** — macro_eval 不再 frame-by-frame 推进，直接跳到下一个状态变化时刻（CD 结束 / buff 过期 / tick / channel end / boss attack / swing / bufftime 翻转）
3. **collect_recipes 倒排索引** — 启动时建 `(skill_id) → [recipe_idx]` 表，每次 cast 不再扫全 74 条秘籍
4. **active_ids 缓存** — 父事件 + N 个触发子事件复用同一个 HashSet
5. **Recipe ID 集 Player.buff_recipes** — buff 激活的隐藏秘籍（联动机制，可移除）
6. **Decision generation cache** — `Player::decision_generation` 在每次状态变更后 +1，给后续 macro 缓存预留接口
7. **`buff_idx_cache` / `target_idx_cache`** — Player 持有 `(buff_generation → AHashMap<buff_id, idx>)` 缓存，phase1 不再每次重建 HashMap。`process_buff_ticks` 已清理过期 buff，故 phase1 入口的 active_buffs 全部为活跃，无需 expires_at 过滤
8. **Phase1/Phase2 debug bookkeeping 关闭** — `evaluate_phase1` / `evaluate_phase2` 接 `enable_debug: bool` 参数。生产路径传 false，跳过 `Vec<Phase{1,2}Debug>` 构建（含 `display_string()` / `format!()` 等堆分配）。原 `debug_steps` 写入循环已注释，所以收集等于 0 消费（pure waste removal）
9. **`fill_event_damage` 非 override 路径不再 clone SkillSpec** — 旧代码 `(*spec).clone()` 在每次 fill_event 都跑（即使没 override），1300+ 次/sim 的无谓堆分配。现在仅在 override_attack_coeff = Some 时 clone

## 安全防线（4 层）

修改任何代码后，跑一次：
```bash
bash backend/tests/run_regression.sh             # 默认（快）
bash backend/tests/run_regression.sh --fuzz=30   # 加 30 个随机宏 fuzz
bash backend/tests/run_regression.sh --fuzz=100  # 高强度（CI 用）
```

跑完会执行：

### Layer 0：静态守卫（grep）
检查脚本里没有直写状态字段（绕过 setter）。任何 `player.rage = X` / `inst.stacks = N` / `player.active_cds.insert` 在 `src/scripts/` 出现都失败。详见 `tests/check_no_direct_writes.sh`。

### Layer 1：Property tests
`cargo test player_setter_tests` 跑 10+ 个不变量：
- `set_rage` / `add_rage` 必须 clamp 到 [0, 100] 且 bump generation
- `set_block_value` clamp 到 [0, max_block_value]
- `decision_generation` 严格单调递增
- `reset_cd` / `reduce_cd` / `reduce_charge_cd` 都 bump

### Layer 2：Versioned Golden v2

2025.10 与 2026.04 各自运行 4 个标杆 case。Golden v2 不只比较 fingerprint，还绑定版本、心法、场景哈希、数据/脚本哈希，并比较 DPS、总伤害、事件数和战斗时长。详见 `tests/diff_baseline.py` 与 `docs/baselines/2026-08-25-versioned-golden-migration.md`。

2026-05-01 的四个旧快照缺少版本元数据，已保存在 `tests/golden/legacy_unversioned/`，不再作为权威基线。

如果有意改了行为（新加 buff、改了 attack_coeff 等），跑：
```bash
python tests/diff_baseline.py --version all --update
```
重新写入 golden，提交时一起 commit。**审核 PR 时务必看到金标准更新理由**。

### Layer 3：随机 fuzz
`fuzz_macros.py` 随机生成宏（不同 stance / rage threshold / bufftime / buff 组合），断言：
- 同请求 3 次 fingerprint 一致
- Lite 与 Full bit-equal

可重现：`python tests/fuzz_macros.py --seed 42 --count 100`。

## 加新 buff / 技能 / 脚本时的检查清单

1. **Buff 加减**：用 `player.add_buff(...)` / `player.remove_buff(...)` / `player.add_buff_extended(...)` 等已有 setter。**不要直接 push/retain `active_buffs`**。
2. **Buff 层数覆盖**（如业火麟光给 9 层）：用 `player.set_buff_stacks(buff_id, n)`。
3. **CD 操作**：用 `player.reset_cd / reduce_cd / reduce_charge_cd`。protect 类 CD 用 `player.add_protect_cd(...)`。
4. **怒气**：`player.set_rage(v)` / `player.add_rage(delta)`。**不要 `player.rage = X`**。
5. **格挡值**：`player.set_block_value(v)` / `player.add_block_value(delta)`。
6. **新 tick 类 buff**：tick_interval > 0 的 buff，确认 `process_buff_ticks` 能正确发出 tick 事件。fingerprint 测试会 catch。
7. **新条件谓词** (新加 `MacroCondition` 变体)：扩展 `MacroCondition::collect_bufftime_thresholds` 把"时间型"条件登记到 `next_decision_time` 的事件源里。否则 macro 决策可能延迟到下一 frame。
8. **跑 `bash tests/run_regression.sh`**：确认 grep + property + golden 全过。
9. **Player 内部 helper 直接改 `active_buffs` / `target_buffs`**（极少见，目前仅 `sync_expectation_buffs` / `set_synthetic_layer`）：**末尾必须** `self.buff_generation += 1; self.bump_decision_gen();`，否则 `buff_idx_cache` / `aggregate cache` 失效不及时，phase1 条件求值或属性聚合会拿到 stale 数据。范式：定义 `mutated: bool` 标志，仅当真改了 active_buffs 成员或 stacks 才 bump。

## Player setter API 速查

| Setter | 用途 | 自动 |
|--------|------|------|
| `set_rage(v)` | 怒气 | clamp [0,100] + bump |
| `add_rage(delta)` | 怒气增减 | 同上 |
| `set_block_value(v)` | 格挡值 | clamp [0, max] + bump |
| `add_block_value(delta)` | 格挡值增减 | 同上 |
| `set_buff_stacks(id, n)` | 强制改层数 | bump |
| `add_protect_cd(name, expires)` | 加 protect 类 CD | bump |
| `reset_cd(cd_id)` | 重置 CD | bump |
| `reduce_cd(cd_id, sec)` | 缩减 CD | bump |
| `reduce_charge_cd(skill_id, sec)` | 缩减充能 CD | bump |
| `set_stance(stance)` | 切姿态 | 内部走 add/remove buff，间接 bump |
| `interrupt_channel(at)` | 打断引导 | bump |
| `add_buff(spec)` 等 6 个变体 | 加 buff | bump |
| `remove_buff(id)` 等 | 移除 buff | bump |
| `buff_idx_lookup()` | phase1 / 调试用：buff_id → idx 查询表（gen-keyed 缓存） | 自动按 buff_generation 失效 |
| `target_idx_lookup()` | 同上，target_buffs | 同上 |

## 调试 tips

- **Debug build 自动跑不变量**：`cargo run` (不带 --release) 时，每个 setter 调用后会 `debug_assert!` 检查字段范围、buff 一致性等。Release build 编译期消除，零成本。
- **fingerprint 漂了**：`python tests/diff_baseline.py` 会指出具体版本和 case。从 git log 找到最近改的脚本 / Player 方法 / data 文件。
- **DPS 对不上但 fingerprint 一样**：施放序列可能没变，但公式、系数或属性链已经变化；Golden v2 会将其判为失败。
- **DPS 对、fingerprint 不一样**：cast 顺序/时序变了。检查 `next_decision_time` 是不是漏了某个新加的事件源。

## 向后兼容承诺

- 每个游戏版本维护独立 Golden，不允许跨版本复用 fingerprint；
- 任何不影响行为的微调（重构、日志、缓存）必须保持对应版本的 fingerprint 与数值不变；
- 任何会改变数值的改动必须说明版本、规则来源和影响范围，并附 Golden v2 diff；
- Golden 更新必须来自干净工作区，并记录生成提交与数据哈希。

## 第二轮修复的潜伏 bug（2026-05-01）

`sync_expectation_buffs` (main.rs:3318-3380) 旧实现直接 `retain` / `push` / `iter_mut().find().stacks=...` `active_buffs`，**没 bump `buff_generation`**。后果：

- `buff_cache`（aggregate_buff_fields 缓存，按 generation）失效不及时 → 选了坚铁(13138) / 寒甲(13134) 奇穴的用户，招架率 / 会心率等聚合属性会读到 1~2 帧前的 stale stacks → DPS 估算偏离
- 第二轮新增 `buff_idx_cache` 也按 generation 失效 → 同样受影响

**修复**：sync 末尾按 `mutated` 标志位条件 bump（仅当真改了 active_buffs 成员或 stacks 才 bump）。
**回归覆盖**：`jueyun_jiantie_hanjia` case 锁定该路径。Versioned Golden v2 会同时检测施放序列与数值漂移。
