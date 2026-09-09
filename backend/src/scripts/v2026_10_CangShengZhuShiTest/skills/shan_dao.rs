//! 闪刀脚本 (ID: 13053)

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 战绝：闪刀无调息
    if player.has_buff(BUFF_ZHAN_JUE) {
        player.reset_cd("cd_闪刀");
    }
}
