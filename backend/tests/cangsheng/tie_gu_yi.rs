use super::*;
use crate::*;

fn player(talents: Vec<u32>) -> Player {
    let constants = load_school_toml(GameVersion::CangShengZhuShiTest, Mount::TieGuYi).unwrap().0;
    Player::with_mount(Mount::TieGuYi, GameVersion::CangShengZhuShiTest, constants, 0, talents, vec![])
}
fn skill(id: u32) -> SkillSpec {
    load_skills(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/2026_10_苍生铸世测试服/铁骨衣/skills"))
        .into_iter().find(|s| s.skill_id == id).unwrap()
}
fn cast(p: &mut Player, id: u32) -> ScriptEmitter {
    let s = skill(id);
    p.cast_skill(&s, None, None, 0.0, 0.0).expect("cast must succeed");
    crate::scripts::run_scripts(p, &s, p.current_time)
}

#[test]
fn shield_rage_and_talent_pool_are_independent() {
    for (id, rage) in [(13044, 10), (13045, 20)] {
        let mut p = player(vec![30769, 91001, 13133]);
        p.set_rage(0);
        cast(&mut p, id);
        assert_eq!(p.rage, rage);
        assert!(!p.uses_berserk());
        assert!(!p.has_talent(30769));
        assert!(p.has_talent(13133));
    }
    let mut p = player(vec![]);
    p.set_rage(0);
    p.add_buff(BUFF_CHENG_WU);
    cast(&mut p, 13045);
    assert_eq!(p.rage, 40);
}

#[test]
fn nu_yan_preserves_weakness_and_refunds_actual_rage_only_once() {
    let mut p = player(vec![13133]);
    p.set_stance(Stance::Blade);
    p.set_rage(15);
    p.add_target_buff(BUFF_XU_RUO);
    cast(&mut p, 13054);
    assert!(p.has_target_buff(BUFF_XU_RUO));
    assert!(!p.has_target_buff(BUFF_LIU_XUE));
    assert!(p.has_buff(BUFF_NU_YAN));
    p.set_rage(65);
    cast(&mut p, 13055);
    assert_eq!(p.rage, 65);
    assert!(!p.has_buff(BUFF_NU_YAN));
    cast(&mut p, 13055);
    assert_eq!(p.rage, 0);
}

#[test]
fn nu_yan_no_weakness_expiry_and_overlapping_free_casts() {
    let mut p = player(vec![13133]);
    skills::zhan_dao::cast_skill(&mut p, &mut ScriptEmitter::new(), 0.0);
    assert!(p.has_buff(BUFF_NU_YAN));
    p.current_time = 6.0;
    assert!(!p.has_buff(BUFF_NU_YAN));
    p.add_buff(BUFF_NU_YAN);
    p.add_buff(BUFF_KUANG_JUE);
    p.add_buff(BUFF_CHENG_WU);
    p.set_rage(0);
    p.last_rage_cost = 35;
    skills::jue_dao::cast_skill(&mut p, &mut ScriptEmitter::new(), 6.0);
    assert_eq!(p.rage, 35);
    assert!(!p.has_buff(BUFF_KUANG_JUE));
    assert!(!p.has_buff(BUFF_NU_YAN));
    let mut plain = player(vec![]);
    plain.add_target_buff(BUFF_XU_RUO);
    skills::zhan_dao::cast_skill(&mut plain, &mut ScriptEmitter::new(), 0.0);
    assert!(!plain.has_target_buff(BUFF_XU_RUO));
    assert!(plain.has_target_buff(BUFF_LIU_XUE));
}

#[test]
fn nu_yan_recipe_only_modifies_jue_dao() {
    let p = player(vec![13133]);
    assert_eq!(runtime_recipes(&skill(13055), &p), vec![RECIPE_NU_YAN]);
    assert!(runtime_recipes(&skill(13054), &p).is_empty());
    assert!(runtime_recipes(&skill(13055), &player(vec![])).is_empty());
    let recipes = load_recipes(std::path::Path::new(&recipes_file_for_mount(p.version, p.mount)));
    let recipe = recipes.iter().find(|r| r.id == RECIPE_NU_YAN).unwrap();
    assert_eq!(recipe.damage_pct, 0.15);
    assert!(recipe.hidden);
}

#[test]
fn vitality_auras_use_base_and_ji_ang_uses_final_vitality() {
    let mut p = player(vec![15072, 13422, 13356, 13124]);
    p.base_attrs.vitality = 950.0;
    p.set_block_value(19);
    assert!(!p.can_cast(&skill(15072)));
    p.set_block_value(100);
    cast(&mut p, 15072);
    assert_eq!(p.block_value, 80);
    assert_eq!(p.buff_stacks(BUFF_HAN_XIAO), 10);
    p.last_rage_cost = 10;
    skills::dun_dang::cast_skill(&mut p, &mut ScriptEmitter::new(), 0.0);
    assert_eq!(p.buff_stacks(BUFF_ZHEN_FEN), 10);
    for id in [13046, 25204, 13054] {
        crate::scripts::run_scripts(&mut p, &skill(id), 0.0);
        assert_eq!(p.buff_stacks(BUFF_JI_ANG), (p.current_stats().vitality / 95.0).floor() as u32);
    }
    let slots = aggregate_buff_fields(&p);
    assert_eq!(slot(&slots, AttribField::StrainBase), 80.0);
    assert_eq!(slot(&slots, AttribField::ParryBase), 100.0);
    assert_eq!(slot(&slots, AttribField::SurplusValueBase), 0.0);
    p.current_time = 15.0;
    assert!(!p.has_buff(BUFF_HAN_XIAO));
    assert!(!p.has_buff(BUFF_JI_ANG));
    assert!(p.has_buff(BUFF_ZHEN_FEN));
    p.current_time = 30.0;
    assert!(!p.has_buff(BUFF_ZHEN_FEN));
}

#[test]
fn vitality_units_refresh_without_old_cap_or_accumulation() {
    let mut p = player(vec![13356]);
    for (vitality, stacks) in [(94.0, 0), (95.0, 1), (189.0, 1), (190.0, 2), (9595.0, 101), (95.0, 1)] {
        p.base_attrs.vitality = vitality;
        apply_ji_ang(&mut p);
        assert_eq!(p.buff_stacks(BUFF_JI_ANG), stacks);
    }
    let p = player(vec![]);
    assert!(!p.can_cast(&skill(15072)));
}

#[test]
fn jing_ting_cooldown_and_zheng_bei_threat_are_versioned() {
    assert_eq!(player(vec![TALENT_JING_TING]).effective_charge_cd(&skill(13050)), 8.0);
    assert_eq!(player(vec![]).effective_charge_cd(&skill(13050)), 18.0);
    let mut p = player(vec![]);
    cast(&mut p, 40721);
    assert_eq!(slot(&aggregate_buff_fields(&p), AttribField::ThreatPercent), 1024.0);
    assert!(crate::scripts::all_buff_defs_by_version(p.version, p.mount).iter().any(|b| b.buff_id == super::buffs::defs::BUFF_ZHENG_BEI));
    p.current_time = 8.0;
    assert_eq!(slot(&aggregate_buff_fields(&p), AttribField::ThreatPercent), 0.0);
    assert!(crate::scripts::get_buff_def_by_version(GameVersion::AnYingQianJi, Mount::TieGuYi, super::buffs::defs::BUFF_ZHENG_BEI).is_none());
}

#[test]
fn level50_skills_neither_load_nor_emit_surplus_segments() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data/2026_10_苍生铸世测试服/铁骨衣/skills");
    assert!(load_skills(&path).iter().all(|s| s.damage_kind != DamageKind::SurplusOnly));
    for id in [13054, 13055, 13048] {
        let mut p = player(vec![]);
        let events = crate::scripts::run_scripts(&mut p, &skill(id), 0.0);
        assert!(events.events.iter().all(|e| !e.name.starts_with("破·")));
    }
}
