use crate::*;

#[path = "storage.rs"]
mod storage;

#[test]
fn level50_parameters_and_formal_parameters_are_separate() {
    let new = LevelParams::for_version(GameVersion::CangShengZhuShiTest);
    assert_eq!(new.level, 50);
    for (got, expected) in [(new.crit, 9512.91), (new.crit_effect, 3504.6), (new.overcome, 10378.17), (new.strain, 7045.83), (new.haste, 10107.9), (new.parry, 15674.67), (new.defense, 10802.88)] {
        assert!((got - expected).abs() < 1e-8);
    }
    for version in [GameVersion::ShanHaiYuanLiu, GameVersion::AnYingQianJi, GameVersion::AnYingQianJiTest] {
        let old = LevelParams::for_version(version);
        assert_eq!(old.level, 130);
        assert_eq!(old.crit, 197703.0);
        assert_eq!(old.haste, 210078.0);
        assert!(old.has_surplus);
    }
}

#[test]
fn level50_dummy_defense_and_level_suppression_share_damage_chain() {
    let constants = load_school_toml(GameVersion::CangShengZhuShiTest, Mount::FenShanJin).unwrap().0;
    let rt = build_runtime_stats(&Attributes::default(), &AttribSlots::new(), &constants);
    for (level, defense, rate) in [(51, 2791.0, 0.20), (52, 3841.0, 0.25), (53, 6399.0, 0.35), (54, 6592.0, 0.35)] {
        assert_eq!(target_base_defense(level), defense);
        assert!((defense / (defense + defense_level_param(level)) - rate).abs() < 0.00005);
    }
    let mut spec = load_skills(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/2026_10_苍生铸世测试服/分山劲/skills")).into_iter().find(|s|s.skill_id==13044).unwrap();
    spec.base_damage = 10000.0; spec.attack_coeff = 0.0; spec.weapon_coeff = 0.0; spec.defense_ignore = 1.0;
    let target = TargetConfig {level:54, defense_bonus:0.0, damage_cof:0.0};
    let result = calc_damage(&spec, &Attributes::default(), &target, &rt, &[], &AttribSlots::new(), &AttribSlots::new(), 0.0);
    assert_eq!(result.normal_damage, 8000.0);
}

#[test]
fn haste_boundaries_round_trip_for_both_levels() {
    for level in [50, 130] {
        let cap = (LevelParams::for_level(level).haste * 0.25) as u32;
        for (_, frames, min, max) in haste_tier_boundaries_at_level(24, cap, level) {
            assert_eq!(get_actual_frames_at_level(24, min, level), frames);
            assert_eq!(get_actual_frames_at_level(24, max, level), frames);
            if min > 0 { assert_ne!(get_actual_frames_at_level(24, min - 1, level), frames); }
        }
    }
    assert_eq!(get_actual_frames_at_level(24, 9, 50), 24);
    assert_eq!(get_actual_frames_at_level(24, 10, 50), 23);
    assert_eq!(get_actual_frames(24, 10), 24);
}

fn empty_equipment() -> equip::EquipData {
    equip::EquipData { attrib_table: Default::default(), items: Default::default(), items_by_subtype:Default::default(), enhances:Default::default(), enchants:Default::default(), stones:vec![], sets:Default::default() }
}

#[test]
fn equipment_fast_path_and_runtime_use_the_same_level50_conversions() {
    let data = empty_equipment();
    for mount in [Mount::FenShanJin, Mount::TieGuYi] {
        let (constants, bs, mc, _, _) = load_school_toml(GameVersion::CangShengZhuShiTest, mount).unwrap();
        let req: equip::CalcRequest = serde_json::from_value(serde_json::json!({"slots": {}})).unwrap();
        let result = equip::calculate(&data, &req, &bs, &mc);
        let ctx = equip::search_calc::prepare_init(&data, &req.slots, 0, &[], &bs, &mc);
        let fast = equip::search_calc::calc_leaf_raw(&ctx, &ctx.initial_accum, &ctx.initial_set_counts, 0, 0);
        assert_eq!(serde_json::to_value(&fast).unwrap(), serde_json::to_value(&result.raw).unwrap());
        let attr = Attributes { shen_fa: result.raw.agility, li_dao:result.raw.strength, vitality: result.raw.vitality, base_attack:result.raw.base_attack, crit_level:result.raw.crit_level, parry_level:result.raw.parry_level, parry_value:result.raw.parry_value, ..Default::default() };
        let rt = build_runtime_stats(&attr, &AttribSlots::new(), &constants);
        assert_eq!(rt.panel_attack, result.panel.physics_attack_power);
        assert_eq!(rt.crit_rate, result.panel.crit_rate);
        assert_eq!(rt.parry_rate, result.panel.parry_rate);
        assert_eq!(rt.surplus_value, 0.0);
        if mount == Mount::FenShanJin { assert_eq!(result.panel.max_life, 4136.0); }
        let mut slots = AttribSlots::new(); slots.insert(AttribField::AgilityBase, 100.0); slots.insert(AttribField::PhysicsAttackPowerPercent, 1024.0);
        let boosted = build_runtime_stats(&attr, &slots, &constants);
        assert_eq!(boosted.panel_attack, attr.base_attack * 2.0 + ((attr.shen_fa+100.0)*constants.shenfa_to_attack).floor() + (attr.vitality*constants.vitality_to_attack).floor());
    }
}
