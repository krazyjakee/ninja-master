//! A part drawn the way the game draws it, with the editor's overlays:
//! the metatile grid, collision, enemies, the start and the exit.
use super::{PartRef, document::Document};
use crate::{
    HEIGHT, WIDTH,
    assets::StageAssets,
    content::{Cartridge, StageSource},
    expansion::free_ids,
    render::Frame,
    rom::Result,
    stage::{Orientation, StageId},
    viewer::{View, Viewer, glyph},
};
use std::sync::Arc;

/// An enemy as the part places it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub kind: u16,
    pub x: i32,
    pub y: i32,
    pub flags: u16,
}

/// An enemy type a part can place: its art is loaded by the part's base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kind {
    pub kind: u16,
    /// The handler it runs (`Roster::label`).
    pub label: String,
}

/// What the overlays draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overlays {
    pub grid: bool,
    pub collision: bool,
    pub objects: bool,
    /// Plane B and the scenery sprites.
    pub background: bool,
}
impl Default for Overlays {
    fn default() -> Self {
        Self {
            grid: true,
            collision: false,
            objects: true,
            background: true,
        }
    }
}

/// Marks for the cursor and the current selection, in world pixels.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Highlight {
    /// Cells to outline: the cursor, or a rectangle being dragged
    /// (corner cells, inclusive).
    pub cells: Option<((usize, usize), (usize, usize))>,
    /// The selected enemy.
    pub enemy: Option<usize>,
}

/// One part loaded for drawing.
pub struct Preview {
    pub part: PartRef,
    pub base: StageId,
    viewer: Viewer,
    /// The playable scene's size in pixels.
    pub size: (i32, i32),
    pub start: (i32, i32),
    pub exit: i32,
    /// The enemies the part places: its own list, or its base's.
    pub enemies: Vec<Marker>,
    /// Whether `enemies` is the part's own list.
    pub own_enemies: bool,
    pub kinds: Vec<Kind>,
    /// Metatiles the base's own layout uses.
    pub base_metatiles: Vec<u8>,
    /// Art cells' characters and metatile numbers.
    pub art: Vec<(char, u8)>,
    /// Why the part's enemies do not load, if they do not.
    pub problem: Option<String>,
}

impl Preview {
    pub fn build(rom: &[u8], document: &Document, at: PartRef) -> Result<Self> {
        let part = document.part(at).ok_or("no such part")?;
        let base = StageId::parse(&part.base)?;
        let expansion = Arc::new(document.expansion_for(at)?);
        let source = expansion.source();
        let id = at.stage();
        let assets = source.stage_assets(rom, id)?;
        let base_combat = Cartridge.combat(rom, base)?;
        let mut drawable: Vec<u16> = base_combat.art_kinds().collect();
        drawable.extend(base_combat.spawns.iter().map(|s| s.kind));
        let code = expansion.code_enemies(rom)?;
        let (combat, problem) = match source.combat(rom, id) {
            Ok(combat) => (combat, None),
            Err(e) => (base_combat, Some(e)),
        };
        let mut kinds: Vec<Kind> = drawable
            .iter()
            .copied()
            .filter(|&kind| kind < 0x100)
            .chain(
                code.iter()
                    .filter(|(_, art)| drawable.contains(art))
                    .map(|&(kind, _)| kind),
            )
            .map(|kind| Kind {
                kind,
                label: combat.kind_label(kind),
            })
            .collect();
        kinds.sort_by_key(|k| k.kind);
        kinds.dedup_by_key(|k| k.kind);
        let own_enemies = part.enemies.is_some() || part.layout.is_some();
        let enemies = if problem.is_some() {
            part.enemies
                .iter()
                .flatten()
                .map(|e| Marker {
                    kind: u16::from_str_radix(&e.kind, 16).unwrap_or(0),
                    x: e.x,
                    y: e.y,
                    flags: e
                        .flags
                        .as_deref()
                        .and_then(|f| u16::from_str_radix(f, 16).ok())
                        .unwrap_or(0),
                })
                .collect()
        } else {
            combat
                .spawns
                .iter()
                .map(|s| Marker {
                    kind: s.kind,
                    x: s.x,
                    y: s.y,
                    flags: s.flags,
                })
                .collect()
        };
        let mut base_metatiles = Cartridge.stage_assets(rom, base)?.data.layout;
        base_metatiles.sort_unstable();
        base_metatiles.dedup();
        let art = part
            .art
            .iter()
            .flat_map(|a| a.cells.keys())
            .filter_map(|key| key.chars().next())
            .zip(free_ids(&part.legend))
            .collect();
        let section = &assets.data.section;
        let (start, exit) = (section.player_start, section.exit.0);
        let viewer = Viewer::new(assets, View::Scene);
        let (_, right) = viewer.world_bounds();
        let (width, height) = viewer.assets.data.pixel_size();
        let size = match viewer.assets.data.orientation {
            Orientation::Horizontal => (right, height),
            Orientation::Vertical => (width, height),
        };
        Ok(Self {
            part: at,
            base,
            viewer,
            size,
            start,
            exit,
            enemies,
            own_enemies,
            kinds,
            base_metatiles,
            art,
            problem,
        })
    }

    pub fn assets(&self) -> &StageAssets {
        &self.viewer.assets
    }
    pub fn horizontal(&self) -> bool {
        self.viewer.assets.data.orientation == Orientation::Horizontal
    }
    /// Metatiles the stage defines.
    pub fn metatile_count(&self) -> usize {
        self.viewer.assets.data.metatiles.len().min(256)
    }
    /// The layout's metatile at a cell.
    pub fn metatile_at(&self, col: usize, row: usize) -> Option<u8> {
        self.viewer.assets.data.metatile_at(col as i32, row as i32)
    }
    /// A metatile's collision material (`crate::movement::Material`).
    pub fn material(&self, metatile: u8) -> u8 {
        self.viewer
            .assets
            .data
            .collision
            .get(usize::from(metatile))
            .map_or(0, |c| c & 15)
    }
    /// The enemy marker under a point.
    pub fn enemy_at(&self, x: i32, y: i32) -> Option<usize> {
        self.enemies.iter().rposition(|e| {
            let (left, top, w, h) = enemy_box(e);
            (left..left + w).contains(&x) && (top..top + h).contains(&y)
        })
    }

    /// Draw `width`x`height` pixels of the part from world pixel `(x, y)`
    /// as RGB24.
    pub fn render(
        &mut self,
        (x, y): (i32, i32),
        (width, height): (usize, usize),
        overlays: Overlays,
        highlight: &Highlight,
    ) -> Vec<u8> {
        let mut canvas = Canvas {
            width,
            height,
            origin: (x, y),
            rgb: vec![0; width * height * 3],
        };
        let frame_width = width.max(WIDTH);
        let mut frame = Frame::new(frame_width);
        let viewer = &mut self.viewer;
        viewer.layer = if overlays.background { 0 } else { 1 };
        let mut band = 0;
        while band < height {
            // A centred frame's first column is `viewer.x` less the margin.
            viewer.x = x + ((frame_width - WIDTH) / 2) as i32;
            viewer.y = y + band as i32;
            viewer.background_y = i64::from(viewer.y) << 16;
            frame.center_content();
            let (backdrop, index) = viewer.backdrop();
            frame.clear_backdrop(backdrop, index);
            viewer.scene_game_cut(&mut frame, None);
            viewer.front_game(&mut frame);
            let rgb = frame.rgb();
            for line in 0..HEIGHT.min(height - band) {
                let from = line * frame_width * 3;
                let to = (band + line) * width * 3;
                canvas.rgb[to..to + width * 3].copy_from_slice(&rgb[from..from + width * 3]);
            }
            band += HEIGHT;
        }
        // Outside the scene.
        let (w, h) = self.size;
        canvas.shade_outside(w, h);
        self.overlays(&mut canvas, overlays, highlight);
        canvas.rgb
    }

    fn overlays(&self, canvas: &mut Canvas, overlays: Overlays, highlight: &Highlight) {
        let (x, y) = canvas.origin;
        let cols = x.div_euclid(16)..=(x + canvas.width as i32).div_euclid(16);
        let rows = y.div_euclid(16)..=(y + canvas.height as i32).div_euclid(16);
        if overlays.collision {
            for col in cols.clone().filter(|&c| c >= 0 && c * 16 < self.size.0) {
                for row in rows.clone().filter(|&r| r >= 0 && r * 16 < self.size.1) {
                    let Some(metatile) = self.metatile_at(col as usize, row as usize) else {
                        continue;
                    };
                    if let Some(color) = material_color(self.material(metatile)) {
                        canvas.blend(col * 16, row * 16, 16, 16, color, 110);
                    }
                }
            }
        }
        if overlays.grid {
            for col in cols.clone() {
                let screen = (col * 16).rem_euclid(WIDTH as i32) == 0;
                canvas.blend(
                    col * 16,
                    y,
                    1,
                    canvas.height as i32,
                    [255; 3],
                    if screen { 90 } else { 28 },
                );
            }
            for row in rows.clone() {
                canvas.blend(x, row * 16, canvas.width as i32, 1, [255; 3], 28);
            }
        }
        if let Some((a, b)) = highlight.cells {
            let (left, top) = (a.0.min(b.0) as i32 * 16, a.1.min(b.1) as i32 * 16);
            let (right, bottom) = (
                (a.0.max(b.0) as i32 + 1) * 16,
                (a.1.max(b.1) as i32 + 1) * 16,
            );
            canvas.outline(left, top, right - left, bottom - top, [255; 3]);
        }
        if overlays.objects {
            let exit = [80, 220, 255];
            canvas.blend(self.exit, y, 2, canvas.height as i32, exit, 200);
            canvas.text(self.exit + 4, self.size.1 - 40, "EXIT", exit);
            let start = [90, 240, 120];
            let (sx, sy) = self.start;
            canvas.blend(sx - 8, sy - 40, 16, 40, start, 70);
            canvas.outline(sx - 8, sy - 40, 16, 40, start);
            canvas.text(sx - 9, sy - 47, "START", start);
            for (i, enemy) in self.enemies.iter().enumerate() {
                let color = if highlight.enemy == Some(i) {
                    [255, 230, 60]
                } else if self.own_enemies {
                    [255, 90, 90]
                } else {
                    [200, 140, 140]
                };
                let (left, top, w, h) = enemy_box(enemy);
                canvas.blend(left, top, w, h, color, 60);
                canvas.outline(left, top, w, h, color);
                canvas.text(left + 1, top + 2, &format!("{:02X}", enemy.kind), color);
            }
        }
    }

    /// Every metatile and art cell the palette offers, as one RGB24 sheet
    /// of 16x16 cells in `columns` columns, transparency shown as a
    /// checkerboard. Returns the sheet's size.
    pub fn sheet(&self, metatiles: &[u8], columns: usize) -> (usize, usize, Vec<u8>) {
        let rows = metatiles.len().div_ceil(columns).max(1);
        let (width, height) = (columns * 16, rows * 16);
        let mut rgb = vec![0u8; width * height * 3];
        let assets = &self.viewer.assets;
        for (n, &id) in metatiles.iter().enumerate() {
            let (cx, cy) = (n % columns * 16, n / columns * 16);
            let attrs = assets
                .data
                .metatiles
                .get(usize::from(id))
                .copied()
                .unwrap_or_default();
            for py in 0..16 {
                for px in 0..16 {
                    let attr = attrs[py / 8 * 2 + px / 8];
                    let (x, y) = (px % 8, py % 8);
                    let sx = if attr & 0x800 != 0 { 7 - x } else { x };
                    let sy = if attr & 0x1000 != 0 { 7 - y } else { y };
                    let byte = assets.tiles[usize::from(attr & 0x7ff)].0[sy * 4 + sx / 2];
                    let value = usize::from(if sx % 2 == 0 { byte >> 4 } else { byte & 15 });
                    let color = if value == 0 {
                        if (px / 4 + py / 4) % 2 == 0 {
                            [44, 48, 56]
                        } else {
                            [30, 33, 40]
                        }
                    } else {
                        assets.palettes[usize::from(attr >> 13 & 3)][value]
                    };
                    let at = ((cy + py) * width + cx + px) * 3;
                    rgb[at..at + 3].copy_from_slice(&color);
                }
            }
        }
        (width, height, rgb)
    }
}

/// An enemy's marker: 16x32 standing on its position.
fn enemy_box(e: &Marker) -> (i32, i32, i32, i32) {
    (e.x - 8, e.y - 32, 16, 32)
}

/// The overlay colour of a collision material.
pub fn material_color(material: u8) -> Option<[u8; 3]> {
    Some(match material {
        0 => return None,
        1 => [230, 60, 60],       // solid
        2 => [245, 160, 40],      // ledge
        3 | 4 => [170, 90, 230],  // conveyors
        5 => [255, 40, 200],      // spikes
        6 => [60, 210, 235],      // hang bar
        7 => [240, 225, 60],      // platform
        8 => [130, 150, 210],     // no cling
        9 | 13 => [80, 210, 110], // walls
        12 => [160, 110, 60],     // mire
        15 => [60, 120, 245],     // splash
        _ => [255, 255, 255],
    })
}

/// A collision material's manifest name.
pub fn material_name(material: u8) -> &'static str {
    crate::expansion::Collision::ALL
        .iter()
        .find(|c| c.material() == material)
        .map_or("other", |c| c.name())
}

/// An RGB24 picture of the world from `origin`.
struct Canvas {
    width: usize,
    height: usize,
    origin: (i32, i32),
    rgb: Vec<u8>,
}

impl Canvas {
    /// Mix `color` over a world rectangle, `alpha` of 255.
    fn blend(&mut self, x: i32, y: i32, w: i32, h: i32, color: [u8; 3], alpha: u16) {
        let (ox, oy) = self.origin;
        let left = (x - ox).max(0);
        let top = (y - oy).max(0);
        let right = (x - ox + w).min(self.width as i32);
        let bottom = (y - oy + h).min(self.height as i32);
        for py in top..bottom {
            for px in left..right {
                let at = (py as usize * self.width + px as usize) * 3;
                for (c, &v) in self.rgb[at..at + 3].iter_mut().zip(&color) {
                    *c = ((u16::from(*c) * (255 - alpha) + u16::from(v) * alpha) / 255) as u8;
                }
            }
        }
    }
    fn outline(&mut self, x: i32, y: i32, w: i32, h: i32, color: [u8; 3]) {
        self.blend(x, y, w, 1, color, 255);
        self.blend(x, y + h - 1, w, 1, color, 255);
        self.blend(x, y, 1, h, color, 255);
        self.blend(x + w - 1, y, 1, h, color, 255);
    }
    fn text(&mut self, x: i32, y: i32, value: &str, color: [u8; 3]) {
        for (i, ch) in value.chars().enumerate() {
            for (dy, row) in glyph(ch.to_ascii_uppercase()).iter().enumerate() {
                for dx in 0..3 {
                    if row & (4 >> dx) != 0 {
                        self.blend(x + i as i32 * 4 + dx, y + dy as i32, 1, 1, color, 255);
                    }
                }
            }
        }
    }
    /// Darken what lies outside a `w`x`h` scene.
    fn shade_outside(&mut self, w: i32, h: i32) {
        let (ox, oy) = self.origin;
        let (cw, ch) = (self.width as i32, self.height as i32);
        let dark = [8, 10, 14];
        self.blend(ox, oy, -ox, ch, dark, 210);
        self.blend(w, oy, ox + cw - w, ch, dark, 210);
        self.blend(ox.max(0), oy, cw, -oy, dark, 210);
        self.blend(ox.max(0), h, cw, oy + ch - h, dark, 210);
    }
}
