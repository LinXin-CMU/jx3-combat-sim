//! 自身受击事件处理（Boss 周期攻击触发）
//!
//! 受击结算顺序（严格）：
//! 1. 判定招架：用当前招架率（含坚铁加成）掷骰
//! 2. 叠层 + 刷新：若无 8321(恋战CD)，坚铁 +1 层 / 刷新 8s
//! 3. 锁定判定：若本次招架成功，添加 8321(恋战CD, 132帧)
//!
//! 扩展点：后续承伤模型在此添加（受击伤害计算、减伤 buff 等）

use crate::*;

/// 自身受击事件主入口
pub fn on_player_hit(player: &mut Player, t: f64) -> Vec<CastEvent> {
    let events = Vec::new();

    // 当前招架率（含坚铁加成）
    let parry_rate = {
        let buff_slots = aggregate_buff_fields(player);
        let stats = build_runtime_stats(&player.base_attrs, &buff_slots, &player.constants);
        stats.parry_rate
    };

    // 默认：招架成功（确定性模拟；后续可改为随机掷骰）
    let parried = parry_rate > 0.0;

    // ── 坚铁叠层（奇穴 13138）──
    // 期望传播存在时坚铁由概率模型管理（sync_expectation_buffs），跳过真实叠层
    if player.has_talent(13138) && player.expectation.is_none() {
        process_jiantie(player, t, parried);
    }

    // ── 寒甲刷新（奇穴 13134，非期望模式）──
    if parried {
        let hanjia_exp = player
            .expectation
            .as_ref()
            .map_or(false, |e| e.hanjia_expectation);
        if player.has_talent(13134) && !hanjia_exp {
            player.add_buff(BUFF_HAN_JIA);
            // Boss 受击模式下寒甲 tick_interval 已被禁用，手动 recalc A/B
            if player.next_boss_attack.is_some() {
                recalc_hanjia_stacks(player);
            }
        }
    }

    // ── 扩展点：承伤计算 ──
    // TODO: 受击伤害、减伤、招架化解

    events
}

/// 坚铁叠层 + 招架锁定
fn process_jiantie(player: &mut Player, _t: f64, parried: bool) {
    // 8321(恋战CD) 存在期间不叠层
    if player.has_buff(BUFF_LIAN_ZHAN_CD) {
        return;
    }

    // 步骤 2：叠层 + 刷新（无论是否招架）
    player.add_buff(BUFF_JIAN_TIE);

    // 步骤 3：招架成功 → 添加恋战 CD（132帧）
    if parried {
        player.add_buff(BUFF_LIAN_ZHAN_CD);
    }
}

/// 手动重算寒甲 A/B 层数（复用 buff_han_jia 的逻辑）
fn recalc_hanjia_stacks(player: &mut Player) {
    let parry_value = player.current_stats().parry_value;
    let bonus = (parry_value * 0.07).floor() as u32;
    let big = (bonus / 30000).min(125);
    let small = ((bonus % 30000) / 300).min(125);

    player.remove_buff(BUFF_HAN_JIA_SMALL);
    player.remove_buff(BUFF_HAN_JIA_LARGE);
    for _ in 0..big {
        player.add_buff(BUFF_HAN_JIA_LARGE);
    }
    for _ in 0..small {
        player.add_buff(BUFF_HAN_JIA_SMALL);
    }
}
