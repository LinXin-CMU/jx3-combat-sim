use crate::*;

#[test]
fn attribute_profile_paths_follow_level_without_changing_formal_paths() {
    for mount in [Mount::FenShanJin, Mount::TieGuYi] {
        let formal = attrs_prefix(GameVersion::AnYingQianJi, mount);
        assert_eq!(formal, format!("attrs_{}", mount_dir_name(mount)));
        assert_eq!(attrs_prefix(GameVersion::ShanHaiYuanLiu, mount), formal);
        assert_eq!(attrs_prefix(GameVersion::CangShengZhuShiTest, mount),
            format!("attrs_level50_{}", mount_dir_name(mount)));
    }
}

fn equipment_with_vitality(vitality: i64) -> equip::EquipData {
    let mut data = super::empty_equipment();
    let item = serde_json::from_value(serde_json::json!({
        "id":1,"name":"测试护腕","sub_type":10,"detail_type":0,"level":700,"quality":4,
        "max_strength":6,"require_level":30,"max_durability":100,"belong_school":"苍云",
        "magic_kind":"防御","magic_type":"","set_id":0,"icon_id":0,"belong_map":"",
        "bases":[],"magics":[{"slot":"atVitalityBase","label":"体质","value":vitality}],
        "diamonds":[],"attr_tags":[]
    })).unwrap();
    data.items.insert((10, 1), item);
    data
}

#[test]
fn legacy_gear_profile_recalculates_from_new_items_without_scaling_manual_numbers() {
    let old = equipment_with_vitality(300000);
    let new = equipment_with_vitality(3000);
    let req: equip::CalcRequest = serde_json::from_value(serde_json::json!({
        "slots":{"WRIST":{"equip_id":1,"strength":0,"embedding":[],"enhance_id":0,"enchant_id":0}}
    })).unwrap();
    let (_, old_bs, old_mc, _, _) = load_school_toml(GameVersion::AnYingQianJi, Mount::TieGuYi).unwrap();
    let saved = serde_json::to_value(raw_to_attributes(&equip::calculate(&old, &req, &old_bs, &old_mc).raw)).unwrap();
    let settings = serde_json::json!({
        "mount_choice":{"mount":"TieGuYi"},
        "eq_config_v1":{"slots":req.slots,"stoneId":0}
    });
    let converted = attribute_storage::recalculate_legacy_equipment(&saved, &settings, Mount::TieGuYi, &old, &new).unwrap();
    assert_eq!(converted.vitality, 3018.0);
    assert_eq!(converted.surplus_value, 0.0);
    let mut custom = saved.clone(); custom["vitality"] = serde_json::json!(123456);
    assert!(attribute_storage::recalculate_legacy_equipment(&custom, &settings, Mount::TieGuYi, &old, &new).is_none());
    assert!(attribute_storage::recalculate_legacy_equipment(&saved, &settings, Mount::FenShanJin, &old, &new).is_none());
    assert!(attribute_storage::recalculate_legacy_equipment(&saved, &settings, Mount::TieGuYi, &old, &super::empty_equipment()).is_none());
}

#[test]
fn test_tiegu_agent_scenario_can_be_captured_verified_and_round_tripped() {
    for mount in [Mount::FenShanJin, Mount::TieGuYi] {
        let simulation = serde_json::from_value(serde_json::json!({
            "sequence":["盾刀"], "haste_level":0,
            "attributes":{"base_attack":3,"weapon_damage":0,"crit_level":4,"crit_effect_level":0,
                "overcome_level":1,"strain_level":0,"haste_level":0,"vitality":18,"shen_fa":18,"li_dao":17},
            "target":{"level":54,"defense_bonus":0}
        })).unwrap();
        let snapshot = agent::ScenarioSnapshotV1::capture(GameVersion::CangShengZhuShiTest, mount, simulation).unwrap();
        let decoded: agent::ScenarioSnapshotV1 = serde_json::from_value(serde_json::to_value(snapshot).unwrap()).unwrap();
        decoded.verify_hash().unwrap();
    }
}

#[test]
fn orange_weapon_strain_uses_character_level() {
    assert_eq!(shen_bing_wu_shuang_at_level(45320, 130), Some((10, 1272.0)));
    assert_eq!(shen_bing_wu_shuang_at_level(45320, 50), Some((10, 15.0)));
    assert_eq!(shen_bing_wu_shuang_at_level(44121, 50), Some((11, 9.0)));
    assert_eq!(shen_bing_wu_shuang_at_level(0, 50), None);
}
