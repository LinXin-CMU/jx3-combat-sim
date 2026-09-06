//! 盾压脚本 (ID: 13045)
//!
//! 释放盾压时，若自身无"缓深"debuff，则：
//! 1. 给目标添加"步残"（封轻功，4秒）
//! 2. 给自身添加"缓深"（封轻功内置CD，15秒）
//!
//! 橙武 buff 期间：
//! - 盾压无调息（重置 CD）
//! - 怒气回复 +100%（额外 +15 怒气）
//!
//! 驭焰橙武装备：1024 制累计 prob=205，触发"盾压·神兵"(25797)

use crate::equip_effects::YU_YAN_WEAPON_IDS;
use crate::*;

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 封轻功
    if !player.has_buff(BUFF_HUAN_SHEN) {
        player.add_target_buff(BUFF_BU_CAN);
        player.add_buff(BUFF_HUAN_SHEN);
    }

    // 盾压释放后重置期望 CD 状态
    if let Some(ref mut d) = player.dunya_cd {
        d.cd_remain = d.cd_frames as f64;
        d.avail_credit = 0.0;
    }

    // 橙武效果
    if player.has_buff(BUFF_CHENG_WU) {
        // 无调息：重置盾压 CD
        player.reset_cd("cd_盾压");
        // 怒气回复 +100%：盾压回15怒，再加15
        player.add_rage_from(15, "盾压额外回怒");
    }

    // 驭焰装备特效：盾压·神兵 期望累计触发（prob=205/1024 ≈ 20%）
    if player.has_equip_in("PRIMARY_WEAPON", YU_YAN_WEAPON_IDS)
        && player.accum_equip_effect(25797, 205, 1024)
    {
        em.emit("盾压·神兵", 25797, t);
    }
}
