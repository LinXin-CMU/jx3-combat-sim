use super::*;
use crate::*;

fn run(req: &SimulateRequest, mount: Mount) -> SimulateResponse {
    let version = GameVersion::AnYingQianJi;
    let (constants, _, _, _, _) = load_school_toml(version, mount).unwrap();
    let skills = load_skills(std::path::Path::new(&skills_dir(version, mount)));
    let recipes = load_recipes(std::path::Path::new(&recipes_file_for_mount(version, mount)));
    simulate_core(req, &skills, version, mount, constants, &recipes, &[], &[])
}

#[test]
fn macro_solidify_records_once_and_replays_waits_and_resources() {
    for mount in [Mount::FenShanJin, Mount::TieGuYi] {
        let mut request: SimulateRequest = serde_json::from_value(serde_json::json!({
            "haste_level": 9233, "sequence": ["__macro__", "__macro__", "__macro__", "__macro__"],
            "macro_text": "/cast [nobuff:血怒] 血怒\n/cast [bufftime:血怒<26] 盾压\n/cast 盾刀",
            "macro_duration": 12.0, "initial_rage": 0, "network_delay": 31,
            "experimental": true, "hanjia_expectation": true, "pauses": [[1.0, 2.0]]
        })).unwrap();
        let source = run(&request, mount);
        let expected: Vec<_> = source.timeline.iter().filter(|e| !e.triggered).collect();
        assert_eq!(expected.len(), 4);
        assert!(expected.iter().any(|e| !e.solidify.as_ref().unwrap().waits.is_empty()));
        for event in &expected {
            let index = event.sequence_index.unwrap();
            request.sequence[index] = event.name.split('·').next().unwrap().into();
            request.solidified_casts.insert(index.to_string(), event.solidify.clone().unwrap());
        }
        let frozen = run(&request, mount);
        let actual: Vec<_> = frozen.timeline.iter().filter(|e| !e.triggered).collect();
        assert_eq!(expected.len(), actual.len());
        for (a, b) in expected.iter().zip(actual) {
            assert_eq!((a.skill_id, a.cast_time, a.rage_after, a.channel_ticks),
                (b.skill_id, b.cast_time, b.rage_after, b.channel_ticks));
        }
        assert_eq!(source.total_damage, frozen.total_damage);
        assert_eq!(source.fight_time, frozen.fight_time);
        // Editing to a different action must fail visibly instead of forcing an old rank.
        request.sequence[0] = "盾飞".into();
        assert!(run(&request, mount).skipped.iter().any(|(index, _)| *index == 0));
    }
}

#[test]
fn macro_solidify_rejects_bad_waits_without_mutating_player() {
    let mut player = Player::new(0, vec![], vec![]);
    let mut timeline = Vec::new(); let mut previous = 0.0; let mut first = true;
    let specs = HashMap::new(); let ids = HashMap::new();
    let ctx = macro_eval::CastCtx { skill_by_id: &ids, recipes_table: &[], dmg_ctx: None,
        network_delay: 0.0, is_macro: false };
    for waits in [vec![1.0, 0.5], vec![f64::NAN], vec![-1.0], vec![f64::INFINITY]] {
        assert!(replay(&FrozenCast { skill_id: 13044, waits, fcast: false }, "盾刀", &mut player,
            &specs, &mut timeline, &mut previous, &mut first, &ctx).is_err());
        assert_eq!(player.current_time, 0.0);
        assert!(timeline.is_empty());
    }
}
