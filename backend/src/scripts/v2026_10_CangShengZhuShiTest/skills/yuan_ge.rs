//! 援戈子技能（Buff 27030）— 暗影千机版
//!
//! 调用时：检查是否有援戈Buff，有则消耗一层并触发援戈·血影伤害
//! 血怒期间伤害+50%（通过 runtime recipe 99230 实现）

use crate::*;

/// 血怒期间援戈·血影 +50% 秘籍 ID
const RECIPE_YUAN_GE_XUE_NU: u32 = 99230;

/// 援戈子技能：消耗一层Buff，触发血影伤害
pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    if player.has_buff(27030) {
        player.remove_buff_stack(27030);
        // 血怒期间 +50%
        if player.has_buff(BUFF_XUE_NU) || player.has_buff(BUFF_XUE_NU_JY) {
            em.emit_with_recipes("援戈·血影", 36482, t, vec![RECIPE_YUAN_GE_XUE_NU]);
        } else {
            em.emit("援戈·血影", 36482, t);
        }
    }
}
