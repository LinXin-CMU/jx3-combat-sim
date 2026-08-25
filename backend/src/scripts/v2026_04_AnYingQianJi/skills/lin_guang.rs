//! 麟光甲寒触发逻辑（苍雪刀公共模块）— 暗影千机版
//!
//! 有麟光玄甲（9层计数）时：
//! 1. 消耗一层麟光玄甲
//! 2. emit 麟光甲寒伤害 + 破招段
//! 3. 计数+1，满3次触发重置+回怒+业火焚城

use crate::*;

pub fn try_trigger(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    if !player.has_buff(BUFF_LIN_GUANG) { return; }

    // 消耗一层麟光玄甲
    player.remove_buff_stack(BUFF_LIN_GUANG);

    // emit 麟光甲寒伤害 + 独立的破招段
    em.emit("麟光甲寒", 34674, t);
    em.emit("破·麟光", 34674901, t);

    // 实验性武学：麟光破招额外 +1 长驱万里（需破招值>0）
    if player.experimental && player.has_talent(30769) && player.base_attrs.surplus_value > 0.0 {
        player.add_buff(BUFF_CHANG_QU);
    }

    // 计数+1
    player.add_buff(BUFF_LIN_GUANG_COUNT);

    // 满3次：重置CD + 回怒 + 业火焚城
    let count = player.active_buffs.iter()
        .find(|b| b.buff_id == BUFF_LIN_GUANG_COUNT)
        .map(|b| b.stacks)
        .unwrap_or(0);

    if count >= 3 {
        player.reset_cd("cd_斩刀");
        player.reset_cd("cd_绝刀");
        player.reset_cd("cd_劫刀");
        player.reset_cd("cd_闪刀");
        player.add_rage(65);
        // 业火焚城伤害
        em.emit("业火焚城", 34714, t);
        player.remove_buff(BUFF_LIN_GUANG_COUNT);
    }
}
