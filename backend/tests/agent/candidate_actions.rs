use super::*;
use crate::agent::AgentRuntime;

#[test]
fn fixed_tier_replacement_cannot_keep_the_old_choice_but_can_replace_it() {
    let runtime = AgentRuntime::fixture_for(crate::GameVersion::CangShengZhuShiTest, crate::Mount::TieGuYi);
    let mut request = runtime.fixture_scenario().simulation;
    request.talents = vec![15072];
    let baseline = ScenarioSnapshotV1::capture(runtime.game_version(), runtime.mount(), request).unwrap();
    let candidate = |talents| CandidatePatchV1 { label:"替换第一重".into(), patch:ScenarioPatchV1 {talents:Some(talents), ..Default::default()} };
    let mut budget = ToolBudget::new(4);
    let result = compare_scenarios("conflicting-tier", &baseline, &[candidate(vec![15072,25213])],
        &runtime.context(), runtime.provenance(), &mut budget);
    assert!(matches!(result, Err(ToolError::ConflictingCandidateTalents)));
    assert_eq!(budget.used_simulations,0);
    assert!(compare_scenarios("replace-tier", &baseline, &[candidate(vec![25213])],
        &runtime.context(), runtime.provenance(), &mut budget).is_ok());
    assert_eq!(budget.used_simulations,2);
}

#[test]
fn mixed_pool_cannot_exceed_three_choices() {
    let runtime = AgentRuntime::fixture();
    let talents = runtime.context().talents.iter().filter(|talent| talent.tier == 8).take(4).map(|talent|talent.id).collect::<Vec<_>>();
    assert_eq!(talents.len(),4);
    let mut budget = ToolBudget::new(4);
    let result = compare_scenarios("mixed-capacity", &runtime.fixture_scenario(), &[CandidatePatchV1 {
        label:"混池超额".into(),patch:ScenarioPatchV1 {talents:Some(talents),..Default::default()}
    }], &runtime.context(), runtime.provenance(), &mut budget);
    assert!(matches!(result,Err(ToolError::ConflictingCandidateTalents)));
    assert_eq!(budget.used_simulations,0);
}

#[test]
fn unsupported_macro_skill_is_rejected_before_any_simulation() {
    let runtime = AgentRuntime::fixture_for(crate::GameVersion::CangShengZhuShiTest, crate::Mount::TieGuYi);
    let baseline = runtime.fixture_scenario();
    for patch in [
        ScenarioPatchV1 {macro_text:Some(PatchValueV1::Set("/cast 阵云\n/cast 盾击".into())), ..Default::default()},
        ScenarioPatchV1 {sequence:Some(vec!["阵云结晦".into()]), ..Default::default()},
        ScenarioPatchV1 {talents:Some(vec![30769]), ..Default::default()},
    ] {
        let mut budget = ToolBudget::new(4);
        let result = compare_scenarios("invalid-rule-transplant", &baseline,
            &[CandidatePatchV1 {label:"候选".into(), patch}], &runtime.context(), runtime.provenance(), &mut budget);
        assert!(matches!(result, Err(ToolError::UnavailableCandidateAction { .. })));
        assert_eq!(budget.used_simulations, 0);
    }
}

#[test]
fn repairing_an_existing_invalid_macro_remains_possible() {
    let runtime = AgentRuntime::fixture();
    let mut request = runtime.fixture_scenario().simulation;
    request.sequence = vec!["__macro__".into(); 60];
    request.macro_text = Some("/cast 不存在\n/cast 盾击".into());
    request.macro_duration = Some(10.0);
    let baseline = ScenarioSnapshotV1::capture(runtime.game_version(),runtime.mount(),request).unwrap();
    let mut budget = ToolBudget::new(4);
    let result = compare_scenarios("repair-existing-invalid", &baseline, &[
        CandidatePatchV1 {label:"移除无效行".into(), patch:ScenarioPatchV1 {
            macro_text:Some(PatchValueV1::Set("/cast 盾击".into())), ..Default::default()}},
        CandidatePatchV1 {label:"其它调整保留原无效行".into(), patch:ScenarioPatchV1 {
            macro_text:Some(PatchValueV1::Set("/cast 不存在\n/cast 盾压\n/cast 盾击".into())), ..Default::default()}},
    ], &runtime.context(), runtime.provenance(), &mut budget);
    assert!(result.is_ok());
    assert_eq!(budget.used_simulations, 3);
}
