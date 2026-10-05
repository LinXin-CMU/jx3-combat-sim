//! 闪刀脚本 (ID: 13053)

use super::{lin_guang, yuan_ge};
use crate::*;

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 苍雪刀公共效果
    lin_guang::try_trigger(player, em, t);

    if player.has_talent(36058) {
        yuan_ge::cast_skill(player, em, t);
    }

    // 战绝：闪刀无调息
    if player.has_buff(BUFF_ZHAN_JUE) {
        player.reset_cd("cd_闪刀");
    }
}
