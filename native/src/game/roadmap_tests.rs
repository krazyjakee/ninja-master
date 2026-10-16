use super::*;
fn cartridge(region: &str) -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(
        std::fs::read(directory.join(format!("shinobi3_{region}.bin"))).unwrap(),
    )
    .unwrap()
}
struct Floor;
impl Terrain for Floor {
    fn width(&self) -> i32 {
        4096
    }
    fn solid_at(&self, _: i32, y: i32) -> bool {
        y >= 496
    }
}
#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn leader_entrance_pan_advances_the_displayed_walk_cycle_before_the_clamp() {
    for region in ["us", "eu", "jp"] {
        let rom = cartridge(region);
        let mut game = Game::load_stage(&rom, StageId::new(1, 1, 2)).unwrap();
        game.options.invincible = true;
        game.player.x = 1740 * 256;
        game.player.camera_x = 1580;
        Mode::tick(&mut game, Input::default());
        assert!(game.arena.is_some());
        let mut frames = std::collections::BTreeSet::new();
        let mut sprites = std::collections::BTreeSet::new();
        let palettes = game.stage.assets.palettes;
        let mut walking = 0;
        let mut saw_gate_drop = false;
        for tick in 0..220 {
            let panning = !game.arena.as_ref().unwrap().settled;
            let before = game.player.position();
            Mode::tick(&mut game, Input::default());
            if panning {
                assert_eq!(
                    game.player.animator.state,
                    crate::movement::state::WALK,
                    "{region}: idle sliding during entrance pan, tick {tick}"
                );
                assert!(game.player.position().0 > before.0);
                assert_eq!(game.player.position().1, 496);
                if frames.insert(game.player.display_frame()) {
                    // Draw the real player sprite at a fixed anchor with
                    // a fixed palette, so camera/palette changes cannot
                    // make a static pose pass this visual check.
                    let mut frame = Frame::default();
                    game.draw_player(
                        &mut frame,
                        &crate::render::cram_lines(&palettes),
                        (game.player.position().0 - 160, 496 - 200),
                    );
                    sprites.insert(frame.rgb().to_vec());
                }
                walking += 1;
            }
            saw_gate_drop |= game
                .combat
                .effects
                .iter()
                .any(|e| e.kind == crate::combat::EffectKind::Gate && e.phase == 1);
        }
        assert!(walking >= 30);
        assert!(
            frames.len() >= 5,
            "{region}: walking must change the displayed frame: {frames:?}"
        );
        assert!(
            sprites.len() >= 5,
            "{region}: walking must change the rendered sprite"
        );
        assert!(saw_gate_drop);
    }
}
#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn cave_boss_takes_eight_damage_once_from_each_actual_ninjutsu_cast() {
    use crate::ninjutsu::{Cast, Kind};
    for region in ["us", "eu", "jp"] {
        let rom = cartridge(region);
        for (kind, cursor) in [(Kind::Kariu, 0x0d00), (Kind::Mijin, 0x1400)] {
            let mut game = Game::load_stage(&rom, StageId::new(1, 3, 0)).unwrap();
            Mode::tick(&mut game, Input::default());
            let index = game
                .combat
                .enemies
                .iter()
                .position(|e| game.combat.spawns[e.spawn].kind == crate::bosses::BOSS_KIND)
                .unwrap();
            let enemy = &mut game.combat.enemies[index];
            enemy.routine = 0x18;
            enemy.x = 160;
            enemy.mapping = 0x23d;
            game.player.animator.state = kind.state();
            game.player.animator.cursor = cursor;
            game.cast = Some(Cast::new(kind));
            Mode::tick(&mut game, Input::default());
            assert_eq!(game.combat.enemies[index].health, 4, "{region}: {kind:?}");
            for _ in 0..5 {
                Mode::tick(&mut game, Input::default());
            }
            assert_eq!(
                game.combat.enemies[index].health, 4,
                "{region}: repeated {kind:?} hit"
            );
        }
    }
}
#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn double_jump_matches_original_position_velocity_and_animation_on_every_frame() {
    for region in ["us", "eu", "jp"] {
        let rom = cartridge(region);
        PlayerAssets::load(&rom).unwrap();
        let mut player = Player::new(32, 496);
        player.tick(Input::default(), &Floor);
        let mut previous = 0;
        for line in include_str!("../../tests/data/double_jump.csv")
            .lines()
            .skip(1)
        {
            let v: Vec<i64> = line.split(',').map(|s| s.parse().unwrap()).collect();
            let held = v[1] as u16;
            player.tick(
                Input {
                    held,
                    pressed: held & !previous,
                },
                &Floor,
            );
            previous = held;
            assert_eq!(
                (
                    player.x,
                    player.y,
                    player.velocity_y,
                    player.animator.state,
                    player.animator.cursor
                ),
                (
                    (v[2] >> 8) as i32,
                    (v[3] >> 8) as i32,
                    (v[4] >> 8) as i32,
                    v[5] as usize,
                    v[6] as u16
                ),
                "{region}: {line}"
            );
        }
    }
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn leader_walks_through_opening_and_transitions_before_leaving_floor() {
    let rom = cartridge("us");
    let mut game = Game::load_stage(&rom, StageId::new(1, 1, 2)).unwrap();
    game.options.invincible = true;
    game.player.x = 1740 * 256;
    game.player.camera_x = 1628;
    // Enter the lock and let the boulders break. No traversal input needed.
    for _ in 0..220 {
        Mode::tick(&mut game, Input::default());
    }
    assert!(game.arena.as_ref().unwrap().settled);
    for enemy in &mut game.combat.enemies {
        if game.combat.spawns[enemy.spawn].kind == 0xb8 {
            enemy.state = crate::combat::EnemyState::Defeated;
            enemy.dying = true;
            enemy.reaction = Some(crate::combat::Reaction::Death);
        }
    }
    let start = game.player.position().0;
    let mut modes = crate::modes::Modes::new(Box::new(game));
    let mut furthest = start;
    let hostile = Input {
        held: Button::Left.mask() | Button::Attack.mask() | Button::Jump.mask(),
        pressed: Button::Attack.mask() | Button::Jump.mask(),
    };
    // After the death sequence, forced WalkRight must override all gameplay inputs.
    for tick in 0..600 {
        modes.tick(if tick > 200 {
            hostile
        } else {
            Input::default()
        });
        if let Some(game) = modes.game() {
            if game.stage_id.substage == 2 {
                assert!(furthest >= 2040);
                return;
            }
            furthest = furthest.max(game.player.position().0);
            assert!(
                game.player.position().1 <= 496,
                "must fade before walking off the floor at tick {tick}, pos {:?}, width {}, requests {}",
                game.player.position(),
                game.stage.assets.width(),
                game.requests
            );
            assert!(game.death.is_none());
        }
    }
    panic!("did not advance through the opening; furthest x={furthest}");
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn disabled_shurikens_and_empty_technique_selection_survive_lifecycle() {
    let rom = cartridge("us");
    let mut game = Game::load(&rom).unwrap();
    game.start_session(crate::options::GameOptions {
        shurikens: crate::options::DISABLED_SHURIKENS,
        control: 5,
        infinite: true,
        ..Default::default()
    });
    game.combat.weapons.ninjutsu_uses = 0;
    for index in 0..4 {
        assert!(game.select_ninjutsu(index));
        assert!(game.combat.begin_ninjutsu().is_none());
    }
    assert!(!game.select_ninjutsu(4));
    let mut pause = crate::pause::Pause::new(Box::new(game.clone()));
    pause.stage = crate::pause::Stage::Menu;
    let mut paused = crate::modes::Modes::new(Box::new(pause));
    paused.tick(crate::input::press(Button::Right));
    assert_eq!(paused.game().unwrap().ninjutsu(), (0, 0));
    game.combat.weapons.ninjutsu_uses = 1;
    assert_eq!(game.ninjutsu(), (3, 1));
    game.combat.weapons.ninjutsu_uses = 0;
    for button in [Button::X, Button::Y] {
        Mode::tick(&mut game, crate::input::press(button));
        assert_eq!(game.ammo(), 0);
    }
    Mode::tick(&mut game, crate::input::press(Button::Mode));
    assert_eq!(game.ninjutsu(), (0, 0));
    game.select_ninjutsu(2);
    game.lose_life();
    assert!(game.options.shurikens_disabled());
    assert_eq!(game.ammo(), 0);
    assert_eq!(game.ninjutsu().0, 2);
    game = game.continue_game().unwrap();
    assert!(game.options.shurikens_disabled());
    assert_eq!(game.ammo(), 0);
    assert_eq!(game.ninjutsu().0, 2);
    game.enter_next_substage();
    assert!(game.options.shurikens_disabled());
    assert_eq!(game.ammo(), 0);
}
