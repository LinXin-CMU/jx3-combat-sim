//! 盾挡（13391）— 铁骨衣专属
//!
//! 消耗 10~100 怒气获得盾挡 buff（10级，对应消耗段）。每级附带 vitality→parry_value
//! 的转化系数（atVitalityToParryValueCof），千山奇穴用数值更高的版本（8448）。
//! cof 通过实例 extra_effects 绑定，聚合时换算成 ParryValueBase。

use crate::*;

/// 普通盾挡（8499）level 1~10 的 vitality→parry_value cof（/1024 制，游戏原始数据）
const DUN_DANG_COEFS: [f64; 10] = [
    41.0, 82.0, 123.0, 164.0, 205.0, 246.0, 287.0, 328.0, 369.0, 410.0,
];

/// 千山盾挡（8448）level 1~10 的 cof（约为普通版 1.25x，各级独立取值）
const DUN_DANG_QIAN_SHAN_COEFS: [f64; 10] = [
    51.0, 102.0, 154.0, 205.0, 256.0, 307.0, 358.0, 410.0, 461.0, 512.0,
];

/// 盾挡按当前怒气分摊 10~100，返回实际消耗量（供 Player::effective_rage_cost 使用）
pub fn effective_rage_cost(player: &Player) -> u32 {
    let tier = ((player.rage / 10).max(1) as u32).min(10);
    tier * 10
}

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    if player.mount != Mount::TieGuYi {
        return;
    }
    // apply_cast_effects 已按 effective_rage_cost 扣了对应怒气；此处只需读 last_rage_cost 决定 buff level
    let tier = (player.last_rage_cost / 10).clamp(1, 10);
    // 千山奇穴 13421：使用强化版盾挡 buff（8448）；否则普通版（8499）
    let (buff_id, cof) = if player.has_talent(13421) {
        (
            BUFF_DUN_DANG_QIAN_SHAN,
            DUN_DANG_QIAN_SHAN_COEFS[(tier - 1) as usize],
        )
    } else {
        (BUFF_DUN_DANG, DUN_DANG_COEFS[(tier - 1) as usize])
    };
    player.add_buff((buff_id, tier));
    // 把 cof 绑到 buff 实例上；聚合时由 aggregate_buff_fields 换算 → ParryValueBase
    player.bind_buff_effects(
        buff_id,
        vec![EffectEntry {
            field: AttribField::VitalityToParryValueCof,
            value: cof,
        }],
    );
    // 振奋奇穴 13422：按"基础体质"算层数（每层 +101 StrainBase，上限100层）
    if player.has_talent(13422) {
        let stacks = ((player.current_stats().base_vitality / 2820.0).floor() as u32).min(100);
        if stacks > 0 {
            // 先清除旧的（避免叠加到上次残留），再循环叠到目标层数
            player.remove_buff(BUFF_ZHEN_FEN);
            for _ in 0..stacks {
                player.add_buff(BUFF_ZHEN_FEN);
            }
        }
    }
}
