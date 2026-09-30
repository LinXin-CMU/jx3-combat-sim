use super::*;
use crate::*;

fn replay(text: &str, version: GameVersion, window: (f64, f64)) -> (SimulateResponse, Collector) {
    replay_target(text, version, window, None)
}

fn replay_target(text: &str, version: GameVersion, window: (f64, f64), target: Option<&str>) -> (SimulateResponse, Collector) {
    let mount = Mount::FenShanJin;
    let (constants, _, _, _, _) = load_school_toml(version, mount).unwrap();
    let skills = load_skills(std::path::Path::new(&skills_dir(version, mount)));
    let req: SimulateRequest = serde_json::from_value(serde_json::json!({
        "haste_level":0,"sequence":vec!["__macro__";100],"macro_text":text,
        "macro_duration":8.0,"network_delay":100
    })).unwrap();
    let normal = simulate_core(&req, &skills, version, mount, constants, &[], &[], &[]);
    let mut trace = Collector::new(window.0, window.1);
    trace.target_skill = target.map(str::to_owned);
    let traced = simulate_core_with_trace(&req, &skills, version, mount, constants, &[], &[], &[], Some(&mut trace));
    assert_eq!(normal.fingerprint, traced.fingerprint);
    assert_eq!(normal.dps, traced.dps);
    assert_eq!(serde_json::to_value(&normal.timeline).unwrap(), serde_json::to_value(&traced.timeline).unwrap());
    assert_eq!(serde_json::to_value(&normal.macro_line_stats).unwrap(), serde_json::to_value(&traced.macro_line_stats).unwrap());
    (traced, trace)
}

#[test]
fn diagnostic_target_proves_priority_without_changing_short_circuit() {
    for version in [GameVersion::AnYingQianJi, GameVersion::CangShengZhuShiTest] {
        let (_, trace) = replay_target("/cast 盾刀\n/cast 盾压", version, (0.0, 8.0), Some("盾压"));
        let first = &trace.decisions[0];
        assert_eq!(first.phase2.len(), 1, "the real executor still short circuits before target");
        let target = first.target.as_ref().unwrap();
        assert_eq!(target.lines.len(), 1);
        assert!(target.lines[0].passed && target.lines[0].probe.castable);
        assert_eq!(target.lines[0].line, 2);
        assert_eq!(first.selected_line, Some(1));
    }
}

#[test]
fn diagnostic_target_distinguishes_condition_failures_and_real_values() {
    let (_, trace) = replay_target("/cast [rage>100&bufftime:盾飞<1|sun>99] 盾压\n/cast 盾刀", GameVersion::CangShengZhuShiTest, (0.0, 8.0), Some("盾压"));
    let line = &trace.decisions[0].target.as_ref().unwrap().lines[0];
    assert!(!line.passed);
    assert!(line.condition.contains("AND") && line.condition.contains("OR"));
    assert_eq!(line.atoms.len(), 3);
    assert_eq!(line.atoms[0].actual, "怒气 0");
    assert!(!line.atoms[0].passed);
    assert_eq!(line.atoms[1].actual, "盾飞 不存在");
    assert!(!line.atoms[1].passed, "absent bufftime is not a zero-second buff");
    assert!(line.probe.castable, "changing Step 1 may admit a currently ready target");
}

#[test]
fn diagnostic_target_distinguishes_inactive_page_and_cooldown() {
    let (_, trace) = replay_target("#page shield\n/cast 盾刀\n#page blade\n/cast 盾压", GameVersion::AnYingQianJi, (0.0, 8.0), Some("盾压"));
    let target = trace.decisions[0].target.as_ref().unwrap();
    assert!(target.lines.is_empty());
    assert_eq!(target.other_pages, vec![2]);
    assert!(target.probe.castable);
    let (_, trace) = replay_target("/cast 盾压\n/cast 盾刀", GameVersion::AnYingQianJi, (0.0, 8.0), Some("盾压"));
    assert!(trace.decisions.iter().any(|step| step.target.as_ref().is_some_and(|target| target.lines[0].passed && target.lines[0].probe.reason.starts_with("CD/GCD未就绪"))));
}

#[test]
fn diagnostic_target_reports_actual_stance_resource_and_combo_restrictions() {
    for version in [GameVersion::AnYingQianJi, GameVersion::CangShengZhuShiTest] {
        for target in ["血怒", "阵云结晦", "月照连营", "雁门迢递", "斩刀", "绝刀"] {
            let (_, trace) = replay_target("/cast 盾刀", version, (0.0, 8.0), Some(target));
            assert!(trace.decisions[0].target.is_some());
            if target == "斩刀" { assert!(trace.decisions[0].target.as_ref().unwrap().probe.reason.contains("擎刀")); }
        }
    }
}

#[test]
fn diagnostic_observes_real_two_phase_short_circuit_and_cast_mapping() {
    for version in [GameVersion::AnYingQianJi, GameVersion::CangShengZhuShiTest] {
        let (result, trace) = replay("#page shield\n/cast [rage>100] 血怒\n/cast 不存在\n/cast 盾刀\n/cast 盾压", version, (0.0, 8.0));
        let first = &trace.decisions[0];
        assert_eq!(first.phase1.len(), 4);
        assert!(!first.phase1[0].passed);
        assert!(first.phase1[3].passed, "Step 1 scans later lines even if an earlier skill can cast");
        assert_eq!(first.phase2.len(), 2);
        assert_eq!(first.phase2[0].reason, "未知技能");
        assert!(!first.phase2[0].castable);
        assert_eq!(first.selected_line, Some(3));
        assert_eq!(first.cast_success, Some(true));
        assert!(first.phase2[1].castable);
        assert!(trace.decisions.iter().any(|step| step.selected.is_none() && !step.phase2.is_empty()), "waiting rounds must remain visible");
        for step in trace.decisions.iter().filter(|step| step.cast_success == Some(true)) {
            assert!(result.timeline.iter().any(|event| !event.triggered && event.macro_page == Some(step.page) && event.macro_line == step.selected_line && Some(event.cast_time) == step.cast_time));
            assert!(step.cast_time.unwrap() >= step.time);
        }
    }
}

#[test]
fn diagnostic_keeps_empty_pool_and_restricts_observation_window() {
    let (_, empty) = replay("/cast [rage>100] 盾刀", GameVersion::AnYingQianJi, (0.0, 8.0));
    assert!(!empty.decisions.is_empty());
    assert!(empty.decisions.iter().all(|step| step.phase2.is_empty() && step.selected.is_none() && step.cast_success.is_none()));
    let (_, trace) = replay("/cast 盾刀", GameVersion::AnYingQianJi, (1.0, 3.0));
    assert!(!trace.decisions.is_empty());
    assert!(trace.decisions.iter().all(|step| (1.0..=3.0).contains(&step.time)));
}

#[test]
fn diagnostic_collectors_are_bounded_and_do_not_leak_between_runs() {
    let player = Player::new(0, vec![], vec![]);
    let mut collector = Collector::new(0.0, 1.0);
    for _ in 0..150 { collector.record(&player, 0, None, vec![], vec![], None); }
    assert_eq!(collector.decisions.len(), 128);
    assert!(collector.truncated);
    let fresh = Collector::new(0.0, 1.0);
    assert!(fresh.decisions.is_empty() && !fresh.truncated);
    let mut collector = Collector::new(0.0, 1.0);
    for _ in 0..100 {
        let rows = (0..100).map(|line| Phase1LineDebug {line, skill:"盾刀".into(), condition:"无条件".into(), passed:true}).collect();
        collector.record(&player, 0, None, rows, vec![], None);
    }
    assert_eq!(collector.decisions.len(), 40);
    assert!(collector.truncated);
}

#[test]
fn diagnostic_rejects_invalid_and_unbounded_requests() {
    let mut request: Request = serde_json::from_value(serde_json::json!({
        "simulation":{"haste_level":0,"sequence":["__macro__"],"macro_text":"/cast 盾刀","macro_duration":8},
        "version":"AnYingQianJi","mount":"FenShanJin","start":0,"end":3
    })).unwrap();
    assert!(validate(&request).is_ok());
    request.end = 11.0; assert!(validate(&request).is_err());
    request.end = 3.0; request.start = f64::NAN; assert!(validate(&request).is_err());
    request.start = 0.0; request.simulation.lite = true; assert!(validate(&request).is_err());
    request.simulation.lite = false; request.simulation.macro_text = Some("/cast 盾刀\n".repeat(129)); assert!(validate(&request).is_err());
}
