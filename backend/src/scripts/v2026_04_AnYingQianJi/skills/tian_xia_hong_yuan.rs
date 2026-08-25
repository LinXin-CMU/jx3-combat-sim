//! 天下宏愿脚本 (ID: 90002)
//!
//! 橙武主动技能：获得橙武 buff（4秒）

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.add_buff(BUFF_CHENG_WU);
}
