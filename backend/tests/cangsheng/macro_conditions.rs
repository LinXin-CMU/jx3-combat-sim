use super::*;
use crate::{GameVersion, Mount, MountConstants};

#[test]
fn test_server_buff_name_conditions_read_stacks_and_expire() {
    let config = crate::macro_parser::parse_macro_text("/cast [buff:威压>=6] 盾猛").unwrap();
    let mut player = Player::with_mount(
        Mount::FenShanJin,
        GameVersion::CangShengZhuShiTest,
        MountConstants::fenshanjin_default(),
        0,
        vec![91003],
        vec![],
    );
    let buff = crate::scripts::v2026_10_CangShengZhuShiTest::buffs::defs::BUFF_WEI_YA;
    let skill_map = HashMap::new();
    let skill_by_id = HashMap::new();
    for stacks in 1..=6 {
        player.add_buff(buff);
        let (pool, _, _) = evaluate_phase1(
            &config.pages[0],
            &player,
            &skill_map,
            &skill_by_id,
            None,
            false,
        );
        assert_eq!(pool.len(), usize::from(stacks == 6));
    }
    player.process_buff_ticks(0.0, 30.0);
    player.current_time = 30.0;
    let (pool, _, _) = evaluate_phase1(
        &config.pages[0],
        &player,
        &skill_map,
        &skill_by_id,
        None,
        false,
    );
    assert!(pool.is_empty());
}

#[test]
fn macro_berserk_reads_its_own_resource_and_respects_version_talent_boundaries() {
    let config =
        crate::macro_parser::parse_macro_text("/cast [sun>=120] 阵云结晦\n/cast [rage>=50] 绝刀")
            .unwrap();
    let skill_map = HashMap::new();
    let skill_by_id = HashMap::new();
    for (version, talents, expected) in [
        (
            GameVersion::CangShengZhuShiTest,
            vec![30769],
            vec!["阵云结晦"],
        ),
        (GameVersion::CangShengZhuShiTest, vec![], vec![]),
        (GameVersion::AnYingQianJi, vec![30769], vec![]),
        (GameVersion::AnYingQianJiTest, vec![30769], vec![]),
        (GameVersion::ShanHaiYuanLiu, vec![30769], vec![]),
    ] {
        let mut player = Player::with_mount(
            Mount::FenShanJin,
            version,
            MountConstants::fenshanjin_default(),
            0,
            talents,
            vec![],
        );
        player.set_rage(10);
        player.set_berserk_value(120);
        let (pool, _, _) = evaluate_phase1(
            &config.pages[0],
            &player,
            &skill_map,
            &skill_by_id,
            None,
            false,
        );
        assert_eq!(
            pool.iter()
                .map(|entry| entry.skill_name.as_str())
                .collect::<Vec<_>>(),
            expected
        );

        player.set_rage(50);
        player.set_berserk_value(0);
        let (pool, _, _) = evaluate_phase1(
            &config.pages[0],
            &player,
            &skill_map,
            &skill_by_id,
            None,
            false,
        );
        assert_eq!(
            pool.iter()
                .map(|entry| entry.skill_name.as_str())
                .collect::<Vec<_>>(),
            vec!["绝刀"]
        );
    }
}

#[test]
fn energy_reads_block_resource_for_all_comparisons_and_version_boundaries() {
    let skill_map = HashMap::new();
    let skill_by_id = HashMap::new();
    for (version, mount, talents, expected) in [
        (
            GameVersion::CangShengZhuShiTest,
            Mount::FenShanJin,
            vec![30769],
            73,
        ),
        (
            GameVersion::CangShengZhuShiTest,
            Mount::FenShanJin,
            vec![],
            73,
        ),
        (
            GameVersion::AnYingQianJi,
            Mount::FenShanJin,
            vec![30769],
            73,
        ),
        (
            GameVersion::AnYingQianJiTest,
            Mount::FenShanJin,
            vec![30769],
            73,
        ),
        (
            GameVersion::ShanHaiYuanLiu,
            Mount::FenShanJin,
            vec![30769],
            73,
        ),
        (GameVersion::AnYingQianJi, Mount::TieGuYi, vec![], 73),
        (GameVersion::ShanHaiYuanLiu, Mount::TieGuYi, vec![], 73),
    ] {
        let mut player = Player::with_mount(
            mount,
            version,
            MountConstants::fenshanjin_default(),
            0,
            talents,
            vec![],
        );
        player.set_rage(10);
        player.set_block_value(73);
        player.set_berserk_value(120);
        for op in [
            CmpOp::Gt,
            CmpOp::Lt,
            CmpOp::Eq,
            CmpOp::GtEq,
            CmpOp::LtEq,
            CmpOp::Neq,
        ] {
            for threshold in [10, 73, 100, 120, 121] {
                let text = format!("/cast [energy{}{threshold}] 盾刀", op.symbol());
                let config = crate::macro_parser::parse_macro_text(&text).unwrap();
                let (pool, _, _) = evaluate_phase1(
                    &config.pages[0],
                    &player,
                    &skill_map,
                    &skill_by_id,
                    None,
                    false,
                );
                assert_eq!(
                    !pool.is_empty(),
                    op.compare_i32(expected, threshold),
                    "{version:?}/{mount:?}, {text}"
                );
            }
        }
    }
}

#[test]
fn energy_uses_tiegu_two_hundred_block_and_keeps_rage_berserk_and_charges_independent() {
    let version = GameVersion::AnYingQianJi;
    let mount = Mount::TieGuYi;
    let skills = crate::load_skills(std::path::Path::new(&crate::skills_dir(version, mount)));
    let blood_rage = skills.iter().find(|s| s.skill_id == 13040).unwrap();
    let mut skill_map = HashMap::new();
    skill_map.insert("血怒", vec![blood_rage]);
    let skill_by_id = HashMap::from([(13040, blood_rage)]);
    let mut player = Player::with_mount(
        mount,
        version,
        MountConstants::fenshanjin_default(),
        0,
        vec![13363],
        vec![],
    );
    player.set_rage(10);
    assert_eq!(player.block_value, 200);
    assert_eq!(player.get_charge_count(blood_rage), 3);
    let config = crate::macro_parser::parse_macro_text(
        "/cast [energy=200] 盾刀\n/cast [rage=10] 绝刀\n/cast [berserk=0&baonu=0] 盾猛\n/cast [skill_energy:血怒=3] 血怒"
    ).unwrap();
    let (pool, _, _) = evaluate_phase1(
        &config.pages[0],
        &player,
        &skill_map,
        &skill_by_id,
        None,
        false,
    );
    assert_eq!(pool.len(), 4);
    player.set_block_value(0);
    let (pool, _, _) = evaluate_phase1(
        &config.pages[0],
        &player,
        &skill_map,
        &skill_by_id,
        None,
        false,
    );
    assert_eq!(
        pool.iter()
            .map(|entry| entry.skill_name.as_str())
            .collect::<Vec<_>>(),
        vec!["绝刀", "盾猛", "血怒"]
    );
    assert_eq!(player.rage, 10);
    assert_eq!(player.get_charge_count(blood_rage), 3);
}
