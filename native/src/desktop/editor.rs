//! The expansion editor's window (`--editor`): Dear ImGui panels around the
//! stage. Editing, drawing and checking live in
//! `ninja_master_native::editor`; this turns the mouse and keys into its
//! calls and starts playtests as separate game processes.
use super::{gui::Gui, set_icon};
use imgui::{Condition, Key, MouseButton, TextureId, TreeNodeFlags, Ui, WindowFlags};
use ninja_master_native::{
    editor::{
        Brush, Collision, Document, Editor, NEW_COLUMNS, Overlays, PartRef, Tool, bases,
        code_names, material_name, stage_text,
    },
    expansion::{KIND, Library, library_directory},
    stage::StageId,
};
use sdl2::{event::Event, pixels::Color};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command},
};

/// Gui image ids.
const STAGE: usize = 1;
const PALETTE: usize = 2;
const SHEET: usize = 3;
const LEFT_WIDTH: f32 = 330.0;
const RIGHT_WIDTH: f32 = 400.0;
const PALETTE_CELL: f32 = 32.0;

pub(crate) fn run(rom: Vec<u8>, pack: Option<&Path>, rom_arg: Option<&Path>) -> Result<(), String> {
    let document = match pack {
        Some(path) => Document::open(path).map_err(|e| format!("{}: {e}", path.display()))?,
        None => Document::new("my-expansion", "My Expansion"),
    };
    let mut editor = Editor::new(document, rom.into());
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    sdl2::hint::set("SDL_RENDER_SCALE_QUALITY", "0");
    let mut window = video
        .window("Ninja Master — Expansion Editor", 1600, 960)
        .position_centered()
        .resizable()
        .allow_highdpi()
        .build()
        .map_err(|e| e.to_string())?;
    set_icon(&mut window)?;
    let mut canvas = window
        .into_canvas()
        .present_vsync()
        .build()
        .map_err(|e| e.to_string())?;
    let creator = canvas.texture_creator();
    let ini = ninja_master_native::settings::config_path()
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| dir.join("editor.ini"));
    let mut gui = Gui::new(&creator, ini)?;
    video.text_input().start();
    let mut events = sdl.event_pump()?;
    let mut app = App::new(&editor, rom_arg);
    while !app.quit {
        for event in events.poll_iter() {
            gui.handle_event(&event);
            match event {
                Event::Quit { .. } => app.request(Pending::Quit, &editor),
                Event::RenderTargetsReset { .. } | Event::RenderDeviceReset { .. } => {
                    gui.reset_textures(&creator)?;
                    app.textures_lost();
                }
                _ => {}
            }
        }
        app.upload(&mut gui, &creator, &mut editor)?;
        canvas.set_draw_color(Color::RGB(14, 15, 18));
        canvas.clear();
        gui.draw(&mut canvas, |ui| app.ui(ui, &mut editor))?;
        canvas.present();
        app.poll_playtest();
    }
    if let Some(mut child) = app.playtest.take() {
        let _ = child.kill();
    }
    Ok(())
}

/// A request that would lose unsaved changes.
#[derive(Clone, Debug)]
enum Pending {
    Quit,
    New,
    Open(PathBuf),
}

/// What the stage texture shows, to redraw it only when this changes.
#[derive(Clone, Debug, PartialEq)]
struct StageKey {
    revision: u64,
    part: PartRef,
    origin: (i32, i32),
    size: (usize, usize),
    overlays: Overlays,
    highlight: ninja_master_native::editor::Highlight,
}

struct App {
    /// The stage view's top-left corner in world pixels.
    camera: (f32, f32),
    zoom: f32,
    /// The stage view's size in screen pixels.
    view: [f32; 2],
    stage: Option<StageKey>,
    /// The palette sheet: its brushes in order, and what it was drawn for.
    palette: Vec<Brush>,
    palette_key: Option<(u64, PartRef, bool)>,
    palette_size: (usize, usize),
    all_metatiles: bool,
    sheet_key: Option<(u64, PartRef)>,
    sheet_size: (usize, usize),
    status: String,
    problems: Option<Vec<(Option<PartRef>, String)>>,
    open_path: String,
    save_path: String,
    sheet_path: String,
    new_width: i32,
    bases: Vec<StageId>,
    library: Library,
    playtest: Option<Child>,
    rom_arg: Option<PathBuf>,
    /// The left button went down on the stage and has not come up.
    dragging: bool,
    pending: Option<Pending>,
    /// The pending request may go ahead: saved, discarded or nothing to lose.
    confirmed: bool,
    open_popup: Option<&'static str>,
    quit: bool,
}

impl App {
    fn new(editor: &Editor, rom_arg: Option<&Path>) -> Self {
        let mut app = Self {
            camera: (0.0, 0.0),
            zoom: 2.0,
            view: [960.0, 800.0],
            stage: None,
            palette: Vec::new(),
            palette_key: None,
            palette_size: (0, 0),
            all_metatiles: false,
            sheet_key: None,
            sheet_size: (0, 0),
            status: String::new(),
            problems: None,
            open_path: String::new(),
            save_path: String::new(),
            sheet_path: String::new(),
            new_width: NEW_COLUMNS as i32,
            bases: bases(editor.rom()),
            library: Library::new(library_directory(), &KIND),
            playtest: None,
            rom_arg: rom_arg.map(Path::to_owned),
            dragging: false,
            pending: None,
            confirmed: false,
            open_popup: None,
            quit: false,
        };
        app.frame_part(editor);
        app
    }

    fn textures_lost(&mut self) {
        self.stage = None;
        self.palette_key = None;
        self.sheet_key = None;
    }

    /// Put the camera on the part's start.
    fn frame_part(&mut self, editor: &Editor) {
        if let Some(preview) = editor.preview() {
            let (w, h) = self.world_view();
            self.camera = (
                (preview.start.0 as f32 - w * 0.3).max(-16.0),
                (preview.start.1 as f32 - h * 0.7).max(-16.0),
            );
        }
    }
    /// The stage view's size in world pixels.
    fn world_view(&self) -> (f32, f32) {
        (
            (self.view[0] / self.zoom).max(16.0),
            (self.view[1] / self.zoom).max(16.0),
        )
    }
    fn clamp_camera(&mut self, editor: &Editor) {
        let Some(preview) = editor.preview() else {
            return;
        };
        let (w, h) = self.world_view();
        let (sw, sh) = (preview.size.0 as f32, preview.size.1 as f32);
        let clamp = |v: f32, view: f32, size: f32| {
            if view >= size {
                (size - view) / 2.0
            } else {
                v.clamp(-32.0, size - view + 32.0)
            }
        };
        self.camera = (clamp(self.camera.0, w, sw), clamp(self.camera.1, h, sh));
    }

    /// Redraw the stage, palette and sheet textures that went stale.
    fn upload<'t>(
        &mut self,
        gui: &mut Gui<'t>,
        creator: &'t sdl2::render::TextureCreator<sdl2::video::WindowContext>,
        editor: &mut Editor,
    ) -> Result<(), String> {
        editor.refresh();
        self.clamp_camera(editor);
        let (w, h) = self.world_view();
        let key = StageKey {
            revision: editor.document.revision(),
            part: editor.selected,
            origin: (self.camera.0.floor() as i32, self.camera.1.floor() as i32),
            size: (w.ceil() as usize, h.ceil() as usize),
            overlays: editor.overlays,
            highlight: editor.highlight(),
        };
        if self.stage.as_ref() != Some(&key)
            && let Some(rgb) = editor.render(key.origin, key.size)
        {
            gui.set_image(creator, STAGE, &rgb, key.size.0 as u32, key.size.1 as u32)?;
            self.stage = Some(key);
        }
        let palette_key = (
            editor.document.revision(),
            editor.selected,
            self.all_metatiles,
        );
        if self.palette_key != Some(palette_key)
            && let Some(preview) = editor.preview()
        {
            let at = editor.selected;
            let art_ids: Vec<u8> = preview.art.iter().map(|&(_, id)| id).collect();
            let mut metatiles: Vec<u8> = if self.all_metatiles {
                (0..preview.metatile_count() as u16)
                    .map(|m| m as u8)
                    .collect()
            } else {
                preview.base_metatiles.clone()
            };
            if let Some(grid) = editor.document.grid(at) {
                for col in 0..grid.width() {
                    for row in 0..32 {
                        if let Some(Brush::Metatile(m)) = editor.document.brush_at(at, col, row) {
                            metatiles.push(m);
                        }
                    }
                }
            }
            metatiles.sort_unstable();
            metatiles.dedup();
            metatiles.retain(|m| !art_ids.contains(m));
            self.palette = metatiles
                .iter()
                .map(|&m| Brush::Metatile(m))
                .chain(preview.art.iter().map(|&(c, _)| Brush::Art(c)))
                .collect();
            let ids: Vec<u8> = metatiles.iter().copied().chain(art_ids).collect();
            let (width, height, rgb) = preview.sheet(&ids, 16);
            gui.set_image(creator, PALETTE, &rgb, width as u32, height as u32)?;
            self.palette_size = (width, height);
            self.palette_key = Some(palette_key);
        }
        let sheet_key = (editor.document.revision(), editor.selected);
        if self.sheet_key != Some(sheet_key) {
            self.sheet_key = Some(sheet_key);
            self.sheet_size = (0, 0);
            if let Some((width, height, rgb)) = editor.document.sheet_rgb(editor.selected) {
                gui.set_image(creator, SHEET, &rgb, width as u32, height as u32)?;
                self.sheet_size = (width, height);
            }
        }
        Ok(())
    }

    fn poll_playtest(&mut self) {
        if let Some(child) = &mut self.playtest
            && let Ok(Some(status)) = child.try_wait()
        {
            self.status = if status.success() {
                "Playtest finished.".into()
            } else {
                format!("Playtest ended: {status}")
            };
            self.playtest = None;
        }
    }

    /// Play the pack as it stands, from `at` or from its start, in a
    /// separate game window.
    fn playtest(&mut self, editor: &Editor, at: Option<PartRef>) {
        let dir =
            std::env::temp_dir().join(format!("ninja-master-playtest-{}", std::process::id()));
        if let Err(e) = editor.export_playtest(&dir) {
            self.status = format!("Can't playtest: {e}");
            return;
        }
        if let Some(mut child) = self.playtest.take() {
            let _ = child.kill();
        }
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                self.status = format!("Can't playtest: {e}");
                return;
            }
        };
        let stage = at.unwrap_or_default().stage();
        let mut command = Command::new(exe);
        command.arg("--play").arg("--expansion").arg(&dir).args([
            "--round".to_string(),
            stage.round.to_string(),
            "--substage".into(),
            stage.substage.to_string(),
            "--part".into(),
            stage.part.to_string(),
        ]);
        if let Some(rom) = &self.rom_arg {
            command.arg("--rom").arg(rom);
        }
        match command.spawn() {
            Ok(child) => {
                self.playtest = Some(child);
                self.status = format!(
                    "Playtesting from {}-{} part {}. Close the game window to stop.",
                    stage.round, stage.substage, stage.part
                );
            }
            Err(e) => self.status = format!("Can't playtest: {e}"),
        }
    }

    /// Carry out `pending` next frame, asking first if it would lose changes.
    fn request(&mut self, pending: Pending, editor: &Editor) {
        self.pending = Some(pending);
        self.confirmed = !editor.document.dirty();
        if !self.confirmed {
            self.open_popup = Some("Unsaved changes");
        }
    }
    fn replace(&mut self, pending: Pending, editor: &mut Editor) {
        let document = match pending {
            Pending::New => Ok(Document::new("my-expansion", "My Expansion")),
            Pending::Open(path) => {
                Document::open(&path).map_err(|e| format!("{}: {e}", path.display()))
            }
            Pending::Quit => return,
        };
        match document {
            Ok(document) => {
                editor.replace_document(document);
                self.problems = None;
                self.status = match &editor.document.path {
                    Some(path) => format!("Opened {}", path.display()),
                    None => "New pack.".into(),
                };
                self.textures_lost();
                self.frame_part(editor);
            }
            Err(e) => self.status = e,
        }
    }

    fn save(&mut self, editor: &mut Editor, dir: Option<PathBuf>) {
        let Some(dir) = dir.or_else(|| editor.document.path.clone()) else {
            self.save_path = library_directory()
                .join(&editor.document.manifest().id)
                .display()
                .to_string();
            self.open_popup = Some("Save as");
            return;
        };
        self.status = match editor.document.save(&dir) {
            Ok(()) => {
                self.library.rescan();
                format!("Saved {}", dir.display())
            }
            Err(e) => format!("Couldn't save: {e}"),
        };
    }

    fn ui(&mut self, ui: &Ui, editor: &mut Editor) {
        if self.confirmed {
            self.confirmed = false;
            match self.pending.take() {
                Some(Pending::Quit) => self.quit = true,
                Some(pending) => self.replace(pending, editor),
                None => {}
            }
        }
        self.shortcuts(ui, editor);
        let menu_height = self.menu(ui, editor);
        let [width, height] = ui.io().display_size;
        let panel = |name: &'static str, x: f32, w: f32| {
            ui.window(name)
                .position([x, menu_height], Condition::Always)
                .size([w, height - menu_height], Condition::Always)
                .flags(WindowFlags::NO_MOVE | WindowFlags::NO_RESIZE | WindowFlags::NO_COLLAPSE)
        };
        // Leave room for the labels beside the fields.
        panel("Pack", 0.0, LEFT_WIDTH).build(|| {
            let _width = ui.push_item_width(-100.0);
            self.pack_panel(ui, editor)
        });
        panel("Part", width - RIGHT_WIDTH, RIGHT_WIDTH).build(|| {
            let _width = ui.push_item_width(-120.0);
            self.part_panel(ui, editor)
        });
        ui.window("Stage")
            .position([LEFT_WIDTH, menu_height], Condition::Always)
            .size(
                [width - LEFT_WIDTH - RIGHT_WIDTH, height - menu_height],
                Condition::Always,
            )
            .flags(
                WindowFlags::NO_MOVE
                    | WindowFlags::NO_RESIZE
                    | WindowFlags::NO_COLLAPSE
                    | WindowFlags::NO_SCROLLBAR
                    | WindowFlags::NO_SCROLL_WITH_MOUSE,
            )
            .build(|| self.stage_panel(ui, editor));
        if self.problems.is_some() {
            self.problems_window(ui, editor);
        }
        self.popups(ui, editor);
    }

    fn shortcuts(&mut self, ui: &Ui, editor: &mut Editor) {
        let io = ui.io();
        if io.want_text_input {
            return;
        }
        let ctrl = io.key_ctrl;
        let shift = io.key_shift;
        if ctrl && ui.is_key_pressed(Key::Z) {
            if shift { editor.redo() } else { editor.undo() }
        }
        if ctrl && ui.is_key_pressed(Key::Y) {
            editor.redo();
        }
        if ctrl && ui.is_key_pressed(Key::S) {
            self.save(editor, None);
        }
        if ctrl {
            return;
        }
        for (key, tool) in [
            (Key::Alpha1, Tool::Paint),
            (Key::Alpha2, Tool::Rectangle),
            (Key::Alpha3, Tool::Enemies),
            (Key::Alpha4, Tool::Start),
            (Key::Alpha5, Tool::Exit),
        ] {
            if ui.is_key_pressed(key) {
                editor.tool = tool;
            }
        }
        if ui.is_key_pressed(Key::G) {
            editor.overlays.grid = !editor.overlays.grid;
        }
        if ui.is_key_pressed(Key::C) {
            editor.overlays.collision = !editor.overlays.collision;
        }
        if ui.is_key_pressed(Key::B) {
            editor.overlays.background = !editor.overlays.background;
        }
        if ui.is_key_pressed(Key::F5) {
            self.playtest(editor, Some(editor.selected));
        }
        if ui.is_key_pressed(Key::F6) {
            self.playtest(editor, None);
        }
        if (ui.is_key_pressed(Key::Delete) || ui.is_key_pressed(Key::Backspace))
            && let Some(index) = editor.selected_enemy.take()
        {
            editor.document.remove_enemy(editor.selected, index);
        }
        let step = 64.0 / self.zoom * 4.0;
        for (key, dx, dy) in [
            (Key::LeftArrow, -step, 0.0),
            (Key::RightArrow, step, 0.0),
            (Key::UpArrow, 0.0, -step),
            (Key::DownArrow, 0.0, step),
        ] {
            if ui.is_key_pressed(key) {
                self.camera.0 += dx;
                self.camera.1 += dy;
            }
        }
    }

    fn menu(&mut self, ui: &Ui, editor: &mut Editor) -> f32 {
        let mut height = 0.0;
        ui.main_menu_bar(|| {
            height = ui.window_size()[1];
            ui.menu("File", || {
                if ui.menu_item("New") {
                    self.request(Pending::New, editor);
                }
                if ui.menu_item("Open...") {
                    self.library.rescan();
                    self.open_popup = Some("Open pack");
                }
                if ui.menu_item_config("Save").shortcut("Ctrl+S").build() {
                    self.save(editor, None);
                }
                if ui.menu_item("Save as...") {
                    self.save_path = editor
                        .document
                        .path
                        .clone()
                        .unwrap_or_else(|| library_directory().join(&editor.document.manifest().id))
                        .display()
                        .to_string();
                    self.open_popup = Some("Save as");
                }
                ui.separator();
                if ui.menu_item("Quit") {
                    self.request(Pending::Quit, editor);
                }
            });
            ui.menu("Edit", || {
                if ui
                    .menu_item_config("Undo")
                    .shortcut("Ctrl+Z")
                    .enabled(editor.document.can_undo())
                    .build()
                {
                    editor.undo();
                }
                if ui
                    .menu_item_config("Redo")
                    .shortcut("Ctrl+Y")
                    .enabled(editor.document.can_redo())
                    .build()
                {
                    editor.redo();
                }
            });
            ui.menu("View", || {
                let overlays = &mut editor.overlays;
                ui.menu_item_config("Grid")
                    .shortcut("G")
                    .build_with_ref(&mut overlays.grid);
                ui.menu_item_config("Collision")
                    .shortcut("C")
                    .build_with_ref(&mut overlays.collision);
                ui.menu_item_config("Enemies, start and exit")
                    .build_with_ref(&mut overlays.objects);
                ui.menu_item_config("Background")
                    .shortcut("B")
                    .build_with_ref(&mut overlays.background);
                ui.separator();
                ui.menu_item_config("Snap positions")
                    .build_with_ref(&mut editor.snap);
                for zoom in [1.0, 2.0, 3.0, 4.0] {
                    if ui
                        .menu_item_config(format!("Zoom {zoom}x"))
                        .selected(self.zoom == zoom)
                        .build()
                    {
                        self.zoom = zoom;
                    }
                }
            });
            ui.menu("Pack", || {
                if ui.menu_item("Check for problems") {
                    self.problems = Some(editor.problems());
                }
                if ui
                    .menu_item_config("Playtest this part")
                    .shortcut("F5")
                    .build()
                {
                    self.playtest(editor, Some(editor.selected));
                }
                if ui
                    .menu_item_config("Playtest from the start")
                    .shortcut("F6")
                    .build()
                {
                    self.playtest(editor, None);
                }
            });
            let title = format!(
                "{}{}",
                editor.document.manifest().name,
                if editor.document.dirty() { " *" } else { "" }
            );
            ui.text_disabled(title);
            if !self.status.is_empty() {
                ui.same_line();
                ui.text_disabled(format!("  |  {}", self.status));
            }
        });
        height
    }

    fn pack_panel(&mut self, ui: &Ui, editor: &mut Editor) {
        let doc = &mut editor.document;
        let mut id = doc.manifest().id.clone();
        if ui.input_text("ID", &mut id).build() {
            doc.set_id(&id);
        }
        let mut name = doc.manifest().name.clone();
        if ui.input_text("Name", &mut name).build() {
            doc.set_name(&name);
        }
        let mut description = doc.manifest().description.clone();
        if ui
            .input_text_multiline("Description", &mut description, [0.0, 54.0])
            .build()
        {
            doc.set_description(&description);
        }
        let mut codes = vec!["(none)"];
        codes.extend(code_names());
        let mut code = doc
            .manifest()
            .code
            .as_deref()
            .and_then(|c| codes.iter().position(|n| *n == c))
            .unwrap_or(0);
        if ui.combo_simple_string("Code", &mut code, &codes) {
            doc.set_code((code > 0).then(|| codes[code]));
        }
        hint(
            ui,
            "Compiled behaviour the pack uses: new enemy types and bosses.",
        );
        ui.text_disabled(match &doc.path {
            Some(path) => path.display().to_string(),
            None => "Not saved yet".into(),
        });
        ui.separator();

        let mut select = None;
        for (r, round) in doc.manifest().rounds.iter().enumerate() {
            let label = format!("Round {}: {}###round{r}", r + 1, round.name);
            let Some(_round) = ui
                .tree_node_config(label)
                .flags(TreeNodeFlags::DEFAULT_OPEN)
                .push()
            else {
                continue;
            };
            for (s, substage) in round.substages.iter().enumerate() {
                let label = format!("Stage {}-{}###stage{r}-{s}", r + 1, s + 1);
                let Some(_substage) = ui
                    .tree_node_config(label)
                    .flags(TreeNodeFlags::DEFAULT_OPEN)
                    .push()
                else {
                    continue;
                };
                for (p, part) in substage.parts.iter().enumerate() {
                    let at = PartRef::new(r, s, p);
                    let mut label = format!("Part {p} on {}", part.base);
                    if part.layout.is_some() {
                        label += "  (layout)";
                    }
                    if ui
                        .selectable_config(format!("{label}###part{r}-{s}-{p}"))
                        .selected(editor.selected == at)
                        .build()
                    {
                        select = Some(at);
                    }
                }
            }
        }
        ui.separator();
        let at = editor.selected;
        let base = editor
            .document
            .part(at)
            .and_then(|p| StageId::parse(&p.base).ok())
            .unwrap_or(StageId::FOREST);
        if ui.button("+ Part") {
            select = editor.document.add_part(at);
        }
        hint(ui, "A part after this one, on the next part of its base.");
        ui.same_line();
        if ui.button("+ Stage") {
            select = editor.document.add_substage(at, base.with_part(0));
        }
        ui.same_line();
        if ui.button("+ Round") {
            select = Some(editor.document.add_round(at.round, base.with_part(0)));
        }
        if ui.button("Remove part") {
            select = editor.document.remove_part(at);
        }
        for (label, level) in [("Part", 0), ("Stage", 1), ("Round", 2)] {
            ui.text(label);
            for (arrow, delta) in [("Up", -1), ("Down", 1)] {
                ui.same_line();
                if ui.button(format!("{arrow}##{label}")) {
                    let doc = &mut editor.document;
                    select = match level {
                        0 => doc.move_part(at, delta),
                        1 => doc.move_substage(at, delta),
                        _ => doc.move_round(at, delta),
                    };
                }
            }
        }
        if let Some(at) = select {
            editor.refresh();
            editor.select(at);
            self.frame_part(editor);
        }
    }

    fn part_panel(&mut self, ui: &Ui, editor: &mut Editor) {
        let at = editor.selected;
        let Some(part) = editor.document.part(at).cloned() else {
            return;
        };
        let stage = at.stage();
        ui.text(format!(
            "Stage {}-{} part {}",
            stage.round, stage.substage, stage.part
        ));
        if let Some(error) = &editor.error {
            ui.text_colored([1.0, 0.45, 0.45, 1.0], format!("Doesn't load: {error}"));
        }
        if let Some(problem) = editor.preview().and_then(|p| p.problem.as_ref()) {
            ui.text_colored([1.0, 0.7, 0.3, 1.0], format!("Enemies: {problem}"));
        }
        let mut round_name = editor.document.manifest().rounds[at.round].name.clone();
        if ui.input_text("Round name", &mut round_name).build() {
            editor.document.set_round_name(at.round, &round_name);
        }
        let labels: Vec<String> = self.bases.iter().map(|&b| stage_text(b)).collect();
        let base = StageId::parse(&part.base).ok();
        let mut index = base
            .and_then(|b| self.bases.iter().position(|&x| x == b))
            .unwrap_or(0);
        if ui.combo_simple_string("Base", &mut index, &labels) {
            editor.document.set_base(at, self.bases[index]);
        }
        hint(
            ui,
            "The original part this one is built on: its art, palettes, music, enemy art, boss and stage rules.",
        );
        let mut music_labels = vec!["(the base's)".to_string()];
        music_labels.extend(labels.iter().cloned());
        let mut music = part
            .music
            .as_deref()
            .and_then(|m| StageId::parse(m).ok())
            .and_then(|m| self.bases.iter().position(|&x| x == m))
            .map_or(0, |i| i + 1);
        if ui.combo_simple_string("Music", &mut music, &music_labels) {
            editor
                .document
                .set_music(at, (music > 0).then(|| self.bases[music - 1]));
        }

        if ui.collapsing_header("Layout", TreeNodeFlags::DEFAULT_OPEN) {
            match editor.document.grid(at).map(|g| g.width()) {
                None => {
                    ui.text_wrapped("This part uses its base's layout, stage objects and enemies.");
                    ui.input_int("Columns", &mut self.new_width).build();
                    if ui.button("Create a blank layout") {
                        editor
                            .document
                            .create_layout(at, self.new_width.max(1) as usize);
                        editor.tool = Tool::Paint;
                    }
                    hint(
                        ui,
                        "Replaces the base's layout, objects and enemies with your own.",
                    );
                }
                Some(width) => {
                    ui.text_disabled(part.layout.clone().unwrap_or_default());
                    let mut columns = width as i32;
                    if ui.input_int("Columns", &mut columns).build() && columns > 0 {
                        editor.document.resize_layout(at, columns as usize);
                    }
                    if ui.button("Remove layout") {
                        editor.document.remove_layout(at);
                    }
                    hint(ui, "Go back to the base's layout. Undo brings it back.");
                }
            }
        }
        if ui.collapsing_header("Tools", TreeNodeFlags::DEFAULT_OPEN) {
            let layout = part.layout.is_some();
            for (tool, label, enabled) in [
                (Tool::Paint, "Paint (1)", layout),
                (Tool::Rectangle, "Rectangle (2)", layout),
                (Tool::Enemies, "Enemies (3)", true),
                (Tool::Start, "Start (4)", true),
                (Tool::Exit, "Exit (5)", true),
            ] {
                ui.disabled(!enabled, || {
                    if ui.radio_button_bool(label, editor.tool == tool) {
                        editor.tool = tool;
                    }
                });
            }
            ui.text_disabled(match editor.tool {
                Tool::Paint => "Left paints, right picks up a metatile.",
                Tool::Rectangle => "Drag to fill, right picks up a metatile.",
                Tool::Enemies => "Left places or drags, right deletes.",
                Tool::Start => "Click or drag the player's start (feet).",
                Tool::Exit => "Click or drag the exit line.",
            });
        }
        if part.layout.is_some() && ui.collapsing_header("Metatiles", TreeNodeFlags::DEFAULT_OPEN) {
            self.palette_panel(ui, editor);
        }
        if ui.collapsing_header("Start and exit", TreeNodeFlags::empty()) {
            if let Some((start, exit)) = editor.preview().map(|p| (p.start, p.exit)) {
                let mut start = [start.0, start.1];
                if ui.input_int2("Start", &mut start).build() {
                    editor.document.set_start(at, Some((start[0], start[1])));
                }
                let mut exit = exit;
                if ui.input_int("Exit X", &mut exit).build() {
                    editor.document.set_exit(at, Some(exit));
                }
            }
            if ui.button("Base's start") {
                editor.document.set_start(at, None);
            }
            ui.same_line();
            if ui.button("Base's exit") {
                editor.document.set_exit(at, None);
            }
        }
        if ui.collapsing_header("Enemies", TreeNodeFlags::DEFAULT_OPEN) {
            self.enemy_panel(ui, editor);
        }
        if part.layout.is_some() && ui.collapsing_header("Art", TreeNodeFlags::empty()) {
            self.art_panel(ui, editor);
        }
    }

    fn palette_panel(&mut self, ui: &Ui, editor: &mut Editor) {
        ui.checkbox("All of the base's metatiles", &mut self.all_metatiles);
        let (sheet_w, sheet_h) = (self.palette_size.0 as f32, self.palette_size.1 as f32);
        if sheet_w == 0.0 {
            return;
        }
        let per_row =
            ((ui.content_region_avail()[0] + 4.0) / (PALETTE_CELL + 4.0)).max(1.0) as usize;
        let mut pick = None;
        for (n, &brush) in self.palette.iter().enumerate() {
            if n % per_row != 0 {
                ui.same_line_with_spacing(0.0, 4.0);
            }
            let (cx, cy) = ((n % 16) as f32 * 16.0, (n / 16) as f32 * 16.0);
            let selected = editor.brush == brush;
            let clicked = ui
                .image_button_config(
                    format!("brush{n}"),
                    TextureId::new(PALETTE),
                    [PALETTE_CELL, PALETTE_CELL],
                )
                .uv0([cx / sheet_w, cy / sheet_h])
                .uv1([(cx + 16.0) / sheet_w, (cy + 16.0) / sheet_h])
                .build();
            if selected {
                ui.get_window_draw_list()
                    .add_rect(
                        ui.item_rect_min(),
                        ui.item_rect_max(),
                        [1.0, 0.85, 0.2, 1.0],
                    )
                    .thickness(3.0)
                    .build();
            }
            if clicked {
                pick = Some(brush);
            }
            if ui.is_item_hovered() {
                ui.tooltip_text(self.brush_label(editor, brush));
            }
        }
        if let Some(brush) = pick {
            editor.brush = brush;
            if !matches!(editor.tool, Tool::Paint | Tool::Rectangle) {
                editor.tool = Tool::Paint;
            }
        }
        ui.text_disabled(format!("Brush: {}", self.brush_label(editor, editor.brush)));
    }
    fn brush_label(&self, editor: &Editor, brush: Brush) -> String {
        let Some(preview) = editor.preview() else {
            return String::new();
        };
        match brush {
            Brush::Metatile(m) => {
                format!("metatile {m:02X}, {}", material_name(preview.material(m)))
            }
            Brush::Art(c) => {
                let collision = editor
                    .document
                    .cell(editor.selected, c)
                    .map_or("none", |cell| cell.collision.name());
                format!("art cell '{c}', {collision}")
            }
        }
    }

    fn enemy_panel(&mut self, ui: &Ui, editor: &mut Editor) {
        let at = editor.selected;
        let Some((base_enemies, kinds_list)) =
            editor.preview().map(|p| (p.enemies.len(), p.kinds.clone()))
        else {
            return;
        };
        let has_layout = editor.document.grid(at).is_some();
        let mut own = editor.document.enemies(at).is_some();
        if has_layout {
            ui.text_disabled("Parts with their own layout place their own enemies.");
        } else if ui.checkbox("Own enemy list", &mut own) {
            editor.document.set_own_enemies(at, own);
        }
        if !own && !has_layout {
            ui.text_disabled(format!(
                "The base places {base_enemies} enemies. Placing one starts the part's own list."
            ));
        }
        let kinds: Vec<String> = kinds_list
            .iter()
            .map(|k| format!("{:02X}  {}", k.kind, k.label))
            .collect();
        let mut index = kinds_list
            .iter()
            .position(|k| k.kind == editor.kind)
            .unwrap_or(0);
        if !kinds.is_empty() && ui.combo_simple_string("Place", &mut index, &kinds) {
            editor.kind = kinds_list[index].kind;
            editor.tool = Tool::Enemies;
        }
        if let Some(k) = kinds_list.get(index) {
            editor.kind = k.kind;
        }
        hint(
            ui,
            "Enemy types whose art the base loads, and the code's own types.",
        );
        let Some(list) = editor.document.enemies(at).map(<[_]>::to_vec) else {
            return;
        };
        ui.child_window("enemies")
            .size([0.0, 140.0])
            .border(true)
            .build(|| {
                for (i, e) in list.iter().enumerate() {
                    if ui
                        .selectable_config(format!("{i:2}  {}  at {}, {}", e.kind, e.x, e.y))
                        .selected(editor.selected_enemy == Some(i))
                        .build()
                    {
                        editor.selected_enemy = Some(i);
                        editor.tool = Tool::Enemies;
                    }
                }
            });
        let Some(i) = editor.selected_enemy.filter(|&i| i < list.len()) else {
            return;
        };
        let e = &list[i];
        let kind = u16::from_str_radix(&e.kind, 16).unwrap_or(0);
        let mut index = kinds_list.iter().position(|k| k.kind == kind);
        if let Some(index) = &mut index
            && ui.combo_simple_string("Type", index, &kinds)
        {
            editor
                .document
                .set_enemy_kind(at, i, kinds_list[*index].kind);
        }
        let mut position = [e.x, e.y];
        if ui.input_int2("Position", &mut position).build() {
            editor.document.move_enemy(at, i, position[0], position[1]);
        }
        let mut flags = e.flags.clone().unwrap_or_default();
        if ui.input_text("Flags (hex)", &mut flags).build() {
            match flags.trim() {
                "" => editor.document.set_enemy_flags(at, i, None),
                text => {
                    if let Ok(value) = u16::from_str_radix(text, 16) {
                        editor.document.set_enemy_flags(at, i, Some(value));
                    }
                }
            }
        }
        hint(ui, "The spawn flags word. Each type reads its own bits.");
        if ui.button("Delete enemy") {
            editor.document.remove_enemy(at, i);
            editor.selected_enemy = None;
        }
    }

    fn art_panel(&mut self, ui: &Ui, editor: &mut Editor) {
        let at = editor.selected;
        ui.text_wrapped(
            "Art cells are 16x16 squares of a PNG sheet. Each 8x8 quarter may only use colours from one of the base's palette lines. Export a template with --expansion-template.",
        );
        ui.input_text("PNG file", &mut self.sheet_path).build();
        if ui.button("Import sheet") {
            let path = PathBuf::from(self.sheet_path.trim());
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.status = match std::fs::read(&path) {
                Ok(bytes) => match editor.document.set_art_sheet(at, &name, bytes) {
                    Ok(()) => format!("Imported {name}"),
                    Err(e) => format!("Couldn't import: {e}"),
                },
                Err(e) => format!("{}: {e}", path.display()),
            };
        }
        let sheet = editor.document.part(at).and_then(|p| p.art.clone());
        if let (Some(art), Some(dir)) = (&sheet, editor.document.path.clone()) {
            ui.same_line();
            if ui.button("Reload") {
                let path = dir.join(&art.sheet);
                self.status = match std::fs::read(&path) {
                    Ok(bytes) => match editor.document.set_art_sheet(at, &art.sheet, bytes) {
                        Ok(()) => format!("Reloaded {}", art.sheet),
                        Err(e) => format!("Couldn't reload: {e}"),
                    },
                    Err(e) => format!("{}: {e}", path.display()),
                };
            }
            hint(
                ui,
                "Read the sheet again from the pack folder after painting it.",
            );
        }
        let Some(art) = sheet else {
            return;
        };
        let (w, h) = (self.sheet_size.0 as f32, self.sheet_size.1 as f32);
        if w > 0.0 {
            let scale = (ui.content_region_avail()[0] / w)
                .clamp(0.5, 3.0)
                .floor()
                .max(1.0);
            ui.text_disabled("Click a cell to use it as a metatile.");
            imgui::Image::new(TextureId::new(SHEET), [w * scale, h * scale]).build(ui);
            let origin = ui.item_rect_min();
            let draw = ui.get_window_draw_list();
            for cell in art.cells.values() {
                let (x, y) = (
                    origin[0] + cell.at[0] as f32 * 16.0 * scale,
                    origin[1] + cell.at[1] as f32 * 16.0 * scale,
                );
                draw.add_rect(
                    [x, y],
                    [x + 16.0 * scale, y + 16.0 * scale],
                    [0.3, 0.9, 0.5, 1.0],
                )
                .build();
            }
            if ui.is_item_clicked() {
                let mouse = ui.io().mouse_pos;
                let cell = [
                    ((mouse[0] - origin[0]) / (16.0 * scale)) as usize,
                    ((mouse[1] - origin[1]) / (16.0 * scale)) as usize,
                ];
                if let Some(c) = editor.document.add_art_cell(at, cell) {
                    editor.brush = Brush::Art(c);
                    editor.tool = Tool::Paint;
                }
            }
        }
        let names: Vec<&str> = Collision::ALL.iter().map(|c| c.name()).collect();
        for (key, cell) in &art.cells {
            let Some(c) = key.chars().next() else {
                continue;
            };
            let _id = ui.push_id(key.as_str());
            ui.text(format!("'{c}' at {},{}", cell.at[0], cell.at[1]));
            ui.same_line();
            let mut index = Collision::ALL
                .iter()
                .position(|&x| x == cell.collision)
                .unwrap_or(0);
            ui.set_next_item_width(130.0);
            if ui.combo_simple_string("##collision", &mut index, &names) {
                editor
                    .document
                    .set_cell_collision(at, c, Collision::ALL[index]);
            }
            ui.same_line();
            let mut front = cell.front;
            if ui.checkbox("Front", &mut front) {
                editor.document.set_cell_front(at, c, front);
            }
            ui.same_line();
            if ui.small_button("X") {
                editor.document.remove_art_cell(at, c);
            }
        }
    }

    fn stage_panel(&mut self, ui: &Ui, editor: &mut Editor) {
        let avail = ui.content_region_avail();
        let status_height = ui.text_line_height_with_spacing();
        self.view = [avail[0].max(64.0), (avail[1] - status_height).max(64.0)];
        let Some(key) = &self.stage else {
            ui.text(editor.error.as_deref().unwrap_or("Loading..."));
            return;
        };
        let size = [key.size.0 as f32 * self.zoom, key.size.1 as f32 * self.zoom];
        // The texture covers whole world pixels from the camera's floor.
        let offset = [
            (self.camera.0.floor() - self.camera.0) * self.zoom,
            (self.camera.1.floor() - self.camera.1) * self.zoom,
        ];
        let start = ui.cursor_screen_pos();
        ui.set_cursor_screen_pos([start[0] + offset[0], start[1] + offset[1]]);
        imgui::Image::new(TextureId::new(STAGE), size).build(ui);
        ui.set_cursor_screen_pos([start[0], start[1] + self.view[1]]);
        let io = ui.io();
        let mouse = io.mouse_pos;
        let inside = (start[0]..start[0] + self.view[0]).contains(&mouse[0])
            && (start[1]..start[1] + self.view[1]).contains(&mouse[1])
            && ui.is_window_hovered();
        let world = (
            (self.camera.0 + (mouse[0] - start[0]) / self.zoom).floor() as i32,
            (self.camera.1 + (mouse[1] - start[1]) / self.zoom).floor() as i32,
        );
        if inside || self.dragging {
            editor.pointer_move(Some(world));
        } else {
            editor.pointer_move(None);
        }
        if inside && ui.is_mouse_clicked(MouseButton::Left) {
            editor.pointer_down(world, false);
            self.dragging = true;
        }
        if inside && ui.is_mouse_clicked(MouseButton::Right) {
            editor.pointer_down(world, true);
            editor.pointer_up();
        }
        if self.dragging && !ui.is_mouse_down(MouseButton::Left) {
            editor.pointer_up();
            self.dragging = false;
        }
        if inside && ui.is_mouse_dragging(MouseButton::Middle) {
            let delta = io.mouse_delta;
            self.camera.0 -= delta[0] / self.zoom;
            self.camera.1 -= delta[1] / self.zoom;
        }
        if inside && (io.mouse_wheel != 0.0 || io.mouse_wheel_h != 0.0) {
            if io.key_ctrl {
                let before = self.zoom;
                self.zoom = (self.zoom + io.mouse_wheel.signum()).clamp(1.0, 6.0);
                // Keep the point under the mouse still.
                let (mx, my) = (mouse[0] - start[0], mouse[1] - start[1]);
                self.camera.0 += mx / before - mx / self.zoom;
                self.camera.1 += my / before - my / self.zoom;
            } else if io.key_shift {
                self.camera.0 -= io.mouse_wheel * 64.0;
            } else {
                self.camera.0 -= io.mouse_wheel_h * 64.0;
                self.camera.1 -= io.mouse_wheel * 48.0;
            }
        }
        // Status line: the cell under the mouse.
        let mut status;
        if inside {
            status = format!("x {} y {}", world.0, world.1);
            if let Some(preview) = editor.preview()
                && world.0 >= 0
                && world.1 >= 0
            {
                let (col, row) = (world.0 as usize / 16, world.1 as usize / 16);
                if let Some(m) = preview.metatile_at(col, row)
                    && world.0 < preview.size.0
                    && world.1 < preview.size.1
                {
                    status += &format!(
                        "   column {col} row {row}   metatile {m:02X} {}",
                        material_name(preview.material(m))
                    );
                }
                if let Some(i) = preview.enemy_at(world.0, world.1) {
                    let e = &preview.enemies[i];
                    let label = preview
                        .kinds
                        .iter()
                        .find(|k| k.kind == e.kind)
                        .map_or(String::new(), |k| k.label.clone());
                    status += &format!("   enemy {:02X} {label}", e.kind);
                }
            }
        } else {
            status = "Middle-drag or arrows pan, wheel scrolls, Ctrl+wheel zooms.".into();
        }
        ui.text_disabled(status);
    }

    fn problems_window(&mut self, ui: &Ui, editor: &mut Editor) {
        let mut open = true;
        let mut select = None;
        let [width, height] = ui.io().display_size;
        ui.window("Problems")
            .opened(&mut open)
            .position([LEFT_WIDTH + 20.0, height - 240.0], Condition::FirstUseEver)
            .size(
                [width - LEFT_WIDTH - RIGHT_WIDTH - 40.0, 200.0],
                Condition::FirstUseEver,
            )
            .build(|| {
                let problems = self.problems.as_ref().unwrap();
                if problems.is_empty() {
                    ui.text_colored([0.5, 0.9, 0.5, 1.0], "No problems: the pack loads.");
                }
                for (i, (at, problem)) in problems.iter().enumerate() {
                    let place = match at {
                        Some(at) => {
                            let s = at.stage();
                            format!("{}-{} part {}", s.round, s.substage, s.part)
                        }
                        None => "Pack".into(),
                    };
                    if ui
                        .selectable_config(format!("{place}: {problem}###problem{i}"))
                        .build()
                    {
                        select = *at;
                    }
                }
                if ui.button("Check again") {
                    self.problems = Some(editor.problems());
                }
            });
        if !open {
            self.problems = None;
        }
        if let Some(at) = select {
            editor.select(at);
            self.frame_part(editor);
        }
    }

    fn popups(&mut self, ui: &Ui, editor: &mut Editor) {
        if let Some(name) = self.open_popup.take() {
            ui.open_popup(name);
        }
        ui.modal_popup_config("Unsaved changes")
            .always_auto_resize(true)
            .build(|| {
                ui.text("This pack has changes that aren't saved.");
                if ui.button("Save") {
                    ui.close_current_popup();
                    self.save(editor, None);
                    self.confirmed = !editor.document.dirty();
                }
                ui.same_line();
                if ui.button("Discard") {
                    ui.close_current_popup();
                    self.confirmed = true;
                }
                ui.same_line();
                if ui.button("Cancel") {
                    ui.close_current_popup();
                    self.pending = None;
                }
            });
        ui.modal_popup_config("Open pack")
            .always_auto_resize(true)
            .build(|| {
                ui.text(format!("Packs in {}", self.library.root.display()));
                let mut choice = None;
                for pack in &self.library.packs {
                    if ui.selectable(format!("{}  ({})", pack.name, pack.path.display())) {
                        choice = Some(pack.path.clone());
                    }
                }
                for error in &self.library.errors {
                    ui.text_disabled(error);
                }
                ui.separator();
                ui.set_next_item_width(520.0);
                ui.input_text("Folder", &mut self.open_path).build();
                if ui.button("Open folder") && !self.open_path.trim().is_empty() {
                    choice = Some(PathBuf::from(self.open_path.trim()));
                }
                ui.same_line();
                if ui.button("Cancel") {
                    ui.close_current_popup();
                }
                if let Some(path) = choice {
                    ui.close_current_popup();
                    self.request(Pending::Open(path), editor);
                }
            });
        ui.modal_popup_config("Save as")
            .always_auto_resize(true)
            .build(|| {
                ui.text("Save the pack into a folder:");
                ui.set_next_item_width(520.0);
                ui.input_text("##folder", &mut self.save_path).build();
                ui.text_disabled(format!(
                    "Packs in {} appear under EXPANSIONS.",
                    library_directory().display()
                ));
                if ui.button("Save") && !self.save_path.trim().is_empty() {
                    ui.close_current_popup();
                    let dir = PathBuf::from(self.save_path.trim());
                    self.save(editor, Some(dir));
                    self.confirmed = self.pending.is_some() && !editor.document.dirty();
                }
                ui.same_line();
                if ui.button("Cancel") {
                    ui.close_current_popup();
                    self.pending = None;
                }
            });
    }
}

/// A `(?)` marker with a tooltip.
fn hint(ui: &Ui, text: &str) {
    ui.same_line();
    ui.text_disabled("(?)");
    if ui.is_item_hovered() {
        ui.tooltip_text(text);
    }
}
