use super::*;
use crate::profile::{Hit, HitKind, Stats};
fn cartridge(region: &str) -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(
        std::fs::read(directory.join(format!("shinobi3_{region}.bin"))).unwrap(),
    )
    .unwrap()
}
fn encounter(index: usize) -> Encounter {
    Encounter {
        achievement: index,
        start: Stats::default(),
        jumped: false,
        floor: false,
        wrong_attack: false,
        damaged: false,
        mijin: false,
        hits: 1,
    }
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn each_encounter_checks_its_failures_and_cannot_restart_during_victory() {
    let rom = cartridge("us");
    let mut game = Game::load(&rom).unwrap();
    for index in 8..=20 {
        game.run = Default::default();
        game.run.encounter = Some(encounter(index));
        game.finish_encounter();
        assert_ne!(game.run.candidates & (1 << index), 0, "challenge {index}");
        for failure in 0..3 {
            game.run = Default::default();
            let mut e = encounter(index);
            match failure {
                0 => game.run.stats.deaths = 1,
                1 => {
                    if index == 11 {
                        e.mijin = true;
                    } else {
                        game.run.stats.ninjutsu = 1;
                    }
                }
                _ => {
                    if index == 11 {
                        e.wrong_attack = true;
                    } else {
                        e.damaged = true;
                    }
                }
            }
            game.run.encounter = Some(e);
            game.finish_encounter();
            assert_eq!(
                game.run.candidates & (1 << index),
                0,
                "challenge {index}, failure {failure}"
            );
        }
    }
    for (index, failure) in [(8, 0), (10, 1), (11, 2), (12, 2), (14, 3)] {
        game.run = Default::default();
        let mut e = encounter(index);
        match failure {
            0 => e.jumped = true,
            1 => e.floor = true,
            2 => e.wrong_attack = true,
            _ => game.run.stats.shurikens = 1,
        }
        game.run.encounter = Some(e);
        game.finish_encounter();
        assert_eq!(game.run.candidates & (1 << index), 0);
    }
    let mut boss = Game::load_stage(&rom, StageId::new(1, 3, 0)).unwrap();
    boss.track_begin();
    assert_eq!(boss.run.encounter.as_ref().unwrap().achievement, 14);
    boss.run.encounter.as_mut().unwrap().damaged = true;
    boss.finish_encounter();
    boss.track_begin();
    assert!(boss.run.encounter.is_none());
}
#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn secrets_require_the_correct_room_and_hidden_life_requires_collection() {
    for region in ["us", "eu", "jp"] {
        let rom = cartridge(region);
        for secret in SECRETS {
            let mut game = Game::load_stage(&rom, secret.stage).unwrap();
            let (x, y) = (secret.x.0 + 8, secret.y.1 - 1);
            game.player.x = x * 256;
            game.player.y = y * 256;
            game.track_secrets();
            assert_ne!(
                game.run.candidates & (1 << secret.achievement),
                0,
                "{region} {}",
                secret.stage
            );
            game.run.candidates = 0;
            game.player.y = (secret.y.1 + 128) * 256;
            game.track_secrets();
            assert_eq!(game.run.candidates, 0);
            assert!(!secret.contains(secret.x.0 - 1, y));
            assert!(!secret.contains(x, secret.y.1));
        }
        let mut game = Game::load_stage(&rom, StageId::new(4, 2, 2)).unwrap();
        game.player.x = 684 * 256;
        game.player.y = 24 * 256;
        game.track_secrets();
        assert_eq!(game.run.candidates, 0);
        game.track_pickups(&[(0, 684, 24), (3, 1724, 168)]);
        assert_eq!(game.run.candidates, 0);
        game.track_pickups(&[(3, 684, 24)]);
        assert_ne!(game.run.candidates & (1 << 43), 0);
    }
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn round_and_whole_game_awards_use_flags_ammo_difficulty_and_continues() {
    let rom = cartridge("us");
    for round in 1..=7 {
        let mut game = Game::load_stage(&rom, StageId::new(round, 3, 0)).unwrap();
        game.start_session(crate::options::GameOptions::default());
        game.run.round_ticks = 600;
        game.requests = REQUEST_NEXT_SUBSTAGE;
        game.track_end();
        assert_ne!(game.run.candidates & (1 << (round - 1)), 0);
        assert_ne!(game.run.candidates & (1 << (23 + round)), 0);
        assert_ne!(game.run.candidates & (1 << 34), 0);
        assert_eq!(game.run.stats.rounds, 1);
        game.track_end();
        assert_eq!(game.run.stats.rounds, 1);
        let mut dirty = Game::load_stage(&rom, StageId::new(round, 3, 0)).unwrap();
        dirty.start_session(crate::options::GameOptions::default());
        dirty.round_flags = RoundFlags {
            damaged: true,
            thrown: true,
            ninjutsu: true,
        };
        dirty.requests = REQUEST_NEXT_SUBSTAGE;
        dirty.track_end();
        assert_eq!(
            dirty.run.candidates & ((1 << 34) | (1 << 33) | (1 << (23 + round))),
            0
        );
    }
    for (continues, magic, expert, expected_master, expected_no_magic) in [
        (0, 0, true, true, true),
        (1, 0, true, false, true),
        (0, 1, true, true, false),
        (0, 0, false, false, true),
    ] {
        let mut game = Game::load_stage(&rom, StageId::new(7, 3, 0)).unwrap();
        game.start_session(crate::options::GameOptions {
            level: if expert { 3 } else { 0 },
            ..Default::default()
        });
        game.run.cleared = 0x3f;
        game.run.stats.continues = continues;
        game.run.stats.ninjutsu = magic;
        game.requests = REQUEST_NEXT_SUBSTAGE;
        game.track_end();
        assert_eq!(game.run.candidates & (1 << 7) != 0, expected_master);
        assert_eq!(game.run.candidates & (1 << 31) != 0, expected_no_magic);
        assert_eq!(game.run.stats.completions, 1);
    }
    let mut empty = Game::load_stage(&rom, StageId::new(1, 3, 0)).unwrap();
    empty.run.round_start_ammo = 0;
    empty.requests = REQUEST_NEXT_SUBSTAGE;
    empty.track_end();
    assert_eq!(empty.run.candidates & (1 << 33), 0);
    let mut pit = Game::load_stage(&rom, StageId::new(1, 3, 0)).unwrap();
    pit.start_death(crate::movement::state::PIT_DEATH);
    pit.reload_part();
    pit.requests = REQUEST_NEXT_SUBSTAGE;
    pit.track_end();
    assert_eq!(
        pit.run.candidates & (1 << 34),
        0,
        "pit deaths cannot earn Perfect Assassin after a refill"
    );
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn event_counters_survive_death_continue_and_part_and_round_loads() {
    let rom = cartridge("us");
    let mut game = Game::load(&rom).unwrap();
    game.start_session(crate::options::GameOptions::default());
    game.run.id = 42;
    game.run.stats.pickups = 3;
    game.run.stats.melee_hits = 2;
    game.start_death(crate::movement::state::DEATH);
    game.start_death(crate::movement::state::DEATH);
    assert_eq!(game.run.stats.deaths, 1);
    game.lose_life();
    assert_eq!(game.run.stats.deaths, 1);
    assert_eq!(game.run.id, 42);
    let continued = game.continue_game().unwrap();
    assert_eq!(continued.run.stats.continues, 1);
    assert_eq!(continued.run.stats.pickups, 3);
    let next = continued.load_next(StageId::new(2, 1, 0)).unwrap();
    assert_eq!(next.run.id, 42);
    assert_eq!(next.run.stats.melee_hits, 2);
    assert_eq!(next.run.round_ticks, 0);
    for ammo in [1, 7, 8] {
        let mut game = Game::load(&rom).unwrap();
        game.combat.hud.ammo = ammo;
        game.combat
            .start_player_attack(AttackPosture::Spread, game.player.position(), false);
        game.track_end();
        assert_eq!(game.run.stats.shurikens, u64::from(ammo));
        game.track_end();
        assert_eq!(game.run.stats.shurikens, u64::from(ammo));
    }
    let mut game = Game::load(&rom).unwrap();
    game.combat.weapons.ninjutsu_uses = 0;
    assert!(game.combat.begin_ninjutsu().is_none());
    game.track_end();
    assert_eq!(game.run.stats.ninjutsu, 0);
    game.combat.weapons.ninjutsu_uses = 1;
    assert!(game.combat.begin_ninjutsu().is_some());
    game.track_end();
    assert_eq!(game.run.stats.ninjutsu, 1);
    game.run.hits(&[
        Hit {
            kind: HitKind::Melee,
            target: 0x14,
            defeated: false,
        },
        Hit {
            kind: HitKind::Shuriken,
            target: 0x14,
            defeated: true,
        },
    ]);
    assert_eq!(game.run.stats.melee_hits, 1);
    assert_eq!(game.run.stats.enemies, 1);
}
