//! 绝刀脚本 (ID: 13055)
//!
//! - 怒气段：按当前怒气选最大段（25/35/45/55/65 或秘籍3005下 10/20/30/40/50）
//! - 怒气段加成：通过隐藏秘籍 99035/45/55/65（damage_pct + surplus_pct）
//! - 狂绝 buff：不消耗怒气（返还已扣除的怒气），消耗狂绝 buff
//! - 橙武 buff：擎刀绝刀消耗归零
//! - 嗜血奇穴(21281)：施展绝刀获得嗜血 buff（伤害+5%，12秒）
//! - 援戈

use super::{lin_guang, xue_shi, yuan_ge};
use crate::*;

/// 怒气段表（基础 / 秘籍3005 减费后）
const RAGE_SEGMENTS: [u32; 5] = [25, 35, 45, 55, 65];
const RAGE_SEGMENTS_3005: [u32; 5] = [10, 20, 30, 40, 50];

fn segments(player: &Player) -> &'static [u32; 5] {
    if player.has_recipe(3005) {
        &RAGE_SEGMENTS_3005
    } else {
        &RAGE_SEGMENTS
    }
}

/// 当前怒气段（按当前 rage 选最大可用段）
/// 橙武时仍按段返回（cast_skill 脚本会事后返还，等效免耗）；
/// 橙武 + 怒气不足最低段：返回 0 让 can_cast 通过
pub fn effective_rage_cost(player: &Player, _skill: &SkillSpec) -> u32 {
    let segs = segments(player);
    let max_seg = segs
        .iter()
        .rev()
        .copied()
        .find(|&seg| player.rage >= seg as i32);
    match max_seg {
        Some(seg) => seg,
        None if player.has_buff(BUFF_CHENG_WU) => 0,
        None => segs[0], // 怒气不够最低段，让 can_cast 拦截
    }
}

/// 按本次实际消耗的怒气选择附加的隐藏秘籍 ID
/// 25/10怒（最低段）不加成；35/20怒 +20%；以此类推
pub fn runtime_recipes(player: &Player) -> Vec<u32> {
    let cost = player.last_rage_cost;
    let segs = segments(player);
    let tier = match cost {
        c if c == segs[0] => 0,
        c if c == segs[1] => 1,
        c if c == segs[2] => 2,
        c if c == segs[3] => 3,
        c if c == segs[4] => 4,
        _ => 0,
    };
    match tier {
        1 => vec![99035],
        2 => vec![99045],
        3 => vec![99055],
        4 => vec![99065],
        _ => Vec::new(),
    }
}

pub fn cast_skill(player: &mut Player, em: &mut ScriptEmitter, t: f64) {
    // 破·绝刀（独立显示的破招段）
    // 分山劲 13055901 受怒气段秘籍加成；铁骨衣 13055902 不吃怒气段加成
    let pojue_id = match player.mount {
        Mount::TieGuYi => 13055902,
        _ => 13055901,
    };
    em.emit("破·绝刀", pojue_id, t);

    // 苍雪刀公共效果
    lin_guang::try_trigger(player, em, t);
    xue_shi::try_trigger(player, em, t);

    // 狂绝：返还已消耗的怒气，消耗狂绝 buff
    if player.has_buff(BUFF_KUANG_JUE) {
        player.add_rage_from(player.last_rage_cost as i32, "狂绝返还绝刀怒气");
        player.remove_buff(BUFF_KUANG_JUE);
    }

    // 橙武：擎刀绝刀消耗归零（事后返还，让 last_rage_cost 仍按段算秘籍/name）
    if player.has_buff(BUFF_CHENG_WU) {
        player.add_rage_from(player.last_rage_cost as i32, "橙武返还绝刀怒气");
    }

    // 嗜血奇穴：施展绝刀获得嗜血 buff
    if player.has_talent(21281) {
        player.add_buff(BUFF_SHI_XUE);
    }

    // 援戈
    if player.has_talent(36058) {
        yuan_ge::cast_skill(player, em, t);
    }
}
