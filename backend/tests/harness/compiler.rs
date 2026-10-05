use super::*;
use crate::{GameVersion, Mount};

fn fixture(text: &str) -> (MacroCompileRequestV1, AgentRuntime, ScenarioSnapshotV1) {
    let fixture: Value =
        serde_json::from_str(include_str!("../agent_diagnostic_eval/scenario.json")).unwrap();
    let mut simulation: SimulateRequest =
        serde_json::from_value(fixture["simulation"].clone()).unwrap();
    simulation.sequence = vec!["盾刀".into(); 6];
    simulation.macro_text = None;
    simulation.macro_duration = None;
    simulation.network_delay = 0;
    simulation.talents.clear();
    simulation.recipes.clear();
    let request = MacroCompileRequestV1 {
        simulation,
        version: GameVersion::AnYingQianJi,
        mount: Mount::FenShanJin,
        initial_macro: Some(text.into()),
        max_simulations: 32,
        wall_time_ms: 60_000,
        max_rounds: 3,
        max_pages: 2,
        time_tolerance: 1.0 / 16.0,
    };
    let runtime = AgentRuntime::fixture();
    let scenario =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .unwrap();
    (request, runtime, scenario)
}

#[test]
fn exact_macro_is_verified_by_full_frozen_replay() {
    let (request, runtime, scenario) = fixture("/cast 盾刀");
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.stop_reason, "target_reproduced");
    assert_eq!(result.simulations, 2);
    let best = result.best.unwrap();
    assert!(best.verified && best.reproduced && best.page_constraints_passed);
    let replay = run(
        &runtime,
        &macro_request(
            &scenario.simulation,
            &best.macro_text,
            result.window_seconds,
        ),
    );
    assert_eq!(best.fingerprint, replay.fingerprint.to_string());
    assert!(replay
        .timeline
        .iter()
        .filter(|e| alignment::is_active(e))
        .all(|e| e.state_before.is_some()));
}

#[test]
fn blocked_condition_is_diagnosed_and_repaired_with_a_new_replay() {
    let (request, runtime, scenario) = fixture("/cast [rage>100] 盾刀");
    let mut phases = Vec::new();
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |p| phases.push(p.phase),
    )
    .unwrap();
    assert_eq!(result.stop_reason, "target_reproduced");
    assert!(
        result.simulations >= 4,
        "baseline, initial, diagnosis and repaired replay are all charged"
    );
    assert!(phases.iter().any(|p| p == "repair"));
    assert!(result.best.unwrap().reproduced);
    assert!(result
        .history
        .iter()
        .any(|t| t.origin == "unconditional_contrast" && t.accepted));
}

#[test]
fn simulation_budget_keeps_only_the_tested_candidate() {
    let (mut request, runtime, scenario) = fixture("/cast [rage>100] 盾刀");
    request.max_simulations = 2;
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.stop_reason, "budget_exhausted");
    assert_eq!(result.simulations, 2);
    assert_eq!(result.history.len(), 1);
    let best = result.best.unwrap();
    assert!(best.verified && !best.reproduced);
    assert!(
        best.diagnosis.is_none(),
        "diagnostic replay cannot bypass the budget"
    );
}

#[test]
fn cancelled_before_baseline_does_not_simulate() {
    let (request, runtime, scenario) = fixture("/cast 盾刀");
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(true),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.stop_reason, "cancelled");
    assert_eq!(result.simulations, 0);
    assert!(result.best.is_none() && result.baseline.is_none());
}

#[test]
fn cancellation_after_candidate_preserves_verified_result() {
    let (request, runtime, scenario) = fixture("/cast [rage>100] 盾刀");
    let cancelled = AtomicBool::new(false);
    let result = compile(&request, &runtime, &scenario, &cancelled, |p| {
        if p.phase == "candidate" {
            cancelled.store(true, Ordering::Relaxed);
        }
    })
    .unwrap();
    assert_eq!(result.stop_reason, "cancelled");
    assert_eq!(result.simulations, 2);
    assert!(result.best.unwrap().verified);
}

#[test]
fn page_overflow_can_never_be_reported_as_reproduced() {
    let (request, runtime, scenario) = fixture(&format!("/cast 盾刀\n//{}", "a".repeat(140)));
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let best = result.best.unwrap();
    assert_eq!(
        best.alignment.summary.missing
            + best.alignment.summary.extra
            + best.alignment.summary.changed,
        0
    );
    assert!(!best.page_constraints_passed && !best.reproduced);
    assert_ne!(result.stop_reason, "target_reproduced");
}

#[test]
fn macro_request_preserves_environment_and_only_replaces_axis_fields() {
    let (mut request, _, _) = fixture("/cast 盾刀");
    request.simulation.pre_releases.push(crate::PreReleaseSpec {
        skill: "血怒".into(),
        time_before: 2.0,
    });
    request.simulation.pauses = vec![(3.0, 1.0)];
    request.simulation.channel_ticks.insert("0".into(), 1);
    request.simulation.timing_offsets.insert("0".into(), 0.4);
    request.simulation.qijin_buffs.insert("0".into(), 123);
    let actual = macro_request(&request.simulation, "/cast 盾刀", 8.0);
    let mut expected = request.simulation;
    expected.sequence = actual.sequence.clone();
    expected.macro_text = Some("/cast 盾刀".into());
    expected.macro_duration = Some(8.0);
    expected.channel_ticks.clear();
    expected.timing_offsets.clear();
    expected.qijin_buffs.clear();
    expected.lite = false;
    expected.lite_keep_timeline = false;
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn skipped_manual_actions_are_rejected_instead_of_silently_removed() {
    let (mut request, runtime, _) = fixture("/cast 盾刀");
    request.simulation.sequence = vec!["绝刀".into()];
    let scenario =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .unwrap();
    let error = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .err()
    .unwrap();
    assert!(error.contains("未成功释放"), "{error}");
}

#[test]
fn runtime_version_and_mount_must_match_the_frozen_scene() {
    let (mut request, runtime, scenario) = fixture("/cast 盾刀");
    request.mount = Mount::TieGuYi;
    let error = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .err()
    .unwrap();
    assert!(error.contains("版本或心法"));
}

#[test]
fn final_target_cast_is_included_in_the_shared_window() {
    let (request, runtime, scenario) = fixture("/cast 盾刀");
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let baseline = run(&runtime, &scenario.simulation);
    let last_cast = baseline
        .timeline
        .iter()
        .filter(|e| alignment::is_active(e))
        .last()
        .unwrap();
    assert!(result.window_seconds > last_cast.cast_time);
    assert_eq!(result.baseline.as_ref().unwrap().active_casts, 6);
    let mut missing_tail = baseline.clone();
    missing_tail
        .timeline
        .retain(|e| e.cast_time < last_cast.cast_time);
    let candidate = make_candidate(
        "/cast 盾刀".into(),
        "missing_tail_fixture".into(),
        &missing_tail,
        &baseline,
        result.window_seconds,
        &request,
    )
    .unwrap();
    assert_eq!(candidate.alignment.summary.missing, 1);
    assert!(!candidate.reproduced);
}

#[test]
fn manual_debug_actions_are_explicitly_unsupported() {
    let (mut request, runtime, _) = fixture("/cast 盾刀");
    request.simulation.sequence.push("清除冷却".into());
    let scenario =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .unwrap();
    let error = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .err()
    .unwrap();
    assert!(error.contains("不支持"));
}

#[test]
fn missing_full_state_can_never_pass_reproduction() {
    let (request, runtime, scenario) = fixture("/cast 盾刀");
    let baseline = run(&runtime, &scenario.simulation);
    let window = baseline.fight_time + 1.0 / 16.0;
    let mut response = baseline.clone();
    for event in &mut response.timeline {
        event.state_before = None;
    }
    let candidate = make_candidate(
        "/cast 盾刀".into(),
        "missing_state_fixture".into(),
        &response,
        &baseline,
        window,
        &request,
    )
    .unwrap();
    assert!(!candidate.full_snapshots && !candidate.reproduced);
}

#[test]
fn empty_initial_macro_uses_generator_then_full_replay() {
    for initial in [None, Some("  \n ".into())] {
        let (mut request, runtime, scenario) = fixture("/cast 盾刀");
        request.initial_macro = initial;
        let result = compile(
            &request,
            &runtime,
            &scenario,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let best = result.best.unwrap();
        assert!(best.verified && best.reproduced, "{}", best.macro_text);
        assert_eq!(best.reference_actions.len(), 6);
        assert_eq!(best.actual_actions.len(), 6);
        assert!(best
            .reference_actions
            .iter()
            .all(|e| e.name.starts_with("盾刀")));
    }
}

#[test]
fn actual_runtime_matrix_is_used_for_full_candidate_replays() {
    for version in [
        GameVersion::ShanHaiYuanLiu,
        GameVersion::AnYingQianJi,
        GameVersion::CangShengZhuShiTest,
    ] {
        for mount in [Mount::FenShanJin, Mount::TieGuYi] {
            let (mut request, _, _) = fixture("/cast 盾刀");
            request.version = version;
            request.mount = mount;
            request.simulation.equipment.clear();
            let runtime = AgentRuntime::fixture_for(version, mount);
            let scenario =
                ScenarioSnapshotV1::capture(version, mount, request.simulation.clone()).unwrap();
            let result = compile(
                &request,
                &runtime,
                &scenario,
                &AtomicBool::new(false),
                |_| {},
            )
            .unwrap();
            let best = result.best.unwrap();
            assert!(best.reproduced, "{version:?}/{mount:?}");
            let response = run(
                &runtime,
                &macro_request(
                    &scenario.simulation,
                    &best.macro_text,
                    result.window_seconds,
                ),
            );
            assert_eq!(best.fingerprint, response.fingerprint.to_string());
        }
    }
}

#[test]
fn budget_expiry_after_baseline_does_not_create_untested_macro() {
    let (mut request, runtime, scenario) = fixture("/cast 盾刀");
    request.max_simulations = 1;
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(result.stop_reason, "budget_exhausted");
    assert_eq!(result.simulations, 1);
    assert!(result.baseline.is_some() && result.best.is_none());
}

#[test]
fn actual_false_casts_generate_thresholds_that_are_replayed_not_assumed() {
    let text = "/cast [skill_energy:血怒>0] 血怒\n/cast [rage>=0] 盾刀";
    let (mut request, runtime, _) = fixture(text);
    request.simulation.sequence.insert(0, "血怒".into());
    let scenario =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .unwrap();
    let baseline = run(&runtime, &scenario.simulation);
    let window = baseline.fight_time + 1.0 / 16.0;
    let actual = run(&runtime, &macro_request(&scenario.simulation, text, window));
    let mut best = make_candidate(
        text.into(),
        "fixture".into(),
        &actual,
        &baseline,
        window,
        &request,
    )
    .unwrap();
    assert!(
        best.alignment.summary.extra > 0,
        "fixture must contain real false blood-rage casts"
    );
    best.diagnosis = Some(
        diagnose(
            &runtime,
            &scenario.simulation,
            &baseline,
            &actual,
            &best,
            window,
        )
        .unwrap(),
    );
    let proposals = repair::proposals(&best, &baseline, &actual, window, || false);
    for proposal in &proposals {
        assert_eq!(
            proposal.text,
            repair::game_macro_text(&proposal.text).unwrap(),
            "every proposal family must use export syntax"
        );
    }
    let thresholds = proposals
        .iter()
        .filter(|p| p.origin == "counterexample_threshold")
        .collect::<Vec<_>>();
    assert!(!thresholds.is_empty() && thresholds.len() <= 8);
    assert!(
        proposals
            .iter()
            .filter(|p| p.origin == "joint_counterexample_thresholds")
            .count()
            <= 4
    );
    assert!(
        thresholds.into_iter().any(|proposal| {
            let response = run(
                &runtime,
                &macro_request(&scenario.simulation, &proposal.text, window),
            );
            let measured = make_candidate(
                proposal.text.clone(),
                proposal.origin.clone(),
                &response,
                &baseline,
                window,
                &request,
            )
            .unwrap();
            measured.alignment.summary.extra < best.alignment.summary.extra
        }),
        "at least one threshold must remove a real false cast in full replay"
    );
}

#[test]
fn successful_threshold_repair_exports_bracketed_game_syntax_and_replays_that_text() {
    let (mut request, runtime, _) = fixture("/cast [rage>100] 盾刀");
    request.simulation.initial_rage = Some(50);
    let scenario =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .unwrap();
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let best = result.best.unwrap();
    assert!(best.reproduced);
    assert_eq!(best.macro_text.trim(), "/cast [rage>49] 盾刀");
    let actual = run(
        &runtime,
        &macro_request(
            &scenario.simulation,
            &best.macro_text,
            result.window_seconds,
        ),
    );
    assert_eq!(best.fingerprint, actual.fingerprint.to_string());
    assert_eq!(
        best.pages[0].chars,
        "/cast [rage>49] 盾刀".encode_utf16().count()
    );
}

#[test]
fn unbracketed_user_seed_is_normalized_before_validation() {
    let (mut request, runtime, _) = fixture("/cast rage>49 盾刀");
    request.simulation.initial_rage = Some(50);
    let scenario =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .unwrap();
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let best = result.best.unwrap();
    assert!(best.reproduced);
    assert_eq!(best.macro_text, "/cast [rage>49] 盾刀");
    assert_eq!(result.simulations, 2);
}

#[test]
fn bracket_characters_cannot_bypass_page_limit_and_separators_are_preserved() {
    let raw = "/cast rage>49 盾刀";
    let padding = 128 - raw.encode_utf16().count() - 3; // newline plus comment prefix
    let text = format!("{raw}\n//{}", "x".repeat(padding));
    assert_eq!(pages(&text)[0].chars, 128);
    let (mut request, runtime, _) = fixture(&text);
    request.max_simulations = 2;
    request.simulation.initial_rage = Some(50);
    let scenario =
        ScenarioSnapshotV1::capture(request.version, request.mount, request.simulation.clone())
            .unwrap();
    let result = compile(
        &request,
        &runtime,
        &scenario,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let best = result.best.unwrap();
    assert_eq!(best.pages[0].chars, 130);
    assert!(!best.page_constraints_passed && !best.reproduced);
    let multi = "// preserved\n#page shield\n/fcast rage>49 盾刀\n#page\n/cast [rage>49&buff:血怒|nobuff:盾飞] 盾刀\n";
    let normalized = repair::game_macro_text(multi).unwrap();
    assert_eq!(normalized, "// preserved\n#page shield\n/fcast [rage>49] 盾刀\n#page\n/cast [rage>49&buff:血怒|nobuff:盾飞] 盾刀\n");
    assert_eq!(
        crate::macro_parser::parse_macro_text(&normalized)
            .unwrap()
            .pages
            .len(),
        2
    );
}
