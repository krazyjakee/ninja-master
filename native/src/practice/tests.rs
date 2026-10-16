use super::*;
#[test]
fn only_completed_normal_stages_unlock_practice() {
    let mut data = Data::default();
    let stage = StageId::new(1, 2, 0);
    assert!(!unlocked(&data, stage));
    data.completed_stages.insert(stage_key(stage));
    assert!(unlocked(&data, stage));
    assert!(!unlocked(&data, StageId::FOREST));
    data.unlocked.insert(0, 1);
    assert!(unlocked(&data, StageId::FOREST));
    assert!(!unlocked(&data, StageId::new(2, 1, 0)));
}
#[test]
fn records_keep_the_fastest_complete_valid_ghost() {
    let stage = StageId::FOREST;
    let mut profile = crate::profile::Profile::default();
    let attempt = |ticks, completed, valid| Attempt {
        completed,
        valid,
        record: Record {
            ticks,
            frames: vec![GhostFrame(0, 32, 496, 0, false, None); ticks as usize + 1],
        },
    };
    assert!(!profile.record_trial(stage, attempt(60, false, true)));
    assert!(!profile.record_trial(stage, attempt(60, true, false)));
    assert!(profile.record_trial(stage, attempt(120, true, true)));
    assert!(!profile.record_trial(stage, attempt(180, true, true)));
    assert!(!profile.record_trial(stage, attempt(120, true, true)));
    assert!(profile.record_trial(stage, attempt(60, true, true)));
    let bytes = serde_json::to_vec(&profile.data).unwrap();
    let data: Data = serde_json::from_slice(&bytes).unwrap();
    let record = &data.time_trials[&stage_key(stage)];
    assert_eq!(record.ticks, 60);
    assert_eq!(record.frames.len(), 61);
    assert!(record.valid());
    assert_eq!(time(3691), "01:01.51");
}

fn cartridge() -> Vec<u8> {
    let dir = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(std::fs::read(dir.join("shinobi3_us.bin")).unwrap()).unwrap()
}
fn advance(modes: &mut crate::modes::Modes, n: usize) {
    for _ in 0..n {
        modes.tick(Input::default());
        modes.take_audio();
    }
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn title_and_practice_support_keyboard_mouse_scroll_and_back_with_locked_stages() {
    use crate::{input::press, modes::Modes};
    let rom = cartridge();
    let mut modes = Modes::new(Box::new(Title::menu(Front::load(&rom).unwrap())));
    advance(&mut modes, 100);
    modes.tick(press(Button::Down));
    modes.tick(press(Button::Start));
    advance(&mut modes, 30);
    assert_eq!(modes.name(), "Practice");
    modes.tick(press(Button::Start)); // Explore picker
    modes.tick(press(Button::Start)); // Locked forest
    assert!(modes.game().is_none());
    modes.menu_event(MenuEvent::Back);
    modes.tick(Input::default());
    modes.menu_event(MenuEvent::Click(100, 80, false)); // Time trial
    modes.tick(Input::default());
    modes.menu_event(MenuEvent::Scroll(100));
    modes.menu_event(MenuEvent::Click(100, 170, false));
    modes.tick(Input::default());
    assert!(modes.game().is_none());
    modes.tick(press(Button::Mode));
    modes.tick(press(Button::Mode));
    assert_eq!(modes.name(), "Title");
    advance(&mut modes, 100);
    modes.tick(press(Button::Start)); // Restored Practice focus
    advance(&mut modes, 30);
    assert_eq!(modes.name(), "Practice");
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn trial_ghost_persists_and_practice_preserves_continue_and_rejects_modified_times() {
    use crate::{devtools::DevTools, game::Game, input::press, modes::Modes};
    let rom = cartridge();
    let dir = std::env::temp_dir().join(format!(
        "ninja-practice-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = dir.join("continue.nms");
    let mut modes = Modes::new(Box::new(Game::load(&rom).unwrap()));
    let mut tools = DevTools::new(&rom);
    tools.enable_disk_saves(&mut modes, [true; 4], None);
    modes.enable_progress(Some(path.clone()), &rom);
    advance(&mut modes, 10);
    modes.checkpoint();
    let checkpoint = std::fs::read(&path).unwrap();
    let mut run = crate::profile::Run::default();
    modes.profile.begin(&mut run);
    run.completed_stages.insert("1-1".into());
    modes.profile.observe(&run);
    let data = modes.profile.data.clone();
    modes.replace(Box::new(PracticeScreen::new(
        Front::load(&rom).unwrap(),
        data,
        Some(Kind::TimeTrial),
        Some(StageId::FOREST),
        String::new(),
    )));
    modes.tick(press(Button::Start));
    assert_eq!(modes.name(), "Game");
    assert!(!modes.game().unwrap().combat.enemies.is_empty());
    advance(&mut modes, 30);
    modes.checkpoint();
    tools.save(&modes, 0);
    assert!(!tools.status.starts_with("saved"));
    assert_eq!(std::fs::read(&path).unwrap(), checkpoint);
    modes.game_mut().unwrap().request_next_substage();
    modes.tick(Input::default());
    assert_eq!(modes.name(), "Practice");
    assert_eq!(modes.profile.data.time_trials["1-1"].ticks, 30);
    modes.tick(press(Button::Start));
    assert!(
        modes
            .game()
            .unwrap()
            .practice
            .as_ref()
            .unwrap()
            .ghost
            .is_some()
    );
    advance(&mut modes, 4);
    let game = modes.game_mut().unwrap();
    game.player.x += 64 * 256;
    let mut with_ghost = Frame::default();
    game.draw(&mut with_ghost);
    let ghost = game.practice.as_mut().unwrap().ghost.take();
    let mut without_ghost = Frame::default();
    game.draw(&mut without_ghost);
    assert_ne!(
        with_ghost.checksum(),
        without_ghost.checksum(),
        "saved ghost must render in world space"
    );
    game.practice.as_mut().unwrap().ghost = ghost;
    modes.mark_practice();
    modes.game_mut().unwrap().request_next_substage();
    modes.tick(Input::default());
    assert_eq!(modes.profile.data.time_trials["1-1"].ticks, 30);
    modes.tick(press(Button::Start));
    advance(&mut modes, 1);
    modes.game_mut().unwrap().requests |= crate::game::REQUEST_LIFE_LOST;
    modes.tick(Input::default());
    assert_eq!(modes.name(), "Practice");
    assert_eq!(modes.profile.data.time_trials["1-1"].ticks, 30);
    modes.tick(press(Button::Start));
    modes.tick(press(Button::Start));
    advance(&mut modes, 30);
    assert_eq!(modes.name(), "Pause");
    modes.tick(press(Button::Attack));
    modes.tick(press(Button::Jump));
    advance(&mut modes, 16);
    assert_eq!(modes.name(), "Practice");
    assert_eq!(modes.profile.data.time_trials["1-1"].ticks, 30);
    assert_eq!(std::fs::read(&path).unwrap(), checkpoint);
    drop(modes);
    let mut modes = Modes::new(Box::new(PracticeScreen::new(
        Front::load(&rom).unwrap(),
        Data::default(),
        None,
        None,
        String::new(),
    )));
    modes.enable_progress(Some(path.clone()), &rom);
    assert!(modes.profile.data.time_trials["1-1"].valid());
    assert_eq!(std::fs::read(path).unwrap(), checkpoint);
    drop(modes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn every_practice_back_input_returns_one_level_and_cancels_queued_clicks() {
    use crate::input::press;
    let rom = cartridge();
    for kind in [Kind::Explore, Kind::TimeTrial] {
        for source in 0..5 {
            let mut data = Data::default();
            data.completed_stages.insert("1-1".into());
            let mut screen = PracticeScreen::new(
                Front::load(&rom).unwrap(),
                data,
                Some(kind),
                None,
                String::new(),
            );
            screen.menu_event(MenuEvent::Click(100, 62, false));
            let input = match source {
                0 => {
                    screen.menu_event(MenuEvent::Back);
                    Input::default()
                }
                1 | 2 => {
                    screen.menu_event(MenuEvent::Click(
                        if source == 1 { 188 } else { 280 },
                        16,
                        false,
                    ));
                    Input::default()
                }
                3 => press(Button::Ninjutsu),
                _ => press(Button::Mode),
            };
            assert!(
                screen.tick(input).is_none(),
                "{kind:?}, back source {source}"
            );
            assert_eq!(screen.kind, None, "{kind:?}, back source {source}");
            assert_eq!(screen.menu.selected, usize::from(kind == Kind::TimeTrial));
            assert!(screen.tick(Input::default()).is_none());
            assert_eq!(
                screen.kind, None,
                "a queued stage click must not reopen the picker"
            );
            match source {
                0 => screen.menu_event(MenuEvent::Back),
                1 | 2 => screen.menu_event(MenuEvent::Click(
                    if source == 1 { 188 } else { 280 },
                    16,
                    false,
                )),
                _ => {}
            }
            let change = screen.tick(if source >= 3 { input } else { Input::default() });
            assert!(
                matches!(change, Some(ModeChange::Replace(ref mode)) if mode.name() == "Title")
            );
        }
    }
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn routed_controller_back_and_confirm_only_return_one_practice_level() {
    use crate::{input::press, modes::Modes};
    let rom = cartridge();
    let mut modes = Modes::new(Box::new(PracticeScreen::new(
        Front::load(&rom).unwrap(),
        Data::default(),
        Some(Kind::TimeTrial),
        None,
        String::new(),
    )));
    let mut input = press(Button::Mode);
    input.pressed |= Button::Start.mask();
    modes.tick(input);
    assert_eq!(modes.name(), "Practice");
    for _ in 0..30 {
        modes.tick(Input {
            held: Button::Mode.mask(),
            pressed: 0,
        });
    }
    assert_eq!(
        modes.name(),
        "Practice",
        "holding Back must not leave the submenu"
    );
    modes.tick(press(Button::Mode));
    assert_eq!(modes.name(), "Title");
}
