use super::*;
use crate::input::press;

fn screen(region: &str) -> OptionsScreen {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    let rom = crate::rom::regions::prepare(
        std::fs::read(directory.join(format!("shinobi3_{region}.bin"))).unwrap(),
    )
    .unwrap();
    let mut screen = OptionsScreen::new(Front::load(&rom).unwrap());
    for _ in 0..100 {
        screen.tick(Input::default());
    }
    screen.take_audio();
    screen
}

fn hold(screen: &mut OptionsScreen, button: Button, ticks: u16) {
    for _ in 0..ticks {
        screen.tick(Input {
            held: button.mask(),
            pressed: 0,
        });
    }
}

#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn every_region_and_scheme_keeps_rows_visible_and_diagrams_read_only() {
    for region in ["us", "eu", "jp"] {
        let mut options = screen(region);
        for control in 0..6 {
            let settings = GameOptions {
                control,
                ..options.options()
            };
            options.set_options(settings);
            for (row, y) in [
                (LEVEL, 40),
                (SHURIKINS, 56),
                (CONTROL, 72),
                (CONTROL, 120),
                (MUSIC, 140),
                (SE, 156),
                (VOICE, 172),
                (EXIT, 188),
            ] {
                options.menu_event(MenuEvent::Hover(80, y));
                assert_eq!(options.cursor, row, "{region}: scheme {control}, y={y}");
            }
            options.menu_event(MenuEvent::Click(292, 120, false));
            assert_eq!(
                options.options(),
                settings,
                "reading the diagram must not change it"
            );
            for (x, y) in [
                (23, 72),
                (304, 72),
                (80, 30),
                (80, 130),
                (80, 198),
                (80, 220),
            ] {
                options.menu_event(MenuEvent::Click(x, y, false));
                assert_eq!(options.options(), settings);
                assert_eq!(options.cursor, CONTROL);
            }
            let diagram = options.control_diagram();
            assert!(diagram[..6].iter().all(|text| text.len() <= 15));
            if control >= 4 {
                assert!(diagram[6].contains("MODE:CYCLE"));
            }
            let mut frame = Frame::default();
            options.draw(&mut frame);
            // The footer has a solid backing even over the original portrait.
            assert_eq!(&frame.rgb()[(200 * 320 + 16) * 3..][..3], &PANEL);
        }
    }
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn widescreen_recenters_after_gameplay_and_notices_wait_until_options_closes() {
    let options = screen("us");
    let mut native = Frame::default();
    options.draw(&mut native);
    for width in [crate::WIDE_WIDTH, 768] {
        let mut wide = Frame::new(width);
        wide.align_world(800, 4096);
        options.draw(&mut wide);
        let margin = (width - crate::WIDTH) / 2;
        for y in 0..crate::HEIGHT {
            assert_eq!(
                &wide.rgb()[(y * width + margin) * 3..(y * width + margin + crate::WIDTH) * 3],
                &native.rgb()[y * crate::WIDTH * 3..(y + 1) * crate::WIDTH * 3]
            );
        }
    }
    let rom = options.rom().to_vec();
    let mut modes = crate::modes::Modes::new(Box::new(options));
    modes.enable_progress(None, &rom);
    let run = crate::profile::Run {
        id: 1,
        candidates: 1,
        ..Default::default()
    };
    modes.profile.observe(&run);
    for _ in 0..300 {
        modes.tick(Input::default());
    }
    let mut frame = Frame::default();
    modes.draw(&mut frame);
    assert_eq!(
        frame.checksum(),
        native.checksum(),
        "notices must not cover Back or help"
    );
    assert_eq!(modes.profile.notices.front(), Some(&0));
    assert_eq!(modes.profile.notice_ticks, 0);
    modes.menu_event(MenuEvent::Back);
    for _ in 0..16 {
        modes.tick(Input::default());
    }
    assert_eq!(modes.name(), "Title");
    assert!(
        modes.profile.notice_ticks > 0,
        "deferred notices resume on the title"
    );
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn clicks_stay_with_their_row_and_preserve_order_before_a_tick() {
    let mut options = screen("us");
    options.menu_event(MenuEvent::Click(292, 40, false));
    options.menu_event(MenuEvent::Hover(80, 140));
    options.tick(Input::default());
    assert_eq!(
        options.options().level,
        2,
        "the Level click must not change Music"
    );
    assert_eq!(options.music, 1);
    assert!(options.take_audio().commands.is_empty());

    options.menu_event(MenuEvent::Click(292, 72, false));
    options.menu_event(MenuEvent::Click(292, 72, false));
    options.menu_event(MenuEvent::Click(130, 72, false));
    options.menu_event(MenuEvent::Click(292, 56, false));
    options.menu_event(MenuEvent::Hover(80, 188));
    options.tick(Input::default());
    assert_eq!(
        options.options().control,
        1,
        "two next clicks and one previous click"
    );
    assert_eq!(options.options().start_ammo(options.rom()), 60);
    assert_eq!(options.cursor, EXIT);
    options.menu_event(MenuEvent::Scroll(i32::MIN));
    assert_eq!(options.cursor, LEVEL);
    options.menu_event(MenuEvent::Scroll(i32::MAX));
    assert_eq!(options.cursor, EXIT);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn sound_preview_play_stop_and_feedback_work_for_every_row_and_mouse_zone() {
    let mut options = screen("us");
    for (row, y, table, stop) in [
        (MUSIC, 140, Table::Music, STOP_ALL),
        (SE, 156, Table::Effect, STOP_EFFECTS),
        (VOICE, 172, Table::Voice, STOP_EFFECTS),
    ] {
        options.menu_event(MenuEvent::Click(80, y, false));
        assert_eq!(options.cursor, row);
        assert_eq!(
            options.take_audio().commands,
            [table.command(options.rom(), 1).unwrap()]
        );
        assert!(options.feedback.starts_with("LAST PLAYED:"));
        hold(&mut options, Button::Attack, 40);
        assert!(
            options.take_audio().commands.is_empty(),
            "holding Play must not retrigger"
        );
        for x in [32, 130, 200, 292] {
            options.menu_event(MenuEvent::Click(x, y, true));
            assert_eq!(options.take_audio().commands, [stop]);
            assert!(options.feedback.contains("STOPPED"));
        }
        options.tick(press(Button::Jump));
        assert_eq!(options.take_audio().commands, [stop]);
    }
    options.menu_event(MenuEvent::Click(292, 140, false));
    assert_ne!(options.music, 1, "the right arrow selects, without playing");
    assert!(options.take_audio().commands.is_empty());
    options.tick(press(Button::Ninjutsu));
    assert_eq!(options.take_audio().commands, [FADE_MUSIC]);
    assert_eq!(options.feedback, "MUSIC FADING OUT");
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn held_navigation_and_values_repeat_after_a_delay_and_diagonals_do_not_edit() {
    let mut options = screen("us");
    options.tick(press(Button::Down));
    assert_eq!(options.cursor, SHURIKINS);
    hold(&mut options, Button::Down, REPEAT_DELAY - 1);
    assert_eq!(options.cursor, SHURIKINS);
    hold(&mut options, Button::Down, 1);
    assert_eq!(options.cursor, CONTROL);
    hold(&mut options, Button::Down, 4);
    assert_eq!(options.cursor, MUSIC);
    options.menu_event(MenuEvent::Hover(80, 72));
    options.tick(press(Button::Right));
    assert_eq!(options.options().control, 1);
    hold(&mut options, Button::Right, REPEAT_DELAY - 1);
    assert_eq!(options.options().control, 1);
    hold(&mut options, Button::Right, 1);
    assert_eq!(options.options().control, 2);
    options.tick(press(Button::Left));
    hold(&mut options, Button::Left, REPEAT_DELAY - 1);
    assert_eq!(
        options.options().control,
        1,
        "a new direction starts its own delay"
    );
    let settings = options.options();
    let diagonal = Button::Up.mask() | Button::Right.mask();
    options.tick(Input {
        held: diagonal,
        pressed: diagonal,
    });
    assert_eq!(options.cursor, SHURIKINS);
    assert_eq!(options.options(), settings);
    options.tick(Input::default());
    hold(&mut options, Button::Left, REPEAT_DELAY - 1);
    assert_eq!(
        options.options(),
        settings,
        "releasing cancels the old repeat timer"
    );
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn ammo_modes_and_both_cartridge_secrets_survive_the_new_interactions() {
    let mut options = screen("us");
    options.set_options(GameOptions {
        shurikens: DISABLED_SHURIKENS,
        ..Default::default()
    });
    options.menu_event(MenuEvent::Hover(80, 56));
    for expected in [0, 10, 20, 30, 40, 50, 60, 70] {
        options.tick(press(Button::Right));
        assert!(!options.options().shurikens_disabled());
        assert_eq!(options.options().start_ammo(options.rom()), expected);
    }
    options.tick(press(Button::Right));
    assert!(options.options().shurikens_disabled());
    options.tick(press(Button::Right)); // Zero, distinct from Disable.
    options.effect = SECRET_EFFECT;
    for _ in 0..SECRET_TICKS - 1 {
        options.tick(Input::default());
    }
    options.menu_event(MenuEvent::Hover(80, 72));
    options.menu_event(MenuEvent::Hover(80, 56));
    options.tick(Input::default());
    assert!(
        !options.options().infinite,
        "the secret requires continuous focus"
    );
    for _ in 1..SECRET_TICKS {
        options.tick(Input::default());
    }
    assert!(options.options().infinite);
    assert_eq!(
        options.take_audio().commands,
        [Table::Voice.command(options.rom(), SECRET_VOICE).unwrap()]
    );
    options.tick(press(Button::Right));
    assert!(!options.options().infinite);

    options.menu_event(MenuEvent::Hover(80, 140));
    for step in 0..INVINCIBLE_LENGTH {
        let expected = byte(options.rom(), INVINCIBLE_SEQUENCE + step);
        options.music = (0..MUSIC_COUNT as u16)
            .find(|&index| Table::Music.command(options.rom(), index as u8).ok() == Some(expected))
            .unwrap();
        options.tick(press(Button::Attack));
    }
    assert!(options.options().invincible);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn back_works_during_fade_and_the_entire_fallback_ui_fades_to_black() {
    let mut options = screen("us");
    options.art = None;
    let mut frame = Frame::default();
    options.draw(&mut frame);
    assert!(
        frame.rgb().iter().any(|&channel| channel != 0),
        "fallback is usable"
    );
    options.menu_event(MenuEvent::Click(80, 188, false));
    assert_eq!(options.take_audio().commands, [STOP_ALL]);
    options.menu_event(MenuEvent::Click(292, 40, false));
    assert_eq!(options.options().level, 0, "ignore edits while leaving");
    for _ in 0..FADE_STEPS - 1 {
        options.tick(Input::default());
    }
    options.draw(&mut frame);
    assert!(frame.rgb().iter().all(|&channel| channel == 0));
    assert!(options.tick(Input::default()).is_some());

    let mut options = screen("us");
    options.screen.fade = Fade::In(0);
    options.menu_event(MenuEvent::Back);
    assert!(options.screen.leaving());
    assert_eq!(options.take_audio().commands, [STOP_ALL]);
}

#[test]
#[ignore = "requires the three user-supplied cartridges"]
fn display_tabs_work_with_keyboard_controller_and_mouse_in_every_region() {
    for region in ["us", "eu", "jp"] {
        let mut options = screen(region);
        options.display_context(DisplaySettings::default(), None);
        let game = options.options();
        options.tick(press(Button::Up));
        assert!(options.tabs_focused);
        options.tick(press(Button::Right));
        assert!(options.display_page);
        options.tick(press(Button::Down));
        assert!(!options.tabs_focused);
        assert_eq!(options.display_cursor, 0);
        options.tick(press(Button::Right));
        assert!(options.display.fullscreen);
        hold(&mut options, Button::Right, 100);
        assert!(
            options.display.fullscreen,
            "held toggles must not repeatedly switch fullscreen"
        );
        options.tick(press(Button::X));
        assert!(!options.display_page);
        options.tick(press(Button::Y));
        assert!(options.display_page);
        options.menu_event(MenuEvent::Scroll(i32::MAX));
        assert_eq!(options.display_cursor, 10);
        options.menu_event(MenuEvent::Scroll(i32::MIN));
        assert_eq!(options.display_cursor, 0);
        options.menu_event(MenuEvent::Click(140, 20, false));
        assert!(!options.display_page);
        options.menu_event(MenuEvent::Click(240, 20, false));
        assert!(options.display_page);
        assert_eq!(options.options(), game);
        let mut native = Frame::default();
        let mut wide = Frame::new(768);
        wide.align_world(800, 4096);
        options.draw(&mut native);
        options.draw(&mut wide);
        for y in 0..crate::HEIGHT {
            assert_eq!(
                &wide.rgb()[(y * 768 + 224) * 3..(y * 768 + 544) * 3],
                &native.rgb()[y * 320 * 3..(y + 1) * 320 * 3]
            );
        }
    }
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn display_changes_round_trip_through_modes_and_the_existing_settings_file() {
    let mut modes = crate::modes::Modes::new(Box::new(screen("us")));
    let mut settings = crate::settings::Settings::default();
    settings.volume = 37;
    settings.game.control = 5;
    modes.set_options(settings.game);
    modes.display_context((&settings).into(), None);
    modes.menu_event(MenuEvent::Click(240, 20, false));
    for (x, y) in [
        (292, 60),
        (292, 72),
        (292, 100),
        (292, 112),
        (292, 140),
        (292, 152),
        (248, 164),
        (279, 176),
    ] {
        modes.menu_event(MenuEvent::Click(x, y, false));
    }
    // A desktop refresh arriving before the next tick must preserve pending edits.
    modes.display_context((&settings).into(), None);
    let changed = modes.take_display_settings().unwrap();
    assert!(modes.take_display_settings().is_none());
    assert_eq!(
        changed,
        DisplaySettings {
            scale: 4,
            fullscreen: false,
            widescreen: false,
            aspect: 1,
            integer_scale: false,
            smooth_wave_effects: false,
            crt: true,
            crt_curvature: 50,
            scanlines: 100,
        }
    );
    changed.apply(&mut settings);
    assert_eq!(settings.volume, 37);
    assert_eq!(settings.game.control, 5);
    assert_eq!(modes.options(), Some(settings.game));
    let path =
        std::env::temp_dir().join(format!("ninja-options-display-{}.ini", std::process::id()));
    settings.save(&path).unwrap();
    let (restored, warnings) = crate::settings::Settings::load(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(warnings.is_empty());
    assert_eq!(restored, settings);

    modes.menu_event(MenuEvent::Click(180, 190, false));
    let defaults = modes.take_display_settings().unwrap();
    assert_eq!(defaults, DisplaySettings::default());
    defaults.apply(&mut settings);
    assert_eq!(settings.volume, 37);
    assert_eq!(
        settings.game.control, 5,
        "reset must not reset controls or game options"
    );
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn display_dependencies_disable_edits_and_numeric_values_stay_in_range() {
    let mut options = screen("us");
    options.menu_event(MenuEvent::Click(240, 20, false));
    options.menu_event(MenuEvent::Click(292, 48, false));
    assert!(
        options.take_display_settings().is_none(),
        "headless mode cannot edit desktop settings"
    );
    options.display_context(
        DisplaySettings {
            fullscreen: true,
            ..Default::default()
        },
        None,
    );
    for y in [60, 100, 164] {
        options.menu_event(MenuEvent::Click(292, y, false));
        assert!(
            options.take_display_settings().is_none(),
            "dependent setting at y={y} is disabled"
        );
    }
    options.display_context(
        DisplaySettings {
            crt: true,
            ..Default::default()
        },
        Some("No OpenGL renderer"),
    );
    for y in [152, 164] {
        options.menu_event(MenuEvent::Click(292, y, false));
        assert!(options.take_display_settings().is_none(), "CRT unavailable");
    }
    options.display_context(
        DisplaySettings {
            crt: true,
            ..Default::default()
        },
        None,
    );
    for y in [164, 176] {
        options.menu_event(MenuEvent::Click(216, y, false));
        options.menu_event(MenuEvent::Click(130, y, false));
        assert_eq!(
            if y == 164 {
                options.display.crt_curvature
            } else {
                options.display.scanlines
            },
            0
        );
        options.menu_event(MenuEvent::Click(279, y, false));
        options.menu_event(MenuEvent::Click(292, y, false));
        assert_eq!(
            if y == 164 {
                options.display.crt_curvature
            } else {
                options.display.scanlines
            },
            100
        );
    }
    options.menu_event(MenuEvent::Hover(80, 60));
    options.tick(press(Button::Right));
    hold(&mut options, Button::Right, 100);
    assert_eq!(options.display.scale, crate::settings::MAX_SCALE);
    options.tick(press(Button::Left));
    hold(&mut options, Button::Left, 100);
    assert_eq!(options.display.scale, crate::settings::MIN_SCALE);
    let current = options.display;
    options.menu_event(MenuEvent::Click(116, 190, false));
    assert_eq!(
        options.display, current,
        "the gap between Back and Reset is inactive"
    );
}
