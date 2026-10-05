//! 属性存档按人物等级隔离；可确认由配装生成的旧面板按对应装备重算。
use crate::{equip, load_school_toml, raw_to_attributes, GameVersion, Mount};
use serde_json::Value;

fn setting(settings: &Value, key: &str) -> Option<Value> {
    let value = settings.get(key)?;
    if let Some(text) = value.as_str() { serde_json::from_str(text).ok() }
    else { Some(value.clone()) }
}

pub(super) fn recalculate_legacy_equipment(
    old_attributes: &Value, settings: &Value, mount: Mount,
    old_data: &equip::EquipData, new_data: &equip::EquipData,
) -> Option<crate::Attributes> {
    let choice = setting(settings, "mount_choice")?;
    if choice.get("mount")?.as_str()? != format!("{mount:?}") { return None; }
    let config = setting(settings, "eq_config_v1")?;
    let slots: std::collections::HashMap<String, equip::SlotConfig> =
        serde_json::from_value(config.get("slots")?.clone()).ok()?;
    if slots.is_empty() || slots.iter().any(|(pos, cfg)| {
        cfg.equip_id != 0 && new_data.get_item(equip::pos_to_subtype(pos), cfg.equip_id).is_none()
    }) { return None; }
    let mut req = equip::CalcRequest {
        slots, stone_id: config.get("stoneId").and_then(Value::as_u64).unwrap_or(0) as u32,
        mount: if mount == Mount::TieGuYi { 10389 } else { 10390 }, talents: Vec::new(),
    };
    let (_, old_bs, old_mc, _, _) = load_school_toml(GameVersion::AnYingQianJi, mount).ok()?;
    let mut choices = vec![Vec::new()];
    if let Some(talents) = setting(settings, "jx3_autosave_loop_v1").and_then(|v|v.get("talents").cloned()) {
        let selected = talents.as_object().map(|m| m.values().flat_map(|v| {
            if let Some(ids) = v.as_array() { ids.iter().filter_map(Value::as_u64).map(|v|v as u32).collect() }
            else { v.as_u64().map(|v|vec![v as u32]).unwrap_or_default() }
        }).collect()).unwrap_or_default();
        choices.push(selected);
    }
    // 逐字段匹配已保存面板；手填属性不能被推测成某一套配装。
    let matches = choices.into_iter().any(|talents| {
        req.talents = talents;
        let raw = equip::calculate(old_data, &req, &old_bs, &old_mc).raw;
        let expected = serde_json::to_value(raw_to_attributes(&raw)).unwrap_or(Value::Null);
        old_attributes.as_object().is_some_and(|attrs| {
            attrs.len() >= 10 && attrs.iter().all(|(key, value)| {
                expected.get(key).map_or(key.starts_with('_'), |v| v.as_f64() == value.as_f64())
            })
        })
    });
    if !matches { return None; }
    let (_, bs, mc, _, _) = load_school_toml(GameVersion::CangShengZhuShiTest, mount).ok()?;
    let raw = equip::calculate(new_data, &req, &bs, &mc).raw;
    let mut attrs = raw_to_attributes(&raw);
    attrs.gen_gu = 18.0; attrs.yuan_qi = 17.0;
    Some(attrs)
}
