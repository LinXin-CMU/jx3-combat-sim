use super::*;

fn fixture(text: &str) -> (RotationRequest, AgentRuntime, ScenarioSnapshotV1) {
    let data: Value =
        serde_json::from_str(include_str!("../agent_diagnostic_eval/scenario.json")).unwrap();
    let mut simulation: SimulateRequest =
        serde_json::from_value(data["simulation"].clone()).unwrap();
    simulation.sequence = vec!["盾刀".into(); 5];
    simulation.macro_text = None;
    simulation.macro_duration = None;
    simulation.talents.clear();
    simulation.recipes.clear();
    simulation.equipment.clear();
    simulation.network_delay = 0;
    let request = RotationRequest {
        simulation,
        version: GameVersion::AnYingQianJi,
        mount: Mount::FenShanJin,
        initial_macro: Some(text.into()),
        allowed_skills: vec!["盾刀".into()],
        max_simulations: 12,
        wall_time_ms: 60_000,
        max_rounds: 2,
        duration_seconds: 10.0,
        max_pages: 2,
    };
    let runtime = AgentRuntime::fixture();
    let scenario = request.snapshot(&runtime).unwrap();
    (request, runtime, scenario)
}

#[test]
fn blocked_seed_improves_through_full_replay_and_held_out_delay() {
    let (request, runtime, scenario) = fixture("/cast rage>100 盾刀");
    let result = run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert!(result["baseline"]["macro_text"]
        .as_str()
        .unwrap()
        .contains("[rage>100]"));
    assert_eq!(result["baseline"]["metrics"]["dps"], 0.0);
    assert!(result["best"]["metrics"]["dps"].as_f64().unwrap() > 0.0);
    assert_eq!(result["baseline"]["metrics"]["fight_time"], 10.0);
    assert_eq!(result["best"]["metrics"]["fight_time"], 10.0);
    assert_eq!(result["validation"]["status"], "completed");
    assert_eq!(result["validation"]["improved"], true);
    assert_eq!(result["validation"]["network_delay"], 50);
    assert!(result["simulations"].as_u64().unwrap() <= u64::from(request.max_simulations));
    assert!(result["history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["accepted"] == true));
    let sim: SimulateRequest =
        serde_json::from_value(result["best"]["simulation"].clone()).unwrap();
    let c = runtime.context();
    let replay = crate::simulate_core(
        &sim,
        c.skills,
        c.game_version,
        c.mount,
        c.constants,
        c.recipes,
        c.team_buffs,
        c.formations,
    );
    assert_eq!(
        result["best"]["fingerprint"],
        replay.fingerprint.to_string()
    );
    assert_eq!(
        result["best"]["scenario_hash"],
        ScenarioSnapshotV1::capture(request.version, request.mount, sim)
            .unwrap()
            .scenario_hash
    );
}

#[test]
fn two_replays_do_not_claim_completed_validation() {
    let (mut request, runtime, scenario) = fixture("/cast 盾刀");
    request.max_simulations = 2;
    let result = run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result["simulations"], 2);
    assert_eq!(result["stop_reason"], "budget_exhausted");
    assert_eq!(result["best"]["verified"], true);
    assert_ne!(result["validation"]["status"], "completed");
    assert_eq!(result["validation"]["improved"], false);
}

#[test]
fn cancellation_returns_last_tested_candidate_and_charges_no_more() {
    let (request, runtime, scenario) = fixture("/cast 盾刀");
    let cancel = AtomicBool::new(false);
    let result = run(&request, &runtime, &scenario, &cancel, |p| {
        if p["phase"] == "baseline" {
            cancel.store(true, Ordering::Relaxed);
        }
    })
    .unwrap();
    assert_eq!(result["stop_reason"], "cancelled");
    assert_eq!(result["simulations"], 1);
    assert_eq!(result["best"]["verified"], true);
    assert_eq!(result["validation"]["improved"], false);
    let already_cancelled = run(&request, &runtime, &scenario, &cancel, |_| {}).unwrap();
    assert_eq!(already_cancelled["simulations"], 0);
    assert!(already_cancelled["best"].is_null());
}

#[test]
fn full_environment_survives_macro_conversion_and_snapshot_binding() {
    let (mut request, runtime, _) = fixture("/cast 盾刀");
    request.simulation.dunya_reset_seed = 79;
    request.simulation.initial_rage = Some(45);
    request.simulation.network_delay = 33;
    request.simulation.pauses = vec![(2.0, 0.5)];
    request.simulation.channel_ticks.insert("盾刀".into(), 1);
    request.simulation.timing_offsets.insert("0".into(), 0.25);
    request.simulation.qijin_buffs.insert("0".into(), 1);
    let scenario = request.snapshot(&runtime).unwrap();
    let result = run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let actual = &result["best"]["simulation"];
    let original = serde_json::to_value(&scenario.simulation).unwrap();
    for key in [
        "attributes",
        "target",
        "equipment",
        "team_buffs",
        "formation",
        "pre_releases",
        "network_delay",
        "initial_rage",
        "dunya_reset_seed",
        "pauses",
        "boss_attack_interval",
        "hanjia_expectation",
        "tiegu_mode",
        "experimental",
        "talents",
        "recipes",
        "haste_level",
    ] {
        assert_eq!(actual[key], original[key], "lost environment field {key}");
    }
    assert_eq!(actual["channel_ticks"], json!({}));
    assert_eq!(actual["timing_offsets"], json!({}));
    assert_eq!(actual["qijin_buffs"], json!({}));
    request.simulation.network_delay += 1;
    assert!(run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {}
    )
    .is_err());
}

#[test]
fn all_three_versions_and_both_mounts_replay_their_own_runtime() {
    for version in [
        GameVersion::ShanHaiYuanLiu,
        GameVersion::AnYingQianJi,
        GameVersion::CangShengZhuShiTest,
    ] {
        for mount in [Mount::FenShanJin, Mount::TieGuYi] {
            let (mut request, _, _) = fixture("/cast 盾刀");
            request.version = version;
            request.mount = mount;
            request.max_simulations = 3;
            let runtime = AgentRuntime::fixture_for(version, mount);
            let scenario = request.snapshot(&runtime).unwrap();
            let result = run(
                &request,
                &runtime,
                &scenario,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            assert_eq!(result["best"]["verified"], true, "{version:?}/{mount:?}");
            assert_eq!(result["best"]["metrics"]["fight_time"], 10.0);
            assert_eq!(result["validation"]["status"], "completed");
            assert_eq!(result["simulations"], 3);
        }
    }
}

#[test]
fn oversized_seed_is_not_returned_as_executable() {
    let (mut request, runtime, scenario) = fixture(&format!("//{}\n/cast 盾刀", "x".repeat(128)));
    request.max_simulations = 2;
    let result = run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result["baseline"]["page_constraints_passed"], false);
    assert!(result["best"].is_null());
    assert_eq!(result["validation"]["improved"], false);
}

#[test]
fn proposals_export_game_brackets_and_bidirectional_thresholds() {
    let generated = proposals(
        "/cast [rage>49] 盾刀",
        &BTreeSet::from(["盾刀".into()]),
        None,
    );
    assert!(generated.iter().any(|(m, _)| m.contains("[rage>48]")));
    assert!(generated.iter().any(|(m, _)| m.contains("[rage>50]")));
    for (text, _) in generated {
        let parsed = crate::macro_parser::parse_macro_text(&text).unwrap();
        if parsed
            .pages
            .iter()
            .flat_map(|p| &p.lines)
            .any(|l| l.condition.is_some())
        {
            assert!(text.contains('['), "{text}");
        }
    }
}

#[test]
fn allowed_skills_are_a_hard_bound_and_validate_runtime_eligibility() {
    let (mut request, runtime, scenario) = fixture("/cast 盾刀");
    request.allowed_skills = vec!["并不存在的技能".into()];
    assert!(run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {}
    )
    .is_err());
    let (request, runtime, scenario) = fixture("/cast 盾猛\n/cast 盾刀");
    let result = run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result["baseline"]["allowed_skills_passed"], false);
    assert_eq!(result["best"]["allowed_skills_passed"], true);
    assert!(!result["best"]["macro_text"]
        .as_str()
        .unwrap()
        .contains("盾猛"));
}

#[test]
fn manual_seed_is_charged_separately_and_compared_as_a_fixed_duration_macro() {
    let (mut request, runtime, _) = fixture("/cast 盾刀");
    request.initial_macro = None;
    request.max_simulations = 4;
    let scenario = request.snapshot(&runtime).unwrap();
    let result = run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result["simulations"], 4);
    assert_eq!(result["baseline"]["metrics"]["fight_time"], 10.0);
    assert_eq!(result["validation"]["status"], "completed");
}

#[test]
fn request_defaults_and_density_limits_are_enforced() {
    let (request, _, _) = fixture("/cast 盾刀");
    let mut value = serde_json::to_value(&request).unwrap();
    for key in [
        "max_simulations",
        "wall_time_ms",
        "max_rounds",
        "duration_seconds",
        "max_pages",
        "allowed_skills",
    ] {
        value.as_object_mut().unwrap().remove(key);
    }
    let defaults: RotationRequest = serde_json::from_value(value).unwrap();
    assert_eq!(defaults.max_simulations, 96);
    assert_eq!(defaults.duration_seconds, 120.0);
    assert_eq!(defaults.max_pages, 2);
    for duration in [0.0, 601.0, f64::INFINITY, f64::NAN] {
        let mut invalid = request.clone();
        invalid.duration_seconds = duration;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = request.clone();
    invalid.simulation.boss_attack_interval = Some(0.00001);
    assert!(invalid.validate().is_err());
    let mut invalid = request.clone();
    invalid.max_simulations = 257;
    assert!(invalid.validate().is_err());
    let mut invalid = request;
    invalid.initial_macro = None;
    invalid.simulation.sequence = vec!["__clearCD__:盾刀".into()];
    assert!(invalid.validate().is_err());
}

#[test]
fn failed_manual_seed_keeps_its_simulation_charge() {
    let (mut request, runtime, _) = fixture("/cast 盾刀");
    request.initial_macro = None;
    request.simulation.sequence = vec!["不存在的动作".into()];
    let scenario = request.snapshot(&runtime).unwrap();
    let result = run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result["simulations"], 1);
    assert!(result["baseline"].is_null());
    assert!(result["best"].is_null());
    assert_eq!(result["validation"]["improved"], false);
}

#[test]
fn uncastable_seed_is_observed_but_never_an_applicable_best() {
    let (mut request, runtime, scenario) = fixture("/cast [rage>100] 盾刀");
    request.max_simulations = 3;
    let result = run(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result["baseline"]["verified"], false);
    assert_eq!(result["baseline"]["metrics"]["active_casts"], 0);
    assert!(result["best"].is_null());
    assert_eq!(result["validation"]["improved"], false);
}
