//! 战绝 Buff 事件脚本 (BUFF_ZHAN_JUE)
//!
//! on_tick: 每3秒回复100点怒气
//! on_expire: 切换回擎盾姿态

use crate::*;

pub fn on_tick(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.rage = (player.rage + 100).min(100);
}

pub fn on_expire(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.set_stance(Stance::Shield);
}
