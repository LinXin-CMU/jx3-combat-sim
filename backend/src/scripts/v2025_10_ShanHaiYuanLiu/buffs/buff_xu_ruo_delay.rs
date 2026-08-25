//! 虚弱添加延迟 Buff 事件脚本
//!
//! on_expire: 0.125秒后给目标添加虚弱

use crate::*;

pub fn on_expire(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.add_target_buff(BUFF_XU_RUO);
}
