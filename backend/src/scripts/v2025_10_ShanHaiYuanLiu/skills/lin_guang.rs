//! 麟光甲寒触发逻辑（苍雪刀公共模块）
//!
//! 有麟光玄甲时：
//! 1. emit 麟光甲寒伤害
//! 2. 麟黯不存在时：计数+1，满3次触发重置+回怒+添加虚弱+获得麟黯

use crate::*;

pub fn try_trigger(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    if !player.has_buff(BUFF_LIN_GUANG) { return; }

    // emit 麟光甲寒伤害 + 独立的破招段
    em.emit("麟光甲寒", 34674, t);
    em.emit("破·麟光", 34674901, t);

    // 计数和重置机制（麟黯存在时跳过）
    if !player.has_buff(BUFF_LIN_AN) {
        // 计数+1
        player.add_buff(BUFF_LIN_GUANG_COUNT);

        // 满3次：重置CD + 回怒 + 虚弱 + 麟黯
        let count = player.active_buffs.iter()
            .find(|b| b.buff_id == BUFF_LIN_GUANG_COUNT)
            .map(|b| b.stacks)
            .unwrap_or(0);

        if count >= 3 {
            player.reset_cd("cd_斩刀");
            player.reset_cd("cd_绝刀");
            player.reset_cd("cd_劫刀");
            player.reset_cd("cd_闪刀");
            player.rage = (player.rage + 65).min(100);
            player.add_target_buff(BUFF_XU_RUO);
            player.add_buff(BUFF_LIN_AN);
            player.remove_buff(BUFF_LIN_GUANG_COUNT);
        }
    }
}
