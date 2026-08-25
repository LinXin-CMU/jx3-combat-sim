//! 阵云结晦 (ID: 30769) 一段脚本
//!
//! 授予阵云_2 连招 buff。
//! 二/三段由 combo_follow 机制在宏/序列层重定向到月照连营(30855)/雁门迢递(30856)，
//! 本脚本仅处理一段的副作用。

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    player.add_state_buff(combo_buff_id("阵云_2"), 720);
}
