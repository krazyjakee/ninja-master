//! The expansion editor: build expansion packs without editing their files
//! by hand.
//!
//! [`Editor`] holds an open [`Document`] (a pack's manifest, layouts and
//! art sheets, with undo), the part being edited and its [`Preview`],
//! drawn through the same loader and renderer as play. Tools act on world
//! pixel positions, so the desktop window (`desktop/editor.rs`) only turns
//! the mouse and keys into calls here, and everything else runs headless.
//!
//! The editor reads the player's ROM to draw bases, but a saved pack holds
//! only what the author made: the manifest, layouts naming metatiles by
//! number, and art they painted.
use crate::{
    expansion::{Expansion, LAYOUT_ROWS},
    stage::StageId,
};
use std::{path::Path, sync::Arc};

mod document;
mod preview;
pub use crate::expansion::Collision;
pub use document::{Document, Grid, MIN_COLUMNS, NEW_COLUMNS, stage_text};
pub use preview::{Highlight, Kind, Marker, Overlays, Preview, material_color, material_name};

/// A part of the pack: indices into its rounds, substages and parts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartRef {
    pub round: usize,
    pub substage: usize,
    pub part: usize,
}
impl PartRef {
    pub const fn new(round: usize, substage: usize, part: usize) -> Self {
        Self {
            round,
            substage,
            part,
        }
    }
    /// The part's stage number within the expansion.
    pub fn stage(self) -> StageId {
        StageId::new(
            (self.round + 1) as u8,
            (self.substage + 1) as u8,
            self.part as u8,
        )
    }
}

/// What the layout tools paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brush {
    /// A metatile of the base stage's set.
    Metatile(u8),
    /// One of the part's art cells, by its layout character.
    Art(char),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// Paint cells with the brush; the secondary button picks one up.
    #[default]
    Paint,
    /// Fill a dragged rectangle.
    Rectangle,
    /// Place, drag and (secondary button) delete enemies.
    Enemies,
    /// Drag the player's start.
    Start,
    /// Drag the exit.
    Exit,
}

/// A drag in progress.
#[derive(Clone, Copy, Debug)]
enum Drag {
    Paint((usize, usize)),
    Rectangle((usize, usize), (usize, usize)),
    Enemy(usize, (i32, i32)),
    Start,
    Exit,
}

pub struct Editor {
    pub document: Document,
    rom: Arc<[u8]>,
    pub selected: PartRef,
    preview: Option<Preview>,
    /// The document revision and part the preview shows.
    shown: Option<(u64, PartRef)>,
    /// Why the selected part does not load.
    pub error: Option<String>,
    pub tool: Tool,
    pub brush: Brush,
    /// The enemy type the enemy tool places.
    pub kind: u16,
    pub selected_enemy: Option<usize>,
    /// Snap enemies and the start to 8 pixels across and 16 down.
    pub snap: bool,
    pub overlays: Overlays,
    drag: Option<Drag>,
    hover: Option<(i32, i32)>,
}

impl Editor {
    pub fn new(document: Document, rom: Arc<[u8]>) -> Self {
        let mut editor = Self {
            document,
            rom,
            selected: PartRef::default(),
            preview: None,
            shown: None,
            error: None,
            tool: Tool::default(),
            brush: Brush::Metatile(0),
            kind: 0x14,
            selected_enemy: None,
            snap: true,
            overlays: Overlays::default(),
            drag: None,
            hover: None,
        };
        editor.refresh();
        editor
    }
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }
    /// Edit another pack.
    pub fn replace_document(&mut self, document: Document) {
        self.cancel_drag();
        self.document = document;
        self.selected = PartRef::default();
        self.selected_enemy = None;
        self.shown = None;
        self.preview = None;
        self.refresh();
    }

    /// Rebuild the preview after the document or selection changed.
    pub fn refresh(&mut self) {
        if !self.document.parts().contains(&self.selected) {
            self.selected = PartRef::default();
            self.selected_enemy = None;
        }
        let key = (self.document.revision(), self.selected);
        if self.shown == Some(key) {
            return;
        }
        self.shown = Some(key);
        match Preview::build(&self.rom, &self.document, self.selected) {
            Ok(preview) => {
                if self
                    .selected_enemy
                    .is_some_and(|i| i >= preview.enemies.len())
                {
                    self.selected_enemy = None;
                }
                self.preview = Some(preview);
                self.error = None;
            }
            Err(e) => {
                // Keep drawing the last good preview of the same part.
                if self
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.part != self.selected)
                {
                    self.preview = None;
                }
                self.error = Some(e);
            }
        }
    }
    pub fn preview(&self) -> Option<&Preview> {
        self.preview.as_ref()
    }
    pub fn select(&mut self, part: PartRef) {
        if part != self.selected {
            self.selected = part;
            self.selected_enemy = None;
            self.drag = None;
            self.refresh();
        }
    }
    pub fn undo(&mut self) {
        self.cancel_drag();
        self.document.undo();
        self.refresh();
    }
    pub fn redo(&mut self) {
        self.cancel_drag();
        self.document.redo();
        self.refresh();
    }

    /// Draw the selected part from world pixel `origin`.
    pub fn render(&mut self, origin: (i32, i32), size: (usize, usize)) -> Option<Vec<u8>> {
        self.refresh();
        let highlight = self.highlight();
        let overlays = self.overlays;
        Some(
            self.preview
                .as_mut()?
                .render(origin, size, overlays, &highlight),
        )
    }
    /// What the next [`Editor::render`] marks.
    pub fn highlight(&self) -> Highlight {
        let cells = match (self.drag, self.tool, self.hover) {
            (Some(Drag::Rectangle(a, b)), _, _) => Some((a, b)),
            (_, Tool::Paint | Tool::Rectangle, Some(point)) => {
                self.cell(point).map(|cell| (cell, cell))
            }
            _ => None,
        };
        Highlight {
            cells,
            enemy: self.selected_enemy,
        }
    }

    /// The layout cell under a world point.
    pub fn cell(&self, (x, y): (i32, i32)) -> Option<(usize, usize)> {
        let width = self.document.grid(self.selected)?.width();
        let (col, row) = (x.div_euclid(16), y.div_euclid(16));
        ((0..width as i32).contains(&col) && (0..LAYOUT_ROWS as i32).contains(&row))
            .then_some((col as usize, row as usize))
    }
    fn snapped(&self, (x, y): (i32, i32)) -> (i32, i32) {
        if self.snap {
            ((x + 4).div_euclid(8) * 8, (y + 8).div_euclid(16) * 16)
        } else {
            (x, y)
        }
    }

    pub fn pointer_move(&mut self, point: Option<(i32, i32)>) {
        self.hover = point;
        let Some(point) = point else { return };
        match self.drag {
            Some(Drag::Paint(last)) => {
                if let Some(cell) = self.cell(point) {
                    // Fill the gap a fast drag leaves between two events.
                    let steps = last.0.abs_diff(cell.0).max(last.1.abs_diff(cell.1)).max(1);
                    for i in 1..=steps {
                        let lerp = |a: usize, b: usize| {
                            (a as f32 + (b as f32 - a as f32) * i as f32 / steps as f32).round()
                                as usize
                        };
                        let step = (lerp(last.0, cell.0), lerp(last.1, cell.1));
                        self.document
                            .paint(self.selected, step.0, step.1, self.brush);
                    }
                    self.drag = Some(Drag::Paint(cell));
                }
            }
            Some(Drag::Rectangle(from, _)) => {
                if let Some(cell) = self.cell(point) {
                    self.drag = Some(Drag::Rectangle(from, cell));
                }
            }
            Some(Drag::Enemy(index, (dx, dy))) => {
                let (x, y) = self.snapped((point.0 + dx, point.1 + dy));
                self.document.move_enemy(self.selected, index, x, y);
            }
            Some(Drag::Start) => {
                let start = self.snapped(point);
                self.document.set_start(self.selected, Some(start));
            }
            Some(Drag::Exit) => {
                self.document.set_exit(self.selected, Some(point.0.max(0)));
            }
            None => {}
        }
    }

    /// A button pressed over the stage; `secondary` is the right button.
    pub fn pointer_down(&mut self, point: (i32, i32), secondary: bool) {
        self.hover = Some(point);
        self.cancel_drag();
        let at = self.selected;
        match self.tool {
            Tool::Paint | Tool::Rectangle => {
                let Some(cell) = self.cell(point) else { return };
                if secondary {
                    if let Some(brush) = self.document.brush_at(at, cell.0, cell.1) {
                        self.brush = brush;
                    }
                } else if self.tool == Tool::Paint {
                    self.document.begin_stroke();
                    self.document.paint(at, cell.0, cell.1, self.brush);
                    self.drag = Some(Drag::Paint(cell));
                } else {
                    self.drag = Some(Drag::Rectangle(cell, cell));
                }
            }
            Tool::Enemies => {
                let own = self.document.enemies(at).is_some();
                let hit = self
                    .preview
                    .as_ref()
                    .filter(|_| own)
                    .and_then(|p| p.enemy_at(point.0, point.1));
                match (hit, secondary) {
                    (Some(index), true) => {
                        self.document.remove_enemy(at, index);
                        self.selected_enemy = None;
                    }
                    (Some(index), false) => {
                        let e = &self.document.enemies(at).unwrap()[index];
                        let offset = (e.x - point.0, e.y - point.1);
                        self.selected_enemy = Some(index);
                        self.document.begin_stroke();
                        self.drag = Some(Drag::Enemy(index, offset));
                    }
                    (None, true) => self.selected_enemy = None,
                    (None, false) => {
                        let (x, y) = self.snapped(point);
                        self.selected_enemy = self.document.add_enemy(at, self.kind, x, y);
                    }
                }
            }
            Tool::Start => {
                self.document.begin_stroke();
                self.drag = Some(Drag::Start);
                self.pointer_move(Some(point));
            }
            Tool::Exit => {
                self.document.begin_stroke();
                self.drag = Some(Drag::Exit);
                self.pointer_move(Some(point));
            }
        }
        self.refresh();
    }

    pub fn pointer_up(&mut self) {
        match self.drag.take() {
            Some(Drag::Rectangle(a, b)) => {
                self.document.fill(self.selected, a, b, self.brush);
            }
            Some(_) => self.document.end_stroke(),
            None => {}
        }
        self.refresh();
    }
    fn cancel_drag(&mut self) {
        if let Some(drag) = self.drag.take()
            && !matches!(drag, Drag::Rectangle(..))
        {
            self.document.end_stroke();
        }
    }

    /// Everything that stops the pack from loading, with the part it is
    /// in when the part alone shows it.
    pub fn problems(&self) -> Vec<(Option<PartRef>, String)> {
        let mut problems = Vec::new();
        for at in self.document.parts() {
            if let Err(e) = Preview::build(&self.rom, &self.document, at)
                .and_then(|p| p.problem.map_or(Ok(()), Err))
            {
                problems.push((Some(at), e));
            }
        }
        if problems.is_empty()
            && let Err(e) = self
                .document
                .expansion()
                .map(Arc::new)
                .and_then(|e| e.check(&self.rom))
        {
            problems.push((None, e));
        }
        problems
    }

    /// Write the pack as it stands, saved or not, into `dir` for a
    /// playtest, and check that it loads.
    pub fn export_playtest(&self, dir: &Path) -> Result<(), String> {
        let _ = std::fs::remove_dir_all(dir);
        self.document.export(dir)?;
        Arc::new(Expansion::open(dir)?).check(&self.rom)
    }
}

/// Every original part, which parts can be built on.
pub fn bases(rom: &[u8]) -> Vec<StageId> {
    let mut list = Vec::new();
    for round in 1..=7u8 {
        for substage in 1..=3u8 {
            let Ok(sections) =
                crate::stage::sections(rom, round as usize - 1, substage as usize - 1)
            else {
                continue;
            };
            list.extend((0..sections.len()).map(|p| StageId::new(round, substage, p as u8)));
        }
    }
    list
}

/// The code names a pack can choose.
pub fn code_names() -> Vec<&'static str> {
    crate::expansion::code::names().collect()
}

#[cfg(test)]
mod tests;
