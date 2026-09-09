//! 盾飞延迟切换 Buff 事件脚本
//!
//! on_expire: 0.375秒后切换擎刀姿态

use crate::*;

pub fn on_expire(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.set_stance(Stance::Blade);
}
