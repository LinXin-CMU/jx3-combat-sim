//! 盾舞脚本 (ID: 13048)
//!
//! 擎盾体态下每命中一个目标回复1点怒气（单目标 = 每跳1怒）
//! 测试服不再附带破招段。

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    let ticks = player.last_channel_ticks;
    // 擎盾体态：每跳回1怒 × 实际跳数（单目标）
    if player.stance() == Stance::Shield {
        player.add_rage_from(ticks as i32, "盾舞引导回怒");
    }
}
