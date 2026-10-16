use super::*;
use crate::combat::Enemy;

fn cartridge() -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(std::fs::read(directory.join("shinobi3_us.bin")).unwrap()).unwrap()
}

#[test]
fn proximity_matches_704_original_rom_boundary_samples() {
    for line in include_str!("../../../tests/data/melee_boundary.csv")
        .lines()
        .skip(1)
    {
        let v: Vec<i32> = line.split(',').map(|s| s.parse().unwrap()).collect();
        let mut distance = 0x7fff;
        Combat::measure_enemy(
            (256, 256),
            v[3] != 0,
            v[2] != 0,
            false,
            AttackTarget {
                x: 256 + v[0],
                y: 256 + v[1],
                high_priority: v[4] == 0,
            },
            &mut distance,
        );
        assert_eq!(distance, v[5], "original sample {line}");
    }
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn bursts_charge_only_allocated_stars_and_respect_disabled_and_infinite() {
    let rom = cartridge();
    let mut base = Combat::load_stage(&rom, crate::stage::StageId::FOREST).unwrap();
    base.enemies.clear();
    for ammo in [0u16, 1, 3, 8, 12] {
        for repeated in [false, true] {
            let mut combat = base.clone();
            combat.hud.ammo = ammo;
            combat.weapons.spread_fired = repeated;
            combat.start_player_attack(AttackPosture::Spread, (100, 496), false);
            assert_eq!(combat.weapons.shots.len(), usize::from(ammo.min(8)));
            assert_eq!(combat.hud.ammo, ammo.saturating_sub(8));
            assert_eq!(combat.take_player_sounds().len(), usize::from(ammo > 0));
        }
    }
    let mut combat = base.clone();
    combat.hud.ammo = 100;
    for _ in 0..6 {
        assert_eq!(
            combat.start_player_attack(AttackPosture::Standing, (100, 496), false),
            Some(Attack::Throw)
        );
    }
    let before = combat.hud.ammo;
    combat.start_player_attack(AttackPosture::Spread, (100, 496), false);
    assert_eq!(combat.weapons.shots.len(), 2);
    assert_eq!(combat.hud.ammo, before - 2);
    let mut combat = base.clone();
    combat.infinite_shurikens = true;
    combat.hud.ammo = 0;
    combat.start_player_attack(AttackPosture::Spread, (100, 496), false);
    assert_eq!(combat.weapons.shots.len(), 8);
    assert_eq!(combat.hud.ammo, 0);
    for forced in [None, Some(false), Some(true)] {
        let mut combat = base.clone();
        combat.hud.ammo = 99;
        combat.shurikens_disabled = true;
        combat.infinite_shurikens = true;
        combat.weapons.force_throw = forced;
        let result = combat.start_player_attack(AttackPosture::Spread, (100, 496), false);
        assert_eq!(
            result,
            if forced == Some(true) {
                None
            } else {
                Some(Attack::Slash)
            }
        );
        assert_eq!(combat.hud.ammo, 99);
        assert!(combat.weapons.shots.is_empty());
        assert!(combat.projectiles.is_empty());
    }
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn attack_choice_filters_targets_and_samples_before_turning() {
    let rom = cartridge();
    let mut base = Combat::load_stage(&rom, crate::stage::StageId::FOREST).unwrap();
    base.enemies.clear();
    for posture in [
        AttackPosture::Standing,
        AttackPosture::Crouching,
        AttackPosture::Airborne,
        AttackPosture::WallAir,
        AttackPosture::Hang,
        AttackPosture::Horse,
        AttackPosture::Surf,
    ] {
        for (distance, expected) in [
            (63, Attack::Slash),
            (64, Attack::Throw),
            (65, Attack::Throw),
        ] {
            let mut combat = base.clone();
            combat.set_item_box_targets([(100 + distance, 496, false)]);
            assert_eq!(
                combat.start_player_attack(posture, (100, 496), false),
                Some(expected)
            );
            combat.hud.ammo = 0;
            assert_eq!(
                combat.start_player_attack(posture, (100, 496), false),
                Some(Attack::Slash)
            );
        }
    }
    let mut combat = base.clone();
    combat.set_item_box_targets([(132, 496, false)]);
    combat.sample_attack_proximity((100, 496), false);
    assert_eq!(
        combat.start_player_attack(AttackPosture::Standing, (100, 496), true),
        Some(Attack::Slash),
        "turning must use the pre-handler sample"
    );
    combat.weapons.force_throw = Some(true);
    assert_eq!(
        combat.start_player_attack(AttackPosture::Standing, (100, 496), false),
        Some(Attack::Throw)
    );
    combat.weapons.force_throw = Some(false);
    assert_eq!(
        combat.start_player_attack(AttackPosture::Standing, (100, 496), false),
        Some(Attack::Slash)
    );
    for state in [
        EnemyState::Dormant,
        EnemyState::Emerging,
        EnemyState::Defeated,
        EnemyState::Waiting,
    ] {
        for offscreen in [false, true] {
            let mut combat = base.clone();
            combat.spawns[0].kind = 0x14;
            let mut enemy = Enemy::new(0, 132, 496, 1, 1);
            enemy.state = state;
            enemy.offscreen = offscreen;
            combat.enemies.push(enemy);
            assert_eq!(
                combat.find_nearest_enemy((100, 496), false) < 64,
                state == EnemyState::Waiting && !offscreen
            );
        }
    }
    for kind in [0x8c, 0x90, 0x94] {
        let mut combat = base.clone();
        combat.spawns[0].kind = kind;
        let mut enemy = Enemy::new(0, 132, 496, 1, 1);
        enemy.state = EnemyState::Waiting;
        combat.enemies.push(enemy);
        assert_eq!(combat.find_nearest_enemy((100, 496), false), 0x7fff);
    }
    let mut combat = base;
    combat.set_item_box_targets([(100, 496, true), (164, 496, false), (163, 496, false)]);
    assert_eq!(combat.find_nearest_enemy((100, 496), false), 63);
}
