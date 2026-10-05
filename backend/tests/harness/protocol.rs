use crate::harness::run_schema::{contains_secret_value, RunRequest};
use serde_json::{json, Value};

fn request() -> Value {
    let fixture: Value =
        serde_json::from_str(include_str!("../agent_diagnostic_eval/scenario.json")).unwrap();
    let mut simulation = fixture["simulation"].clone();
    simulation["sequence"] = json!(["盾刀", "盾刀"]);
    simulation["macro_text"] = Value::Null;
    simulation["macro_duration"] = Value::Null;
    json!({"goal":"验证当前技能轴","simulation":simulation,"version":"AnYingQianJi","mount":"FenShanJin"})
}

#[test]
fn nested_free_text_never_becomes_a_credentials_store() {
    let secret = format!("{}{}", "sk-", "abcdefghijklmnopqrstuvxyz");
    assert!(contains_secret_value(
        &json!({"metadata":{"source_label":secret}})
    ));
    let mut value = request();
    value["constraints"] = json!({"allowed_sources":[secret]});
    let parsed: RunRequest = serde_json::from_value(value).unwrap();
    assert!(parsed.validate().is_err());
    assert!(!contains_secret_value(&json!({"goal":"如何配置API key？"})));
}

#[test]
fn typed_run_rejects_unregistered_effects_and_accepts_full_scene() {
    let mut value = request();
    assert!(serde_json::from_value::<RunRequest>(value.clone())
        .unwrap()
        .validate()
        .is_ok());
    value["shell"] = json!("arbitrary-command");
    assert!(serde_json::from_value::<RunRequest>(value).is_err());
}

#[test]
fn narrative_numbers_must_match_measured_evidence() {
    let facts = vec![json!({"baseline":{"dps":1000.0},"best":{"dps":1250.0},"comparison_verified":true})];
    assert!(crate::agent::report::experiment_prose_is_grounded(
        "本次实测 DPS 为1250，较基线提升25%。",
        &facts
    ));
    assert!(!crate::agent::report::experiment_prose_is_grounded(
        "本次实测 DPS 为987654321.12。",
        &facts
    ));
    assert!(crate::agent::report::experiment_prose_is_grounded(
        "本次候选可执行，结论以证据所列环境为限。",
        &facts
    ));
}
