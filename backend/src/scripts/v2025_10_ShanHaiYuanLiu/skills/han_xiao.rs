//! 寒啸千军脚本 (ID: 15072)
//!
//! 消耗 20 格挡值，给自身添加寒啸千军 buff（无双率 +5%，15秒）。
//! 仅铁骨衣心法可用。

use crate::{Player, ScriptEmitter, BUFF_HAN_XIAO};

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 消耗 20 格挡值
    player.block_value = (player.block_value - 20).max(0);

    // 给自身添加寒啸千军 buff
    player.add_buff(BUFF_HAN_XIAO);
}
