//! 苍生铸世测试服（2026.10）独立分山劲规则。
#![allow(non_snake_case)]

pub mod buffs;
pub mod on_hit;
pub mod skills;

#[cfg(test)]
#[path = "../../../tests/cangsheng/rules.rs"]
mod tests;

use crate::{Mount, Player, SkillSpec};

/// 公告尚未给出原生 ID 的新增奇穴，使用版本私有临时 ID。
pub const TALENT_BU_GUI: u32 = 91001;
pub const TALENT_SHEN_WEI: u32 = 91002;
pub const TALENT_WEI_YA: u32 = 91003;
pub const TALENT_DUN_SHENG_FENG: u32 = 91004;

pub fn normalize_talents(talents: Vec<u32>) -> Vec<u32> {
    const ALLOWED: &[u32] = &[
        13090, 41740, 18354, 30769, 37559, 36205, 38969, 13418, 91001, 41982, 13152, 13153, 91002,
        13086, 13111, 13304, 21281, 13073, 37558, 20984, 91003, 15196, 25213, 29066, 37239, 13414,
        25212, 37240, 13317, 21282, 41834, 14838, 22897, 91004, 13395, 34540,
    ];
    let has_zhen_yun = talents.contains(&30769);
    let has_beng_xue = talents.contains(&41740);
    let mut normalized = Vec::new();
    for id in talents {
        if !ALLOWED.contains(&id) || normalized.contains(&id) {
            continue;
        }
        if id == TALENT_BU_GUI && !has_zhen_yun {
            continue;
        }
        if matches!(id, 37559 | 37558) && !has_beng_xue {
            continue;
        }
        normalized.push(id);
    }
    normalized
}

/// 主伤害结算发生在 cast_skill 脚本前，威压在这里读取施展前层数。
pub fn runtime_recipes(skill: &SkillSpec, player: &Player) -> Vec<u32> {
    if skill.skill_id == 13046 && player.has_talent(TALENT_WEI_YA) {
        let stacks = player.buff_stacks(buffs::defs::BUFF_WEI_YA).min(6);
        if stacks > 0 {
            return vec![99400 + stacks];
        }
    }
    if player.has_talent(TALENT_SHEN_WEI) {
        return match skill.skill_id { 13044 => vec![99441], 13045 => vec![99442], _ => Vec::new() };
    }
    Vec::new()
}

/// 新版仅提供分山劲；旧版撤出的招式不可施展，阵云核心自带后续段。
pub fn skill_allowed(player: &Player, skill: &SkillSpec) -> bool {
    if player.mount != Mount::FenShanJin {
        return false;
    }
    match skill.skill_id {
        30855 | 30856 => player.has_talent(30769),
        34912 | 34674 | 34674901 | 34714 | 36065 | 36482 | 33097 | 90010..=90012 => false,
        _ => true,
    }
}

/// 云城盾伤害招式按每次施展贡献一次重置概率；引导/盾飞子事件不重复贡献。
pub fn on_post_cast(player: &mut Player, skill: &SkillSpec) {
    if !player.has_talent(TALENT_DUN_SHENG_FENG)
        && matches!(skill.skill_id, 13044..=13048 | 13050 | 13458 | 25204)
    {
        player.try_reset_test_dunya();
    }
}
