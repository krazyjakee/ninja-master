//! Cartridge integration checks. Run with --ignored and a directory containing
//! the three verified retail dumps; game data is never part of the test suite.
use ninja_master_native::{
    assets::{PlayerAssets, SoundAssets, StageAssets},
    ending::Ending,
    front_end::{Front, SegaLogo, Title},
    game::Game,
    input::{Button, Input},
    menu::MenuEvent,
    modes::{Mode, Modes},
    options::{GameOptions, OptionsScreen},
    render::Frame,
    rom::regions::{Region, identify, prepare},
    sega_logo::{Logo, Variant},
    sound::SoundPlayer,
    stage::{StageId, all_substages, part_count},
    title_art::TitleArt,
};
use std::path::PathBuf;

fn cartridge(region: &str) -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    prepare(
        std::fs::read(directory.join(format!("shinobi3_{region}.bin")))
            .expect("verified local cartridge required"),
    )
    .unwrap()
}
fn press(button: Button) -> Input {
    Input {
        held: button.mask(),
        pressed: button.mask(),
    }
}
fn settle(mode: &mut impl Mode) {
    for _ in 0..100 {
        assert!(mode.tick(Input::default()).is_none());
    }
}

#[test]
#[ignore = "requires the three user-supplied retail cartridges"]
fn every_region_decodes_every_part_and_front_end() {
    for (name, region) in [
        ("us", Region::Usa),
        ("eu", Region::Europe),
        ("jp", Region::Japan),
    ] {
        let rom = cartridge(name);
        assert_eq!(identify(&rom).unwrap(), region);
        TitleArt::title(&rom).unwrap();
        TitleArt::options(&rom).unwrap();
        let mut story = ninja_master_native::story::Story::new(Front::load(&rom).unwrap());
        let mut story_ticks = 0;
        let mut frame = Frame::default();
        while story.tick(Input::default()).is_none() {
            story_ticks += 1;
            if story_ticks % 100 == 0 {
                story.draw(&mut frame);
            }
            assert!(story_ticks < 4000, "{name}: story did not finish");
        }
        assert!(
            story_ticks > if region == Region::Japan { 1300 } else { 1800 },
            "{name}: story reader exited early at {story_ticks}"
        );
        for counter in 0..2 {
            let mut front = Front::load(&rom).unwrap();
            front.demo_counter = counter;
            let mut demo = ninja_master_native::attract::Demo::new(front).unwrap();
            for tick in 0..1900 {
                if tick % 100 == 0 {
                    demo.draw(&mut frame);
                }
                if demo.tick(Input::default()).is_some() {
                    break;
                }
                assert!(tick < 1899, "{name}: attract demo did not finish");
            }
        }
        PlayerAssets::load(&rom).unwrap();
        SoundAssets::load(&rom).unwrap();
        let stages = all_substages(&rom).unwrap();
        assert_eq!(stages.len(), 21);
        let mut frame = Frame::default();
        let mut parts = 0;
        for id in stages {
            for part in 0..part_count(&rom, id).unwrap() {
                let id = id.with_part(part as u8);
                StageAssets::load_stage(&rom, id).unwrap_or_else(|e| panic!("{name} {id}: {e}"));
                let mut game =
                    Game::load_stage(&rom, id).unwrap_or_else(|e| panic!("{name} {id}: {e}"));
                for _ in 0..30 {
                    game.tick(Input::default());
                }
                game.draw(&mut frame);
                parts += 1;
            }
        }
        eprintln!("{name}: decoded and ran {parts} stage parts");
        let game = Game::load(&rom).unwrap();
        let mut ending = Ending::new(&game);
        assert!(!ending.pages.is_empty(), "{name}: missing epilogue");
        assert!(!ending.roll.is_empty(), "{name}: missing staff roll");
        for tick in 0..60_000 {
            if tick % 300 == 0 {
                ending.draw(&mut frame);
            }
            if ending.tick(Input::default()).is_some() {
                break;
            }
            assert!(
                tick < 59_999,
                "{name}: ending did not finish ({:?}, {} pages, {} staff rows)",
                ending.step,
                ending.pages.len(),
                ending.roll.len()
            );
        }
    }
}

#[test]
#[ignore = "requires the three user-supplied retail cartridges"]
fn every_region_plays_music_effects_and_voices() {
    for name in ["us", "eu", "jp"] {
        let rom = cartridge(name);
        for command in (0x81..=0xef).filter(|&command| command != 0xd9) {
            let mut player = SoundPlayer::new(&rom, 8_000).unwrap();
            player.play_command(command);
            let mut samples = vec![0; 800];
            player
                .render(&mut samples)
                .unwrap_or_else(|e| panic!("{name} ${command:02x}: {e}"));
        }
    }
}

#[test]
#[ignore = "requires the three user-supplied retail cartridges"]
fn logo_skip_works_at_every_phase_and_does_not_activate_title() {
    for name in ["us", "eu", "jp"] {
        let rom = cartridge(name);
        for variant in [Variant::Zoom, Variant::Slide] {
            for skip_at in [0, 1, 8, 40, 100, 150, 190] {
                let front = Front::load(&rom).unwrap();
                let mut modes = Modes::new(Box::new(SegaLogo::with_variant(front, variant)));
                for _ in 0..skip_at {
                    modes.tick(Input::default());
                }
                modes.tick(press(Button::Start));
                for _ in 0..120 {
                    modes.tick(Input {
                        held: Button::Start.mask(),
                        pressed: 0,
                    });
                }
                assert_eq!(modes.name(), "Title");
                assert!(modes.game().is_none());
            }
            let mut logo = Logo::new(&rom, variant, 0).unwrap();
            let expected = match variant {
                Variant::Zoom => ninja_master_native::sega_logo::ZOOM_TICKS,
                Variant::Slide => ninja_master_native::sega_logo::SLIDE_TICKS,
            };
            for _ in 1..expected {
                assert!(logo.tick(false).is_none());
            }
            assert_eq!(
                logo.tick(false),
                Some(ninja_master_native::sega_logo::Exit::Intro)
            );
        }
    }
}

#[test]
#[ignore = "requires a user-supplied US retail cartridge"]
fn mouse_options_scroll_and_return_to_main_menu_focus() {
    let rom = cartridge("us");
    let mut title = Title::menu(Front::load(&rom).unwrap());
    settle(&mut title);
    title.menu_event(MenuEvent::Scroll(1));
    title.menu_event(MenuEvent::Click(100, 175, false)); // Visible Options row.
    let mut modes = Modes::new(Box::new(title));
    for _ in 0..100 {
        modes.tick(Input::default());
    }
    assert_eq!(modes.name(), "Options");
    modes.menu_event(MenuEvent::Click(250, 100, false)); // Change control scheme.
    modes.tick(Input::default());
    assert_eq!(modes.options().unwrap().control, 1);
    modes.menu_event(MenuEvent::Scroll(10));
    modes.menu_event(MenuEvent::Click(100, 169, false)); // Back in the final row.
    for _ in 0..100 {
        modes.tick(Input::default());
    }
    assert_eq!(modes.name(), "Title");
    // Selected Options remains under the same visible row after returning.
    modes.tick(press(Button::Start));
    for _ in 0..100 {
        modes.tick(Input::default());
    }
    assert_eq!(modes.name(), "Options");
    let mut options = OptionsScreen::new(Front::load(&rom).unwrap());
    settle(&mut options);
    for _ in 0..6 {
        options.tick(press(Button::Down));
    }
    assert_eq!(options.cursor, 6);
    options.tick(press(Button::Up));
    assert_eq!(options.cursor, 5);
}

#[test]
#[ignore = "requires a user-supplied US retail cartridge"]
fn continue_survives_restart_and_new_game_requires_confirmation() {
    let rom = cartridge("us");
    let dir = std::env::temp_dir().join(format!("ninja-roadmap-resume-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("continue.nms");
    let _ = std::fs::remove_file(&path);
    let demo = ninja_master_native::attract::Demo::new(Front::load(&rom).unwrap()).unwrap();
    let mut attract = Modes::new(Box::new(demo));
    attract.enable_progress(Some(path.clone()), &rom);
    attract.checkpoint();
    assert!(
        !path.exists(),
        "the attract demo must not create a saved run"
    );
    let mut front = Front::load(&rom).unwrap();
    front.options = GameOptions {
        control: 5,
        level: 1,
        ..GameOptions::default()
    };
    let mut modes = Modes::new(Box::new(Title::menu(front.clone())));
    modes.enable_progress(Some(path.clone()), &rom);
    for _ in 0..100 {
        modes.tick(Input::default());
    }
    modes.tick(press(Button::Start));
    for _ in 0..900 {
        modes.tick(Input::default());
    }
    for _ in 0..80 {
        modes.tick(Input {
            held: Button::Right.mask(),
            pressed: 0,
        });
    }
    modes.tick(press(Button::Start));
    for _ in 0..30 {
        modes.tick(Input::default());
    }
    assert_eq!(modes.name(), "Pause");
    modes.menu_event(MenuEvent::Click(250, 81, false));
    modes.tick(Input::default());
    assert_eq!(modes.game().unwrap().ninjutsu().0, 1);
    modes.tick(press(Button::Mode)); // Controller Back resumes.
    for _ in 0..30 {
        modes.tick(Input::default());
    }
    assert_eq!(modes.name(), "Game");
    modes.checkpoint();
    let game = modes.game().unwrap();
    let expected = (
        game.player.position(),
        game.health(),
        game.ammo(),
        game.options,
        game.ninjutsu(),
    );
    assert!(path.is_file());
    modes.tick(press(Button::Start));
    for _ in 0..30 {
        modes.tick(Input::default());
    }
    modes.tick(press(Button::Attack));
    modes.tick(press(Button::Jump)); // Quit to Menu also makes a replayable checkpoint.
    for _ in 0..30 {
        modes.tick(Input::default());
    }
    assert_eq!(modes.name(), "Title");
    drop(modes);
    let mut modes = Modes::new(Box::new(Title::menu(front.clone())));
    modes.enable_progress(Some(path.clone()), &rom);
    for _ in 0..100 {
        modes.tick(Input::default());
    }
    modes.tick(press(Button::Down)); // Continue.
    modes.tick(press(Button::Start));
    for _ in 0..16 {
        modes.tick(Input::default());
    }
    let game = modes.game().expect("Continue must restore gameplay");
    assert_eq!(
        (
            game.player.position(),
            game.health(),
            game.ammo(),
            game.options,
            game.ninjutsu()
        ),
        expected
    );
    assert_eq!(game.stage_id, StageId::FOREST);
    let saved = std::fs::read(&path).unwrap();
    attract.enable_progress(Some(path.clone()), &rom);
    attract.checkpoint();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        saved,
        "the attract demo must preserve the saved run"
    );
    let mut modes = Modes::new(Box::new(Title::menu(front)));
    modes.enable_progress(Some(path.clone()), &rom);
    for _ in 0..100 {
        modes.tick(Input::default());
    }
    modes.tick(press(Button::Start)); // Ask to replace.
    for _ in 0..30 {
        modes.tick(Input::default());
    }
    assert_eq!(modes.name(), "Title");
    modes.menu_event(MenuEvent::Back);
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    modes.tick(press(Button::Start));
    modes.tick(press(Button::Down)); // Confirm New Game.
    modes.tick(press(Button::Start));
    for _ in 0..100 {
        modes.tick(Input::default());
    }
    assert!(modes.game().is_some());
    std::fs::write(&path, b"broken save").unwrap();
    let mut modes = Modes::new(Box::new(Title::menu(Front::load(&rom).unwrap())));
    modes.enable_progress(Some(path), &rom);
    for _ in 0..100 {
        modes.tick(Input::default());
    }
    modes.tick(press(Button::Down));
    modes.tick(press(Button::Start));
    for _ in 0..30 {
        modes.tick(Input::default());
    }
    // With the damaged save rejected, the row after New Game is Options.
    assert_eq!(modes.name(), "Options");
    assert!(modes.game().is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires a user-supplied US retail cartridge"]
fn classic_and_dedicated_combat_select_the_requested_action() {
    use ninja_master_native::movement::state;
    let rom = cartridge("us");
    let mut base = Game::load(&rom).unwrap();
    for _ in 0..180 {
        Mode::tick(&mut base, Input::default());
    }
    for scheme in [4, 5] {
        let mut game = base.clone();
        game.options.control = scheme;
        let guard = if scheme == 4 {
            Button::Ninjutsu
        } else {
            Button::Y
        };
        let ammo = game.ammo();
        Mode::tick(&mut game, press(guard));
        assert_eq!(
            game.player.animator.state,
            state::GUARD,
            "scheme {scheme}: direct guard"
        );
        assert_eq!(game.ammo(), ammo);
        Mode::tick(&mut game, Input::default());
        assert_eq!(game.player.animator.state, state::IDLE);
        for (button, ammunition) in [(Button::Attack, ammo), (Button::X, ammo - 1)] {
            let mut game = base.clone();
            game.options.control = scheme;
            Mode::tick(&mut game, press(button));
            for _ in 0..8 {
                Mode::tick(&mut game, Input::default());
            }
            assert_eq!(game.ammo(), ammunition, "scheme {scheme}, {button:?}");
        }
        let mut game = base.clone();
        game.options.control = scheme;
        game.set_ammo(0);
        let state = game.player.animator.state;
        Mode::tick(&mut game, press(Button::X));
        assert_eq!(game.ammo(), 0);
        assert_eq!(
            game.player.animator.state, state,
            "empty ranged action must not swing a sword"
        );
        Mode::tick(&mut game, Input::default());
        Mode::tick(&mut game, press(Button::Attack));
        assert_ne!(
            game.player.animator.state, state,
            "melee remains available with no ammo"
        );
    }
    let ammo = base.ammo();
    Mode::tick(&mut base, press(Button::Attack));
    for _ in 0..8 {
        Mode::tick(&mut base, Input::default());
    }
    assert_eq!(
        base.ammo(),
        ammo - 1,
        "classic still throws when no enemy is nearby"
    );
}
