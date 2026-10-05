use super::*;

#[test]
fn damage_ranges_supply_both_public_metadata_and_calculation_means() {
    let specs = load_skills(&data_path("2026_10_苍生铸世测试服").join("分山劲/skills"));
    let mut p = player(vec![]);
    ensure_recipe_index(&[]);
    p.constants.non_player_bonus = 0.0;
    let target = TargetConfig { level: PLAYER_LEVEL, defense_bonus: 0.0, damage_cof: 0.0 };
    for spec in specs.iter().filter(|s| s.base_damage_range.is_some()) {
        let [min, max] = spec.base_damage_range.unwrap();
        assert!(min >= 0.0 && max >= min, "{}", spec.name);
        assert_eq!(spec.base_damage, (min + max) / 2.0, "{}", spec.name);
        let serialized = serde_json::to_value(spec).unwrap();
        assert_eq!(serialized["base_damage_range"], serde_json::json!([min, max]));
        let mut isolated = spec.clone();
        isolated.attack_coeff = 0.0;
        isolated.weapon_coeff = 0.0;
        isolated.defense_ignore = 1.0;
        isolated.high_berserk_damage = None;
        let (damage, _, _) = calc_event_damage(&isolated, &Attributes::default(), &target, &p, &[], &[], 1);
        assert_eq!(damage.normal_damage, ((min + max) / 2.0).floor(), "{}", spec.name);
    }
    assert_eq!(skill(13044).base_damage_range, Some([13.0, 14.0]));
    assert_eq!(skill(13045).base_damage, 20.75);
    assert_eq!(skill(13153).attack_coeff, 1.968299480435659);
    assert!((skill(8249).attack_coeff - 0.013888179670448326).abs() < 1e-15);
}

#[test]
fn high_berserk_changes_yanmen_base_and_coeff_as_one_damage_spec() {
    let attack = skill(30856);
    let mut p = player(vec![30769]);
    let preview = crate::scripts::effective_damage_spec(&p, &attack);
    assert_eq!(preview.base_damage_range, Some([13.8, 14.7]));
    assert_eq!(preview.base_damage, 14.25);
    assert_eq!(preview.attack_coeff, 1.5749175927497188);
    let first = skill(30769);
    p.set_berserk_value(50);
    p.cast_skill(&first, None, None, 0.0, 0.0).unwrap();
    p.set_berserk_value(120);
    let locked = crate::scripts::effective_damage_spec(&p, &attack);
    assert_eq!(locked.base_damage_range, Some([12.0, 12.9]));
    assert_eq!(locked.attack_coeff, 0.8993571778544289);
    // Preparing another preview must not mutate the loaded TOML specification.
    assert_eq!(attack.base_damage_range, Some([12.0, 12.9]));
}

#[test]
fn auto_attack_coefficients_keep_version_and_haste_boundaries() {
    let mut test = player(vec![]);
    let auto = skill(13039);
    let zero_haste = crate::scripts::override_attack_coeff(&test, &auto).unwrap();
    assert!((zero_haste - 0.01598548306850994).abs() < 1e-14);
    let fast = skills::juan_xue::attack_coeff(1024);
    assert!(fast < zero_haste);
    test.version = GameVersion::AnYingQianJi;
    assert_eq!(crate::scripts::override_attack_coeff(&test, &auto), Some(0.14375000000000002));
}
