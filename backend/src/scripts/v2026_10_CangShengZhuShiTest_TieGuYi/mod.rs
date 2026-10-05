//! 苍生铸世测试服（2026.10）铁骨衣版本脚本集
#![allow(non_snake_case)]

pub mod buffs;
pub mod on_hit;
pub mod skills;

use crate::{Player, SkillSpec};

// 保留原雷云的选择 ID，兼容已有铁骨衣存档。
pub const TALENT_JING_TING: u32 = 34540;
pub const RECIPE_NU_YAN: u32 = 99451;

pub fn normalize_talents(talents: Vec<u32>) -> Vec<u32> {
    const ALLOWED: &[u32] = &[
        25203, 14849, 15072, 25213, 26729, 26897, 25216, 13071,
        14840, 13171, 13420, 13134, 13421, 13422, 13418, 13113,
        44566, 13364, 39045, 13138, 13132, 13133, 34912, 13124,
        34540, 13363, 13356, 13366, 13367, 13321, 13073, 13098,
        13320, 37240, 34602, 13368,
    ];
    let mut result = Vec::new();
    for id in talents {
        if ALLOWED.contains(&id) && !result.contains(&id) { result.push(id); }
    }
    result
}

pub fn runtime_recipes(skill: &SkillSpec, player: &Player) -> Vec<u32> {
    if skill.skill_id == 13055 && player.has_talent(13133) {
        vec![RECIPE_NU_YAN]
    } else {
        Vec::new()
    }
}

/// 按完整的95点体质单位覆盖层数；刷新时不会累加上一次施展的属性。
pub fn apply_vitality_buff(player: &mut Player, buff_id: u32, vitality: f64, seconds: u32) {
    let stacks = (vitality.max(0.0) / 95.0).floor() as u32;
    if stacks == 0 {
        player.remove_buff(buff_id);
    } else {
        player.add_buff_with_stacks(buff_id, stacks, seconds * 16);
    }
}

pub fn apply_ji_ang(player: &mut Player) {
    if player.has_talent(13356) {
        apply_vitality_buff(player, crate::BUFF_JI_ANG, player.current_stats().vitality, 15);
    }
}

#[cfg(test)]
#[path = "../../../tests/cangsheng/tie_gu_yi.rs"]
mod tests;
