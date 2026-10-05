//! 流血 Buff 事件脚本 (ID: 8249)
//!
//! on_tick: 每2秒触发流血伤害

use crate::*;

pub fn on_tick(_player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    em.emit("流血·每跳", 8249, t);
}
