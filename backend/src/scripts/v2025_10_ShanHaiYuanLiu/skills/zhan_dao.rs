//! 斩刀脚本 (ID: 13054)
//!
//! - 命中有虚弱/流血目标：移除虚弱，添加流血（26秒，每2秒跳伤害）
//! - 奇穴[绝返13090]：命中后获得[狂绝] 6秒
//! - 奇穴[麾远37239]：追加伤害，眩晕成功返还15怒+6秒斩刀调息
//! - 援戈

use super::{lin_guang, xue_shi, yuan_ge};
use crate::*;

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 破·斩刀（独立显示的破招段）
    em.emit("破·斩刀", 13054901, t);

    // 苍雪刀公共效果
    lin_guang::try_trigger(player, em, t);
    xue_shi::try_trigger(player, em, t);

    // 命中有虚弱或流血的目标：移除虚弱，添加/刷新流血
    if player.has_target_buff(BUFF_XU_RUO) || player.has_target_buff(BUFF_LIU_XUE) {
        player.remove_target_buff(BUFF_XU_RUO);
        player.add_target_buff(BUFF_LIU_XUE);
    }

    // 奇穴绝返：获得狂绝 buff
    if player.has_talent(13090) {
        player.add_buff(BUFF_KUANG_JUE);
    }

    // 奇穴麾远：追加伤害（眩晕假定失败，不返还怒气和调息）
    if player.has_talent(37239) {
        em.emit("麾远", 37239, t);
    }

    // 援戈
    if player.has_talent(36058) {
        yuan_ge::cast_skill(player, em, t);
    }

    // 战绝：斩刀无调息
    if player.has_buff(BUFF_ZHAN_JUE) {
        player.reset_cd("cd_斩刀");
    }
}
