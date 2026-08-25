//! 盾压脚本 (ID: 13045)
//!
//! 释放盾压时，若自身无"缓深"debuff，则：
//! 1. 给目标添加"步残"（封轻功，4秒）
//! 2. 给自身添加"缓深"（封轻功内置CD，15秒）
//!
//! 橙武 buff 期间：
//! - 盾压无调息（重置 CD）
//! - 怒气回复 +100%（额外 +15 怒气）

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 封轻功
    if !player.has_buff(BUFF_HUAN_SHEN) {
        player.add_target_buff(BUFF_BU_CAN);
        player.add_buff(BUFF_HUAN_SHEN);
    }

    // 严阵奇穴：盾压叠严阵 buff（每层 +50% 破招，max 3，20s）
    if player.has_talent(25356) {
        player.add_buff(BUFF_YAN_ZHEN);
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
        player.add_rage(15);
    }
}
