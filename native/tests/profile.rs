//! ROM-backed profile, menu and disk-replay integration. No cartridge assets included.
use ninja_master_native::{
    devtools::DevTools,
    front_end::{Front, Title},
    game::Game,
    input::{Button, Input},
    menu::MenuEvent,
    modes::Modes,
    render::Frame,
};
fn rom() -> Vec<u8> {
    let dir = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    ninja_master_native::rom::regions::prepare(std::fs::read(dir.join("shinobi3_us.bin")).unwrap())
        .unwrap()
}
fn press(b: Button) -> Input {
    Input {
        held: b.mask(),
        pressed: b.mask(),
    }
}
fn directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "ninja-profile-{}-{label}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
fn advance(modes: &mut Modes, n: usize) {
    for _ in 0..n {
        modes.tick(Input::default());
        modes.take_audio();
    }
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn main_menu_opens_both_profile_screens_and_restores_focus_with_every_input_path() {
    let rom = rom();
    let mut modes = Modes::new(Box::new(Title::menu(Front::load(&rom).unwrap())));
    advance(&mut modes, 100);
    // NEW GAME, PRACTICE, EXPANSIONS, EDITOR, OPTIONS, then ACHIEVEMENTS.
    for _ in 0..5 {
        modes.tick(press(Button::Down));
    }
    modes.tick(press(Button::Start));
    advance(&mut modes, 100);
    assert_eq!(modes.name(), "Profile");
    let mut frame = Frame::default();
    modes.draw(&mut frame);
    let first = frame.checksum();
    modes.tick(press(Button::Up));
    modes.draw(&mut frame);
    assert_ne!(first, frame.checksum());
    modes.menu_event(MenuEvent::Scroll(-49));
    modes.menu_event(MenuEvent::Hover(48, 62));
    modes.draw(&mut frame);
    assert_eq!(first, frame.checksum());
    modes.menu_event(MenuEvent::Click(280, 12, false));
    modes.tick(Input::default());
    assert_eq!(modes.name(), "Title");
    advance(&mut modes, 100);
    // The title restored the Achievements row. Down selects Statistics.
    modes.tick(press(Button::Down));
    modes.tick(press(Button::Start));
    advance(&mut modes, 100);
    assert_eq!(modes.name(), "Profile");
    modes.draw(&mut frame);
    let normal = frame.checksum();
    modes.tick(press(Button::Right));
    modes.draw(&mut frame);
    assert_ne!(normal, frame.checksum());
    modes.menu_event(MenuEvent::Scroll(100));
    modes.tick(Input::default());
    modes.draw(&mut frame);
    modes.menu_event(MenuEvent::Back);
    modes.tick(Input::default());
    assert_eq!(modes.name(), "Title");
    advance(&mut modes, 100);
    // Mouse reopens the focused, scrolled Statistics row.
    modes.menu_event(MenuEvent::Click(80, 181, false));
    modes.tick(Input::default());
    advance(&mut modes, 100);
    assert_eq!(modes.name(), "Profile");
    modes.tick(press(Button::Mode));
    assert_eq!(modes.name(), "Title");
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn disk_replay_preserves_identity_and_counters_and_loading_cannot_farm_totals() {
    let rom = rom();
    let dir = directory("replay");
    let checkpoint = dir.join("continue.nms");
    let saves = dir.join("saves");
    let mut modes = Modes::new(Box::new(Game::load(&rom).unwrap()));
    let mut tools = DevTools::new(&rom);
    tools.enable_disk_saves(&mut modes, [true; 4], Some(saves.clone()));
    modes.enable_progress(Some(checkpoint.clone()), &rom);
    advance(&mut modes, 30);
    let id = modes.game().unwrap().run.id;
    assert!(id > 0);
    let before = modes.game().unwrap().run.stats.clone();
    tools.save(&modes, 0);
    advance(&mut modes, 20);
    let high = modes.game().unwrap().run.stats.ticks;
    modes.profile.flush();
    drop(modes);
    let mut modes = Modes::new(Box::new(Title::menu(Front::load(&rom).unwrap())));
    let mut tools = DevTools::new(&rom);
    tools.enable_disk_saves(&mut modes, [true; 4], Some(saves));
    modes.enable_progress(Some(checkpoint), &rom);
    tools.load(&mut modes);
    assert!(tools.status.starts_with("loaded"), "{}", tools.status);
    assert_eq!(modes.game().unwrap().run.id, id);
    assert_eq!(modes.game().unwrap().run.stats, before);
    assert!(modes.game().unwrap().run.practice);
    advance(&mut modes, 20);
    assert_eq!(modes.profile.data.practice.ticks, high);
    advance(&mut modes, 1);
    assert_eq!(modes.profile.data.practice.ticks, high + 1);
    drop(modes);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn new_game_allocates_a_fresh_eligible_run_and_cheats_stay_practice_after_rewind() {
    let rom = rom();
    let dir = directory("newgame");
    let mut modes = Modes::new(Box::new(Title::menu(Front::load(&rom).unwrap())));
    let mut tools = DevTools::new(&rom);
    tools.enable_disk_saves(&mut modes, [true; 4], None);
    modes.enable_progress(Some(dir.join("continue.nms")), &rom);
    advance(&mut modes, 100);
    modes.tick(press(Button::Start));
    advance(&mut modes, 100);
    modes.tick(press(Button::Start));
    advance(&mut modes, 100);
    assert!(modes.game().is_some());
    assert!(!modes.game().unwrap().run.practice);
    let id = modes.game().unwrap().run.id;
    assert!(id > 0);
    tools.save(&modes, 0);
    tools.set_invincible(&mut modes, true);
    assert!(modes.game().unwrap().run.practice);
    tools.load(&mut modes);
    assert!(modes.game().unwrap().run.practice);
    assert_eq!(modes.game().unwrap().run.id, id);
    assert!(!modes.game().unwrap().options.invincible);
    drop(modes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn disabled_shurikens_new_game_and_automatic_continue_remain_eligible() {
    let rom = rom();
    let dir = directory("disabled-shurikens");
    let checkpoint = dir.join("continue.nms");
    let mut front = Front::load(&rom).unwrap();
    front.options.shurikens = ninja_master_native::options::DISABLED_SHURIKENS;
    let mut modes = Modes::new(Box::new(Title::menu(front)));
    modes.enable_progress(Some(checkpoint.clone()), &rom);
    advance(&mut modes, 100);
    modes.tick(press(Button::Start));
    advance(&mut modes, 100);
    modes.tick(press(Button::Start));
    advance(&mut modes, 100);
    assert_eq!(modes.name(), "Game");
    let game = modes.game().unwrap();
    let id = game.run.id;
    assert!(id > 0);
    assert!(!game.run.practice);
    assert!(game.options.shurikens_disabled());
    assert_eq!(game.ammo(), 0);
    modes.checkpoint();
    drop(modes);

    let mut modes = Modes::new(Box::new(Title::menu(Front::load(&rom).unwrap())));
    modes.enable_progress(Some(checkpoint), &rom);
    advance(&mut modes, 100);
    modes.tick(press(Button::Down));
    modes.tick(press(Button::Start));
    advance(&mut modes, 16);
    let game = modes
        .game()
        .expect("automatic Continue must restore the disabled-shuriken run");
    assert_eq!(game.run.id, id);
    assert!(!game.run.practice);
    assert!(game.options.shurikens_disabled());
    assert_eq!(game.ammo(), 0);
    assert!(!modes.profile.data.tainted.contains(&id));
    assert!(modes.profile.data.lifetime.ticks > 0);
    drop(modes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn the_main_menu_editor_item_asks_for_the_editor_and_stays_on_the_title() {
    let rom = rom();
    let mut modes = Modes::new(Box::new(Title::menu(Front::load(&rom).unwrap())));
    advance(&mut modes, 100);
    assert!(!modes.take_editor_request());
    // NEW GAME, PRACTICE, EXPANSIONS, then EDITOR.
    for _ in 0..3 {
        modes.tick(press(Button::Down));
    }
    modes.tick(press(Button::Start));
    advance(&mut modes, 100);
    assert!(modes.take_editor_request());
    assert!(!modes.take_editor_request(), "taken once");
    assert_eq!(modes.name(), "Title");
    // The menu comes back on EDITOR: Up then Start opens EXPANSIONS.
    modes.tick(press(Button::Up));
    modes.tick(press(Button::Start));
    advance(&mut modes, 100);
    assert_eq!(modes.name(), "Expansions");
    assert!(!modes.take_editor_request());
}
