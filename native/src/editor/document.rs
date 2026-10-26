//! An expansion pack being edited: its manifest, layout grids and art
//! sheets, with undo. Layouts stay as the characters the pack's files
//! hold, so a pack that is opened and saved keeps its legend.
use super::{Brush, PartRef};
use crate::{
    WIDTH,
    expansion::{
        self, ArtEntry, CellEntry, Collision, EnemyEntry, Expansion, Files, KIND, LAYOUT_COLUMNS,
        LAYOUT_ROWS, MANIFEST, Manifest, PartEntry, RoundEntry, SubstageEntry,
    },
    pack::Source,
    rom::Result,
    stage::StageId,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The narrowest layout: one screen and a column.
pub const MIN_COLUMNS: usize = WIDTH / 16 + 1;
/// Columns of a new layout: two screens.
pub const NEW_COLUMNS: usize = 40;
const MAX_LAYOUT_BYTES: usize = 64 * 1024;
const MAX_SHEET_BYTES: usize = 16 * 1024 * 1024;
/// Characters new legend entries and art cells take, in order. Once these
/// run out, letters from Latin-1 and Latin Extended follow.
const POOL: &str = ".=#abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789\
                    !$%&()*+,-/:;<>?@[]^_{|}~";

/// A layout: 32 rows of one character per metatile column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    rows: Vec<Vec<char>>,
}

impl Grid {
    pub fn blank(width: usize, fill: char) -> Self {
        Self {
            rows: vec![vec![fill; width]; LAYOUT_ROWS],
        }
    }
    /// A layout file's grid, checked as the game reads it.
    pub fn parse(text: &str) -> Result<Self> {
        let mut rows: Vec<Vec<char>> = text.lines().map(|line| line.chars().collect()).collect();
        while rows.last().is_some_and(Vec::is_empty) {
            rows.pop();
        }
        if rows.len() != LAYOUT_ROWS {
            return Err(format!("needs {LAYOUT_ROWS} rows, has {}", rows.len()));
        }
        let width = rows[0].len();
        if !(MIN_COLUMNS..=LAYOUT_COLUMNS).contains(&width) {
            return Err(format!(
                "needs {MIN_COLUMNS} to {LAYOUT_COLUMNS} columns, has {width}"
            ));
        }
        if let Some(row) = rows.iter().position(|row| row.len() != width) {
            return Err(format!("row {} is not {width} columns wide", row + 1));
        }
        Ok(Self { rows })
    }
    pub fn text(&self) -> String {
        let mut text = String::new();
        for row in &self.rows {
            text.extend(row);
            text.push('\n');
        }
        text
    }
    pub fn width(&self) -> usize {
        self.rows[0].len()
    }
    pub fn get(&self, col: usize, row: usize) -> Option<char> {
        self.rows.get(row)?.get(col).copied()
    }
    fn set(&mut self, col: usize, row: usize, c: char) -> bool {
        match self.rows.get_mut(row).and_then(|row| row.get_mut(col)) {
            Some(cell) if *cell != c => {
                *cell = c;
                true
            }
            _ => false,
        }
    }
    /// Widen by repeating the last column, as the game pads the buffer,
    /// or cut columns off the right.
    fn resize(&mut self, width: usize) {
        for row in &mut self.rows {
            let last = *row.last().unwrap();
            row.resize(width, last);
        }
    }
    fn replace(&mut self, from: char, to: char) {
        for c in self.rows.iter_mut().flatten() {
            if *c == from {
                *c = to;
            }
        }
    }
    fn contains(&self, c: char) -> bool {
        self.rows.iter().flatten().any(|&cell| cell == c)
    }
}

/// Everything an edit can change; one undo step restores one of these.
#[derive(Clone, Debug, PartialEq)]
struct State {
    manifest: Manifest,
    /// Layout grids by file name.
    layouts: BTreeMap<String, Grid>,
    /// Art sheets (PNG bytes) by file name.
    sheets: BTreeMap<String, Arc<[u8]>>,
}

/// An expansion pack open in the editor.
pub struct Document {
    /// The folder it was opened from or last saved to.
    pub path: Option<PathBuf>,
    state: State,
    saved: Option<State>,
    undo: Vec<State>,
    redo: Vec<State>,
    /// The last edit's merge key: repeated edits with the same key (typing
    /// in one field) undo together.
    merge: Option<String>,
    /// Inside a paint stroke, whose edits undo together.
    stroke: bool,
    revision: u64,
}

impl Document {
    /// A new pack with one round of one substage of one part.
    pub fn new(id: &str, name: &str) -> Self {
        let manifest = Manifest {
            version: 1,
            id: id.to_owned(),
            name: name.to_owned(),
            description: String::new(),
            code: None,
            rounds: vec![RoundEntry {
                name: "Round 1".into(),
                substages: vec![SubstageEntry {
                    parts: vec![part_on(StageId::FOREST)],
                }],
            }],
        };
        Self::with_state(
            None,
            State {
                manifest,
                layouts: BTreeMap::new(),
                sheets: BTreeMap::new(),
            },
        )
    }

    /// Open a pack folder or ZIP. A ZIP has no folder to save back to.
    pub fn open(path: &Path) -> Result<Self> {
        let manifest = expansion::read_manifest(path)?;
        let mut source = Source::open(path, &KIND)?;
        let mut layouts = BTreeMap::new();
        let mut sheets = BTreeMap::new();
        for part in parts(&manifest) {
            if let Some(name) = &part.layout
                && !layouts.contains_key(name)
            {
                let bytes = source.read(name, MAX_LAYOUT_BYTES)?;
                let text = String::from_utf8(bytes).map_err(|_| format!("{name}: not UTF-8"))?;
                layouts.insert(
                    name.clone(),
                    Grid::parse(&text).map_err(|e| format!("{name}: {e}"))?,
                );
            }
            if let Some(art) = &part.art
                && !sheets.contains_key(&art.sheet)
            {
                let bytes = source.read(&art.sheet, MAX_SHEET_BYTES)?;
                sheets.insert(art.sheet.clone(), bytes.into());
            }
        }
        Ok(Self::with_state(
            path.is_dir().then(|| path.to_owned()),
            State {
                manifest,
                layouts,
                sheets,
            },
        ))
    }

    fn with_state(path: Option<PathBuf>, state: State) -> Self {
        Self {
            saved: path.is_some().then(|| state.clone()),
            path,
            state,
            undo: Vec::new(),
            redo: Vec::new(),
            merge: None,
            stroke: false,
            revision: 0,
        }
    }

    /// Write the pack into `dir` and keep saving there.
    pub fn save(&mut self, dir: &Path) -> Result<()> {
        self.export(dir)?;
        self.path = Some(dir.to_owned());
        self.saved = Some(self.state.clone());
        Ok(())
    }
    /// Write the pack into `dir`: the manifest, every layout and sheet its
    /// parts name. Other files in the folder are left alone.
    pub fn export(&self, dir: &Path) -> Result<()> {
        let err = |path: &Path, e: std::io::Error| format!("{}: {e}", path.display());
        std::fs::create_dir_all(dir).map_err(|e| err(dir, e))?;
        let mut json = serde_json::to_string_pretty(&self.state.manifest)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        json.push('\n');
        let mut files: Vec<(String, Vec<u8>)> = vec![(MANIFEST.into(), json.into_bytes())];
        for part in parts(&self.state.manifest) {
            if let Some(name) = &part.layout {
                let grid = self
                    .state
                    .layouts
                    .get(name)
                    .ok_or_else(|| format!("{name}: no layout"))?;
                files.push((name.clone(), grid.text().into_bytes()));
            }
            if let Some(art) = &part.art {
                let sheet = self
                    .state
                    .sheets
                    .get(&art.sheet)
                    .ok_or_else(|| format!("{}: no art sheet", art.sheet))?;
                files.push((art.sheet.clone(), sheet.to_vec()));
            }
        }
        for (name, bytes) in files {
            crate::pack::check_name(&name)?;
            let path = dir.join(&name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| err(parent, e))?;
            }
            std::fs::write(&path, bytes).map_err(|e| err(&path, e))?;
        }
        Ok(())
    }

    /// Changed since it was opened or saved.
    pub fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.state)
    }
    /// Bumped by every change, undo and redo.
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn manifest(&self) -> &Manifest {
        &self.state.manifest
    }
    pub fn part(&self, at: PartRef) -> Option<&PartEntry> {
        self.state
            .manifest
            .rounds
            .get(at.round)?
            .substages
            .get(at.substage)?
            .parts
            .get(at.part)
    }
    fn part_mut(&mut self, at: PartRef) -> Option<&mut PartEntry> {
        self.state
            .manifest
            .rounds
            .get_mut(at.round)?
            .substages
            .get_mut(at.substage)?
            .parts
            .get_mut(at.part)
    }
    /// Every part, in play order.
    pub fn parts(&self) -> Vec<PartRef> {
        let mut list = Vec::new();
        for (r, round) in self.state.manifest.rounds.iter().enumerate() {
            for (s, substage) in round.substages.iter().enumerate() {
                for p in 0..substage.parts.len() {
                    list.push(PartRef::new(r, s, p));
                }
            }
        }
        list
    }
    pub fn grid(&self, at: PartRef) -> Option<&Grid> {
        self.state.layouts.get(self.part(at)?.layout.as_ref()?)
    }
    pub fn sheet(&self, name: &str) -> Option<&[u8]> {
        self.state.sheets.get(name).map(|bytes| &bytes[..])
    }

    /// The pack as the game loads it, from the unsaved files.
    pub fn expansion(&self) -> Result<Expansion> {
        self.build(self.state.manifest.clone())
    }
    /// The pack with every part but `at` reduced to its base, so that
    /// mistakes elsewhere do not stop `at` from loading. Stage numbers
    /// stay the same.
    pub fn expansion_for(&self, at: PartRef) -> Result<Expansion> {
        let mut manifest = self.state.manifest.clone();
        for (r, round) in manifest.rounds.iter_mut().enumerate() {
            for (s, substage) in round.substages.iter_mut().enumerate() {
                for (p, part) in substage.parts.iter_mut().enumerate() {
                    if PartRef::new(r, s, p) != at {
                        let base = StageId::parse(&part.base).unwrap_or(StageId::FOREST);
                        *part = part_on(base);
                    }
                }
            }
        }
        self.build(manifest)
    }
    fn build(&self, manifest: Manifest) -> Result<Expansion> {
        let path = self.path.clone().unwrap_or_default();
        Expansion::build(manifest, &mut Unsaved(&self.state), &path)
    }

    // History.

    /// Record the state before an edit. Edits sharing `merge` with the
    /// previous one, and edits inside a stroke, join its undo step.
    fn checkpoint(&mut self, merge: Option<String>) {
        self.revision += 1;
        if self.stroke {
            return;
        }
        if merge.is_some() && merge == self.merge {
            return;
        }
        self.merge = merge;
        self.undo.push(self.state.clone());
        self.redo.clear();
        if self.undo.len() > 500 {
            self.undo.remove(0);
        }
    }
    /// Start a paint or drag: its edits undo as one step.
    pub fn begin_stroke(&mut self) {
        self.checkpoint(None);
        self.stroke = true;
    }
    pub fn end_stroke(&mut self) {
        self.stroke = false;
        // A stroke that changed nothing leaves no undo step.
        if self.undo.last() == Some(&self.state) {
            self.undo.pop();
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self) -> bool {
        let Some(state) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.state, state));
        self.merge = None;
        self.revision += 1;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(state) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.state, state));
        self.merge = None;
        self.revision += 1;
        true
    }

    // Pack and round properties.

    pub fn set_id(&mut self, id: &str) {
        self.checkpoint(Some("id".into()));
        self.state.manifest.id = id.to_owned();
    }
    pub fn set_name(&mut self, name: &str) {
        self.checkpoint(Some("name".into()));
        self.state.manifest.name = name.to_owned();
    }
    pub fn set_description(&mut self, description: &str) {
        self.checkpoint(Some("description".into()));
        self.state.manifest.description = description.to_owned();
    }
    pub fn set_code(&mut self, code: Option<&str>) {
        self.checkpoint(None);
        self.state.manifest.code = code.map(str::to_owned);
    }
    pub fn set_round_name(&mut self, round: usize, name: &str) {
        if round < self.state.manifest.rounds.len() {
            self.checkpoint(Some(format!("round {round}")));
            self.state.manifest.rounds[round].name = name.to_owned();
        }
    }

    // Structure.

    /// Add a round after `round` with one part on `base`; returns its part.
    pub fn add_round(&mut self, round: usize, base: StageId) -> PartRef {
        self.checkpoint(None);
        let rounds = &mut self.state.manifest.rounds;
        let at = (round + 1).min(rounds.len());
        rounds.insert(
            at,
            RoundEntry {
                name: format!("Round {}", at + 1),
                substages: vec![SubstageEntry {
                    parts: vec![part_on(base)],
                }],
            },
        );
        PartRef::new(at, 0, 0)
    }
    /// Add a substage after `at`'s with one part on `base`.
    pub fn add_substage(&mut self, at: PartRef, base: StageId) -> Option<PartRef> {
        let round = self.state.manifest.rounds.get(at.round)?;
        let index = (at.substage + 1).min(round.substages.len());
        self.checkpoint(None);
        self.state.manifest.rounds[at.round].substages.insert(
            index,
            SubstageEntry {
                parts: vec![part_on(base)],
            },
        );
        Some(PartRef::new(at.round, index, 0))
    }
    /// Add a part after `at`, on the next part of `at`'s base.
    pub fn add_part(&mut self, at: PartRef) -> Option<PartRef> {
        let base = StageId::parse(&self.part(at)?.base).unwrap_or(StageId::FOREST);
        let next = base.with_part(base.part.saturating_add(1));
        self.checkpoint(None);
        let parts = &mut self.state.manifest.rounds[at.round].substages[at.substage].parts;
        parts.insert(at.part + 1, part_on(next));
        Some(PartRef::new(at.round, at.substage, at.part + 1))
    }
    /// Remove `at`'s part, and its substage and round when they empty.
    /// The last part of the pack stays. Returns a part to select.
    pub fn remove_part(&mut self, at: PartRef) -> Option<PartRef> {
        self.part(at)?;
        if self.parts().len() == 1 {
            return None;
        }
        self.checkpoint(None);
        let rounds = &mut self.state.manifest.rounds;
        let substages = &mut rounds[at.round].substages;
        substages[at.substage].parts.remove(at.part);
        if substages[at.substage].parts.is_empty() {
            substages.remove(at.substage);
            if substages.is_empty() {
                rounds.remove(at.round);
            }
        }
        let parts = self.parts();
        parts
            .iter()
            .rev()
            .find(|p| **p <= at)
            .or(parts.first())
            .copied()
    }
    /// Swap `at`'s part with its neighbour in the substage (`delta` -1 or
    /// 1); returns where it went.
    pub fn move_part(&mut self, at: PartRef, delta: isize) -> Option<PartRef> {
        let parts = &self
            .state
            .manifest
            .rounds
            .get(at.round)?
            .substages
            .get(at.substage)?
            .parts;
        let to = at
            .part
            .checked_add_signed(delta)
            .filter(|&to| to < parts.len())?;
        self.checkpoint(None);
        self.state.manifest.rounds[at.round].substages[at.substage]
            .parts
            .swap(at.part, to);
        Some(PartRef::new(at.round, at.substage, to))
    }
    /// Swap `at`'s substage with its neighbour in the round.
    pub fn move_substage(&mut self, at: PartRef, delta: isize) -> Option<PartRef> {
        let substages = &self.state.manifest.rounds.get(at.round)?.substages;
        let to = at
            .substage
            .checked_add_signed(delta)
            .filter(|&to| to < substages.len())?;
        self.checkpoint(None);
        self.state.manifest.rounds[at.round]
            .substages
            .swap(at.substage, to);
        Some(PartRef::new(at.round, to, 0))
    }
    /// Swap `at`'s round with its neighbour.
    pub fn move_round(&mut self, at: PartRef, delta: isize) -> Option<PartRef> {
        let rounds = &self.state.manifest.rounds;
        let to = at
            .round
            .checked_add_signed(delta)
            .filter(|&to| to < rounds.len())?;
        self.checkpoint(None);
        self.state.manifest.rounds.swap(at.round, to);
        Some(PartRef::new(to, 0, 0))
    }

    // Part properties.

    pub fn set_base(&mut self, at: PartRef, base: StageId) {
        if self.part(at).is_some() {
            self.checkpoint(None);
            self.part_mut(at).unwrap().base = stage_text(base);
        }
    }
    pub fn set_music(&mut self, at: PartRef, music: Option<StageId>) {
        if self.part(at).is_some() {
            self.checkpoint(None);
            self.part_mut(at).unwrap().music = music.map(stage_text);
        }
    }
    pub fn set_start(&mut self, at: PartRef, start: Option<(i32, i32)>) {
        if self.part(at).is_some() {
            self.checkpoint(Some(format!("start {at:?}")));
            self.part_mut(at).unwrap().start = start.map(|(x, y)| [x, y]);
        }
    }
    pub fn set_exit(&mut self, at: PartRef, exit: Option<i32>) {
        if self.part(at).is_some() {
            self.checkpoint(Some(format!("exit {at:?}")));
            self.part_mut(at).unwrap().exit = exit;
        }
    }

    // Layout.

    /// Give `at` a blank layout of `width` columns of metatile 0, replacing
    /// its base's layout (and with it the base's enemies and objects).
    pub fn create_layout(&mut self, at: PartRef, width: usize) -> bool {
        let Some(part) = self.part(at) else {
            return false;
        };
        if part.layout.is_some() {
            return false;
        }
        let width = width.clamp(MIN_COLUMNS, LAYOUT_COLUMNS);
        let mut name = format!(
            "layout-{}-{}-{}.txt",
            at.round + 1,
            at.substage + 1,
            at.part
        );
        let mut n = 2;
        while self.state.layouts.contains_key(&name)
            || parts(&self.state.manifest).any(|p| p.layout.as_ref() == Some(&name))
        {
            name = format!(
                "layout-{}-{}-{}-{n}.txt",
                at.round + 1,
                at.substage + 1,
                at.part
            );
            n += 1;
        }
        self.checkpoint(None);
        let part = self.part_mut(at).unwrap();
        part.layout = Some(name.clone());
        part.legend.insert(".".into(), "00".into());
        self.state.layouts.insert(name, Grid::blank(width, '.'));
        true
    }
    /// Drop `at`'s layout, legend and art: the part shows its base's again.
    pub fn remove_layout(&mut self, at: PartRef) {
        if self.part(at).is_some_and(|p| p.layout.is_some()) {
            self.checkpoint(None);
            let part = self.part_mut(at).unwrap();
            part.layout = None;
            part.legend.clear();
            part.art = None;
        }
    }
    pub fn resize_layout(&mut self, at: PartRef, width: usize) {
        let width = width.clamp(MIN_COLUMNS, LAYOUT_COLUMNS);
        if self.grid(at).is_some_and(|g| g.width() != width) {
            self.checkpoint(Some(format!("width {at:?}")));
            let name = self.part(at).unwrap().layout.clone().unwrap();
            self.state.layouts.get_mut(&name).unwrap().resize(width);
        }
    }
    /// What the layout holds at a cell.
    pub fn brush_at(&self, at: PartRef, col: usize, row: usize) -> Option<Brush> {
        let c = self.grid(at)?.get(col, row)?;
        let part = self.part(at)?;
        if part
            .art
            .as_ref()
            .is_some_and(|a| a.cells.contains_key(&c.to_string()))
        {
            return Some(Brush::Art(c));
        }
        legend_metatile(part, c).map(Brush::Metatile)
    }
    /// Paint one cell; false when nothing changed.
    pub fn paint(&mut self, at: PartRef, col: usize, row: usize, brush: Brush) -> bool {
        self.fill(at, (col, row), (col, row), brush)
    }
    /// Paint the rectangle between two corner cells.
    pub fn fill(
        &mut self,
        at: PartRef,
        a: (usize, usize),
        b: (usize, usize),
        brush: Brush,
    ) -> bool {
        let Some(grid) = self.grid(at) else {
            return false;
        };
        let (cols, rows) = (
            a.0.min(b.0)..=a.0.max(b.0).min(grid.width() - 1),
            a.1.min(b.1)..=a.1.max(b.1).min(LAYOUT_ROWS - 1),
        );
        let Some(c) = self.brush_char(at, brush) else {
            return false;
        };
        if !cols
            .clone()
            .any(|col| rows.clone().any(|row| grid.get(col, row) != Some(c)))
        {
            return false;
        }
        self.checkpoint(None);
        self.legend_char(at, brush);
        let name = self.part(at).unwrap().layout.clone().unwrap();
        let grid = self.state.layouts.get_mut(&name).unwrap();
        for col in cols {
            for row in rows.clone() {
                grid.set(col, row, c);
            }
        }
        true
    }
    /// The character a brush paints with in `at`, if it has one or can
    /// take one without changing the layout.
    fn brush_char(&self, at: PartRef, brush: Brush) -> Option<char> {
        let part = self.part(at)?;
        match brush {
            Brush::Art(c) => part
                .art
                .as_ref()
                .is_some_and(|a| a.cells.contains_key(&c.to_string()))
                .then_some(c),
            Brush::Metatile(id) => Some(
                part.legend
                    .iter()
                    .find(|(_, v)| parse_hex(v) == Some(id))
                    .and_then(|(k, _)| single(k))
                    .unwrap_or_else(|| free_char(part)),
            ),
        }
    }
    /// Add the legend entry a metatile brush needs.
    fn legend_char(&mut self, at: PartRef, brush: Brush) {
        if let Brush::Metatile(id) = brush {
            let c = self.brush_char(at, brush).unwrap();
            let part = self.part_mut(at).unwrap();
            part.legend
                .entry(c.to_string())
                .or_insert_with(|| format!("{id:02X}"));
        }
    }

    // Enemies.

    /// The part's own enemy list, if it has one.
    pub fn enemies(&self, at: PartRef) -> Option<&[EnemyEntry]> {
        self.part(at)?.enemies.as_deref()
    }
    /// Place an enemy; returns its index.
    pub fn add_enemy(&mut self, at: PartRef, kind: u16, x: i32, y: i32) -> Option<usize> {
        self.part(at)?;
        self.checkpoint(None);
        let list = self.part_mut(at).unwrap().enemies.get_or_insert_default();
        list.push(EnemyEntry {
            kind: format!("{kind:02X}"),
            x,
            y,
            flags: None,
        });
        Some(list.len() - 1)
    }
    pub fn move_enemy(&mut self, at: PartRef, index: usize, x: i32, y: i32) {
        if let Some(e) = self.enemies(at).and_then(|list| list.get(index))
            && (e.x, e.y) != (x, y)
        {
            self.checkpoint(Some(format!("move {at:?} {index}")));
            let e = &mut self.part_mut(at).unwrap().enemies.as_mut().unwrap()[index];
            (e.x, e.y) = (x, y);
        }
    }
    pub fn remove_enemy(&mut self, at: PartRef, index: usize) {
        if self.enemies(at).is_some_and(|list| index < list.len()) {
            self.checkpoint(None);
            self.part_mut(at)
                .unwrap()
                .enemies
                .as_mut()
                .unwrap()
                .remove(index);
        }
    }
    pub fn set_enemy_kind(&mut self, at: PartRef, index: usize, kind: u16) {
        if self.enemies(at).is_some_and(|list| index < list.len()) {
            self.checkpoint(None);
            self.part_mut(at).unwrap().enemies.as_mut().unwrap()[index].kind =
                format!("{kind:02X}");
        }
    }
    pub fn set_enemy_flags(&mut self, at: PartRef, index: usize, flags: Option<u16>) {
        if self.enemies(at).is_some_and(|list| index < list.len()) {
            self.checkpoint(Some(format!("flags {at:?} {index}")));
            self.part_mut(at).unwrap().enemies.as_mut().unwrap()[index].flags =
                flags.map(|f| format!("{f:04X}"));
        }
    }
    /// Give the part its own enemy list (starting empty), or go back to
    /// its base's enemies.
    pub fn set_own_enemies(&mut self, at: PartRef, own: bool) {
        if self.part(at).is_some_and(|p| p.enemies.is_some() != own) {
            self.checkpoint(None);
            self.part_mut(at).unwrap().enemies = own.then(Vec::new);
        }
    }

    // Art.

    /// Use `bytes` (a PNG) as `at`'s art sheet, stored as `name`. The part
    /// needs a layout.
    pub fn set_art_sheet(&mut self, at: PartRef, name: &str, bytes: Vec<u8>) -> Result<()> {
        crate::pack::check_name(name)?;
        crate::art_pack::decode_png(std::io::Cursor::new(&bytes))
            .map_err(|e| format!("{name}: {e}"))?;
        if self.part(at).is_none_or(|p| p.layout.is_none()) {
            return Err("art needs a layout".into());
        }
        self.checkpoint(None);
        self.state.sheets.insert(name.to_owned(), bytes.into());
        let part = self.part_mut(at).unwrap();
        match &mut part.art {
            Some(art) => art.sheet = name.to_owned(),
            None => {
                part.art = Some(ArtEntry {
                    sheet: name.to_owned(),
                    cells: BTreeMap::new(),
                })
            }
        }
        Ok(())
    }
    /// The size of `at`'s art sheet in cells.
    pub fn sheet_cells(&self, at: PartRef) -> Option<(usize, usize)> {
        let art = self.part(at)?.art.as_ref()?;
        let (width, height, _) =
            crate::art_pack::decode_png(std::io::Cursor::new(self.sheet(&art.sheet)?)).ok()?;
        Some((width / 16, height / 16))
    }
    /// `at`'s art sheet as RGB24 over a checkerboard, and its size.
    pub fn sheet_rgb(&self, at: PartRef) -> Option<(usize, usize, Vec<u8>)> {
        let art = self.part(at)?.art.as_ref()?;
        let (width, height, pixels) =
            crate::art_pack::decode_png(std::io::Cursor::new(self.sheet(&art.sheet)?)).ok()?;
        let mut rgb = Vec::with_capacity(width * height * 3);
        for (i, p) in pixels.iter().enumerate() {
            let (x, y) = (i % width, i / width);
            let back: u16 = if (x / 4 + y / 4) % 2 == 0 { 44 } else { 30 };
            let alpha = u16::from(p[3]);
            rgb.extend(
                p[..3]
                    .iter()
                    .map(|&c| ((u16::from(c) * alpha + back * (255 - alpha)) / 255) as u8),
            );
        }
        Some((width, height, rgb))
    }
    /// Make sheet cell `cell` a metatile of `at`; returns its character.
    pub fn add_art_cell(&mut self, at: PartRef, cell: [usize; 2]) -> Option<char> {
        let part = self.part(at)?;
        let art = part.art.as_ref()?;
        if let Some((key, _)) = art.cells.iter().find(|(_, c)| c.at == cell) {
            return single(key);
        }
        let c = free_char(part);
        self.checkpoint(None);
        let art = self.part_mut(at).unwrap().art.as_mut().unwrap();
        art.cells.insert(
            c.to_string(),
            CellEntry {
                at: cell,
                collision: Collision::None,
                front: false,
            },
        );
        Some(c)
    }
    /// Stop using an art cell; the layout shows metatile 0 where it was.
    pub fn remove_art_cell(&mut self, at: PartRef, key: char) {
        let Some(part) = self.part(at) else { return };
        if !part
            .art
            .as_ref()
            .is_some_and(|a| a.cells.contains_key(&key.to_string()))
        {
            return;
        }
        let in_layout = self.grid(at).is_some_and(|g| g.contains(key));
        let blank = self.brush_char(at, Brush::Metatile(0)).unwrap();
        self.checkpoint(None);
        if in_layout {
            self.legend_char(at, Brush::Metatile(0));
            let name = self.part(at).unwrap().layout.clone().unwrap();
            self.state
                .layouts
                .get_mut(&name)
                .unwrap()
                .replace(key, blank);
        }
        let art = self.part_mut(at).unwrap().art.as_mut().unwrap();
        art.cells.remove(&key.to_string());
    }
    pub fn set_cell_collision(&mut self, at: PartRef, key: char, collision: Collision) {
        if let Some(cell) = self.cell(at, key)
            && cell.collision != collision
        {
            self.checkpoint(None);
            self.cell_mut(at, key).unwrap().collision = collision;
        }
    }
    pub fn set_cell_front(&mut self, at: PartRef, key: char, front: bool) {
        if let Some(cell) = self.cell(at, key)
            && cell.front != front
        {
            self.checkpoint(None);
            self.cell_mut(at, key).unwrap().front = front;
        }
    }
    pub fn cell(&self, at: PartRef, key: char) -> Option<&CellEntry> {
        self.part(at)?.art.as_ref()?.cells.get(&key.to_string())
    }
    fn cell_mut(&mut self, at: PartRef, key: char) -> Option<&mut CellEntry> {
        self.part_mut(at)?
            .art
            .as_mut()?
            .cells
            .get_mut(&key.to_string())
    }
}

/// A part that is its base unchanged.
fn part_on(base: StageId) -> PartEntry {
    PartEntry {
        base: stage_text(base),
        music: None,
        layout: None,
        legend: BTreeMap::new(),
        start: None,
        exit: None,
        enemies: None,
        art: None,
    }
}

/// `"R-S:P"`.
pub fn stage_text(id: StageId) -> String {
    format!("{}-{}:{}", id.round, id.substage, id.part)
}

fn parts(manifest: &Manifest) -> impl Iterator<Item = &PartEntry> {
    manifest
        .rounds
        .iter()
        .flat_map(|r| &r.substages)
        .flat_map(|s| &s.parts)
}

fn parse_hex(value: &str) -> Option<u8> {
    u8::from_str_radix(value.trim_start_matches('$'), 16).ok()
}

fn single(key: &str) -> Option<char> {
    let mut chars = key.chars();
    chars.next().filter(|_| chars.next().is_none())
}

/// The metatile a legend character stands for.
pub(crate) fn legend_metatile(part: &PartEntry, c: char) -> Option<u8> {
    parse_hex(part.legend.get(&c.to_string())?)
}

/// A character neither the legend nor the art uses.
fn free_char(part: &PartEntry) -> char {
    let taken = |c: char| {
        let key = c.to_string();
        part.legend.contains_key(&key)
            || part
                .art
                .as_ref()
                .is_some_and(|a| a.cells.contains_key(&key))
    };
    POOL.chars()
        .chain(('\u{c0}'..='\u{24f}').filter(|c| c.is_alphabetic()))
        .find(|&c| !taken(c))
        .expect("more characters than metatiles")
}

/// The document's files for [`Expansion::build`].
struct Unsaved<'a>(&'a State);

impl Files for Unsaved<'_> {
    fn read(&mut self, name: &str, _: usize) -> Result<Vec<u8>> {
        if let Some(grid) = self.0.layouts.get(name) {
            return Ok(grid.text().into_bytes());
        }
        self.0
            .sheets
            .get(name)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| format!("{name}: missing"))
    }
}
