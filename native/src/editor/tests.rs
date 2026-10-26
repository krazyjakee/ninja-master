use super::*;
use crate::{
    expansion::{LAYOUT_COLUMNS, MANIFEST},
    game::Game,
};
use std::path::PathBuf;

fn samples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("sample-expansions")
}

fn cartridge() -> Arc<[u8]> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(std::fs::read(directory.join("shinobi3_us.bin")).unwrap())
        .unwrap()
        .into()
}

/// A fresh temporary folder.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ninja-editor-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

const FIRST: PartRef = PartRef::new(0, 0, 0);

#[test]
fn grids_read_and_write_layout_files() {
    let mut text = String::new();
    for row in 0..LAYOUT_ROWS {
        text += &if row == 31 { "=" } else { "." }.repeat(24);
        text.push('\n');
    }
    let grid = Grid::parse(&text).unwrap();
    assert_eq!(grid.width(), 24);
    assert_eq!(grid.get(3, 31), Some('='));
    assert_eq!(grid.text(), text);
    // Trailing blank lines are allowed, as the game allows them.
    assert_eq!(Grid::parse(&(text.clone() + "\n")).unwrap(), grid);
    assert!(Grid::parse(&text.replace("=\n", "\n")).is_err(), "ragged");
    assert!(Grid::parse(&"..\n".repeat(32)).is_err(), "too narrow");
    let wide = format!("{}\n", ".".repeat(LAYOUT_COLUMNS + 1)).repeat(32);
    assert!(Grid::parse(&wide).is_err(), "too wide");
}

#[test]
fn painting_takes_legend_characters_and_undoes_by_stroke() {
    let mut doc = Document::new("test", "Test");
    assert!(doc.grid(FIRST).is_none());
    assert!(doc.create_layout(FIRST, NEW_COLUMNS));
    assert_eq!(doc.grid(FIRST).unwrap().width(), NEW_COLUMNS);
    assert_eq!(doc.brush_at(FIRST, 0, 0), Some(Brush::Metatile(0)));

    // A new metatile takes the next free character.
    doc.begin_stroke();
    assert!(doc.paint(FIRST, 0, 31, Brush::Metatile(8)));
    assert!(doc.paint(FIRST, 1, 31, Brush::Metatile(8)));
    assert!(!doc.paint(FIRST, 1, 31, Brush::Metatile(8)), "unchanged");
    doc.end_stroke();
    let part = doc.part(FIRST).unwrap();
    assert_eq!(part.legend.get("="), Some(&"08".to_string()));
    assert_eq!(doc.grid(FIRST).unwrap().get(1, 31), Some('='));
    assert_eq!(doc.brush_at(FIRST, 1, 31), Some(Brush::Metatile(8)));

    // Metatile numbers are matched however the legend writes them.
    assert!(doc.fill(FIRST, (5, 30), (2, 31), Brush::Metatile(8)));
    assert_eq!(doc.part(FIRST).unwrap().legend.len(), 2);
    assert_eq!(doc.grid(FIRST).unwrap().get(4, 30), Some('='));

    // The fill undoes alone, then the whole stroke.
    assert!(doc.undo());
    assert_eq!(doc.grid(FIRST).unwrap().get(4, 30), Some('.'));
    assert_eq!(doc.grid(FIRST).unwrap().get(1, 31), Some('='));
    assert!(doc.undo());
    assert_eq!(doc.grid(FIRST).unwrap().get(1, 31), Some('.'));
    assert!(doc.part(FIRST).unwrap().legend.get("=").is_none());
    assert!(doc.redo());
    assert_eq!(doc.grid(FIRST).unwrap().get(1, 31), Some('='));

    // Widening repeats the last column, as the game pads layouts.
    doc.resize_layout(FIRST, 50);
    assert_eq!(doc.grid(FIRST).unwrap().width(), 50);
    assert_eq!(doc.grid(FIRST).unwrap().get(49, 31), Some('.'));
    doc.resize_layout(FIRST, 1);
    assert_eq!(doc.grid(FIRST).unwrap().width(), MIN_COLUMNS);
}

#[test]
fn typing_in_one_field_is_one_undo_step() {
    let mut doc = Document::new("test", "T");
    for name in ["Te", "Tes", "Test"] {
        doc.set_name(name);
    }
    doc.set_description("About");
    doc.set_name("Test 2");
    assert!(doc.undo());
    assert_eq!(doc.manifest().name, "Test");
    assert!(doc.undo());
    assert!(doc.undo());
    assert_eq!(doc.manifest().name, "T");
    assert!(!doc.can_undo());
}

#[test]
fn parts_are_added_moved_and_removed() {
    let mut doc = Document::new("test", "Test");
    let second = doc.add_part(FIRST).unwrap();
    assert_eq!(second, PartRef::new(0, 0, 1));
    assert_eq!(doc.part(second).unwrap().base, "1-1:1");
    let substage = doc.add_substage(second, StageId::new(1, 3, 0)).unwrap();
    assert_eq!(substage, PartRef::new(0, 1, 0));
    let round = doc.add_round(0, StageId::new(2, 1, 0));
    assert_eq!(round, PartRef::new(1, 0, 0));
    assert_eq!(doc.parts().len(), 4);
    assert_eq!(doc.manifest().rounds[1].name, "Round 2");

    assert_eq!(doc.move_part(second, -1), Some(FIRST));
    assert_eq!(doc.part(FIRST).unwrap().base, "1-1:1");
    assert_eq!(doc.move_part(FIRST, -1), None);
    assert_eq!(doc.move_round(round, -1), Some(FIRST));
    assert_eq!(doc.part(FIRST).unwrap().base, "2-1:0");

    // Removing a round's only part removes the round.
    assert_eq!(doc.remove_part(FIRST), Some(FIRST));
    assert_eq!(doc.manifest().rounds.len(), 1);
    assert_eq!(doc.parts().len(), 3);
    doc.remove_part(FIRST);
    doc.remove_part(FIRST);
    assert_eq!(doc.remove_part(FIRST), None, "the last part stays");
    assert_eq!(doc.parts(), [FIRST]);
}

#[test]
fn enemies_are_placed_moved_and_removed() {
    let mut doc = Document::new("test", "Test");
    assert!(doc.enemies(FIRST).is_none(), "the base's enemies");
    assert_eq!(doc.add_enemy(FIRST, 0x14, 400, 496), Some(0));
    assert_eq!(doc.add_enemy(FIRST, 0x200, 500, 496), Some(1));
    doc.move_enemy(FIRST, 0, 410, 480);
    doc.set_enemy_flags(FIRST, 0, Some(0x11));
    doc.set_enemy_kind(FIRST, 1, 0x24);
    let list = doc.enemies(FIRST).unwrap();
    assert_eq!(
        (list[0].kind.as_str(), list[0].x, list[0].y),
        ("14", 410, 480)
    );
    assert_eq!(list[0].flags.as_deref(), Some("0011"));
    assert_eq!(list[1].kind, "24");
    doc.remove_enemy(FIRST, 0);
    assert_eq!(doc.enemies(FIRST).unwrap().len(), 1);
    doc.set_own_enemies(FIRST, false);
    assert!(doc.enemies(FIRST).is_none());
}

#[test]
fn removing_an_art_cell_clears_it_from_the_layout() {
    let mut doc = Document::new("test", "Test");
    assert!(doc.set_art_sheet(FIRST, "art.png", Vec::new()).is_err());
    doc.create_layout(FIRST, NEW_COLUMNS);
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 32, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0; 32 * 16 * 4]).unwrap();
    }
    doc.set_art_sheet(FIRST, "art.png", png).unwrap();
    assert_eq!(doc.sheet_cells(FIRST), Some((2, 1)));
    let c = doc.add_art_cell(FIRST, [1, 0]).unwrap();
    assert_ne!(c, '.', "the legend's character is taken");
    assert_eq!(doc.add_art_cell(FIRST, [1, 0]), Some(c), "already a cell");
    doc.set_cell_collision(FIRST, c, Collision::Solid);
    doc.set_cell_front(FIRST, c, true);
    assert!(doc.paint(FIRST, 3, 3, Brush::Art(c)));
    assert_eq!(doc.brush_at(FIRST, 3, 3), Some(Brush::Art(c)));
    let cell = doc.cell(FIRST, c).unwrap();
    assert_eq!((cell.collision, cell.front), (Collision::Solid, true));
    doc.remove_art_cell(FIRST, c);
    assert!(doc.cell(FIRST, c).is_none());
    assert_eq!(doc.brush_at(FIRST, 3, 3), Some(Brush::Metatile(0)));
    assert!(!doc.paint(FIRST, 3, 3, Brush::Art(c)), "no such cell");
}

#[test]
fn the_samples_survive_an_open_and_save() {
    for name in ["forest-trail", "hangar-run"] {
        let path = samples().join(name);
        let mut doc = Document::open(&path).unwrap();
        assert!(!doc.dirty());
        let out = scratch(name);
        doc.save(&out).unwrap();
        assert!(!doc.dirty());
        assert_eq!(doc.path.as_deref(), Some(out.as_path()));
        // The manifest means the same; layouts and sheets are identical.
        let json = |dir: &Path| -> serde_json::Value {
            serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).unwrap()).unwrap()
        };
        assert_eq!(json(&out), json(&path), "{name}");
        for entry in std::fs::read_dir(&out).unwrap() {
            let file = entry.unwrap().file_name();
            if file != MANIFEST {
                assert_eq!(
                    std::fs::read(out.join(&file)).unwrap(),
                    std::fs::read(path.join(&file)).unwrap(),
                    "{name}/{file:?}"
                );
            }
        }
        let again = Document::open(&out).unwrap();
        assert_eq!(again.manifest(), doc.manifest());
        doc.set_name("Changed");
        assert!(doc.dirty());
        std::fs::remove_dir_all(out).unwrap();
    }
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn the_preview_draws_the_part_and_its_markers() {
    let rom = cartridge();
    let doc = Document::open(&samples().join("forest-trail")).unwrap();
    let mut editor = Editor::new(doc, rom);
    assert!(editor.error.is_none(), "{:?}", editor.error);
    let preview = editor.preview().unwrap();
    assert_eq!(preview.base, StageId::FOREST);
    assert_eq!(preview.size.1, 512);
    assert!(preview.own_enemies);
    assert_eq!(preview.enemies.len(), 11);
    assert_eq!((preview.start, preview.exit), ((32, 496), 2480));
    assert!(preview.kinds.iter().any(|k| k.kind == 0x200), "the monk");
    // Row 31 is ground: '=' is metatile 08, which is solid.
    assert_eq!(preview.metatile_at(5, 31), Some(0x08));
    assert_eq!(preview.material(0x08), 1);
    assert_eq!(preview.enemy_at(400, 480), Some(0));
    let (width, height, sheet) = preview.sheet(&[0, 8], 2);
    assert_eq!((width, height, sheet.len()), (32, 16, 32 * 16 * 3));

    let rgb = editor.render((0, 288), (400, 224)).unwrap();
    assert_eq!(rgb.len(), 400 * 224 * 3);
    let distinct: std::collections::HashSet<&[u8]> = rgb.chunks(3).collect();
    assert!(distinct.len() > 16, "the stage's art is drawn");
    // A view past the scene's end is shaded, not garbage.
    let rgb = editor.render((2400, 288), (400, 224)).unwrap();
    assert!(rgb.len() == 400 * 224 * 3);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn tools_paint_place_and_drag() {
    let rom = cartridge();
    let mut doc = Document::new("built", "Built");
    doc.create_layout(FIRST, NEW_COLUMNS);
    let mut editor = Editor::new(doc, rom);

    // Paint a floor along row 31 in one stroke, and pick it back up.
    editor.brush = Brush::Metatile(0x08);
    editor.pointer_down((8, 500), false);
    editor.pointer_move(Some((NEW_COLUMNS as i32 * 16 - 1, 500)));
    editor.pointer_up();
    assert_eq!(
        editor.preview().unwrap().metatile_at(NEW_COLUMNS - 1, 31),
        Some(8)
    );
    editor.brush = Brush::Metatile(0);
    editor.pointer_down((40, 500), true);
    assert_eq!(editor.brush, Brush::Metatile(8));

    // A rectangle fills on release.
    editor.tool = Tool::Rectangle;
    editor.pointer_down((160, 448), false);
    editor.pointer_move(Some((200, 470)));
    assert_eq!(editor.preview().unwrap().metatile_at(10, 28), Some(0));
    editor.pointer_up();
    assert_eq!(editor.preview().unwrap().metatile_at(12, 29), Some(8));

    // Enemies: place, drag, delete.
    editor.tool = Tool::Enemies;
    editor.kind = 0x14;
    editor.pointer_down((300, 495), false);
    assert_eq!(editor.selected_enemy, Some(0));
    let e = &editor.document.enemies(FIRST).unwrap()[0];
    assert_eq!((e.x, e.y), (304, 496), "snapped");
    editor.pointer_down((304, 490), false);
    editor.pointer_move(Some((400, 490)));
    editor.pointer_up();
    assert_eq!(editor.document.enemies(FIRST).unwrap()[0].x, 400);
    assert_eq!(editor.preview().unwrap().enemies[0].x, 400);

    // The start and exit follow the pointer.
    editor.tool = Tool::Start;
    editor.pointer_down((48, 496), false);
    editor.pointer_up();
    editor.tool = Tool::Exit;
    editor.pointer_down((600, 300), false);
    editor.pointer_up();
    let preview = editor.preview().unwrap();
    assert_eq!((preview.start, preview.exit), ((48, 496), 600));

    // One undo step per gesture.
    editor.undo();
    assert!(editor.document.part(FIRST).unwrap().exit.is_none());
    editor.undo();
    editor.undo();
    assert_eq!(editor.document.enemies(FIRST).unwrap()[0].x, 304);

    assert!(editor.problems().is_empty(), "{:?}", editor.problems());
    let dir = scratch("playtest");
    editor.export_playtest(&dir).unwrap();
    let expansion = Arc::new(Expansion::open(&dir).unwrap());
    Game::load_from(editor.rom(), expansion.source(), StageId::FOREST).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn problems_name_the_part_they_are_in() {
    let rom = cartridge();
    let mut doc = Document::open(&samples().join("forest-trail")).unwrap();
    let at = doc.add_part(FIRST).unwrap();
    // No forest part loads art for type $FC.
    doc.add_enemy(at, 0xfc, 100, 496);
    let mut editor = Editor::new(doc, rom);
    let problems = editor.problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].0, Some(at));
    assert!(problems[0].1.contains("FC"), "{problems:?}");
    editor.select(at);
    assert!(editor.preview().unwrap().problem.is_some());
    editor.document.set_base(at, StageId::new(9, 9, 0));
    editor.refresh();
    assert!(editor.error.is_some());
    editor.undo();
    assert!(editor.error.is_none(), "{:?}", editor.error);
    editor.undo();
    assert!(editor.problems().is_empty());
}
