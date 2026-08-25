//! 援戈子技能（Buff 27030）
//!
//! 调用时：检查是否有援戈Buff，有则消耗一层并触发援戈·血影伤害

use crate::*;

/// 援戈子技能：消耗一层Buff，触发血影伤害
pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    if player.has_buff(27030) {
        player.remove_buff_stack(27030);
        em.emit("援戈·血影", 36482, t);
    }
}
