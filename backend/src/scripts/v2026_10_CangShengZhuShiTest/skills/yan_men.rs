//! 雁门迢递 (ID: 30856) 阵云结晦三段
//! 测试服不再附带破招段。

use crate::*;

pub fn override_attack_coeff(player: &Player) -> Option<f64> {
    super::zhen_yun::uses_high_coefficient(player).then_some(14.1625)
}

pub fn cast_skill(_player: &mut Player, _em: &mut ScriptEmitter, _t: f64) {}
