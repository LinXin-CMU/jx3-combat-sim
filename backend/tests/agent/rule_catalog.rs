use super::*;

#[test]
fn effect_keywords_retrieve_resource_rules_without_requiring_a_skill_named_after_resource() {
    let runtime = AgentRuntime::fixture_for(GameVersion::CangShengZhuShiTest, Mount::TieGuYi);
    let result = lookup("resource-definitions", DefinitionQuery {query:"暴怒".into(), limit:12, ..Default::default()},
        &runtime.fixture_scenario(), &runtime).unwrap().result;
    assert_eq!(result["search_mode"],"description");
    assert!(result["matches"].as_array().unwrap().iter().any(|item| item["kind"] == "talent" && item["definition"]["name"] == "不归"));
    assert!(result["matches"].as_array().unwrap().iter().any(|item| item["definition"]["name"] == "阵云结晦"));
}

#[test]
fn cross_mount_lookup_reads_unselected_definitions_without_mutating_scenario() {
    let runtime = AgentRuntime::fixture_for(GameVersion::CangShengZhuShiTest, Mount::TieGuYi);
    let scenario = runtime.fixture_scenario();
    let hash = scenario.scenario_hash.clone();
    let result = lookup("catalog-cross-mount", DefinitionQuery { query:"阵云".into(), ..Default::default() }, &scenario, &runtime).unwrap().result;
    assert_eq!(scenario.scenario_hash, hash);
    assert_eq!(result["applies_rules_to_scenario"], false);
    let found = result["matches"].as_array().unwrap();
    assert!(found.iter().all(|item| item["scope"]["mount"] == "fenshanjin"));
    let skill = found.iter().find(|item| item["kind"] == "skill" && item["definition"]["skill_id"] == 30769).unwrap();
    assert!(skill["definition"]["description"].as_str().unwrap().contains("暴怒"));
    assert_eq!(skill["talent_enabled_in_current_scenario"], false);
    assert_eq!(skill["stance_requirement"], "擎盾或擎刀可施放，盾墙体态不可施放");
    assert_eq!(skill["scope"]["is_current_simulation_scope"], false);
    assert_eq!(skill["scope"]["definition_hash"].as_str().unwrap().len(), 64);
    assert!(found.iter().any(|item| item["definition"]["skill_id"] == 30855));
    assert!(found.iter().any(|item| item["definition"]["skill_id"] == 30856));
    let current = lookup("catalog-current", DefinitionQuery { query:"断马".into(), ..Default::default() }, &scenario, &runtime).unwrap().result;
    assert!(current["matches"].as_array().unwrap().iter().any(|item|
        item["scope"]["mount"] == "tieguyi" && item["kind"] == "talent"));
}

#[test]
fn lookup_is_bounded_and_validates_fixed_scope() {
    let runtime = AgentRuntime::fixture();
    let scenario = runtime.fixture_scenario();
    for query in [DefinitionQuery {query:"".into(), ..Default::default()},
        DefinitionQuery {query:"盾".into(), game_version:"../../userdata".into(), ..Default::default()},
        DefinitionQuery {query:"盾".into(), limit:13, ..Default::default()}] {
        assert!(lookup("catalog-invalid", query, &scenario, &runtime).is_err());
    }
    let result = lookup("catalog-limit", DefinitionQuery { query:"盾".into(), limit:1, ..Default::default() }, &scenario, &runtime).unwrap().result;
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["truncated"], true);
}
