//! 业火麟光脚本 (ID: 34912) — 暗影千机版
//!
//! 获得 9 层麟光甲（25秒） + 返还盾飞25秒调息
//! 每次苍雪刀消耗一层，触发麟光甲寒

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 获得 9 层麟光甲
    player.add_buff(BUFF_LIN_GUANG);
    player.set_buff_stacks(BUFF_LIN_GUANG, 9);

    // 返还盾飞25秒调息
    player.reduce_charge_cd(13050, 25.0);
}
