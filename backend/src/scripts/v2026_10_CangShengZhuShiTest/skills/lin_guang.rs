//! 麟光甲寒触发逻辑（苍雪刀公共模块）— 暗影千机版
//!
//! 有麟光玄甲（9层计数）时：
//! 1. 消耗一层麟光玄甲
//! 2. emit 麟光甲寒伤害
//! 3. 计数+1，满3次触发重置+回怒+业火焚城

use crate::*;

pub fn try_trigger(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    if !player.has_buff(BUFF_LIN_GUANG) {
        return;
    }

    // 消耗一层麟光玄甲
    player.remove_buff_stack(BUFF_LIN_GUANG);

    // 此旧招式未在测试服注册；不保留破招触发。
    em.emit("麟光甲寒", 34674, t);

    // 计数+1
    player.add_buff(BUFF_LIN_GUANG_COUNT);

    // 满3次：重置CD + 回怒 + 业火焚城
    let count = player
        .active_buffs
        .iter()
        .find(|b| b.buff_id == BUFF_LIN_GUANG_COUNT)
        .map(|b| b.stacks)
        .unwrap_or(0);

    if count >= 3 {
        player.reset_cd("cd_斩刀");
        player.reset_cd("cd_绝刀");
        player.reset_cd("cd_劫刀");
        player.reset_cd("cd_闪刀");
        player.add_rage_from(65, "麟光甲三层结算回怒");
        // 业火焚城伤害
        em.emit("业火焚城", 34714, t);
        player.remove_buff(BUFF_LIN_GUANG_COUNT);
    }
}
