//! 业火麟光脚本 (ID: 34912)
//!
//! 获得麟光玄甲(14秒) + 返还盾飞25秒调息

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 获得麟光玄甲
    player.add_buff(BUFF_LIN_GUANG);

    // 返还盾飞25秒调息
    player.reduce_charge_cd(13050, 25.0);
}
