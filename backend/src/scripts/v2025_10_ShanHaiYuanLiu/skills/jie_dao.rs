//! 劫刀脚本 (ID: 13052)
//! 秘籍怒气减少已在 effective_rage_cost 中处理

use crate::*;
use super::{yuan_ge, lin_guang, xue_shi};

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 苍雪刀公共效果
    lin_guang::try_trigger(player, em, t);
    xue_shi::try_trigger(player, em, t);

    // 援戈
    if player.has_talent(36058) {
        yuan_ge::cast_skill(player, em, t);
    }
}
