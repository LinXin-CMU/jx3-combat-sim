//! 月照连营 (ID: 30855) 阵云结晦二段
//! 测试服不再附带破招段。

use crate::*;

pub fn override_attack_coeff(player: &Player) -> Option<f64> {
    super::zhen_yun::uses_high_coefficient(player).then_some(11.725)
}

pub fn cast_skill(_player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {}
