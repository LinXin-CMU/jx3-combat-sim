//! 血誓触发逻辑（苍雪刀公共模块）
//!
//! 条件：有血怒 buff + 奇穴32618
//! 每次苍雪刀命中：给目标叠 以血盟誓（最多2层）
//! 以血盟誓满2层后再次命中：触发血誓伤害 + 移除以血盟誓 + 添加血誓 debuff
//! 血誓伤害怒气加成 = 触发时怒气（释放前），按绝刀算法

use crate::*;

/// 血誓怒气段秘籍（血誓复用绝刀的怒气段秘籍 99035-99065）
pub fn rage_recipe(pre_rage: u32, has_recipe_3005: bool) -> Vec<u32> {
    let base = if has_recipe_3005 { 10 } else { 25 };
    let step = 10;
    if pre_rage <= base {
        return Vec::new();
    }
    let tier = ((pre_rage - base) / step).min(4);
    match tier {
        1 => vec![99035],
        2 => vec![99045],
        3 => vec![99055],
        4 => vec![99065],
        _ => Vec::new(),
    }
}

pub fn try_trigger(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 需要奇穴
    if !player.has_talent(32618) {
        return;
    }

    // 需要血怒 buff（普通或惊涌）
    if !player.has_buff(BUFF_XUE_NU) && !player.has_buff(BUFF_XUE_NU_JY) {
        return;
    }

    // 检查以血盟誓层数
    let count = player
        .target_buffs
        .iter()
        .find(|b| b.buff_id == BUFF_XUE_SHI_COUNT)
        .map(|b| b.stacks)
        .unwrap_or(0);

    if count >= 2 {
        // 第三次命中：触发血誓伤害
        // 怒气加成 = 释放前怒气，按绝刀5档算法
        // 有秘籍3005：基准10/20/30/40/50，无秘籍：基准25/35/45/55/65
        let pre_rage = (player.rage + player.last_rage_cost as i32).min(100) as u32;
        let base = if player.has_recipe(3005) {
            10u32
        } else {
            25u32
        };
        let step = 10u32;
        let rage_val = base + ((pre_rage.saturating_sub(base)) / step).min(4) * step;
        let name = format!("血誓·{}怒", rage_val);
        // 怒气段秘籍按 pre_rage 选择（与绝刀同算法）
        let recipes = rage_recipe(pre_rage, player.has_recipe(3005));
        em.emit_with_recipes(&name, 33097, t, recipes);

        // 移除以血盟誓，添加血誓 debuff
        player.remove_target_buff(BUFF_XUE_SHI_COUNT);
        player.add_target_buff(BUFF_XUE_SHI);
    } else {
        // 叠层
        player.add_target_buff(BUFF_XUE_SHI_COUNT);
    }
}
