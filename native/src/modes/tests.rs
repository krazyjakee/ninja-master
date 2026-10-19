use super::*;
use crate::{
    input::{Button, press},
    stage::StageId,
};

fn cartridge() -> Vec<u8> {
    let dir = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(std::fs::read(dir.join("shinobi3_us.bin")).unwrap()).unwrap()
}

/// The title menu with the resume file at `path`, past its opening fade.
fn title(rom: &[u8], path: &std::path::Path) -> Modes {
    let front = crate::front_end::Front::load(rom).unwrap();
    let mut modes = Modes::new(Box::new(crate::front_end::Title::menu(front)));
    modes.enable_progress(Some(path.to_path_buf()), rom);
    for _ in 0..200 {
        modes.tick(Input::default());
    }
    modes
}

fn run_until_game(modes: &mut Modes) {
    for _ in 0..1000 {
        if modes.name() == "Game" {
            return;
        }
        modes.tick(Input::default());
    }
    panic!("no gameplay; still in {}", modes.name());
}

/// Choose CONTINUE (below NEW GAME) and play into the stage.
fn continue_run(modes: &mut Modes) {
    modes.tick(press(Button::Down));
    modes.tick(Input::default());
    modes.tick(press(Button::Start));
    run_until_game(modes);
}

/// Make the saved frame checksum disagree, as a gameplay change does.
fn break_replay(path: &std::path::Path) {
    let mut bytes = std::fs::read(path).unwrap();
    let end = bytes.len() - 4;
    bytes[20] ^= 0xFF;
    let crc = crate::assets::crc32(&bytes[..end]);
    bytes[end..].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn continue_restarts_the_saved_substage_when_the_replay_differs() {
    let rom = cartridge();
    let dir = std::env::temp_dir().join(format!("ninja-continue-{}", std::process::id()));
    let path = dir.join("continue.nms");
    for (warp, practice) in [(None, false), (Some(StageId::new(4, 1, 1)), true)] {
        let _ = std::fs::remove_dir_all(&dir);
        let mut modes = title(&rom, &path);
        modes.tick(press(Button::Start));
        run_until_game(&mut modes);
        if let Some(stage) = warp {
            crate::devtools::DevTools::new(&rom).warp(&mut modes, stage);
            run_until_game(&mut modes);
        }
        for _ in 0..120 {
            modes.tick(Input::default());
        }
        modes.checkpoint();
        let saved = modes.game().unwrap();
        let (stage, old_id) = (saved.stage_id, saved.run.id);
        drop(modes);

        break_replay(&path);
        let mut modes = title(&rom, &path);
        assert!(modes.progress.available());
        continue_run(&mut modes);
        let game = modes.game().unwrap();
        assert!(
            modes.progress.status.is_empty(),
            "{}",
            modes.progress.status
        );
        assert_eq!(game.stage_id, stage.with_part(0));
        assert_ne!(game.run.id, old_id, "the restart is a new run");
        assert_eq!(game.run.practice, practice, "eligibility carries over");
        let new_id = game.run.id;
        drop(modes);

        // The restart replaced the save with one this build replays.
        let mut modes = title(&rom, &path);
        continue_run(&mut modes);
        let game = modes.game().unwrap();
        assert_eq!(game.run.id, new_id, "replayed, not restarted again");
        assert_eq!(game.stage_id, stage.with_part(0));
        assert_eq!(game.run.practice, practice);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
