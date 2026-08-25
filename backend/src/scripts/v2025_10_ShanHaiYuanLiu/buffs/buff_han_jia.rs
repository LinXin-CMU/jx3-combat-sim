//! 寒甲 Buff 事件脚本 (ID: 8437)
//!
//! 奇穴 13134「寒甲」触发的外功攻击强化：
//! - on_tick（每 3s）：按当前面板拆招值重算 8271（+300/层）和 17772（+30000/层）的层数
//! - 同时刷新 寒甲 自身持续时间（假设每 3s 都成功招架一次）
//! - on_expire / on_remove：清除 8271 和 17772（任意方式结束）

use crate::*;

pub fn on_tick(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // Boss 受击系统激活时：寒甲仅在招架成功时由 on_player_hit 刷新，on_tick 不做任何事
    if player.next_boss_attack.is_some() {
        return;
    }
    // 无 Boss 受击时：走原静态 100% 招架假设（每 3s 自刷新 + recalc）
    recalc_stacks(player);
    player.add_buff(BUFF_HAN_JIA);
}

pub fn on_expire(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    clear_stacks(player);
}

pub fn on_remove(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    clear_stacks(player);
}

/// 按当前面板拆招值（含 buff 加成）重算两个数值 buff 的层数
fn recalc_stacks(player: &mut Player) {
    let parry_value = player.current_stats().parry_value;
    let bonus = (parry_value * 0.07).floor() as u32;
    let big = (bonus / 30000).min(125);
    let small = ((bonus % 30000) / 300).min(125);

    // 先清再叠（隐藏 buff，不污染 timeline）
    player.remove_buff(BUFF_HAN_JIA_SMALL);
    player.remove_buff(BUFF_HAN_JIA_LARGE);
    for _ in 0..big {
        player.add_buff(BUFF_HAN_JIA_LARGE);
    }
    for _ in 0..small {
        player.add_buff(BUFF_HAN_JIA_SMALL);
    }
}

fn clear_stacks(player: &mut Player) {
    player.remove_buff(BUFF_HAN_JIA_SMALL);
    player.remove_buff(BUFF_HAN_JIA_LARGE);
}
