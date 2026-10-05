//! 等级对应的属性参数。版本只负责选择等级，现有计算链继续共用。
//! 50级参数： https://www.jx3box.com/bps/109685
//! 基础属性与主属性转化： https://www.jx3box.com/bps/109665
use crate::GameVersion;
use serde::Serialize;

pub fn player_level(version: GameVersion) -> u32 {
    match version {
        GameVersion::CangShengZhuShiTest => 50,
        _ => 130,
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct LevelParams {
    pub level: u32,
    pub crit: f64,
    pub crit_effect: f64,
    pub overcome: f64,
    pub strain: f64,
    pub haste: f64,
    pub parry: f64,
    pub defense: f64,
    pub dodge: f64,
    pub toughness: f64,
    pub toughness_crit_effect: f64,
    pub decritical: f64,
    pub agility_to_crit: f64,
    pub strength_to_attack: f64,
    pub strength_to_overcome: f64,
    pub pvx_to_surplus: f64,
    pub pvx_to_strain: f64,
    pub base_vitality: f64,
    pub base_strength: f64,
    pub base_agility: f64,
    pub base_spirit: f64,
    pub base_spunk: f64,
    pub base_shield: f64,
    pub base_life: f64,
    pub has_surplus: bool,
}

impl LevelParams {
    pub fn for_version(version: GameVersion) -> Self { Self::for_level(player_level(version)) }

    pub fn for_level(level: u32) -> Self {
        // 0 是旧结构体默认值，按原130级解释，兼容既有调用与存档。
        if level == 0 || level >= 100 {
            return Self {
                level: if level == 0 { 130 } else { level },
                crit: 197703.0, crit_effect: 72844.2, overcome: 225957.6,
                strain: 133333.2, haste: 210078.0, parry: 107553.6,
                defense: legacy_defense(level), dodge: 91634.4,
                toughness: 197703.0, toughness_crit_effect: 55123.2,
                decritical: 33046.2,
                agility_to_crit: 0.9, strength_to_attack: 0.163, strength_to_overcome: 0.3,
                pvx_to_surplus: 0.5, pvx_to_strain: 1.5,
                base_vitality: 45.0, base_strength: 44.0, base_agility: 44.0,
                base_spirit: 44.0, base_spunk: 44.0, base_shield: 2850.0,
                base_life: 199476.0, has_surplus: true,
            };
        }
        let c = if level <= 30 { 330.0 } else { 33.0 * level as f64 - 660.0 };
        Self {
            level, crit: 9.609*c, crit_effect: 3.540*c, overcome: 10.483*c,
            strain: 7.117*c, haste: 10.210*c, parry: 15.833*c,
            defense: 10.912*c, dodge: 13.491*c,
            toughness: 19.218*c, toughness_crit_effect: 9.518*c, decritical: 5.2*c,
            agility_to_crit: 0.25, strength_to_attack: 0.195, strength_to_overcome: 0.061,
            pvx_to_surplus: 0.0, pvx_to_strain: 1.22,
            base_vitality: 18.0, base_strength: 17.0, base_agility: 18.0,
            base_spirit: 18.0, base_spunk: 17.0, base_shield: 280.0,
            // 无装备4136气血，扣除基础18体质提供的180点。
            base_life: 3956.0, has_surplus: false,
        }
    }
}

fn legacy_defense(level: u32) -> f64 {
    match level {
        131 => 133357.62, 132 => 140708.04, 133 => 148058.46, 134 => 155408.88,
        _ => 126007.2,
    }
}

#[cfg(test)]
#[path = "../tests/level_params/mod.rs"]
mod tests;
