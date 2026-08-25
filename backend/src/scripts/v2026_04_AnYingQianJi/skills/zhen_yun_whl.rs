//! 阵云结晦·雾海 (ID: 90010) — 雾海寻龙版一段
//!
//! 消耗所有长驱万里层数，记录层数供雁门迢递·雾海读取。

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 读取并消耗长驱万里层数
    let stacks = player.active_buffs.iter()
        .find(|b| b.buff_id == BUFF_CHANG_QU)
        .map(|b| b.stacks)
        .unwrap_or(0);
    player.zhen_yun_consumed_stacks = stacks;
    player.remove_buff(BUFF_CHANG_QU);
}
