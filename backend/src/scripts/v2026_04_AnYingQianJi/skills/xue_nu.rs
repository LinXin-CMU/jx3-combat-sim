//! 血怒脚本 (ID: 13040)
//!
//! 2秒内连续施展血怒可叠加层数（通过叠层窗口 buff 判断）
//! 超过2秒再按血怒则刷新为1层

use crate::*;

pub fn cast_skill(player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {
    // 秘籍：额外回怒（6004/6005/6006 各+5）
    let extra_rage: i32 = player.has_recipe(6004) as i32 * 5
        + player.has_recipe(6005) as i32 * 5
        + player.has_recipe(6006) as i32 * 5;
    if extra_rage > 0 {
        player.add_rage_from(extra_rage, "血怒额外回怒");
    }

    // 秘籍：持续时间增加（6001/6002/6003 各+1秒=16帧）
    let extra_frames: u32 = player.has_recipe(6001) as u32 * 16
        + player.has_recipe(6002) as u32 * 16
        + player.has_recipe(6003) as u32 * 16;

    // 超过2秒窗口：先移除旧血怒，重新获得1层
    if !player.has_buff(BUFF_XUE_NU_CD) {
        player.remove_buff(BUFF_XUE_NU);
        player.remove_buff(BUFF_XUE_NU_JY);
    }

    // 血怒 buff（add_buff_extended 会叠层或新增）
    if player.has_talent(36205) {
        player.add_buff_extended(BUFF_XUE_NU_JY, extra_frames);
    } else {
        player.add_buff_extended(BUFF_XUE_NU, extra_frames);
    }

    // 刷新2秒叠层窗口
    player.add_buff(BUFF_XUE_NU_CD);

    // 劫化：时间累加
    player.add_buff_accumulate(BUFF_JIE_HUA);

    // 奇穴 血魄（38969）：清空斩刀 CD + 返 25 怒气
    if player.has_talent(38969) {
        player.reset_cd("cd_斩刀");
        player.add_rage_from(25, "血怒基础回怒");
    }
}
