//! 雁门迢递·雾海 (ID: 90012) — 雾海寻龙版三段
//!
//! 根据阵云结晦·雾海消耗的长驱万里层数追加 5 段绝国伤害。
//! 绝国 attack_coeff = stacks × 0.19375（1~15层）或 stacks × 0.3875（16~45层）

use crate::*;

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    let stacks = player.zhen_yun_consumed_stacks;
    if stacks == 0 { return; }

    // 按层数选系数
    let coeff_per_hit = if stacks <= 15 {
        stacks as f64 * 0.19375
    } else {
        stacks as f64 * 0.3875
    };

    // emit 5 段绝国
    for _ in 0..5 {
        em.emit_with_coeff("绝国", 30858, t, coeff_per_hit);
    }

    // 清除消耗记录
    player.zhen_yun_consumed_stacks = 0;
}
