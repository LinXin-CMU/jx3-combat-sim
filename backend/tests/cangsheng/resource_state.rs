//! 苍生铸世测试服的资源、连招与完整模拟隔离回归。
use super::*;

const VERSION: GameVersion = GameVersion::CangShengZhuShiTest;
const ZHEN_YUN: u32 = 30769;
const BU_GUI: u32 = 91001;
const SHEN_WEI: u32 = 91002;

struct Fixture {
    constants: MountConstants,
    skills: Vec<SkillSpec>,
    recipes: Vec<RecipeEntry>,
    team_buffs: Vec<TeamBuffEntry>,
    formations: Vec<FormationEntry>,
}

impl Fixture {
    fn load() -> Self {
        let (constants, _, _, _, _) = load_school_toml(VERSION, Mount::FenShanJin).unwrap();
        Self {
            constants,
            skills: load_skills(Path::new(&skills_dir(VERSION, Mount::FenShanJin))),
            recipes: load_recipes(Path::new(&recipes_file(VERSION))),
            team_buffs: load_team_buffs(Path::new(&team_buffs_file(VERSION))),
            formations: load_formations(Path::new(&formations_file(VERSION))),
        }
    }

    fn skill(&self, id: u32) -> &SkillSpec {
        self.skills
            .iter()
            .find(|skill| skill.skill_id == id)
            .unwrap()
    }

    fn player(&self, talents: Vec<u32>) -> Player {
        Player::with_mount(
            Mount::FenShanJin,
            VERSION,
            self.constants,
            0,
            talents,
            vec![],
        )
    }

    fn simulate(&self, request: &SimulateRequest) -> SimulateResponse {
        simulate_core(
            request,
            &self.skills,
            VERSION,
            Mount::FenShanJin,
            self.constants,
            &self.recipes,
            &self.team_buffs,
            &self.formations,
        )
    }
}

fn request(sequence: Vec<String>, talents: Vec<u32>) -> SimulateRequest {
    serde_json::from_value(serde_json::json!({
        "haste_level": 0,
        "sequence": sequence,
        "talents": talents,
        "attributes": {
            "base_attack": 38466.0, "weapon_damage": 10986.0,
            "crit_level": 54841.0, "crit_effect_level": 0.0,
            "overcome_level": 29480.0, "strain_level": 66031.0, "haste_level": 0.0
        },
        "target": {"level": 54, "defense_bonus": 0.0, "damage_cof": 0.0},
        "network_delay": 0, "initial_rage": 0,
        "recipes": [], "equipment": {}, "team_buffs": [], "formation": null,
        "pre_releases": [], "pauses": [], "channel_ticks": {},
        "timing_offsets": {}, "qijin_buffs": {}
    }))
    .unwrap()
}

#[test]
fn berserk_transactions_measure_overflow_and_passive_tick_times() {
    let fixture = Fixture::load();
    let mut p = fixture.player(vec![ZHEN_YUN, BU_GUI]);
    p.set_berserk_value(100);
    p.add_berserk_value_from(20, "测试回复");
    assert_eq!(p.berserk_transactions[0].overflow, 0, "达到上限不等于溢出");
    p.current_time = 0.5;
    p.add_berserk_value_from(50, "不归·血怒回复");
    assert_eq!(p.berserk_transactions[1].overflow, 50);
    assert_eq!(p.berserk_transactions[1].time_seconds, 0.5);
    p.advance_berserk_to(3.5);
    assert_eq!(p.berserk_transactions.iter().skip(2).map(|t| (t.time_seconds, t.overflow)).collect::<Vec<_>>(), vec![(1.0, 2), (2.0, 2), (3.0, 2)]);
    p.advance_berserk_to(3.5);
    assert_eq!(p.berserk_transactions.len(), 5, "重复推进不重复记账");
    p.add_berserk_value_from(-100, "阵云结晦消耗");
    assert_eq!(p.berserk_transactions.last().unwrap().overflow, 0);
    assert_eq!(p.berserk_value, 20);
    let mut disabled = fixture.player(vec![]);
    disabled.add_berserk_value_from(50, "不归·血怒回复");
    assert!(disabled.berserk_transactions.is_empty());
}

#[test]
fn agent_reports_berserk_separately_and_pages_passive_overflow() {
    use crate::agent::{schema::ScenarioSnapshotV1, tools::{SimulatorContext, ToolBudget, simulate_scenario}, evidence::ToolProvenance, timeline::{analyze_timeline, inspect_timeline_events, TimelineEventQueryV1, TimelineEventSelector}};
    let fixture = Fixture::load();
    let req = request(vec!["血怒".into(), "盾压".into(), "盾压".into()], vec![ZHEN_YUN, BU_GUI]);
    let snapshot = ScenarioSnapshotV1::capture(VERSION, Mount::FenShanJin, req.clone()).unwrap();
    let context = SimulatorContext { game_version: VERSION, mount: Mount::FenShanJin, constants: fixture.constants, skills: &fixture.skills, talents: &[], recipes: &fixture.recipes, team_buffs: &fixture.team_buffs, formations: &fixture.formations };
    let provenance = ToolProvenance::fixture();
    let simulation = simulate_scenario("trace-berserk", &snapshot, &context, &provenance, &mut ToolBudget::new(1)).unwrap();
    let analysis = analyze_timeline("trace-berserk-analysis", &simulation, &provenance).unwrap();
    let observed = analysis.evidence.result.berserk.unwrap();
    assert!(observed.modeled && observed.overflow_tracking_available);
    assert_eq!(observed.cap, 120);
    assert_eq!(observed.overflow_by_source["不归·血怒回复"], 50);
    assert!(observed.overflow_by_source["不归每秒回复"] > 0);
    assert_eq!(observed.generated_before_cap, observed.gained_after_cap + observed.overflow_total);
    let mut query = TimelineEventQueryV1 { selector: TimelineEventSelector::BerserkOverflow, rage_cost_below: None, event_number: None, skill_name: None, time_seconds: None, start_match: 0, limit: 2, context_radius: 1, buff_names: vec![] };
    let mut received = Vec::new();
    loop {
        let page = inspect_timeline_events("trace-berserk-page", &simulation, &query, &provenance).unwrap().evidence.result;
        assert_eq!(page.total_matches, observed.overflow_events);
        received.extend(page.berserk_events);
        if let Some(next) = page.next_start_match { query.start_match = next; } else { break; }
    }
    assert_eq!(received.len(), observed.overflow_events);
    assert_eq!(received[0].event_number, Some(1));
    assert_eq!(received[0].transaction.overflow, 50);
    assert!(received.iter().filter(|e| e.transaction.source == "不归每秒回复").all(|e| e.event_number.is_none()));
    query.selector = TimelineEventSelector::Skill; query.skill_name = Some("血怒".into()); query.start_match = 0;
    let detail = inspect_timeline_events("trace-berserk-snapshot", &simulation, &query, &provenance).unwrap().evidence.result;
    assert_eq!(detail.windows[0].matched.state_before.as_ref().unwrap().berserk_value, Some(120));
    assert_eq!(detail.match_index[0].berserk_before, Some(120));
    let mut lite_req = req;
    lite_req.lite = true;
    let lite = fixture.simulate(&lite_req);
    assert!(lite.berserk_transactions.is_none());
    assert_eq!(lite.fingerprint, simulation.response.fingerprint);
    assert_eq!(lite.total_damage, simulation.response.total_damage);
    assert_eq!(lite.berserk_value, simulation.response.berserk_value);
}

#[test]
fn dunya_cooldown_recipes_apply_to_actual_casts_and_tiegu_reset_state() {
    for (version, mount) in [
        (GameVersion::ShanHaiYuanLiu, Mount::FenShanJin),
        (GameVersion::ShanHaiYuanLiu, Mount::TieGuYi),
        (GameVersion::AnYingQianJi, Mount::FenShanJin),
        (GameVersion::AnYingQianJi, Mount::TieGuYi),
        (VERSION, Mount::FenShanJin),
    ] {
        let (constants, _, _, _, _) = load_school_toml(version, mount).unwrap();
        let skills = load_skills(Path::new(&skills_dir(version, mount)));
        let recipes = load_recipes(Path::new(&recipes_file(version)));
        let team_buffs = load_team_buffs(Path::new(&team_buffs_file(version)));
        let formations = load_formations(Path::new(&formations_file(version)));
        for (selected, expected) in [
            (vec![], 12.0), (vec![4005], 11.0), (vec![4006], 11.0),
            (vec![4005, 4006], 10.0), (vec![4005, 4005, 4006], 10.0),
        ] {
            let mut req = request(vec!["盾压".into(), "盾压".into()], vec![]);
            req.recipes = selected;
            let full = simulate_core(&req, &skills, version, mount, constants,
                &recipes, &team_buffs, &formations);
            assert!(full.skipped.is_empty(), "{version:?}/{mount:?}: {:?}", full.skipped);
            let casts: Vec<_> = full.timeline.iter().filter(|e| !e.triggered).collect();
            assert_eq!(casts.len(), 2);
            assert_eq!(casts[1].cast_time - casts[0].cast_time, expected,
                "{version:?}/{mount:?}, recipes={:?}", req.recipes);
            let state = casts[0].state_after.as_ref().unwrap();
            let cd = state.skill_cds.iter().find(|cd| cd.name == "盾压").unwrap();
            assert_eq!(cd.remaining, expected, "cooldown snapshot must match actual cast");
            req.lite = true;
            let lite = simulate_core(&req, &skills, version, mount, constants,
                &recipes, &team_buffs, &formations);
            assert_eq!(full.fingerprint, lite.fingerprint);
        }
        // 秘籍只修改盾压独立调息，不影响它的公共调息。
        let press = skills.iter().find(|s| s.skill_id == 13045).unwrap();
        let mut player = Player::with_mount(mount, version, constants, 0, vec![], vec![4005, 4006]);
        assert!(player.cast_skill(press, None, None, 0.0, 0.0).is_some());
        assert_eq!(player.active_cds.get("cd_盾压"), Some(&10.0));
        assert_eq!(player.active_cds.get("gcd_1.5"), Some(&1.5));
    }
}

#[test]
fn berserk_setters_clamp_and_invalidate_without_mutating_rage_or_block() {
    let fixture = Fixture::load();
    let mut player = fixture.player(vec![ZHEN_YUN]);
    assert!(player.uses_berserk());
    assert_eq!(player.berserk_value, 120);
    assert_eq!(player.max_berserk_value(), 120);
    let original = (player.rage, player.block_value);
    for (input, expected) in [(50, 50), (100, 100), (120, 120), (121, 120), (-1, 0)] {
        let generation = player.decision_generation;
        player.set_berserk_value(input);
        assert_eq!(player.berserk_value, expected);
        assert!(player.decision_generation > generation);
        assert_eq!((player.rage, player.block_value), original);
    }
    player.add_berserk_value(i32::MAX);
    player.add_berserk_value(i32::MAX);
    assert_eq!(player.berserk_value, 120);
    player.add_berserk_value(i32::MIN);
    assert_eq!(player.berserk_value, 0);
}

#[test]
fn test_server_ignores_legacy_surplus_input_and_removes_events() {
    let fixture = Fixture::load();
    assert!(fixture.skills.iter().all(|s|
        !matches!(s.damage_kind, DamageKind::SurplusOnly) && s.surplus_coeff == 0.0));
    assert!(fixture.recipes.iter().all(|r| r.surplus_pct == 0.0));
    let mut req = request([
        "阵云结晦", "阵云结晦", "阵云结晦", "盾舞", "盾飞", "斩刀", "绝刀"
    ].into_iter().map(str::to_string).collect(), vec![ZHEN_YUN, SHEN_WEI]);
    req.initial_rage = Some(100);
    req.attributes.as_mut().unwrap().surplus_value = 0.0;
    let without = fixture.simulate(&req);
    assert!(without.skipped.is_empty(), "{:?}", without.skipped);
    for id in [30769, 30855, 30856, 13048, 13054, 13055] {
        assert!(without.timeline.iter().any(|e| e.skill_id == id));
    }
    assert!(without.timeline.iter().all(|e| !e.name.starts_with("破·")));
    req.attributes.as_mut().unwrap().surplus_value = 1_000_000.0;
    let with = fixture.simulate(&req);
    assert_eq!(without.total_damage.to_bits(), with.total_damage.to_bits());
    assert_eq!(without.fingerprint, with.fingerprint);
    assert!(with.timeline.iter().filter_map(|e| e.runtime_stats.as_ref())
        .all(|stats| stats.surplus_value == 0.0), "50级面板不再提供破招");
    req.lite = true;
    let lite = fixture.simulate(&req);
    assert_eq!(with.fingerprint, lite.fingerprint);
    for version in [GameVersion::ShanHaiYuanLiu, GameVersion::AnYingQianJi] {
        let old = load_skills(Path::new(&skills_dir(version, Mount::FenShanJin)));
        for id in [13048901, 13054901, 13055901, 30855901, 30856901] {
            assert!(old.iter().any(|s| s.skill_id == id &&
                matches!(s.damage_kind, DamageKind::SurplusOnly) && s.surplus_coeff > 0.0));
        }
    }
}

#[test]
fn berserk_regeneration_requires_both_talents_and_never_repeats_elapsed_seconds() {
    let fixture = Fixture::load();
    for talents in [vec![], vec![BU_GUI]] {
        let mut player = fixture.player(talents);
        assert!(!player.uses_berserk());
        player.add_berserk_value(120);
        player.advance_berserk_to(60.0);
        assert_eq!(player.berserk_value, 0);
    }
    let mut without_regen = fixture.player(vec![ZHEN_YUN]);
    without_regen.set_berserk_value(0);
    without_regen.advance_berserk_to(60.0);
    assert_eq!(without_regen.berserk_value, 0);

    let mut player = fixture.player(vec![ZHEN_YUN, BU_GUI]);
    player.set_berserk_value(0);
    for (time, expected) in [
        (0.999, 0),
        (1.0, 2),
        (1.999, 2),
        (10.5, 20),
        (10.5, 20),
        (9.0, 20),
    ] {
        player.advance_berserk_to(time);
        assert_eq!(player.berserk_value, expected, "time={time}");
    }
    player.set_berserk_value(0);
    player.advance_berserk_to(11.0);
    assert_eq!(player.berserk_value, 2);
    player.advance_berserk_to(1_000.0);
    assert_eq!(player.berserk_value, 120);
    player.set_berserk_value(0);
    for time in [1_000.0, f64::INFINITY, f64::NAN, -1.0] {
        player.advance_berserk_to(time);
        assert_eq!(player.berserk_value, 0);
    }
}

#[test]
fn zhenyun_cost_thresholds_and_rejection_use_berserk() {
    let fixture = Fixture::load();
    let skill = fixture.skill(ZHEN_YUN);
    assert_eq!(skill.max_charges, 0);
    assert!(skill
        .cooldowns
        .iter()
        .all(|cooldown| !cooldown.cd_id.starts_with("cd_")));
    for (value, cost, can_cast) in [
        (0, 50, false),
        (49, 50, false),
        (50, 50, true),
        (99, 50, true),
        (100, 100, true),
        (120, 100, true),
    ] {
        let mut player = fixture.player(vec![ZHEN_YUN]);
        player.set_berserk_value(value);
        assert_eq!(player.effective_berserk_cost(skill), cost);
        assert_eq!(player.can_cast(skill), can_cast);
        if can_cast {
            player.apply_cast_effects(skill);
            assert_eq!(player.berserk_value, value - cost as i32);
            assert_eq!(player.zhen_yun_berserk_cost, cost);
        } else {
            assert!(player.reject_reason(skill).unwrap().contains("暴怒值不足"));
            assert!(player.cast_skill(skill, None, None, 0.0, 0.0).is_none());
            assert_eq!(player.berserk_value, value);
        }
    }
}

#[test]
fn innate_followups_require_core_and_expire_after_ninety_seconds() {
    let fixture = Fixture::load();
    let first = fixture.skill(ZHEN_YUN);
    let second = fixture.skill(30855);
    let third = fixture.skill(30856);
    for talents in [vec![], vec![SHEN_WEI]] {
        let mut player = fixture.player(talents);
        player.add_state_buff(combo_buff_id("阵云_2"), 1440);
        player.add_state_buff(combo_buff_id("阵云_3"), 1440);
        assert!(!player.can_cast(second));
        assert!(!player.can_cast(third));
    }
    let mut without_shenwei = fixture.player(vec![ZHEN_YUN]);
    without_shenwei.apply_cast_effects(first);
    scripts::run_scripts(&mut without_shenwei, first, 0.0);
    assert!(without_shenwei.has_buff(combo_buff_id("阵云_2")));
    assert!(without_shenwei.can_cast(second));

    let mut player = fixture.player(vec![ZHEN_YUN, SHEN_WEI]);
    player.apply_cast_effects(first);
    scripts::run_scripts(&mut player, first, 0.0);
    assert_eq!(player.berserk_value, 20);
    assert!(player.can_cast(second));
    assert!(!player.can_cast(third));
    player.current_time = 89.999;
    assert!(player.can_cast(second));
    player.process_buff_ticks(0.0, 90.0);
    player.current_time = 90.0;
    assert!(!player.can_cast(second));
    assert!(!player.has_buff(combo_buff_id("阵云_2")));

    let mut player = fixture.player(vec![ZHEN_YUN, SHEN_WEI]);
    player.apply_cast_effects(first);
    scripts::run_scripts(&mut player, first, 0.0);
    player.current_time = 2.0;
    player.apply_cast_effects(second);
    scripts::run_scripts(&mut player, second, 2.0);
    assert!(!player.can_cast(second));
    assert!(player.can_cast(third));
    assert_eq!(player.berserk_value, 20);
    player.current_time = 91.999;
    assert!(player.can_cast(third));
    player.process_buff_ticks(2.0, 92.0);
    player.current_time = 92.0;
    assert!(!player.can_cast(third));
    assert!(!player.has_buff(combo_buff_id("阵云_3")));
}

#[test]
fn old_versions_ignore_berserk_operations_even_with_matching_talent_ids() {
    for version in [
        GameVersion::ShanHaiYuanLiu,
        GameVersion::AnYingQianJi,
        GameVersion::AnYingQianJiTest,
    ] {
        for mount in [Mount::FenShanJin, Mount::TieGuYi] {
            let mut player = Player::with_mount(
                mount,
                version,
                MountConstants::fenshanjin_default(),
                0,
                vec![ZHEN_YUN, BU_GUI, SHEN_WEI],
                vec![],
            );
            assert!(!player.uses_berserk());
            assert_eq!(player.max_berserk_value(), 0);
            let initial = (player.rage, player.block_value);
            player.set_berserk_value(120);
            player.add_berserk_value(50);
            player.advance_berserk_to(60.0);
            assert_eq!(player.berserk_value, 0);
            assert_eq!((player.rage, player.block_value), initial);
        }
    }
}

#[test]
fn zhenyun_cast_chain_keeps_new_rage_unchanged_and_old_rage_gains() {
    for version in [
        VERSION,
        GameVersion::AnYingQianJi,
        GameVersion::ShanHaiYuanLiu,
    ] {
        let (constants, _, _, _, _) = load_school_toml(version, Mount::FenShanJin).unwrap();
        let skills = load_skills(Path::new(&skills_dir(version, Mount::FenShanJin)));
        let cases = if version == VERSION {
            vec![(50, 50), (99, 50), (100, 100), (120, 100)]
        } else {
            vec![(0, 0)]
        };
        for (initial_berserk, cost) in cases {
            let mut player = Player::with_mount(
                Mount::FenShanJin,
                version,
                constants,
                0,
                vec![ZHEN_YUN, SHEN_WEI],
                vec![],
            );
            player.set_rage(10);
            player.set_berserk_value(initial_berserk);
            for (index, skill_id) in [ZHEN_YUN, 30855, 30856].into_iter().enumerate() {
                let skill = skills
                    .iter()
                    .find(|skill| skill.skill_id == skill_id)
                    .unwrap();
                let cast = player.cast_skill(skill, None, None, 0.0, 0.0);
                assert!(
                    cast.is_some(),
                    "{version:?}, berserk={initial_berserk}, skill={skill_id}"
                );
                scripts::run_scripts(&mut player, skill, cast.unwrap().0);
                let expected_rage = if version == VERSION {
                    10
                } else {
                    10 + 15 * (index as i32 + 1)
                };
                assert_eq!(player.rage, expected_rage, "{version:?}, skill={skill_id}");
                assert_eq!(
                    player.berserk_value,
                    initial_berserk - cost,
                    "{version:?}, skill={skill_id}"
                );
            }
            assert!(!player.has_buff(combo_buff_id("阵云_2")));
            assert!(!player.has_buff(combo_buff_id("阵云_3")));
        }
    }
}

#[test]
fn shenwei_same_name_sequence_reaches_followups_after_spending_berserk() {
    let fixture = Fixture::load();
    let req = request(vec!["阵云结晦".to_string(); 3], vec![ZHEN_YUN, SHEN_WEI]);
    let response = fixture.simulate(&req);
    assert!(response.skipped.is_empty(), "{:?}", response.skipped);
    let casts = response
        .timeline
        .iter()
        .filter(|event| !event.triggered)
        .collect::<Vec<_>>();
    assert_eq!(
        casts.iter().map(|event| event.skill_id).collect::<Vec<_>>(),
        vec![ZHEN_YUN, 30855, 30856]
    );
    assert_eq!(
        casts[0].state_before.as_ref().unwrap().berserk_value,
        Some(120)
    );
    for (index, cast) in casts.iter().enumerate() {
        let before = cast.state_before.as_ref().unwrap();
        let after = cast.state_after.as_ref().unwrap();
        assert_eq!(before.rage, 0);
        assert_eq!(after.rage, 0);
        assert_eq!(
            before.berserk_value,
            Some(if index == 0 { 120 } else { 20 })
        );
        assert_eq!(after.berserk_value, Some(20));
    }
    assert_eq!(response.rage, 0);
    assert_eq!(response.berserk_value, Some(20));
    assert!(response.combo_states.is_empty());
}

#[test]
fn test_server_macro_matches_each_zhenyun_stage_by_its_own_name() {
    let fixture = Fixture::load();
    for command in ["/cast", "/fcast"] {
        for names in [
            ["阵云结晦", "月照连营", "雁门迢递"],
            ["30769", "30855", "30856"],
        ] {
            let mut req = request(vec!["__macro__".to_string(); 3], vec![ZHEN_YUN]);
            req.macro_duration = Some(8.0);
            req.macro_text = Some(format!("{command} {}", names[0]));
            let only_first = fixture.simulate(&req);
            assert_eq!(only_first.timeline.iter().filter(|e| !e.triggered).map(|e| e.skill_id).collect::<Vec<_>>(), vec![ZHEN_YUN]);

            // 只列后续两段，不能凭空启动连招。
            req.macro_text = Some(format!("{command} {}\n{command} {}", names[1], names[2]));
            let no_first = fixture.simulate(&req);
            assert!(!no_first.timeline.iter().any(|e| !e.triggered));

            // 第一段置顶也不能抢占月照/雁门；各自的 last_skill 判定保持独立。
            req.macro_text = Some(format!("{command} {}\n{command} [last_skill={}] {}\n{command} [last_skill={}] {}", names[0], names[0], names[1], names[1], names[2]));
            let full = fixture.simulate(&req);
            assert_eq!(full.timeline.iter().filter(|e| !e.triggered).map(|e| e.skill_id).collect::<Vec<_>>(), vec![ZHEN_YUN, 30855, 30856]);
            assert_eq!(full.berserk_value, Some(20));
            req.lite = true;
            let lite = fixture.simulate(&req);
            assert_eq!(full.fingerprint, lite.fingerprint);
            assert_eq!(full.total_damage, lite.total_damage);
        }
    }
}

#[test]
fn macro_waits_for_berserk_regeneration_and_full_lite_stay_identical() {
    let fixture = Fixture::load();
    let mut req = request(vec!["__macro__".to_string(); 30], vec![ZHEN_YUN, BU_GUI]);
    req.macro_text = Some("/cast [berserk>=50] 阵云结晦\n/cast 月照连营\n/cast 雁门迢递".to_string());
    req.macro_duration = Some(41.0);
    let full = fixture.simulate(&req);
    let casts = full
        .timeline
        .iter()
        .filter(|event| !event.triggered && event.skill_id == ZHEN_YUN)
        .collect::<Vec<_>>();
    assert_eq!(casts.len(), 3);
    for (event, expected_time) in casts.iter().zip([0.0, 15.0, 40.0]) {
        assert_eq!(event.skill_id, ZHEN_YUN);
        assert!(
            (event.cast_time - expected_time).abs() < 0.001,
            "{:?}",
            event.cast_time
        );
    }
    assert!(full.total_damage > 0.0);
    assert_eq!(full.max_berserk_value, Some(120));
    req.lite = true;
    let lite = fixture.simulate(&req);
    assert_eq!(full.fingerprint, lite.fingerprint);
    assert_eq!(full.total_damage.to_bits(), lite.total_damage.to_bits());
    assert_eq!(full.dps.to_bits(), lite.dps.to_bits());
    assert_eq!(full.fight_time.to_bits(), lite.fight_time.to_bits());
    assert_eq!(full.berserk_value, lite.berserk_value);
}

#[test]
fn sun_macro_waits_for_berserk_threshold_and_matches_existing_aliases() {
    let fixture = Fixture::load();
    for (threshold, expected_times) in [(50, vec![0.0, 15.0, 40.0]), (100, vec![0.0, 40.0])] {
        let mut req = request(vec!["__macro__".to_string(); 30], vec![ZHEN_YUN, BU_GUI]);
        req.macro_duration = Some(41.0);
        let mut reference = None;
        for keyword in ["sun", "berserk", "baonu"] {
            req.macro_text = Some(format!("/cast [{keyword}>={threshold}] 阵云结晦\n/cast 月照连营\n/cast 雁门迢递"));
            req.lite = false;
            let full = fixture.simulate(&req);
            assert!(full.skipped.is_empty(), "{keyword}: {:?}", full.skipped);
            let casts = full
                .timeline
                .iter()
                .filter(|event| !event.triggered && event.skill_id == ZHEN_YUN)
                .collect::<Vec<_>>();
            assert_eq!(casts.len(), expected_times.len());
            for (cast, time) in casts.iter().zip(&expected_times) {
                assert_eq!(cast.skill_id, ZHEN_YUN);
                assert!((cast.cast_time - time).abs() < 0.001);
                let before = cast.state_before.as_ref().unwrap();
                let after = cast.state_after.as_ref().unwrap();
                assert!(before.berserk_value.unwrap() >= threshold);
                assert_eq!(after.rage, 0);
            }
            let result = (
                full.total_damage.to_bits(),
                full.dps.to_bits(),
                full.fight_time.to_bits(),
                full.berserk_value,
            );
            if let Some(expected) = reference {
                assert_eq!(result, expected);
            } else {
                reference = Some(result);
            }
            req.lite = true;
            let lite = fixture.simulate(&req);
            assert_eq!(full.fingerprint, lite.fingerprint);
            assert_eq!(
                result,
                (
                    lite.total_damage.to_bits(),
                    lite.dps.to_bits(),
                    lite.fight_time.to_bits(),
                    lite.berserk_value
                )
            );
        }
    }
}

#[test]
fn energy_macro_casts_from_full_tiegu_block_with_or_without_jianren() {
    for version in [GameVersion::ShanHaiYuanLiu, GameVersion::AnYingQianJi] {
        let mount = Mount::TieGuYi;
        let (constants, _, _, _, _) = load_school_toml(version, mount).unwrap();
        let skills = load_skills(Path::new(&skills_dir(version, mount)));
        let recipes = load_recipes(Path::new(&recipes_file(version)));
        let team_buffs = load_team_buffs(Path::new(&team_buffs_file(version)));
        let formations = load_formations(Path::new(&formations_file(version)));
        for (talents, energy) in [(vec![], 100), (vec![13363], 200)] {
            let mut req = request(vec!["__macro__".to_string()], talents);
            req.macro_duration = Some(1.0);
            req.macro_text = Some(format!("/cast [energy={energy}&rage=0&baonu=0] 盾刀"));
            let response = simulate_core(
                &req,
                &skills,
                version,
                mount,
                constants,
                &recipes,
                &team_buffs,
                &formations,
            );
            let casts = response
                .timeline
                .iter()
                .filter(|event| !event.triggered)
                .collect::<Vec<_>>();
            assert_eq!(casts.len(), 1, "{version:?}, energy={energy}");
            assert_eq!(casts[0].skill_id, 13044);
            assert_eq!(
                casts[0].state_before.as_ref().unwrap().block_value,
                Some(energy)
            );
            assert!(response.berserk_value.is_none());
        }
    }
}


#[test]
fn september11_reset_modes_replay_identically_in_full_and_lite() {
    use crate::shield_reset::ResetMode;
    let fixture = Fixture::load();
    for mode in [ResetMode::Cumulative, ResetMode::Random] {
        let mut req = request(vec!["__macro__".into(); 200], vec![ZHEN_YUN, SHEN_WEI, 21281]);
        req.macro_text = Some("/cast 盾压\n/cast 盾刀".into());
        req.macro_duration = Some(60.0);
        req.recipes = vec![4005, 4006, 4007, 4008];
        req.hanjia_expectation = Some(mode == ResetMode::Cumulative);
        req.dunya_reset_seed = 17;
        let full = fixture.simulate(&req);
        assert!(full.timeline.iter().filter(|e| !e.triggered && e.skill_id == 13045).count() > 8);
        let replay = fixture.simulate(&req);
        assert_eq!(full.fingerprint, replay.fingerprint);
        req.lite = true;
        let lite = fixture.simulate(&req);
        assert_eq!(full.fingerprint, lite.fingerprint);
        assert_eq!(full.total_damage.to_bits(), lite.total_damage.to_bits());
        assert_eq!(full.fight_time.to_bits(), lite.fight_time.to_bits());
        if mode == ResetMode::Random {
            req.dunya_reset_seed = 18;
            assert_ne!(full.fingerprint, fixture.simulate(&req).fingerprint);
        }
    }
}
