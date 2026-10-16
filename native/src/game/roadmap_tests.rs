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
