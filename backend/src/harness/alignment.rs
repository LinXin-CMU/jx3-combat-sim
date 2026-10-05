//! Active-cast alignment shared in meaning with frontend/macro-alignment.js.
//! A maximum-length common skill subsequence wins before timestamp proximity.
use serde::Serialize;

use crate::CastEvent;

pub const MAX_ACTIVE_EVENTS: usize = 2048;
pub const DEFAULT_TIME_TOLERANCE: f64 = 1.0 / 16.0;
const EPSILON: f64 = 1e-9;
const MISSING: u8 = 1;
const EXTRA: u8 = 2;
const MATCH: u8 = 3;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResourceDiff {
    pub field: String,
    pub reference: i32,
    pub actual: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AlignmentRow {
    /// Index into the original timeline, including its passive/virtual events.
    pub reference_index: Option<usize>,
    pub actual_index: Option<usize>,
    pub kind: String,
    pub time_delta: Option<f64>,
    pub resource_diffs: Vec<ResourceDiff>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct AlignmentSummary {
    pub missing: usize,
    pub extra: usize,
    pub changed: usize,
    /// Index into `Alignment.rows`, not either timeline.
    pub first_difference: Option<usize>,
    /// Sum of raw absolute time differences over matched pairs, in seconds.
    pub time_error: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Alignment {
    pub rows: Vec<AlignmentRow>,
    pub summary: AlignmentSummary,
}

/// CastEvent represents successful casts; failed manual inputs live in skipped.
pub fn is_active(event: &CastEvent) -> bool {
    let name = event.name.trim();
    !event.triggered
        && !name.is_empty()
        && !name.starts_with("__")
        && !name.starts_with("移除气劲")
        && name != "清除冷却"
        && event.skill_id != 90001
}

/// Keep combo stages and 雾海 separate; tiers/levels share their base identity.
pub fn skill_key(event: &CastEvent) -> String {
    let name = event.name.trim();
    let mut parts = name.split('·');
    let base = parts.next().unwrap_or_default();
    let fog_name =
        matches!(base, "阵云结晦" | "月照连营" | "雁门迢递") && parts.next() == Some("雾海");
    if (90010..=90012).contains(&event.skill_id) || fog_name {
        format!("{base}·雾海")
    } else {
        base.to_string()
    }
}

/// Same half-open window as the macro editor; original indices are retained.
pub fn within_window(event: &CastEvent, window: f64) -> bool {
    event.cast_time.is_finite() && event.cast_time >= 0.0 && event.cast_time < window
}

struct ActiveEvent<'a> {
    event: &'a CastEvent,
    index: usize,
    key: String,
}

fn active_events<'a>(
    timeline: &'a [CastEvent],
    window: f64,
    label: &str,
) -> Result<Vec<ActiveEvent<'a>>, String> {
    let mut active = Vec::new();
    for (index, event) in timeline.iter().enumerate() {
        if !is_active(event) || !within_window(event, window) {
            continue;
        }
        if active.len() == MAX_ACTIVE_EVENTS {
            return Err(format!(
                "MACRO_ALIGNMENT_LIMIT: {label}超过 {MAX_ACTIVE_EVENTS} 个主动技能，请缩短对照时长后重试。"
            ));
        }
        active.push(ActiveEvent {
            event,
            index,
            key: skill_key(event),
        });
    }
    Ok(active)
}

fn match_row(
    reference: &ActiveEvent<'_>,
    actual: &ActiveEvent<'_>,
    tolerance: f64,
) -> AlignmentRow {
    let a = reference.event;
    let b = actual.event;
    let delta = b.cast_time - a.cast_time;
    let mut resource_diffs = Vec::new();
    if let (Some(before), Some(after)) = (&a.state_before, &b.state_before) {
        for (field, reference, actual) in [
            ("rage", Some(before.rage), Some(after.rage)),
            ("berserk_value", before.berserk_value, after.berserk_value),
            (
                "max_berserk_value",
                before.max_berserk_value,
                after.max_berserk_value,
            ),
            ("block_value", before.block_value, after.block_value),
        ] {
            // Missing/not applicable resources cannot be interpreted as zero.
            if let (Some(reference), Some(actual)) = (reference, actual) {
                if reference != actual {
                    resource_diffs.push(ResourceDiff {
                        field: field.into(),
                        reference,
                        actual,
                    });
                }
            }
        }
    }
    let variant_changed = a.name != b.name
        || a.skill_id != b.skill_id
        || matches!((a.channel_ticks, b.channel_ticks), (Some(a), Some(b)) if a != b);
    AlignmentRow {
        reference_index: Some(reference.index),
        actual_index: Some(actual.index),
        kind: if variant_changed || !resource_diffs.is_empty() || delta.abs() > tolerance + EPSILON
        {
            "changed"
        } else {
            "same"
        }
        .into(),
        time_delta: Some(delta),
        resource_diffs,
    }
}

/// O(n*m) time; at most 2049² traceback bytes and O(m) score space.
/// Window filtering happens before the limit, while row indices address inputs.
pub fn align(
    reference: &[CastEvent],
    actual: &[CastEvent],
    window: f64,
    time_tolerance: f64,
) -> Result<Alignment, String> {
    if !window.is_finite() || window <= 0.0 {
        return Err("对照时长必须是正的有限秒数。".into());
    }
    if !time_tolerance.is_finite() || time_tolerance < 0.0 {
        return Err("时间容差必须是非负有限秒数。".into());
    }
    let reference = active_events(reference, window, "模板")?;
    let actual = active_events(actual, window, "实际循环")?;
    let (n, m) = (reference.len(), actual.len());
    let stride = m + 1;
    let mut directions = vec![0u8; (n + 1) * stride];
    let (mut next_counts, mut counts) = (vec![0u16; stride], vec![0u16; stride]);
    let (mut next_costs, mut costs) = (vec![0.0f64; stride], vec![0.0f64; stride]);

    for j in 0..m {
        directions[n * stride + j] = EXTRA;
    }
    for i in (0..n).rev() {
        directions[i * stride + m] = MISSING;
        counts[m] = 0;
        costs[m] = 0.0;
        for j in (0..m).rev() {
            let (mut count, mut cost, mut direction) = (next_counts[j], next_costs[j], MISSING);
            if counts[j + 1] > count || (counts[j + 1] == count && costs[j + 1] < cost - EPSILON) {
                (count, cost, direction) = (counts[j + 1], costs[j + 1], EXTRA);
            }
            if reference[i].key == actual[j].key {
                let match_count = next_counts[j + 1] + 1;
                let delta = actual[j].event.cast_time - reference[i].event.cast_time;
                let match_cost = next_costs[j + 1] + delta.abs().min(1e6);
                // Prefer this pair on an exact tie, matching the JS algorithm.
                if match_count > count || (match_count == count && match_cost <= cost + EPSILON) {
                    (count, cost, direction) = (match_count, match_cost, MATCH);
                }
            }
            counts[j] = count;
            costs[j] = cost;
            directions[i * stride + j] = direction;
        }
        std::mem::swap(&mut counts, &mut next_counts);
        std::mem::swap(&mut costs, &mut next_costs);
    }

    let mut rows = Vec::with_capacity(n + m);
    let mut summary = AlignmentSummary::default();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        let row = match directions[i * stride + j] {
            MATCH => {
                let row = match_row(&reference[i], &actual[j], time_tolerance);
                i += 1;
                j += 1;
                row
            }
            MISSING => {
                let row = AlignmentRow {
                    reference_index: Some(reference[i].index),
                    actual_index: None,
                    kind: "missing".into(),
                    time_delta: None,
                    resource_diffs: Vec::new(),
                };
                i += 1;
                row
            }
            _ => {
                let row = AlignmentRow {
                    reference_index: None,
                    actual_index: Some(actual[j].index),
                    kind: "extra".into(),
                    time_delta: None,
                    resource_diffs: Vec::new(),
                };
                j += 1;
                row
            }
        };
        if let Some(delta) = row.time_delta {
            summary.time_error += delta.abs();
        }
        match row.kind.as_str() {
            "missing" => summary.missing += 1,
            "extra" => summary.extra += 1,
            "changed" => summary.changed += 1,
            _ => {}
        }
        if row.kind != "same" && summary.first_difference.is_none() {
            summary.first_difference = Some(rows.len());
        }
        rows.push(row);
    }
    Ok(Alignment { rows, summary })
}

#[cfg(test)]
#[path = "../../tests/harness/alignment.rs"]
mod tests;
