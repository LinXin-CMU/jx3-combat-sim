//! 寒啸千军脚本 (ID: 15072)
//!
//! 消耗 20 格挡值，鼓舞友方士气：每 3310 点基础体质 +67 无双等级（15s），上限 100 层。
//! 仅铁骨衣心法可用。
//!
//! 实现方式与振奋（dun_dang.rs 13422 奇穴）一致 — 按角色基础体质算 stacks 后
//! `remove_buff + for-add_buff` 重建。原因：buff_id 33210 与团队增益共享 BuffDef
//! 实例，若不显式 remove，团辅先挂上的层数会被 `add_buff` 的 +=1+max 守门卡住，
//! 出现"团辅 100 层 + 自己 cast 维持 100"vs"自己 N 层 + 团辅覆盖 100"的不对称。

use crate::{Player, ScriptEmitter, BUFF_HAN_XIAO};

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 消耗 20 格挡值
    player.add_block_value(-20);

    // 按"基础体质 / 3310"算 stacks（每层 +67 无双等级），上限 100 层
    let stacks = ((player.current_stats().base_vitality / 3310.0).floor() as u32).min(100);
    if stacks > 0 {
        player.remove_buff(BUFF_HAN_XIAO);
        for _ in 0..stacks {
            player.add_buff(BUFF_HAN_XIAO);
        }
    }
}
