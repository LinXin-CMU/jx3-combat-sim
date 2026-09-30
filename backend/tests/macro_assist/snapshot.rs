//! 供编辑器宏条件分析使用的完整状态快照回归。
use super::*;

fn fixture(version: GameVersion, mount: Mount, talents: Vec<u32>) -> (Player, Vec<SkillSpec>) {
    let (constants, _, _, _, _) = load_school_toml(version, mount).unwrap();
    let skills = load_skills(Path::new(&skills_dir(version, mount)));
    assert!(!skills.is_empty());
    let mut player = Player::with_mount(mount, version, constants, 0, talents, vec![]);
    let mut first_ranks = std::collections::BTreeMap::new();
    for skill in skills.iter().filter(|skill| !skill.passive) {
        let name = if (90010..=90012).contains(&skill.skill_id) {
            skill.name.as_str()
        } else {
            skill.name.split('·').next().unwrap()
        };
        first_ranks.entry(name).or_insert_with(|| skill.clone());
    }
    player.snapshot_skill_specs = Some(first_ranks.into_values().collect());
    (player, skills)
}

fn skill<'a>(skills: &'a [SkillSpec], id: u32) -> &'a SkillSpec {
    skills.iter().find(|skill| skill.skill_id == id).unwrap()
}

fn state_skill(state: &EventState, id: u32) -> &EventSkillState {
    state
        .skill_states
        .as_ref()
        .unwrap()
        .iter()
        .find(|skill| skill.skill_id == id)
        .unwrap()
}

fn condition_passes(player: &Player, skills: &[SkillSpec], condition: &str) -> bool {
    let config = macro_parser::parse_macro_text(&format!("/cast [{condition}] 盾刀")).unwrap();
    let mut by_name = HashMap::new();
    let by_id = skills.iter().map(|skill| (skill.skill_id, skill)).collect();
    for skill in skills.iter().filter(|skill| !skill.passive) {
        let name = skill.name.split('·').next().unwrap();
        by_name.entry(name).or_insert_with(Vec::new).push(skill);
    }
    let (pool, _, _) =
        macro_eval::evaluate_phase1(&config.pages[0], player, &by_name, &by_id, None, false);
    !pool.is_empty()
}

#[test]
fn macro_snapshot_records_full_and_recovered_charges_across_versions_and_mounts() {
    for (version, mount) in [
        (GameVersion::ShanHaiYuanLiu, Mount::FenShanJin),
        (GameVersion::ShanHaiYuanLiu, Mount::TieGuYi),
        (GameVersion::AnYingQianJi, Mount::FenShanJin),
        (GameVersion::AnYingQianJi, Mount::TieGuYi),
        (GameVersion::CangShengZhuShiTest, Mount::FenShanJin),
    ] {
        let (mut player, skills) = fixture(version, mount, vec![]);
        let blood_rage = skill(&skills, 13040);
        let initial = snapshot_event_state(&player);
        let maximum = player.effective_max_charges(blood_rage);
        assert_eq!(state_skill(&initial, 13040).charges, Some(maximum));
        assert_eq!(state_skill(&initial, 13040).max_charges, Some(maximum));

        player.consume_charge(blood_rage, 0.0);
        let spent = snapshot_event_state(&player);
        assert_eq!(state_skill(&spent, 13040).charges, Some(maximum - 1));
        player.current_time = player.effective_charge_cd(blood_rage);
        let recovered = snapshot_event_state(&player);
        assert_eq!(state_skill(&recovered, 13040).charges, Some(maximum));
        assert_eq!(
            state_skill(&initial, 13040).charges,
            Some(maximum),
            "历史快照不得随着后续充能消耗改变"
        );
        assert!(condition_passes(
            &player,
            &skills,
            &format!("skill_energy:血怒={maximum}")
        ));
    }
}

#[test]
fn macro_snapshot_uses_talent_adjusted_capacity_and_marks_non_charge_skills() {
    let (player, _) = fixture(GameVersion::ShanHaiYuanLiu, Mount::FenShanJin, vec![36058]);
    let state = snapshot_event_state(&player);
    assert_eq!(state_skill(&state, 13047).charges, Some(4));
    assert_eq!(state_skill(&state, 13047).max_charges, Some(4));

    let (player, _) = fixture(GameVersion::AnYingQianJi, Mount::FenShanJin, vec![36058]);
    let state = snapshot_event_state(&player);
    assert_eq!(state_skill(&state, 13047).charges, Some(999));
    assert_eq!(state_skill(&state, 13047).max_charges, Some(999));

    let (player, _) = fixture(
        GameVersion::CangShengZhuShiTest,
        Mount::FenShanJin,
        vec![30769],
    );
    let state = snapshot_event_state(&player);
    assert_eq!(
        state_skill(&state, 30769).charges,
        None,
        "测试服阵云已移除充能，不能沿用旧版本两层数据"
    );
    assert_eq!(state_skill(&state, 30769).max_charges, None);
    assert_eq!(state_skill(&state, 13045).charges, None);
}

#[test]
fn macro_snapshot_cd_predicate_includes_gcd_exactly_as_macro_phase_one() {
    let (mut player, skills) = fixture(GameVersion::AnYingQianJi, Mount::FenShanJin, vec![]);
    let shield_strike = skill(&skills, 13047);
    assert!(player
        .cast_skill(shield_strike, None, None, 0.0, 0.0)
        .is_some());
    let state = snapshot_event_state(&player);
    assert_eq!(state_skill(&state, 13047).charges, Some(2));
    assert!(
        !state_skill(&state, 13047).not_in_cd,
        "仍有充能也不能跳过技能绑定的 GCD"
    );
    assert_eq!(
        state_skill(&state, 13047).not_in_cd,
        condition_passes(&player, &skills, "skill_notin_cd:盾击")
    );
    assert!(
        state
            .skill_cds
            .iter()
            .all(|cd| !cd.name.starts_with("gcd_")),
        "旧 CD 展示字段不能代替精确宏状态"
    );
    player.current_time = shield_strike
        .cooldowns
        .iter()
        .filter(|cd| cd.cd_id.starts_with("gcd_"))
        .filter_map(|cd| player.active_cds.get(&cd.cd_id).copied())
        .fold(player.current_time, f64::max);
    let ready = snapshot_event_state(&player);
    assert!(state_skill(&ready, 13047).not_in_cd);
    assert!(condition_passes(&player, &skills, "skill_notin_cd:盾击"));
}

#[test]
fn macro_snapshot_keeps_self_debuffs_permanent_buffs_and_expiry_distinct() {
    let (mut player, skills) = fixture(GameVersion::AnYingQianJi, Mount::FenShanJin, vec![]);
    player.add_buff_with_stacks(BUFF_DUN_FEI, 1, 0);
    player.add_buff_with_stacks(BUFF_XUE_NU, 1, 16);
    player.add_target_buff_with_stacks(BUFF_XU_RUO, 1, 0);
    assert!(player.buff_def(BUFF_DUN_FEI).unwrap().is_debuff);
    let state = snapshot_event_state(&player);
    let shield_flight = state
        .buffs
        .iter()
        .find(|buff| buff.buff_id == BUFF_DUN_FEI)
        .unwrap();
    assert_eq!(shield_flight.remaining, 0.0);
    assert!(condition_passes(
        &player,
        &skills,
        "buff:盾飞&bufftime:盾飞>100"
    ));
    assert!(condition_passes(&player, &skills, "tbufftime:虚弱>100"));
    assert_eq!(
        state
            .target_buffs
            .iter()
            .find(|buff| buff.buff_id == BUFF_XU_RUO)
            .unwrap()
            .remaining,
        0.0
    );
    assert!(
        state
            .buffs
            .iter()
            .any(|buff| buff.buff_id == BUFF_STANCE_SHIELD_GAME),
        "姿态宏别名所需的隐藏游戏 buff 必须保留"
    );

    player.current_time = 1.0;
    let expired = snapshot_event_state(&player);
    assert_eq!(expired.time, 1.0);
    assert!(!expired.buffs.iter().any(|buff| buff.buff_id == BUFF_XUE_NU));
    assert!(expired
        .buffs
        .iter()
        .any(|buff| buff.buff_id == BUFF_DUN_FEI && buff.remaining == 0.0));
    assert!(state
        .buffs
        .iter()
        .any(|buff| buff.buff_id == BUFF_XUE_NU && buff.remaining == 1.0));
}

#[test]
fn macro_snapshot_without_skill_catalog_does_not_claim_empty_or_ready_skills() {
    let player = Player::new(0, vec![], vec![]);
    let state = snapshot_event_state(&player);
    assert!(state.skill_states.is_none());
    assert!(serde_json::to_value(state)
        .unwrap()
        .get("skill_states")
        .is_none());
}

#[test]
fn macro_snapshot_game_stance_buffs_match_phase_one_across_versions_and_mounts() {
    for (version, mount) in [
        (GameVersion::ShanHaiYuanLiu, Mount::FenShanJin),
        (GameVersion::ShanHaiYuanLiu, Mount::TieGuYi),
        (GameVersion::AnYingQianJi, Mount::FenShanJin),
        (GameVersion::AnYingQianJi, Mount::TieGuYi),
        (GameVersion::CangShengZhuShiTest, Mount::FenShanJin),
    ] {
        let (mut player, skills) = fixture(version, mount, vec![]);
        for stance in [Stance::Shield, Stance::Blade, Stance::Wall] {
            player.set_stance(stance);
            let state = snapshot_event_state(&player);
            assert_eq!(state.stance, stance);
            for (id, alias) in [
                (BUFF_STANCE_SHIELD_GAME, "擎盾"),
                (BUFF_STANCE_BLADE_GAME, "擎刀"),
            ] {
                let observed = state.buffs.iter().find(|buff| buff.buff_id == id);
                assert_eq!(
                    observed.is_some(),
                    condition_passes(&player, &skills, &format!("buff:{alias}"))
                );
                assert_eq!(
                    observed.is_some(),
                    condition_passes(&player, &skills, &format!("buff:{id}"))
                );
                if let Some(buff) = observed {
                    assert_eq!(buff.name, alias);
                    assert_eq!(buff.remaining, 0.0);
                    assert_eq!(buff.stacks, 1);
                    assert!(buff.icon.is_empty());
                    assert!(condition_passes(
                        &player,
                        &skills,
                        &format!("bufftime:{alias}>100")
                    ));
                }
            }
            assert!(
                state.buffs.iter().all(|buff| ![
                    BUFF_STANCE_SHIELD,
                    BUFF_STANCE_BLADE,
                    BUFF_STANCE_WALL,
                ]
                .contains(&buff.buff_id)),
                "内部合成姿态 ID 不应混入游戏宏条件候选"
            );
        }
    }
}

#[test]
fn macro_snapshot_catalog_is_bound_in_full_simulation_without_changing_lite_combat() {
    let version = GameVersion::AnYingQianJi;
    let mount = Mount::FenShanJin;
    let (player, skills) = fixture(version, mount, vec![]);
    let mut request: SimulateRequest = serde_json::from_value(serde_json::json!({
        "haste_level": 0, "sequence": ["血怒", "盾击", "盾击"],
        "talents": [], "recipes": [], "equipment": {}, "attributes": null,
        "target": null, "team_buffs": [], "formation": null,
        "initial_rage": 0, "pre_releases": [], "network_delay": 0,
        "pauses": [], "channel_ticks": {}, "timing_offsets": {}, "qijin_buffs": {}
    }))
    .unwrap();
    let full = simulate_core(
        &request,
        &skills,
        version,
        mount,
        player.constants,
        &[],
        &[],
        &[],
    );
    let casts = full
        .timeline
        .iter()
        .filter(|event| !event.triggered)
        .collect::<Vec<_>>();
    assert_eq!(casts.len(), 3);
    assert_eq!(
        state_skill(casts[0].state_before.as_ref().unwrap(), 13040).charges,
        Some(3)
    );
    assert_eq!(
        state_skill(casts[0].state_after.as_ref().unwrap(), 13040).charges,
        Some(2)
    );
    for cast in &casts {
        assert!(cast.state_before.as_ref().unwrap().skill_states.is_some());
        assert!(cast.state_before.as_ref().unwrap().time <= cast.cast_time + 0.001);
    }

    request.lite = true;
    request.lite_keep_timeline = true;
    let lite = simulate_core(
        &request,
        &skills,
        version,
        mount,
        player.constants,
        &[],
        &[],
        &[],
    );
    let combat = |response: &SimulateResponse| {
        response
            .timeline
            .iter()
            .map(|event| {
                (
                    event.skill_id,
                    event.cast_time,
                    event.triggered,
                    event.rage_after,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(combat(&full), combat(&lite));
    assert_eq!(full.rage, lite.rage);
    assert_eq!(full.fight_time, lite.fight_time);
    assert!(lite
        .timeline
        .iter()
        .all(|event| event.state_before.is_none() && event.state_after.is_none()));
}
