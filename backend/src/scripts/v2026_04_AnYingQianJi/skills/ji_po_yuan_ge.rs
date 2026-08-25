//! 击破·援戈子脚本 (ID: 36058)
//!
//! 盾击触发：造成破招伤害，获得一层援戈Buff

use crate::*;

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    em.emit("击破·援戈", 36065, t);
    player.add_buff(BUFF_YUAN_GE_ID);
}
