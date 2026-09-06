//! 暗影千机（2026.04）- Buff 事件脚本

pub mod defs;
pub mod team_buffs;

pub mod buff_dun_fei;
pub mod buff_dun_fei_delay;
pub mod buff_han_jia;
pub mod buff_lin_guang;
pub mod buff_liu_xue;
pub mod buff_xu_ruo_delay;
pub mod buff_zhan_jue;

use super::super::SkillScriptFn;
use crate::{
    BUFF_DUN_FEI, BUFF_DUN_FEI_DELAY, BUFF_HAN_JIA, BUFF_JIAN_TIE, BUFF_LIN_GUANG, BUFF_LIU_XUE,
    BUFF_XU_RUO_DELAY, BUFF_ZHAN_JUE,
};

pub fn get_buff_on_tick(buff_id: u32) -> Option<SkillScriptFn> {
    match buff_id {
        BUFF_DUN_FEI => Some(buff_dun_fei::on_tick),
        BUFF_LIU_XUE => Some(buff_liu_xue::on_tick),
        BUFF_ZHAN_JUE => Some(buff_zhan_jue::on_tick),
        BUFF_HAN_JIA => Some(buff_han_jia::on_tick),
        _ => None,
    }
}

pub fn get_buff_on_expire(buff_id: u32) -> Option<SkillScriptFn> {
    match buff_id {
        BUFF_DUN_FEI => Some(buff_dun_fei::on_expire),
        BUFF_DUN_FEI_DELAY => Some(buff_dun_fei_delay::on_expire),
        BUFF_XU_RUO_DELAY => Some(buff_xu_ruo_delay::on_expire),
        BUFF_ZHAN_JUE => Some(buff_zhan_jue::on_expire),
        BUFF_HAN_JIA => Some(buff_han_jia::on_expire),
        BUFF_LIN_GUANG => Some(buff_lin_guang::on_expire),
        _ => None,
    }
}

pub fn get_buff_on_remove(buff_id: u32) -> Option<SkillScriptFn> {
    match buff_id {
        BUFF_DUN_FEI => Some(super::skills::dun_fei_on_remove),
        BUFF_HAN_JIA => Some(buff_han_jia::on_remove),
        _ => None,
    }
}

/// 战斗开始钩子：按奇穴 + 装备特效（大附魔/黄字 EnterFight 类）激活常驻 buff
pub fn on_battle_start(player: &mut crate::Player) {
    if player.has_talent(13134) && !player.has_buff(BUFF_HAN_JIA) {
        player.add_buff(BUFF_HAN_JIA);
        let mut em = super::super::ScriptEmitter::new();
        buff_han_jia::on_tick(player, &mut em, player.current_time);
    }
    // 奇穴 13138「坚铁」：由 Boss 周期受击事件驱动叠层（on_hit.rs）

    // 装备特效（大附魔 + 副本/无修精简黄字）EventType=EnterFight 类
    crate::equip_effects::on_battle_start(player);
}
