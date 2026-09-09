use super::buffs::defs::{BUFF_JIAO_DOU_CHANG, BUFF_JING_YONG_PASSIVE, BUFF_WEI_YA};
use super::*;
use crate::*;

fn player(talents: Vec<u32>) -> Player {
    Player::with_mount(
        Mount::FenShanJin,
        GameVersion::CangShengZhuShiTest,
        MountConstants::fenshanjin_default(),
        0,
        talents,
        vec![],
    )
}

fn data_path(version: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(version)
}

fn skill(id: u32) -> SkillSpec {
    load_skills(&data_path("2026_10_苍生铸世测试服").join("分山劲/skills"))
        .into_iter()
        .find(|s| s.skill_id == id)
        .unwrap()
}

#[test]
fn official_talent_grid_and_shield_data_are_version_isolated() {
    let new_dir = data_path("2026_10_苍生铸世测试服");
    let talents = load_talents(&new_dir.join("分山劲/talents.toml"));
    assert_eq!(talents.len(), 36);
    for tier in 1..=6 {
        assert_eq!(talents.iter().filter(|t| t.tier == tier).count(), 4);
    }
    assert_eq!(talents.iter().filter(|t| t.tier == 8).count(), 12);
    let new_skills = load_skills(&new_dir.join("分山劲/skills"));
    let old_skills = load_skills(&data_path("2026_04_暗影千机").join("分山劲/skills"));
    for old in old_skills
        .iter()
        .filter(|s| matches!(s.skill_id, 13044 | 13045))
    {
        let new = new_skills
            .iter()
            .find(|s| s.skill_id == old.skill_id && s.name == old.name)
            .unwrap();
        assert_eq!(new.rage_gain, if old.skill_id == 13044 { 10 } else { 20 });
        assert_eq!(new.base_damage, old.base_damage * 2.0);
        assert_eq!(new.attack_coeff, if old.skill_id == 13045 { 7.19375 } else { old.attack_coeff * 2.0 });
        assert_eq!(new.weapon_coeff, old.weapon_coeff * 2.0);
    }
    assert!(new_skills
        .iter()
        .all(|s| !matches!(s.skill_id, 34912 | 34674 | 34714 | 36065 | 36482 | 33097)));
    assert!(old_skills.iter().any(|s| s.skill_id == 34912));
}

#[test]
fn removed_talents_and_missing_core_requirements_cannot_activate() {
    assert!(
        normalize_talents(vec![34912, 36058, 32618, 39664, 91001, 91002, 37559, 37558]).is_empty()
    );
    assert_eq!(
        normalize_talents(vec![30769, 91001, 91002, 91001]),
        vec![30769, 91001, 91002]
    );
    assert_eq!(
        normalize_talents(vec![41740, 37559, 37558]),
        vec![41740, 37559, 37558]
    );
}

#[test]
fn weiya_caps_refreshes_consumes_six_and_only_scales_shield_bash() {
    let mut p = player(vec![TALENT_WEI_YA]);
    let mut em = ScriptEmitter::new();
    for _ in 0..4 {
        skills::dun_ya::cast_skill(&mut p, &mut em, 0.0);
    }
    skills::dun_dao::cast_skill(&mut p, &mut em, 0.0);
    assert_eq!(p.buff_stacks(BUFF_WEI_YA), 8);
    p.current_time = 10.0;
    skills::dun_dao::cast_skill(&mut p, &mut em, 10.0);
    assert_eq!(
        p.active_buffs
            .iter()
            .find(|b| b.buff_id == BUFF_WEI_YA)
            .unwrap()
            .expires_at,
        40.0
    );
    let mut bash = skill(13046);
    assert_eq!(runtime_recipes(&bash, &p), vec![99406]);
    assert!(runtime_recipes(&skill(13044), &p).is_empty());

    // Use a clean damage fixture to check the real shared nine-step calculator.
    bash.base_damage = 10000.0;
    bash.attack_coeff = 0.0;
    bash.weapon_coeff = 0.0;
    bash.defense_ignore = 1.0;
    p.constants.non_player_bonus = 0.0;
    let table = load_recipes(&data_path("2026_10_苍生铸世测试服").join("recipes.toml"));
    ensure_recipe_index(&table);
    let target = TargetConfig {
        level: PLAYER_LEVEL,
        defense_bonus: 0.0,
        damage_cof: 0.0,
    };
    let (damage, _, _) = calc_event_damage(
        &bash,
        &Attributes::default(),
        &target,
        &p,
        &runtime_recipes(&bash, &p),
        &table,
        1,
    );
    assert_eq!(damage.normal_damage, 28000.0);

    p.add_protect_cd("cd_盾猛", 25.0);
    skills::dun_meng::cast_skill(&mut p, &mut em, 10.0);
    assert_eq!(p.buff_stacks(BUFF_WEI_YA), 2);
    assert_eq!(p.active_cds.get("cd_盾猛"), Some(&19.0));
    assert_eq!(runtime_recipes(&bash, &p), vec![99402]);
    p.current_time = 40.0;
    assert_eq!(p.buff_stacks(BUFF_WEI_YA), 0);
    assert!(runtime_recipes(&bash, &p).is_empty());

    let mut unselected = player(vec![]);
    skills::dun_dao::cast_skill(&mut unselected, &mut em, 0.0);
    skills::dun_ya::cast_skill(&mut unselected, &mut em, 0.0);
    assert_eq!(unselected.buff_stacks(BUFF_WEI_YA), 0);
}

#[test]
fn jingyong_has_permanent_strain_and_conditional_skill_filtered_pve() {
    let mut p = player(vec![36205]);
    super::buffs::on_battle_start(&mut p);
    assert!(p.has_buff(BUFF_JING_YONG_PASSIVE));
    assert_eq!(
        aggregate_buff_fields(&p).get(&AttribField::StrainBasePercentAdd),
        Some(&204.8)
    );
    let mut em = ScriptEmitter::new();
    skills::xue_nu::cast_skill(&mut p, &mut em, 0.0);
    skills::xue_nu::cast_skill(&mut p, &mut em, 0.0);
    assert_eq!(p.buff_stacks(BUFF_XUE_NU_JY), 1);
    assert_eq!(
        aggregate_buff_fields(&p).get(&AttribField::StrainBasePercentAdd),
        Some(&204.8)
    );
    let table = load_recipes(&data_path("2026_10_苍生铸世测试服").join("recipes.toml"));
    ensure_recipe_index(&table);
    for (id, name, expected) in [
        (13054, "斩刀", 1.0),
        (13055, "绝刀", 0.71),
        (13054901, "破·斩刀", 0.0),
        (13055901, "破·绝刀", 0.0),
        (13048901, "破·盾舞", 0.0),
        (13052, "劫刀", 0.0),
    ] {
        let recipes = collect_recipes(&p, id, name, &[], &table);
        assert_eq!(
            recipes.iter().map(|r| r.pve_addition).sum::<f64>(),
            expected
        );
        assert_eq!(recipes.iter().map(|r| r.surplus_pct).sum::<f64>(), 0.0);
    }
    p.remove_buff(BUFF_XUE_NU_JY);
    assert!(!p.recipe_active(99411));
    assert!(!p.recipe_active(99412));
    assert!(p.has_buff(BUFF_JING_YONG_PASSIVE));
    let mut unselected = player(vec![]);
    super::buffs::on_battle_start(&mut unselected);
    assert!(!unselected.has_buff(BUFF_JING_YONG_PASSIVE));
    let old = Player::new(0, vec![36205], vec![]);
    assert!(old.buff_def(BUFF_JING_YONG_PASSIVE).is_none());
}

#[test]
fn fengming_no_longer_grants_shield_flight_damage_recipe() {
    let mut p = player(vec![22897]);
    skills::dun_fei::cast_skill(&mut p, &mut ScriptEmitter::new(), 0.0);
    assert!(p.has_buff(BUFF_FENG_MING));
    assert!(!p.recipe_active(99201));
    assert_eq!(
        aggregate_buff_fields(&p).get(&AttribField::PhysicsAttackPowerPercent),
        Some(&154.0)
    );
    let mut old = Player::new(0, vec![22897], vec![]);
    crate::scripts::v2026_04_AnYingQianJi::skills::dun_fei::cast_skill(
        &mut old,
        &mut ScriptEmitter::new(),
        0.0,
    );
    assert!(old.recipe_active(99201));
}

#[test]
fn known_arena_effect_has_limited_duration_and_bleed_only_recipe() {
    let mut p = player(vec![29066]);
    skills::shi_jin_bing_qiong::cast_skill(&mut p, &mut ScriptEmitter::new(), 0.0);
    assert!(p.has_buff(BUFF_JIAO_DOU_CHANG));
    let table = load_recipes(&data_path("2026_10_苍生铸世测试服").join("recipes.toml"));
    ensure_recipe_index(&table);
    let bleed = collect_recipes(&p, 8249, "流血", &[], &table);
    assert_eq!(bleed.iter().map(|r| r.damage_pct).sum::<f64>(), 2.0);
    let slash = collect_recipes(&p, 13054, "斩刀", &[], &table);
    assert_eq!(slash.iter().map(|r| r.damage_pct).sum::<f64>(), 0.0);
    p.process_buff_ticks(0.0, 15.0);
    p.current_time = 15.0;
    assert!(!p.has_buff(BUFF_JIAO_DOU_CHANG));
    assert!(!p.recipe_active(99413));
}

#[test]
fn resource_talents_apply_only_when_selected_and_shield_guard_disables_reset_mark() {
    let mut em = ScriptEmitter::new();
    let mut p = player(vec![30769, TALENT_BU_GUI, 13414, TALENT_DUN_SHENG_FENG]);
    p.set_berserk_value(0);
    skills::xue_nu::cast_skill(&mut p, &mut em, 0.0);
    assert_eq!(p.berserk_value, 50);
    p.set_rage(0);
    skills::dun_meng::cast_skill(&mut p, &mut em, 0.0);
    assert_eq!(p.rage, 20);
    assert!(!p.last_cast_shield_non_dunya);
    let mut unselected = player(vec![30769]);
    unselected.set_berserk_value(0);
    skills::xue_nu::cast_skill(&mut unselected, &mut em, 0.0);
    skills::dun_meng::cast_skill(&mut unselected, &mut em, 0.0);
    assert_eq!(unselected.berserk_value, 0);
    assert_eq!(unselected.rage, 0);
    assert!(unselected.last_cast_shield_non_dunya);
}

#[test]
fn bugui_and_canlie_change_actual_cooldowns_and_clamp_resource_gains() {
    let blood = skill(13040);
    let bash = skill(13046);
    let mut p = player(vec![30769, TALENT_BU_GUI, 13414, TALENT_WEI_YA]);
    assert_eq!(p.effective_charge_cd(&blood), 22.0);
    assert_eq!(player(vec![]).effective_charge_cd(&blood), 25.0);
    p.set_berserk_value(100);
    assert!(p.cast_skill(&blood, None, None, 0.0, 0.0).is_some());
    let mut em = ScriptEmitter::new();
    let time = p.current_time;
    skills::xue_nu::cast_skill(&mut p, &mut em, time);
    assert_eq!(p.berserk_value, 120);
    assert_eq!(p.charges.get(&13040).unwrap().1, 22.0);

    let mut base = player(vec![]);
    assert!(base.cast_skill(&bash, None, None, 0.0, 0.0).is_some());
    skills::dun_meng::cast_skill(&mut base, &mut em, 0.0);
    assert_eq!(base.active_cds.get("cd_盾猛"), Some(&15.0));
    assert_eq!(base.rage, 15);

    let mut canlie = player(vec![13414]);
    assert!(canlie.cast_skill(&bash, None, None, 0.0, 0.0).is_some());
    skills::dun_meng::cast_skill(&mut canlie, &mut em, 0.0);
    assert_eq!(canlie.active_cds.get("cd_盾猛"), Some(&12.0));
    assert_eq!(canlie.rage, 35);

    let mut both = player(vec![13414, TALENT_WEI_YA]);
    for _ in 0..8 {
        both.add_buff(BUFF_WEI_YA);
    }
    assert!(both.cast_skill(&bash, None, None, 0.0, 0.0).is_some());
    assert_eq!(compute_runtime_recipes(&bash, &both), vec![99406]);
    skills::dun_meng::cast_skill(&mut both, &mut em, 0.0);
    assert_eq!(both.active_cds.get("cd_盾猛"), Some(&6.0));
    assert_eq!(both.buff_stacks(BUFF_WEI_YA), 2);
    assert_eq!(both.rage, 35);
}

#[test]
fn zhenyun_tiers_and_static_preview_use_exact_shared_damage_chain() {
    let mut attack = skill(30769);
    attack.base_damage = 10000.0;
    attack.attack_coeff = 0.0;
    attack.weapon_coeff = 0.0;
    attack.defense_ignore = 1.0;
    let table = load_recipes(&data_path("2026_10_苍生铸世测试服").join("recipes.toml"));
    ensure_recipe_index(&table);
    let target = TargetConfig {
        level: PLAYER_LEVEL,
        defense_bonus: 0.0,
        damage_cof: 0.0,
    };
    for (value, recipe, expected) in [
        (50, 99421, 17600.0),
        (99, 99421, 17600.0),
        (100, 99422, 29500.0),
        (120, 99422, 29500.0),
    ] {
        let mut p = player(vec![30769]);
        p.constants.non_player_bonus = 0.0;
        p.set_berserk_value(value);
        assert!(p.cast_skill(&attack, None, None, 0.0, 0.0).is_some());
        let runtime = compute_runtime_recipes(&attack, &p);
        assert_eq!(runtime, vec![recipe]);
        let applied = collect_recipes(&p, 30769, "阵云结晦", &runtime, &table);
        assert!(!applied.iter().any(|r| r.id == 99342));
        let (damage, _, _) = calc_event_damage(
            &attack,
            &Attributes::default(),
            &target,
            &p,
            &runtime,
            &table,
            1,
        );
        assert_eq!(damage.normal_damage, expected, "resource={value}");
    }
    // /api/skill_damage 没有实际施放：读取脱战准备120暴怒，高档也只能计一次。
    let mut preview = player(vec![30769]);
    preview.constants.non_player_bonus = 0.0;
    assert_eq!(preview.berserk_value, 120);
    assert_eq!(preview.zhen_yun_berserk_cost, 0);
    let runtime = compute_runtime_recipes(&attack, &preview);
    assert_eq!(runtime, vec![99422]);
    let (damage, _, _) = calc_event_damage(
        &attack,
        &Attributes::default(),
        &target,
        &preview,
        &runtime,
        &table,
        1,
    );
    assert_eq!(damage.normal_damage, 29500.0);
}

#[test]
fn zhenyun_new_coefficients_follow_initial_spend_without_double_scaling() {
    let table = load_recipes(&data_path("2026_10_苍生铸世测试服").join("recipes.toml"));
    ensure_recipe_index(&table);
    let attr = Attributes { base_attack: 10000.0, ..Attributes::default() };
    let target = TargetConfig { level: PLAYER_LEVEL, defense_bonus: 0.0, damage_cof: 0.0 };
    for initial in [50, 99, 100, 120] {
        let high = initial >= 100;
        let mut p = player(vec![30769, TALENT_SHEN_WEI]);
        p.set_berserk_value(initial);
        for (id, low, high_coefficient) in [(30769, 5.33125, 9.33125),
            (30855, 6.7125, 11.725), (30856, 8.0875, 14.1625)] {
            let attack = skill(id);
            assert_eq!(attack.attack_coeff, low);
            let cast = p.cast_skill(&attack, None, None, 0.0, 0.0).unwrap();
            crate::scripts::run_scripts(&mut p, &attack, cast.0);
            let expected = if high { high_coefficient } else { low };
            assert_eq!(crate::scripts::override_attack_coeff(&p, &attack).unwrap_or(attack.attack_coeff), expected);
            let runtime = compute_runtime_recipes(&attack, &p);
            let applied = collect_recipes(&p, id, &attack.name, &runtime, &table);
            assert_eq!(applied.iter().map(|r| r.damage_pct).sum::<f64>(), 0.0);
            assert_eq!(applied.iter().map(|r| r.pve_addition).sum::<f64>(),
                if id == 30769 { if high { 1.95 } else { 0.76 } } else { 0.0 });
            let (damage, _, stats) = calc_event_damage(&attack, &attr, &target, &p, &runtime, &table, 1);
            assert_eq!(damage.coefficient_damage, (expected * stats.panel_attack).floor());
            // 后续资源反转不能改变这轮连段已经选定的档位。
            p.set_berserk_value(if high { 0 } else { 120 });
        }
    }
    let preview = player(vec![30769, TALENT_SHEN_WEI]);
    for (id, expected) in [(30769, 9.33125), (30855, 11.725), (30856, 14.1625)] {
        assert_eq!(crate::scripts::override_attack_coeff(&preview, &skill(id)), Some(expected));
    }
}

#[test]
fn zhenyun_followup_damage_drops_old_pve_bonus_only_in_test_version() {
    let target = TargetConfig {
        level: PLAYER_LEVEL,
        defense_bonus: 0.0,
        damage_cof: 0.0,
    };
    for (version, expected_damage, expected_old_recipe_count) in [
        (GameVersion::CangShengZhuShiTest, 10000.0, 0),
        (GameVersion::AnYingQianJi, 30000.0, 1),
        (GameVersion::ShanHaiYuanLiu, 30000.0, 1),
    ] {
        let version_path = data_path(version_dir_name(version));
        let skills = load_skills(&version_path.join("分山劲/skills"));
        let table = load_recipes(&version_path.join("recipes.toml"));
        ensure_recipe_index(&table);
        let (mut constants, _, _, _, _) = load_school_toml(version, Mount::FenShanJin).unwrap();
        constants.non_player_bonus = 0.0;
        let mut p = Player::with_mount(
            Mount::FenShanJin,
            version,
            constants,
            0,
            vec![30769, 91002],
            vec![],
        );
        for skill_id in [30769, 30855, 30856] {
            let mut attack = skills
                .iter()
                .find(|s| s.skill_id == skill_id)
                .unwrap()
                .clone();
            attack.base_damage = 10000.0;
            attack.attack_coeff = 0.0;
            attack.weapon_coeff = 0.0;
            attack.defense_ignore = 1.0;
            let cast = p.cast_skill(&attack, None, None, 0.0, 0.0).unwrap();
            crate::scripts::run_scripts(&mut p, &attack, cast.0);
            if skill_id == 30769 {
                continue; // 一段的50/100暴怒档位由相邻专项测试验证。
            }
            let runtime = compute_runtime_recipes(&attack, &p);
            let applied = collect_recipes(&p, skill_id, &attack.name, &runtime, &table);
            assert_eq!(
                applied.iter().filter(|r| r.id == 99342).count(),
                expected_old_recipe_count,
                "{version:?}, skill={skill_id}"
            );
            assert!(!applied.iter().any(|r| matches!(r.id, 99421 | 99422)));
            let (damage, _, _) = calc_event_damage(
                &attack,
                &Attributes::default(),
                &target,
                &p,
                &runtime,
                &table,
                1,
            );
            assert_eq!(
                damage.normal_damage, expected_damage,
                "{version:?}, skill={skill_id}"
            );
        }
    }
}

#[test]
fn arena_triples_actual_snapshot_bleed_and_removal_restores_damage() {
    let mut bleed = skill(8249);
    bleed.base_damage = 10000.0;
    bleed.attack_coeff = 0.0;
    bleed.weapon_coeff = 0.0;
    bleed.defense_ignore = 1.0;
    let snapshot = DotSnapshot {
        panel_attack: 0.0,
        crit_rate: 0.0,
        crit_power: 1.75,
        strain: 0.0,
        all_dmg_add: 0.0,
    };
    let table = load_recipes(&data_path("2026_10_苍生铸世测试服").join("recipes.toml"));
    ensure_recipe_index(&table);
    let attr = Attributes::default();
    let target = TargetConfig {
        level: PLAYER_LEVEL,
        defense_bonus: 0.0,
        damage_cof: 0.0,
    };
    let mut p = player(vec![29066]);
    p.constants.non_player_bonus = 0.0;
    let before = calc_damage_with_snapshot(&bleed, &attr, &target, &snapshot, &p, &table);
    assert_eq!(before.normal_damage, 10000.0);
    skills::shi_jin_bing_qiong::cast_skill(&mut p, &mut ScriptEmitter::new(), 0.0);
    let inside = calc_damage_with_snapshot(&bleed, &attr, &target, &snapshot, &p, &table);
    assert_eq!(inside.normal_damage, 30000.0);
    p.remove_buff(BUFF_JIAO_DOU_CHANG);
    let after = calc_damage_with_snapshot(&bleed, &attr, &target, &snapshot, &p, &table);
    assert_eq!(after.normal_damage, before.normal_damage);
}
