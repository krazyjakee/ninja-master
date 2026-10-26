use super::*;
use crate::game::Game;

fn sample() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("sample-expansions/forest-trail")
}

fn cartridge() -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(std::fs::read(directory.join("shinobi3_us.bin")).unwrap()).unwrap()
}

/// A pack folder in a fresh temporary directory, from a manifest and files.
fn pack(name: &str, manifest: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("ninja-expansion-{}-{name}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MANIFEST), manifest).unwrap();
    for (file, text) in files {
        std::fs::write(dir.join(file), text).unwrap();
    }
    dir
}

fn legend(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|&(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// A grid of `width` columns: sky above row 31, ground on it.
fn grid(width: usize) -> String {
    let mut rows = vec![".".repeat(width); LAYOUT_ROWS - 1];
    rows.push("=".repeat(width));
    rows.join("\n") + "\n"
}

#[test]
fn layouts_are_column_major_and_padded_with_the_last_column() {
    let mut text = grid(24);
    // A marker in column 1, row 0.
    text.replace_range(1..2, "#");
    let legend = legend(&[(".", "00"), ("=", "08"), ("#", "4a")]);
    let (bytes, width) = layout(&text, &legend).unwrap();
    assert_eq!(width, 24 * 16);
    assert_eq!(bytes.len(), crate::stage::LAYOUT_BYTES);
    assert_eq!(bytes[LAYOUT_ROWS], 0x4a, "column 1, row 0");
    assert_eq!(bytes[LAYOUT_ROWS - 1], 0x08, "column 0, row 31");
    let last = &bytes[23 * LAYOUT_ROWS..24 * LAYOUT_ROWS];
    assert_eq!(&bytes[bytes.len() - LAYOUT_ROWS..], last);
}

#[test]
fn layouts_reject_bad_grids() {
    let cells = legend(&[(".", "00"), ("=", "08")]);
    let short = grid(24).lines().skip(1).collect::<Vec<_>>().join("\n");
    assert!(layout(&short, &cells).unwrap_err().contains("32 rows"));
    assert!(layout(&grid(20), &cells).unwrap_err().contains("columns"));
    assert!(layout(&grid(257), &cells).unwrap_err().contains("columns"));
    let ragged = grid(24).replacen('.', "..", 1);
    assert!(
        layout(&ragged, &cells)
            .unwrap_err()
            .contains("row 2 is not 25")
    );
    let unknown = grid(24).replacen('.', "?", 1);
    assert!(layout(&unknown, &cells).unwrap_err().contains("legend"));
    let wide = legend(&[(".", "00"), ("=", "08"), ("==", "08")]);
    assert!(layout(&grid(24), &wide).unwrap_err().contains("single"));
}

#[test]
fn the_sample_expansion_opens() {
    let expansion = Expansion::open(&sample()).unwrap();
    assert_eq!(&*expansion.id, "forest-trail");
    assert_eq!(expansion.name, "Forest Trail");
    assert_eq!(
        expansion.substages(),
        [StageId::new(1, 1, 0), StageId::new(1, 2, 0)]
    );
    assert_eq!(expansion.round_name(1), Some("The Hidden Trail"));
    let library = Library::new(sample().parent().unwrap().to_owned(), &KIND);
    assert!(library.errors.is_empty(), "{:?}", library.errors);
    assert_eq!(library.packs.len(), 1);
    assert_eq!(library.packs[0].name, "Forest Trail");
}

#[test]
fn manifests_are_checked() {
    let manifest = |id: &str, version: u32| {
        format!(
            r#"{{"version": {version}, "id": "{id}", "name": "X", "rounds": [{{"substages": [{{"parts": [{{"base": "1-1:0"}}]}}]}}]}}"#
        )
    };
    assert!(Expansion::open(&pack("ok", &manifest("ok", 1), &[])).is_ok());
    let error = Expansion::open(&pack("version", &manifest("v", 2), &[])).unwrap_err();
    assert!(error.contains("version"), "{error}");
    let error = Expansion::open(&pack("id", &manifest("a b", 1), &[])).unwrap_err();
    assert!(error.contains("id"), "{error}");
    let error = Expansion::open(&pack(
        "base",
        r#"{"version": 1, "id": "b", "name": "B", "rounds": [{"substages": [{"parts": [{"base": "one"}]}]}]}"#,
        &[],
    ))
    .unwrap_err();
    assert!(error.contains("stage 1-1 part 0"), "{error}");
    let error = Expansion::open(&pack(
        "empty",
        r#"{"version": 1, "id": "e", "name": "E", "rounds": []}"#,
        &[],
    ))
    .unwrap_err();
    assert!(error.contains("rounds"), "{error}");
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn the_sample_expansion_plays_its_own_layout_and_enemies() {
    let rom = cartridge();
    let expansion = Arc::new(Expansion::open(&sample()).unwrap());
    expansion.check(&rom).unwrap();
    let game = Game::load_from(&rom, expansion.source(), StageId::FOREST).unwrap();
    assert_eq!(game.content(), Content::Expansion("forest-trail".into()));
    assert_eq!(
        game.rules(),
        StageRules {
            routines: StageId::FOREST,
            markers: false
        }
    );
    // The first pit (columns 32..35) has no floor; the ground beside it does.
    let terrain = game.assets();
    assert!(crate::game::Terrain::solid_at(terrain, 31 * 16, 500));
    assert!(!crate::game::Terrain::solid_at(terrain, 33 * 16, 500));
    assert_eq!(game.assets().data.camera.x, (0, 2560 - WIDTH as i32));
    let kinds: Vec<u16> = game.combat.spawns.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [0x14, 0x24, 0x14, 0x14, 0x24, 0x14, 0x24, 0x14, 0x14]
    );
    assert!(game.combat.spawns.iter().all(|s| s.source >= SPAWN_SOURCE));
    assert!(game.objects.objects.is_empty() && game.objects.pickups.is_empty());
    assert_eq!(game.player.position(), (32, 496));
    assert_eq!(game.next_stage(), Some(StageId::new(1, 2, 0)));

    // 1-2 is Boss 1's arena as it is: its boss, and nothing after it.
    let arena = game.load_next(StageId::new(1, 2, 0)).unwrap();
    assert!(
        arena
            .combat
            .spawns
            .iter()
            .any(|s| s.kind == crate::bosses::BOSS_KIND)
    );
    assert_eq!(arena.rules().routines, StageId::new(1, 3, 0));
    assert_eq!(arena.next_stage(), None);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn parts_only_place_enemies_their_base_has_art_for() {
    let rom = cartridge();
    let manifest = r#"{"version": 1, "id": "art", "name": "Art", "rounds": [{"substages": [{"parts": [
        {"base": "1-1:0", "enemies": [{"kind": "98", "x": 400, "y": 496}]}
    ]}]}]}"#;
    let expansion = Arc::new(Expansion::open(&pack("art", manifest, &[])).unwrap());
    let error = expansion.check(&rom).unwrap_err();
    assert!(
        error.contains("enemy type 98 has no art in base 1-1"),
        "{error}"
    );
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn expansion_runs_leave_no_trace_in_the_original_game() {
    use crate::{
        input::{Button, Input, press},
        modes::Modes,
    };
    let rom = cartridge();
    let dir = std::env::temp_dir().join(format!("ninja-expansion-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("continue.nms");
    let front = crate::front_end::Front::load(&rom).unwrap();
    let library = Library::new(sample().parent().unwrap().to_owned(), &KIND);
    let screen = ExpansionScreen::with_library(front, library, None, String::new());
    let mut modes = Modes::new(Box::new(screen));
    modes.enable_progress(Some(path.clone()), &rom);
    let until = |modes: &mut Modes, name: &str| {
        for _ in 0..2000 {
            if modes.name() == name {
                return;
            }
            modes.tick(Input::default());
        }
        panic!("never reached {name}; in {}", modes.name());
    };

    modes.tick(press(Button::Start));
    until(&mut modes, "Game");
    let game = modes.game().unwrap();
    assert_eq!(game.content(), Content::Expansion("forest-trail".into()));
    assert_eq!(game.run.id, 0, "no profile identity");
    for _ in 0..600 {
        modes.tick(Input::default());
    }
    modes.checkpoint();
    assert!(!path.exists(), "no Continue checkpoint");
    assert!(modes.profile.data.credited.is_empty());
    assert_eq!(modes.profile.data.lifetime, Default::default());
    assert_eq!(modes.profile.data.practice, Default::default());

    // Finishing the last stage returns to the menu, not the ending.
    modes.game_mut().unwrap().request_next_substage();
    for _ in 0..600 {
        if modes
            .game()
            .is_some_and(|g| g.stage_id == StageId::new(1, 2, 0))
        {
            break;
        }
        modes.tick(Input::default());
    }
    assert_eq!(modes.game().unwrap().stage_id, StageId::new(1, 2, 0));
    modes.game_mut().unwrap().request_next_substage();
    until(&mut modes, "Expansions");
    assert!(!path.exists());
    std::fs::remove_dir_all(dir).unwrap();
}
