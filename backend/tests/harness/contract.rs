use super::*;

fn request() -> (AgentRuntime, MacroCompileRequestV1) {
    let runtime = AgentRuntime::fixture();
    let scenario = runtime.fixture_scenario();
    let request = serde_json::from_value(json!({"simulation":scenario.simulation,
        "version":"AnYingQianJi","mount":"FenShanJin"}))
    .unwrap();
    (runtime, request)
}

#[test]
fn contract_preserves_full_environment_and_normalizes_transport_flags() {
    let (runtime, mut request) = request();
    request.simulation.pre_releases = vec![crate::PreReleaseSpec {
        skill: "血怒".into(),
        time_before: 1.0,
    }];
    request.simulation.pauses = vec![(8.0, 2.0)];
    request
        .simulation
        .equipment
        .insert("PRIMARY_WEAPON".into(), 123);
    request.simulation.dunya_reset_seed = 73;
    request.simulation.lite = true;
    let snapshot = request.snapshot(&runtime).unwrap();
    assert!(!snapshot.simulation.lite);
    assert_eq!(snapshot.simulation.pre_releases[0].skill, "血怒");
    assert_eq!(snapshot.simulation.pauses, vec![(8.0, 2.0)]);
    assert_eq!(snapshot.simulation.equipment["PRIMARY_WEAPON"], 123);
    assert_eq!(snapshot.simulation.dunya_reset_seed, 73);
    let first = experiment_hash(&request, &snapshot, "runtime-a").unwrap();
    request.simulation.lite = false;
    assert_eq!(
        first,
        experiment_hash(&request, &request.snapshot(&runtime).unwrap(), "runtime-a").unwrap()
    );
    request.max_pages += 1;
    assert_ne!(
        first,
        experiment_hash(&request, &snapshot, "runtime-a").unwrap()
    );
    assert_ne!(
        first,
        experiment_hash(&request, &snapshot, "runtime-b").unwrap()
    );
}

#[test]
fn contract_rejects_bad_limits_nonfinite_attributes_and_wrong_runtime() {
    let (runtime, request) = request();
    assert!(request.snapshot(&runtime).is_ok());
    let mut invalid = request.clone();
    invalid.max_simulations = 0;
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.max_simulations = 257;
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.wall_time_ms = 120_001;
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.time_tolerance = f64::NAN;
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.simulation.attributes.as_mut().unwrap().base_attack = f64::NAN;
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.simulation.macro_text = Some("/cast 盾击".into());
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.simulation.sequence = vec!["__macro__".into()];
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.mount = Mount::TieGuYi;
    assert!(invalid.snapshot(&runtime).is_err());
    invalid = request;
    invalid.initial_macro = Some("/cast [rage???] 盾击".into());
    assert!(invalid.validate().is_err());
}

#[test]
fn runtime_identity_is_stable_and_includes_compiled_engine() {
    let runtime = AgentRuntime::fixture();
    let hash = runtime_hash(&runtime).unwrap();
    assert_eq!(hash, runtime_hash(&runtime).unwrap());
    assert_eq!(executable_hash().len(), 64);
    assert_ne!(
        hash,
        runtime_hash(&AgentRuntime::fixture_for(
            GameVersion::ShanHaiYuanLiu,
            Mount::FenShanJin
        ))
        .unwrap()
    );
}

#[test]
fn contract_bounds_boss_and_nested_team_buff_event_density() {
    let (_, mut request) = request();
    request.simulation.boss_attack_interval = Some(1e-300);
    assert!(request.validate().is_err());
    request.simulation.boss_attack_interval = Some(1.0 / 16.0);
    assert!(request.validate().is_ok());
    request.simulation.macro_duration = Some(1e12);
    assert!(request.validate().is_err());
    request.simulation.macro_duration = Some(f64::NAN);
    assert!(request.validate().is_err());
    request.simulation.macro_duration = Some(1200.0);
    assert!(request.validate().is_ok());
    let buff: crate::TeamBuffSelection = serde_json::from_value(json!({
        "key":"test", "enabled":true, "stacks":1,
        "first_release":0.0, "period":30.0, "duration":10.0,
        "release_times":[0.0, 30.0]
    }))
    .unwrap();
    request.simulation.team_buffs = vec![buff.clone()];
    assert!(request.validate().is_ok());
    request.simulation.team_buffs[0].period = 1e-300;
    assert!(request.validate().is_err());
    request.simulation.team_buffs[0] = buff.clone();
    request.simulation.team_buffs[0].duration = f64::INFINITY;
    assert!(request.validate().is_err());
    request.simulation.team_buffs[0] = buff.clone();
    request.simulation.team_buffs[0].release_times = Some(vec![0.0; 2049]);
    assert!(request.validate().is_err());
    request.simulation.team_buffs[0] = buff.clone();
    request.simulation.team_buffs[0].release_times = Some(vec![f64::NAN]);
    assert!(request.validate().is_err());
    request.simulation.team_buffs = vec![buff; 5];
    for buff in &mut request.simulation.team_buffs {
        buff.release_times = Some(vec![0.0; 2048]);
    }
    assert!(request.validate().is_err());
}
