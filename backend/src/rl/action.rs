//! RL 动作空间定义。
//!
//! 18 个离散动作：0=等待，1~17 对应可主动释放的技能（按技能名查 skill_map）。
//! 连招后续段（阵云·月照连营/雁门迢递、盾击_2/_3、盾刀_2/_3/_4 等）不占动作位，
//! 由 `resolve_combo_follow` / `pick_rank` 在释放时自动重定向到正确 rank。

pub const ACTION_COUNT: usize = 18;
pub const WAIT_ACTION: usize = 0;

/// 动作索引 → 技能名（0 号位为等待，返回 None）。
/// 技能名必须与 skill_map 的 base name 一致（TOML 里 name 的 "·" 前半部分）。
pub const ACTION_SKILLS: [Option<&str>; ACTION_COUNT] = [
    None,               //  0 等待
    Some("盾击"),       //  1
    Some("盾猛"),       //  2
    Some("盾压"),       //  3
    Some("盾飞"),       //  4
    Some("盾回"),       //  5
    Some("盾刀"),       //  6
    Some("斩刀"),       //  7
    Some("绝刀"),       //  8
    Some("劫刀"),       //  9
    Some("闪刀"),       // 10
    Some("血怒"),       // 11
    Some("阵云结晦"),   // 12
    Some("业火麟光"),   // 13
    Some("撼地"),       // 14
    None,               // 15 盾舞（已禁用；slot 保留以维持 obs/action 维度与旧 ckpt 兼容）
    None,               // 16 移除气劲（已禁用；语义需要选目标 buff，先关掉避免误用）
    Some("天下宏愿"),   // 17  橙武
];

/// 供日志/调试用的人类可读名
pub const ACTION_NAMES: [&str; ACTION_COUNT] = [
    "等待", "盾击", "盾猛", "盾压", "盾飞", "盾回",
    "盾刀", "斩刀", "绝刀", "劫刀", "闪刀", "血怒",
    "阵云结晦", "业火麟光", "撼地", "盾舞", "移除气劲", "天下宏愿",
];

/// 技能名 → 动作索引（用于把 macro_decision 输出的技能名映射回动作号）
pub fn skill_to_action(name: &str) -> Option<usize> {
    ACTION_SKILLS.iter().position(|s| s.map_or(false, |n| n == name))
}
