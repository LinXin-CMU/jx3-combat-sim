//! 山海源流（2025.10）- 技能脚本

pub mod dun_bi;
pub mod dun_dang;
pub mod dun_dao;
pub mod dun_fei;
pub mod dun_hui;
pub mod dun_ji;
pub mod dun_meng;
pub mod dun_wu;
pub mod dun_ya;
pub mod han_xiao;
pub mod ji_po_yuan_ge;
pub mod jie_dao;
pub mod juan_xue;
pub mod jue_dao;
pub mod lin_guang;
pub mod shan_dao;
pub mod tian_xia_hong_yuan;
pub mod wu_ju;
pub mod xue_nu;
pub mod xue_shi;
pub mod yan_men;
pub mod yan_shou_gu_jing;
pub mod ye_huo_lin_guang;
pub mod yuan_ge;
pub mod yue_zhao;
pub mod zhan_dao;
pub mod zhen_yun;

use super::super::SkillScriptFn;
use crate::{Player, ScriptEmitter};

/// 按 skill_id 查找本版本的技能脚本（脚本内部用 player.mount 分支心法差异）
pub fn get_skill_script(skill_id: u32) -> Option<SkillScriptFn> {
    match skill_id {
        13047 => Some(dun_ji::cast_skill),
        13052 => Some(jie_dao::cast_skill),
        13053 => Some(shan_dao::cast_skill),
        13054 => Some(zhan_dao::cast_skill),
        13055 => Some(jue_dao::cast_skill),
        13050 => Some(dun_fei::cast_skill),
        13051 => Some(dun_hui::cast_skill),
        13040 => Some(xue_nu::cast_skill),
        13044 => Some(dun_dao::cast_skill),
        13048 => Some(dun_wu::cast_skill),
        13045 => Some(dun_ya::cast_skill),
        90002 => Some(tian_xia_hong_yuan::cast_skill),
        34912 => Some(ye_huo_lin_guang::cast_skill),
        30855 => Some(yue_zhao::cast_skill),
        30856 => Some(yan_men::cast_skill),
        30769 => Some(zhen_yun::cast_skill),
        41982 => Some(yan_shou_gu_jing::cast_skill),
        13391 => Some(dun_dang::cast_skill),
        13070 => Some(dun_bi::cast_skill),
        13042 => Some(wu_ju::cast_skill),
        15072 => Some(han_xiao::cast_skill),
        13046 => Some(dun_meng::cast_skill),
        _ => None,
    }
}

/// 卷雪刀（平砍）attack_coeff：按当前加速实时算
pub fn juan_xue_attack_coeff(haste_level: u32) -> f64 {
    juan_xue::attack_coeff(haste_level)
}

/// 卷雪刀（平砍）产卡：本版本实现
pub fn juan_xue_process_swings(player: &mut Player, to_time: f64) -> Vec<crate::CastEvent> {
    juan_xue::process_swings(player, to_time)
}

/// 绝刀按怒气段的 runtime_recipes
pub fn jue_dao_runtime_recipes(player: &Player) -> Vec<u32> {
    jue_dao::runtime_recipes(player)
}

/// 绝刀按怒气段的 effective_rage_cost
pub fn jue_dao_effective_rage_cost(player: &Player, skill: &crate::SkillSpec) -> u32 {
    jue_dao::effective_rage_cost(player, skill)
}

/// 盾飞手动移除：切擎盾 + 移除延迟（不获得坚定）
pub fn dun_fei_on_remove(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    use crate::{Stance, BUFF_DUN_FEI, BUFF_DUN_FEI_DELAY};
    player.remove_buff(BUFF_DUN_FEI);
    player.remove_buff(BUFF_DUN_FEI_DELAY);
    player.set_stance(Stance::Shield);
    em.emit("盾回", 13051, t);
}
