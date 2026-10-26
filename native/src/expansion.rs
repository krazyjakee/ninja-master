//! Expansions: new stages played from the title's EXPANSIONS menu, apart
//! from the original game.
//!
//! An expansion is a pack (a folder or ZIP in the `expansions` library)
//! whose `expansion.json` lists rounds of substages of parts. Every part
//! is built on an original part, its `base`, which supplies the art,
//! palettes, metatiles, scroll and camera rules, music, the enemy types
//! that have art, any boss, and the stage-specific routines
//! ([`StageRules`]). A part can replace the base's layout with its own
//! metatile grid, place its own enemies, and move the player's start and
//! the exit. Nothing from the cartridge is stored in a pack: parts name
//! original content by stage and type numbers, which the player's own ROM
//! supplies.
//!
//! Expansion runs never touch the original game's records: they have no
//! profile identity, so no statistics, achievements or practice unlocks;
//! no Continue checkpoint; no manual saves (`Game::content`).
use crate::{
    WIDTH,
    assets::StageAssets,
    combat::Combat,
    content::{Cartridge, Content, StageRules, StageSource},
    pack::{Kind, Source},
    rom::Result,
    stage::{CameraBounds, Orientation, ScrollModel, Section, StageId},
    stage_objects::StageObjects,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) mod code;
mod screen;
pub use crate::pack::{Library, PackEntry};
pub use screen::ExpansionScreen;

pub const MANIFEST: &str = "expansion.json";
const MAX_PACK_BYTES: usize = 64 * 1024 * 1024;
const MAX_LAYOUT_BYTES: usize = 64 * 1024;
/// Rows of a horizontal layout: 32 metatiles of 16 pixels.
pub const LAYOUT_ROWS: usize = 32;
/// Columns of a horizontal layout (`$AB36`'s 8192-byte buffer).
pub const LAYOUT_COLUMNS: usize = crate::stage::LAYOUT_BYTES / LAYOUT_ROWS;
/// Spawn record identities outside the cartridge's address space: ROM
/// spawns use their address, and some handlers compare them.
const SPAWN_SOURCE: usize = 0x0100_0000;

pub static KIND: Kind = Kind {
    manifest: MANIFEST,
    folder: "expansions",
    env: "NINJA_MASTER_EXPANSIONS",
    max_bytes: MAX_PACK_BYTES,
    name: |source| Ok(manifest(source)?.name),
};

pub fn library_directory() -> PathBuf {
    crate::pack::library_directory(&KIND)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    /// Stable identity, e.g. `"bamboo-trail"`.
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    /// The compiled behaviour (`code::CODE`) the expansion uses.
    #[serde(default)]
    code: Option<String>,
    rounds: Vec<RoundEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoundEntry {
    #[serde(default)]
    name: String,
    substages: Vec<SubstageEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubstageEntry {
    parts: Vec<PartEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartEntry {
    /// The original part this one is built on, `"R-S:P"`.
    base: String,
    /// The original part whose music plays; the base's by default.
    #[serde(default)]
    music: Option<String>,
    /// A text file of 32 rows, one character per metatile column, read
    /// through `legend`. Without one the base's layout is kept.
    #[serde(default)]
    layout: Option<String>,
    /// Layout character to metatile number (two hex digits).
    #[serde(default)]
    legend: BTreeMap<String, String>,
    /// The player's start (feet), in pixels.
    #[serde(default)]
    start: Option<[i32; 2]>,
    /// The exit's X position, in pixels.
    #[serde(default)]
    exit: Option<i32>,
    /// The part's enemies. Without a list a part keeps its base's enemies,
    /// unless it has its own layout.
    #[serde(default)]
    enemies: Option<Vec<EnemyEntry>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnemyEntry {
    /// Object type in hex: a cartridge type (`$12E9A` byte offset) or one
    /// of the expansion's code's types.
    kind: String,
    x: i32,
    y: i32,
    /// Spawn flags word, four hex digits.
    #[serde(default)]
    flags: Option<String>,
}

fn manifest(source: &mut Source) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_slice(&source.manifest_bytes()?)
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
    if manifest.version != 1 {
        return Err(format!(
            "unsupported expansion version {}",
            manifest.version
        ));
    }
    if manifest.id.is_empty()
        || !manifest
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("expansion id must be letters, digits, '-' or '_'".into());
    }
    if manifest.name.contains('\0') {
        return Err("expansion name cannot contain a null character".into());
    }
    Ok(manifest)
}

fn hex(value: &str, what: &str) -> Result<u16> {
    u16::from_str_radix(value.trim_start_matches('$'), 16)
        .map_err(|_| format!("{what} must be hexadecimal: {value}"))
}

/// An enemy placed by a part.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Placement {
    kind: u16,
    x: i32,
    y: i32,
    flags: u16,
}

#[derive(Clone, Debug)]
struct Part {
    base: StageId,
    music: StageId,
    /// Column-major layout bytes, padded to the full buffer, and the
    /// authored width in pixels.
    layout: Option<(Vec<u8>, i32)>,
    start: Option<(i32, i32)>,
    exit: Option<i32>,
    enemies: Option<Vec<Placement>>,
}

/// A loaded expansion: its identity and stage catalog.
pub struct Expansion {
    pub id: Arc<str>,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    round_names: Vec<String>,
    /// Rounds of substages of parts, 1-based in [`StageId`]s.
    rounds: Vec<Vec<Vec<Part>>>,
    code: Option<&'static dyn code::Code>,
}
impl std::fmt::Debug for Expansion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Expansion")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Expansion {
    /// Read and check an expansion pack's manifest and layouts. Whether its
    /// parts load against a cartridge is [`Expansion::check`].
    pub fn open(path: &Path) -> Result<Self> {
        let mut source = Source::open(path, &KIND)?;
        let manifest = manifest(&mut source)?;
        let code = match &manifest.code {
            Some(name) => Some(
                code::find(name).ok_or_else(|| format!("this build has no code named {name:?}"))?,
            ),
            None => None,
        };
        if manifest.rounds.is_empty() || manifest.rounds.len() > 99 {
            return Err("an expansion needs 1 to 99 rounds".into());
        }
        let mut rounds = Vec::new();
        let mut round_names = Vec::new();
        for (r, round) in manifest.rounds.into_iter().enumerate() {
            if round.substages.is_empty() || round.substages.len() > 99 {
                return Err(format!("round {} needs 1 to 99 substages", r + 1));
            }
            let mut substages = Vec::new();
            for (s, substage) in round.substages.into_iter().enumerate() {
                if substage.parts.is_empty() || substage.parts.len() > 32 {
                    return Err(format!("stage {}-{} needs 1 to 32 parts", r + 1, s + 1));
                }
                let parts = substage
                    .parts
                    .into_iter()
                    .enumerate()
                    .map(|(p, part)| {
                        Part::read(part, &mut source)
                            .map_err(|e| format!("stage {}-{} part {p}: {e}", r + 1, s + 1))
                    })
                    .collect::<Result<Vec<_>>>()?;
                substages.push(parts);
            }
            round_names.push(round.name);
            rounds.push(substages);
        }
        Ok(Self {
            id: manifest.id.into(),
            name: manifest.name,
            description: manifest.description,
            path: path.to_owned(),
            round_names,
            rounds,
            code,
        })
    }
    /// The name of round `round` (1-based), if the expansion gives one.
    pub fn round_name(&self, round: u8) -> Option<&str> {
        self.round_names
            .get(usize::from(round).wrapping_sub(1))
            .map(String::as_str)
            .filter(|name| !name.is_empty())
    }
    /// Every substage (part 0), in play order.
    pub fn substages(&self) -> Vec<StageId> {
        self.rounds
            .iter()
            .enumerate()
            .flat_map(|(r, substages)| {
                (0..substages.len()).map(move |s| StageId::new(r as u8 + 1, s as u8 + 1, 0))
            })
            .collect()
    }
    /// Load every substage against `rom`, as a session would.
    pub fn check(self: &Arc<Self>, rom: &[u8]) -> Result<()> {
        let source = self.source();
        for id in self.substages() {
            crate::game::Game::load_from(rom, source.clone(), id)
                .map_err(|e| format!("stage {}-{}: {e}", id.round, id.substage))?;
        }
        Ok(())
    }
    pub fn source(self: &Arc<Self>) -> Arc<dyn StageSource> {
        Arc::new(ExpansionSource(self.clone()))
    }
    fn substage(&self, id: StageId) -> Result<&[Part]> {
        self.rounds
            .get(id.round_index())
            .and_then(|round| round.get(id.substage_index()))
            .map(Vec::as_slice)
            .ok_or_else(|| format!("{}: no stage {}-{}", self.name, id.round, id.substage))
    }
    fn part(&self, id: StageId) -> Result<&Part> {
        self.substage(id)?
            .get(id.part_index())
            .ok_or_else(|| format!("{}: no {id}", self.name))
    }
}

impl Part {
    fn read(entry: PartEntry, source: &mut Source) -> Result<Self> {
        let base = StageId::parse(&entry.base)?;
        let music = entry
            .music
            .as_deref()
            .map(StageId::parse)
            .transpose()?
            .unwrap_or(base);
        let layout = match &entry.layout {
            Some(name) => {
                let text = source.read(name, MAX_LAYOUT_BYTES)?;
                let text = String::from_utf8(text).map_err(|_| format!("{name}: not UTF-8"))?;
                Some(layout(&text, &entry.legend).map_err(|e| format!("{name}: {e}"))?)
            }
            None if !entry.legend.is_empty() => {
                return Err("a legend needs a layout".into());
            }
            None => None,
        };
        let enemies = entry
            .enemies
            .map(|list| {
                list.iter()
                    .map(|e| {
                        Ok(Placement {
                            kind: hex(&e.kind, "enemy kind")?,
                            x: e.x,
                            y: e.y,
                            flags: e
                                .flags
                                .as_deref()
                                .map_or(Ok(0), |f| hex(f, "enemy flags"))?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        if enemies.as_ref().is_some_and(|list| list.len() > 32) {
            return Err("a part places at most 32 enemies".into());
        }
        Ok(Self {
            base,
            music,
            layout,
            start: entry.start.map(|[x, y]| (x, y)),
            exit: entry.exit,
            enemies,
        })
    }
    /// The base's part record with this part's start, exit and width.
    fn section(&self, rom: &[u8]) -> Result<Section> {
        let mut section = Cartridge
            .sections(rom, self.base)?
            .get(self.base.part_index())
            .cloned()
            .ok_or_else(|| format!("base {} does not exist", self.base))?;
        if let Some((_, width)) = &self.layout {
            // Word 11 is the camera's maximum X (`$1A64`).
            section.raw[11] = (width - WIDTH as i32) as u16;
        }
        if let Some((x, y)) = self.start {
            let max_x = CameraBounds::for_section(rom, &section)?.x.1;
            section.camera_start.0 =
                (section.camera_start.0 + x - section.player_start.0).clamp(0, max_x.max(0));
            section.player_start = (x, y);
        }
        if let Some(x) = self.exit {
            section.exit.0 = x;
        }
        Ok(section)
    }
    /// The enemies this part places instead of its base's, if any.
    fn placements(&self) -> Option<&[Placement]> {
        match (&self.enemies, &self.layout) {
            (Some(list), _) => Some(list),
            (None, Some(_)) => Some(&[]),
            (None, None) => None,
        }
    }
}

/// Parse a layout grid into column-major bytes padded to the full buffer
/// by repeating the last column, and its width in pixels.
fn layout(text: &str, legend: &BTreeMap<String, String>) -> Result<(Vec<u8>, i32)> {
    let mut cells = BTreeMap::new();
    for (key, value) in legend {
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            return Err(format!("legend keys are single characters: {key:?}"));
        };
        let metatile = hex(value, "metatile")?;
        cells.insert(
            c,
            u8::try_from(metatile).map_err(|_| format!("metatile {value} exceeds FF"))?,
        );
    }
    let mut rows: Vec<&str> = text.lines().collect();
    while rows.last().is_some_and(|line| line.is_empty()) {
        rows.pop();
    }
    if rows.len() != LAYOUT_ROWS {
        return Err(format!("needs {LAYOUT_ROWS} rows, has {}", rows.len()));
    }
    let width = rows[0].chars().count();
    if !(WIDTH / 16 + 1..=LAYOUT_COLUMNS).contains(&width) {
        return Err(format!(
            "needs {} to {LAYOUT_COLUMNS} columns, has {width}",
            WIDTH / 16 + 1
        ));
    }
    let mut grid = Vec::with_capacity(LAYOUT_ROWS);
    for (row, line) in rows.iter().enumerate() {
        let cells = line
            .chars()
            .map(|c| {
                cells
                    .get(&c)
                    .copied()
                    .ok_or_else(|| format!("row {}: {c:?} is not in the legend", row + 1))
            })
            .collect::<Result<Vec<u8>>>()?;
        if cells.len() != width {
            return Err(format!("row {} is not {width} columns wide", row + 1));
        }
        grid.push(cells);
    }
    let bytes = (0..LAYOUT_COLUMNS)
        .flat_map(|col| {
            let col = col.min(width - 1);
            grid.iter().map(move |row| row[col])
        })
        .collect();
    Ok((bytes, (width * 16) as i32))
}

/// An expansion's catalog as a [`StageSource`].
struct ExpansionSource(Arc<Expansion>);

impl ExpansionSource {
    /// Spawns for a part's placements: cartridge types, or the code's types
    /// (registered with `combat`), each only where the base loads its art.
    fn spawns(
        &self,
        rom: &[u8],
        part: &Part,
        combat: &mut Combat,
        list: &[Placement],
    ) -> Result<Vec<crate::combat::Spawn>> {
        let types = match self.0.code {
            Some(code) => code.enemies(rom)?,
            None => Vec::new(),
        };
        let mut drawable: Vec<u16> = combat.art_kinds().collect();
        drawable.extend(combat.spawns.iter().map(|s| s.kind));
        let mut spawns = Vec::new();
        for (i, p) in list.iter().enumerate() {
            let source = SPAWN_SOURCE + i * 8;
            let new = types.iter().find(|t| t.kind == p.kind);
            let art = new.map_or(p.kind, |t| t.art);
            if p.kind >= code::FIRST_KIND && new.is_none() {
                return Err(format!(
                    "enemy type {:02X} is not in the expansion's code",
                    p.kind
                ));
            }
            if !drawable.contains(&art) {
                return Err(format!(
                    "enemy type {:02X} has no art in base {}",
                    art, part.base
                ));
            }
            spawns.push(match new {
                Some(t) => crate::combat::Spawn {
                    source,
                    x: p.x,
                    y: p.y,
                    flags: p.flags,
                    kind: t.kind,
                    handler: 0,
                    score_bcd: t.score_bcd,
                    contact_damage: t.contact_damage,
                    health_counter: t.health_counter,
                    placeholder_mapping: 0,
                },
                None => crate::combat::spawn(rom, source, (p.x, p.y), p.flags, p.kind)?,
            });
        }
        for t in types {
            if !t.kind.is_multiple_of(4) || t.kind < code::FIRST_KIND {
                return Err(format!(
                    "code enemy type {:X} is outside the expansion range",
                    t.kind
                ));
            }
            combat.add_enemy_type(t.kind, t.behavior);
        }
        Ok(spawns)
    }
}

impl StageSource for ExpansionSource {
    fn content(&self) -> Content {
        Content::Expansion(self.0.id.clone())
    }
    fn rules(&self, id: StageId) -> StageRules {
        self.0.part(id).map_or(
            StageRules {
                routines: StageId::FOREST,
                markers: false,
            },
            |part| StageRules {
                routines: part.base,
                markers: part.layout.is_none(),
            },
        )
    }
    fn sections(&self, rom: &[u8], id: StageId) -> Result<Vec<Section>> {
        self.0
            .substage(id)?
            .iter()
            .map(|part| part.section(rom))
            .collect()
    }
    fn scroll_model(&self, rom: &[u8], id: StageId) -> Result<ScrollModel> {
        Cartridge.scroll_model(rom, self.0.part(id)?.base)
    }
    fn camera_bounds(&self, rom: &[u8], section: &Section) -> Result<CameraBounds> {
        Cartridge.camera_bounds(rom, section)
    }
    fn stage_assets(&self, rom: &[u8], id: StageId) -> Result<StageAssets> {
        let part = self.0.part(id)?;
        let mut assets = Cartridge.stage_assets(rom, part.base)?;
        let section = part.section(rom)?;
        assets.data.camera = CameraBounds::for_section(rom, &section)?;
        assets.data.section = section;
        if let Some((layout, _)) = &part.layout {
            if assets.data.orientation != Orientation::Horizontal {
                return Err(format!("base {} scrolls vertically", part.base));
            }
            assets.data.layout.clone_from(layout);
            assets.data.edits.clear();
            assets.data.spawns.clear();
            // Overlays are drawn at fixed places in the base's layout.
            assets.data.plane_a_overlay = None;
        }
        Ok(assets)
    }
    fn combat(&self, rom: &[u8], id: StageId) -> Result<Combat> {
        let part = self.0.part(id)?;
        let mut combat = Cartridge.combat(rom, part.base)?;
        if let Some(list) = part.placements() {
            let spawns = self.spawns(rom, part, &mut combat, list)?;
            combat.replace_spawns(spawns);
        }
        if let Some(boss) = self
            .0
            .code
            .map(|code| code.boss(rom, id))
            .transpose()?
            .flatten()
            && !combat.replace_boss(boss)
        {
            return Err(format!("base {} has no boss to replace", part.base));
        }
        Ok(combat)
    }
    fn objects(&self, rom: &[u8], id: StageId) -> Result<StageObjects> {
        let part = self.0.part(id)?;
        let mut objects = Cartridge.objects(rom, part.base)?;
        if part.layout.is_some() {
            objects.clear_records();
        }
        Ok(objects)
    }
    fn enter_part_objects(
        &self,
        objects: &mut StageObjects,
        rom: &[u8],
        id: StageId,
    ) -> Result<bool> {
        let part = self.0.part(id)?;
        if part.layout.is_some() {
            *objects = self.objects(rom, id)?;
            return Ok(false);
        }
        Cartridge.enter_part_objects(objects, rom, part.base)
    }
    fn music(&self, rom: &[u8], id: StageId) -> Result<u8> {
        Cartridge.music(rom, self.0.part(id)?.music)
    }
    fn start_music(&self, rom: &[u8], id: StageId) -> Result<u8> {
        Cartridge.start_music(rom, self.0.part(id)?.music)
    }
    fn round_title(&self, round: u8) -> Option<String> {
        Some(self.0.round_name(round).unwrap_or_default().to_owned())
    }
    fn next_substage(&self, _: &[u8], id: StageId) -> Result<Option<StageId>> {
        let rounds = &self.0.rounds;
        let substages = rounds.get(id.round_index()).map_or(0, Vec::len);
        Ok(if usize::from(id.substage) < substages {
            Some(StageId::new(id.round, id.substage + 1, 0))
        } else if usize::from(id.round) < rounds.len() {
            Some(StageId::new(id.round + 1, 1, 0))
        } else {
            None
        })
    }
}

#[cfg(test)]
mod tests;
