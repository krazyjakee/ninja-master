use super::*;
use crate::input::press;
fn front() -> Front {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    let rom =
        crate::rom::regions::prepare(std::fs::read(directory.join("shinobi3_us.bin")).unwrap())
            .unwrap();
    Front::load(&rom).unwrap()
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn filters_keep_catalog_identity_and_empty_lists_are_safe() {
    let mut data = Data::default();
    for id in [0, 7, 48] {
        data.unlocked.insert(id, 1);
    }
    let mut screen = ProfileScreen::new(front(), ScreenKind::Achievements, data, String::new());
    screen.menu_event(MenuEvent::Click(302, 80, false));
    assert_eq!(screen.menus[0].first, 7, "click below the thumb pages down");
    screen.menu_event(MenuEvent::Click(302, 80, false));
    assert_eq!(
        screen.menus[0].first, 7,
        "clicking the thumb does not move it"
    );
    screen.menu_event(MenuEvent::Click(302, 70, false));
    assert_eq!(screen.menus[0].first, 0, "click above the thumb pages up");
    screen.menu_event(MenuEvent::Click(240, 44, false));
    assert_eq!(screen.achievements(), [0, 7, 48]);
    screen.tick(press(Button::Up));
    assert_eq!(screen.achievements()[screen.menus[2].selected], 48);
    screen.tick(press(Button::Left));
    assert!(
        !screen
            .achievements()
            .iter()
            .any(|id| [0, 7, 48].contains(id))
    );
    screen.tick(press(Button::Right));
    assert_eq!(screen.achievements()[screen.menus[2].selected], 48);
    for all_unlocked in [false, true] {
        let mut data = Data::default();
        if all_unlocked {
            for id in 0..CATALOG.len() {
                data.unlocked.insert(id, 1);
            }
        }
        let mut screen = ProfileScreen::new(front(), ScreenKind::Achievements, data, String::new());
        screen.menu_event(MenuEvent::Click(
            if all_unlocked { 140 } else { 240 },
            44,
            false,
        ));
        assert_eq!(screen.count(), 0);
        screen.menu_event(MenuEvent::Scroll(100));
        screen.tick(press(Button::Up));
        screen.draw(&mut Frame::default());
    }
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn statistics_tabs_preserve_comparisons_and_only_buttons_change_tabs() {
    let mut data = Data::default();
    data.last_run = Some(super::super::Run {
        practice: true,
        ..Default::default()
    });
    let mut screen = ProfileScreen::new(front(), ScreenKind::Statistics, data, String::new());
    assert_eq!(screen.tab, 1);
    screen.tick(press(Button::Up));
    assert_eq!(screen.menus[1].selected, 9);
    screen.tick(press(Button::Left));
    assert_eq!(screen.menus[0].selected, 9);
    screen.menu_event(MenuEvent::Click(0, 190, false));
    assert_eq!(screen.tab, 0, "reading the details must not switch totals");
    screen.menu_event(MenuEvent::Click(240, 44, false));
    assert_eq!(screen.tab, 2);
    assert_eq!(screen.count(), 7);
    screen.menu_event(MenuEvent::Click(302, 135, false));
    assert_eq!(screen.menus[2].first, 1);
    screen.menu_event(MenuEvent::Click(48, 44, false));
    assert_eq!(screen.menus[0].selected, 9);
}
#[test]
#[ignore = "requires a user-supplied cartridge"]
fn held_navigation_repeats_and_widescreen_keeps_the_same_canvas() {
    let mut screen = ProfileScreen::new(
        front(),
        ScreenKind::Achievements,
        Data::default(),
        String::new(),
    );
    screen.tick(press(Button::Down));
    for _ in 0..28 {
        screen.tick(Input {
            held: Button::Down.mask(),
            pressed: 0,
        });
    }
    assert!(screen.menus[0].selected > 1);
    let mut native = Frame::default();
    let mut wide = Frame::new(crate::WIDE_WIDTH);
    screen.draw(&mut native);
    screen.draw(&mut wide);
    let margin = (crate::WIDE_WIDTH - crate::WIDTH) / 2;
    for y in 0..crate::HEIGHT {
        let row = &wide.rgb()[y * crate::WIDE_WIDTH * 3..(y + 1) * crate::WIDE_WIDTH * 3];
        assert_eq!(
            &row[margin * 3..(margin + crate::WIDTH) * 3],
            &native.rgb()[y * crate::WIDTH * 3..(y + 1) * crate::WIDTH * 3]
        );
        assert!(row[..margin * 3].chunks_exact(3).all(|p| p == [8, 12, 24]));
        assert!(
            row[(margin + crate::WIDTH) * 3..]
                .chunks_exact(3)
                .all(|p| p == [8, 12, 24])
        );
    }
    screen.menu_event(MenuEvent::Click(280, 12, false));
    assert!(matches!(
        screen.tick(Input::default()),
        Some(ModeChange::Replace(_))
    ));
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn notification_popups_wait_until_the_browser_is_closed() {
    let mut data = Data::default();
    data.unlocked.insert(0, 1);
    let screen = ProfileScreen::new(front(), ScreenKind::Achievements, data, String::new());
    let mut modes = crate::modes::Modes::new(Box::new(screen));
    let mut baseline = Frame::default();
    modes.draw(&mut baseline);
    let mut run = super::super::Run::default();
    run.id = 1;
    run.candidates = 1;
    modes.profile.observe(&run);
    for _ in 0..300 {
        modes.tick(Input::default());
    }
    let mut frame = Frame::default();
    modes.draw(&mut frame);
    assert_eq!(frame.checksum(), baseline.checksum());
    assert_eq!(modes.profile.notices.front(), Some(&0));
    assert_eq!(modes.profile.notice_ticks, 0);
    modes.menu_event(MenuEvent::Back);
    modes.tick(Input::default());
    modes.tick(Input::default());
    assert_eq!(modes.profile.notice_ticks, 240);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn profile_errors_can_be_read_dismissed_and_cleared_after_a_retry() {
    let status = "PROFILE SAVE ERROR: permission denied";
    let mut screen = ProfileScreen::new(
        front(),
        ScreenKind::Statistics,
        Data::default(),
        status.into(),
    );
    assert!(screen.show_status);
    screen.tick(press(Button::Attack));
    assert!(!screen.show_status);
    screen.menu_context(false, status);
    assert!(
        !screen.show_status,
        "unchanged errors must not reopen over details"
    );
    screen.menu_event(MenuEvent::Click(80, 26, false));
    assert!(screen.show_status);
    screen.menu_context(false, "");
    assert!(screen.status.is_empty());
    assert!(!screen.show_status);
    screen.menu_context(false, "PROFILE READ ERROR: permission denied");
    assert!(screen.show_status);
}
