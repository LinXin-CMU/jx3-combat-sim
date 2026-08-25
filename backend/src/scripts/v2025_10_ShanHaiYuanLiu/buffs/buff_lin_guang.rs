//! 麟光玄甲 buff 到期脚本
//!
//! on_expire: 麟光玄甲到期时移除麟黯

use crate::*;

pub fn on_expire(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.remove_buff(BUFF_LIN_AN);
}
