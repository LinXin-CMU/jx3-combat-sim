use super::*;
use crate::scripts::v2026_10_CangShengZhuShiTest::on_post_cast;
use crate::shield_reset::{ResetMode, ResetOptions, ShieldResetProc};

#[test]
fn shenwei_uses_shared_damage_chain_and_only_modifies_two_shield_skills() {
    let table = load_recipes(&data_path("2026_10_苍生铸世测试服").join("recipes.toml"));
    ensure_recipe_index(&table);
    let target = TargetConfig {
        level: PLAYER_LEVEL,
        defense_bonus: 0.0,
        damage_cof: 0.0,
    };
    for (id, expected) in [(13044, 21201.0), (13045, 20898.0), (13046, 10000.0)] {
        let mut spec = skill(id);
        spec.base_damage = 10000.0;
        spec.attack_coeff = 0.0;
        spec.weapon_coeff = 0.0;
        spec.defense_ignore = 1.0;
        for (talents, expected_damage) in [(vec![], 10000.0), (vec![TALENT_SHEN_WEI], expected)] {
            let mut p = player(talents);
            p.constants.non_player_bonus = 0.0;
            let extras = runtime_recipes(&spec, &p);
            let mut duplicate_extras = extras.clone();
            duplicate_extras.extend(&extras);
            let (damage, _, _) = calc_event_damage(
                &spec,
                &Attributes::default(),
                &target,
                &p,
                &duplicate_extras,
                &table,
                1,
            );
            assert_eq!(damage.normal_damage, expected_damage, "skill={id}");
        }
    }
}

#[test]
fn shixue_replaces_all_rage_tiers_with_and_without_cost_recipe() {
    let table = load_recipes(&data_path("2026_10_苍生铸世测试服").join("recipes.toml"));
    assert!(!table.iter().any(|r| r.id == 99240));
    ensure_recipe_index(&table);
    for reduced in [false, true] {
        for selected in [false, true] {
            let mut p = player(if selected { vec![21281] } else { vec![] });
            if reduced {
                p.active_recipes.insert(3005);
            }
            for tier in 0..5 {
                p.last_rage_cost = if reduced { 10 } else { 25 } + tier * 10;
                let ids = skills::jue_dao::runtime_recipes(&p);
                let recipes = collect_recipes(&p, 13055, "绝刀", &ids, &table);
                let bonus: f64 = recipes.iter().map(|r| r.damage_pct).sum();
                let expected = if selected {
                    tier as f64 * 0.3
                } else {
                    [0.0, 0.200195, 0.400391, 0.599609, 0.799805][tier as usize]
                };
                assert!(
                    (bonus - expected).abs() < 1e-8,
                    "tier={tier}, selected={selected}, reduced={reduced}: {bonus}"
                );
                assert!(!recipes.iter().any(|r| r.id == 99240));
                assert_eq!(recipes.iter().any(|r| r.id == 99241), selected);
            }
        }
    }
    let mut p = player(vec![21281]);
    skills::jue_dao::cast_skill(&mut p, &mut ScriptEmitter::new(), 0.0);
    assert!(p.has_buff(BUFF_SHI_XUE));
    p.current_time = 12.0;
    assert!(!p.has_buff(BUFF_SHI_XUE));
}

#[test]
fn shield_reset_accumulates_without_fractionally_shortening_the_cooldown() {
    let mut p = player(vec![]);
    p.add_protect_cd("cd_盾压", 12.0);
    let before_gen = p.decision_generation;
    for _ in 0..2 {
        p.try_reset_test_dunya();
        assert_eq!(p.active_cds.get("cd_盾压"), Some(&12.0));
    }
    p.try_reset_test_dunya();
    assert!(!p.active_cds.contains_key("cd_盾压"));
    assert!(p.decision_generation > before_gen);
    // Residual .05 survives a successful proc; three more trials trigger again.
    p.add_protect_cd("cd_盾压", 12.0);
    for _ in 0..3 {
        p.try_reset_test_dunya();
    }
    assert!(!p.active_cds.contains_key("cd_盾压"));
}

#[test]
fn reset_mode_counts_and_seed_replay_are_correct() {
    for chance in [0.35, 0.40, 0.45] {
        let mut proc = ShieldResetProc::new(ResetOptions::default());
        assert_eq!(
            (0..200).filter(|_| proc.trigger(chance)).count(),
            (200.0 * chance).round() as usize
        );
    }
    let trials = |seed| {
        let mut proc = ShieldResetProc::new(ResetOptions {
            mode: ResetMode::Random,
            seed,
        });
        (0..10000).map(|_| proc.trigger(0.35)).collect::<Vec<_>>()
    };
    let first = trials(17);
    assert_eq!(first, trials(17));
    assert_ne!(first, trials(18));
    assert!((first.iter().filter(|&&v| v).count() as i32 - 3500).abs() < 200);
    assert!(
        first.windows(2).any(|w| w == [true, true]),
        "Independent trials can proc consecutively"
    );
}

#[test]
fn shield_reset_respects_damage_cast_scope_talents_recipes_and_parry_independence() {
    for parry in [0.0, 100000.0] {
        let mut p = player(vec![]);
        p.base_attrs.parry_level = parry;
        p.add_protect_cd("cd_盾压", 12.0);
        for id in [13052, 13055, 30769] {
            on_post_cast(&mut p, &skill(id));
        }
        assert!(p.active_cds.contains_key("cd_盾压"));
        for id in [13045, 13044, 13046] {
            on_post_cast(&mut p, &skill(id));
        }
        assert!(!p.active_cds.contains_key("cd_盾压"));
    }
    let mut disabled = player(vec![TALENT_DUN_SHENG_FENG]);
    disabled.add_protect_cd("cd_盾压", 12.0);
    for _ in 0..10 {
        on_post_cast(&mut disabled, &skill(13044));
    }
    assert!(disabled.active_cds.contains_key("cd_盾压"));
    for (recipes, expected) in [
        (vec![], 7),
        (vec![4007], 8),
        (vec![4008], 8),
        (vec![4007, 4008], 9),
    ] {
        let mut p = player(vec![]);
        p.active_recipes = recipes.into_iter().collect();
        let mut count = 0;
        for _ in 0..20 {
            p.add_protect_cd("cd_盾压", 12.0);
            p.try_reset_test_dunya();
            count += usize::from(!p.active_cds.contains_key("cd_盾压"));
        }
        assert_eq!(count, expected);
    }
    let mut old = Player::new(0, vec![], vec![]);
    old.add_protect_cd("cd_盾压", 12.0);
    for _ in 0..10 {
        old.try_reset_test_dunya();
    }
    assert!(old.active_cds.contains_key("cd_盾压"));
}

#[test]
fn dunya_debuff_matches_five_seconds_and_fifteen_second_internal_cooldown() {
    let mut p = player(vec![]);
    skills::dun_ya::cast_skill(&mut p, &mut ScriptEmitter::new(), 0.0);
    assert_eq!(
        p.target_buffs
            .iter()
            .find(|b| b.buff_id == BUFF_BU_CAN)
            .unwrap()
            .expires_at,
        5.0
    );
    assert_eq!(
        p.active_buffs
            .iter()
            .find(|b| b.buff_id == BUFF_HUAN_SHEN)
            .unwrap()
            .expires_at,
        15.0
    );
}
