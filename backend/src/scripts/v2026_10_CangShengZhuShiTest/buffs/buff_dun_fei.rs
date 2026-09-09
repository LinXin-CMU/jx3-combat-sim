//! 盾飞 Buff 事件脚本 (BUFF_DUN_FEI)
//!
//! on_tick: 每秒触发盾飞·每跳伤害 + 目标无流血时刷新虚弱
//! on_expire: 自然到期时自动触发盾回（切换擎盾）

use crate::*;

pub fn on_tick(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    em.emit("盾飞·每跳", 13463, t);
    // 目标无流血时，每跳刷新虚弱（延迟0.125s，简化为立即）
    if !player.has_target_buff(BUFF_LIU_XUE) {
        player.add_target_buff(BUFF_XU_RUO);
    }
    // 奇穴盾威：每跳触发/刷新盾威 debuff（-5% 伤害输出，15s）
    if player.has_talent(13320) {
        player.add_target_buff(BUFF_DUN_WEI);
    }
}

pub fn on_expire(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    player.set_stance(Stance::Shield);
    em.emit("盾回", 13051, t);
    // 自然到期盾回：触发保护CD（不获得坚定，只有主动盾回才给）
    player.add_protect_cd("protect_盾飞盾回", t + 1.0);
}
