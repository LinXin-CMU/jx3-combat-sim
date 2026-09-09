//! 苍生铸世测试服阵云一段。暴怒由 Player 主施放链扣除。
//! 神威后续段保留 90 秒；三段攻击系数按首段实际消耗选择。

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    if player.has_talent(91002) {
        player.add_state_buff(combo_buff_id("阵云_2"), 1440);
    }
}

pub fn uses_high_coefficient(player: &Player) -> bool {
    if player.zhen_yun_berserk_cost > 0 {
        player.zhen_yun_berserk_cost >= 100
    } else {
        // 静态伤害预览没有实际施放，按当前准备资源选择。
        player.berserk_value >= 100
    }
}

pub fn override_attack_coeff(player: &Player) -> Option<f64> {
    uses_high_coefficient(player).then_some(9.33125)
}

pub fn runtime_recipes(skill: &SkillSpec, player: &Player) -> Vec<u32> {
    if skill.skill_id != 30769 || !player.uses_berserk() {
        return Vec::new();
    }
    // 静态伤害面板没有执行扣资源步骤，按准备完成时的资源计算。
    let cost = if player.zhen_yun_berserk_cost > 0 {
        player.zhen_yun_berserk_cost
    } else {
        player.effective_berserk_cost(skill)
    };
    vec![if cost >= 100 { 99422 } else { 99421 }]
}
