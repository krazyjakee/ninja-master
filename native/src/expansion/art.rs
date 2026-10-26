//! Custom stage art: new metatiles painted in the base stage's palettes.
//!
//! A part's `art` names a PNG sheet of 16x16 cells, each a new metatile the
//! layout places by a character of its own. Every 8x8 quarter of a cell is
//! one Mega Drive tile, so it may only use the colours of one of the base
//! stage's four palette lines; transparent pixels show what is behind it,
//! like colour 0. The tiles load into free pattern slots and draw through
//! the same renderer as the cartridge's art.
use super::Source;
use crate::{
    assets::StageAssets,
    render::{Palette, Tile},
    rom::Result,
};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashMap},
    io::Cursor,
    path::Path,
};

const MAX_SHEET_BYTES: usize = 16 * 1024 * 1024;
/// Stage scenery shares pattern slots below the object art at `$300`; the
/// first 128 hold the font and resident cells.
const SLOTS: std::ops::Range<usize> = 128..0x300;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtEntry {
    /// A PNG of 16x16 cells.
    sheet: String,
    /// Layout character to cell.
    cells: BTreeMap<String, CellEntry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CellEntry {
    /// Cell column and row in the sheet.
    at: [usize; 2],
    #[serde(default)]
    collision: Collision,
    /// Drawn in front of sprites (name table priority bit).
    #[serde(default)]
    front: bool,
}

/// What a metatile is to the player (`crate::movement::Material`).
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Collision {
    #[default]
    None,
    Solid,
    /// A one-way floor that can be hung from.
    Ledge,
    ConveyorRight,
    ConveyorLeft,
    Spikes,
    HangBar,
    /// A one-way floor.
    Platform,
    NoCling,
    WallRight,
    Mire,
    WallLeft,
    Splash,
}
impl Collision {
    fn material(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Solid => 1,
            Self::Ledge => 2,
            Self::ConveyorRight => 3,
            Self::ConveyorLeft => 4,
            Self::Spikes => 5,
            Self::HangBar => 6,
            Self::Platform => 7,
            Self::NoCling => 8,
            Self::WallRight => 9,
            Self::Mire => 12,
            Self::WallLeft => 13,
            Self::Splash => 15,
        }
    }
}

/// One new metatile: its number in the part's layout and its pixels.
#[derive(Clone, Debug)]
struct Cell {
    id: u8,
    /// Where it came from, for messages.
    at: [usize; 2],
    pixels: Vec<[u8; 4]>,
    collision: Collision,
    front: bool,
}

/// A part's new metatiles.
#[derive(Clone, Debug)]
pub(super) struct Art {
    cells: Vec<Cell>,
}

impl Art {
    /// Read the sheet and give each cell a metatile number its layout's
    /// legend does not use, counting down from `$FF`. Returns the cells'
    /// characters with their numbers, for the layout.
    pub(super) fn read(
        entry: &ArtEntry,
        source: &mut Source,
        legend: &BTreeMap<String, String>,
    ) -> Result<(Self, BTreeMap<String, String>)> {
        let bytes = source.read(&entry.sheet, MAX_SHEET_BYTES)?;
        let (width, height, pixels) = crate::art_pack::decode_png(Cursor::new(bytes))
            .map_err(|e| format!("{}: {e}", entry.sheet))?;
        if width % 16 != 0 || height % 16 != 0 {
            return Err(format!("{}: size must be a multiple of 16", entry.sheet));
        }
        let taken: Vec<u8> = legend
            .values()
            .filter_map(|value| u8::from_str_radix(value.trim_start_matches('$'), 16).ok())
            .collect();
        let mut free = (1..=0xffu8).rev().filter(|id| !taken.contains(id));
        let mut cells = Vec::new();
        let mut assigned = BTreeMap::new();
        for (key, cell) in &entry.cells {
            if legend.contains_key(key) {
                return Err(format!("{key:?} is in both the legend and the art"));
            }
            let [col, row] = cell.at;
            if (col + 1) * 16 > width || (row + 1) * 16 > height {
                return Err(format!(
                    "{}: cell {col},{row} is outside the sheet",
                    entry.sheet
                ));
            }
            let id = free
                .next()
                .ok_or("the layout leaves no metatile numbers for its art")?;
            let pixels = (0..16)
                .flat_map(|y| (0..16).map(move |x| (x, y)))
                .map(|(x, y)| pixels[(row * 16 + y) * width + col * 16 + x])
                .collect();
            cells.push(Cell {
                id,
                at: cell.at,
                pixels,
                collision: cell.collision,
                front: cell.front,
            });
            assigned.insert(key.clone(), format!("{id:02X}"));
        }
        Ok((Self { cells }, assigned))
    }

    /// Load the tiles into free pattern slots and define the metatiles,
    /// matching colours against the base's palettes as the part loads.
    pub(super) fn apply(&self, assets: &mut StageAssets) -> Result<()> {
        let palettes = assets.palettes;
        let data = &mut assets.data;
        let free: Vec<usize> = SLOTS.filter(|&slot| !data.loaded[slot]).collect();
        let mut free = free.into_iter();
        let mut slots: HashMap<[u8; 32], u16> = HashMap::new();
        if data.metatiles.len() < 256 {
            data.metatiles.resize(256, [0; 4]);
        }
        for cell in &self.cells {
            let mut attrs = [0u16; 4];
            for (quarter, attr) in attrs.iter_mut().enumerate() {
                let (qx, qy) = (quarter % 2 * 8, quarter / 2 * 8);
                let pixels: Vec<[u8; 4]> = (0..8)
                    .flat_map(|y| (0..8).map(move |x| (x, y)))
                    .map(|(x, y)| cell.pixels[(qy + y) * 16 + qx + x])
                    .collect();
                let Some((tile, line)) = tile(&pixels, &palettes).map_err(|e| {
                    format!(
                        "art cell {},{} quarter {quarter}: {e}",
                        cell.at[0], cell.at[1]
                    )
                })?
                else {
                    continue; // Transparent: pattern 0.
                };
                let slot = match slots.get(&tile.0) {
                    Some(&slot) => slot,
                    None => {
                        let slot = free
                            .next()
                            .ok_or("the base has no pattern slots left for the art")?;
                        data.tiles[slot] = tile;
                        data.loaded[slot] = true;
                        assets.tiles[slot] = tile;
                        slots.insert(tile.0, slot as u16);
                        slot as u16
                    }
                };
                *attr = slot | u16::from(line) << 13 | u16::from(cell.front) << 15;
            }
            data.metatiles[usize::from(cell.id)] = attrs;
            data.collision[usize::from(cell.id)] = cell.collision.material();
        }
        Ok(())
    }
}

/// A Mega Drive colour level (0..7) per channel.
fn level(rgb: [u8; 3]) -> [u8; 3] {
    rgb.map(|c| ((u16::from(c) * 7 + 127) / 255) as u8)
}

/// An 8x8 block of RGBA pixels as a 4bpp pattern on one palette line, or
/// `None` when every pixel is transparent.
fn tile(pixels: &[[u8; 4]], palettes: &[Palette; 4]) -> Result<Option<(Tile, u8)>> {
    let opaque: Vec<[u8; 3]> = pixels
        .iter()
        .filter(|p| p[3] >= 128)
        .map(|p| level([p[0], p[1], p[2]]))
        .collect();
    if opaque.is_empty() {
        return Ok(None);
    }
    let index =
        |line: &Palette, rgb: [u8; 3]| (1..16).find(|&i| level(line[i]) == rgb).map(|i| i as u8);
    let Some(line) = (0..4).find(|&l| opaque.iter().all(|&rgb| index(&palettes[l], rgb).is_some()))
    else {
        let missing = opaque
            .iter()
            .find(|&&rgb| palettes.iter().all(|line| index(line, rgb).is_none()));
        return Err(match missing {
            Some(rgb) => format!("colour {} is in none of the palette lines", hex(*rgb)),
            None => "its colours are not all in one palette line".into(),
        });
    };
    let mut bytes = [0u8; 32];
    for (i, p) in pixels.iter().enumerate() {
        let value = if p[3] >= 128 {
            index(&palettes[line], level([p[0], p[1], p[2]])).unwrap()
        } else {
            0
        };
        bytes[i / 2] |= if i % 2 == 0 { value << 4 } else { value };
    }
    Ok(Some((Tile(bytes), line as u8)))
}

fn hex(level: [u8; 3]) -> String {
    let rgb = level.map(|l| (u16::from(l) * 255 / 7) as u8);
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// Write an art template for parts built on `assets`' stage: its palette
/// lines as swatches (`palettes.png`, one 16x16 swatch per colour, lines
/// top to bottom, colour 0 left blank), a blank sheet (`art.png`), and
/// every metatile the base uses with its number (`metatiles.png`,
/// `metatiles.txt`), for writing layouts.
pub fn export_template(assets: &StageAssets, directory: &Path) -> Result<()> {
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let write = |name: &str, width: usize, height: usize, rgba: &[u8]| {
        crate::art_pack::write_png(&directory.join(name), width, height, rgba)
    };
    let mut swatches = vec![0u8; 256 * 64 * 4];
    for (line, palette) in assets.palettes.iter().enumerate() {
        for (i, colour) in palette.iter().enumerate().skip(1) {
            for y in 0..16 {
                for x in 0..16 {
                    let at = ((line * 16 + y) * 256 + i * 16 + x) * 4;
                    swatches[at..at + 4].copy_from_slice(&[colour[0], colour[1], colour[2], 255]);
                }
            }
        }
    }
    write("palettes.png", 256, 64, &swatches)?;
    write("art.png", 128, 128, &vec![0u8; 128 * 128 * 4])?;
    let mut used: Vec<u8> = assets.data.layout.clone();
    used.sort_unstable();
    used.dedup();
    let columns = 16;
    let rows = used.len().div_ceil(columns);
    let (width, height) = (columns * 20, rows * 20);
    let mut sheet = vec![0u8; width * height * 4];
    let mut list = String::from("# metatile number, sheet column and row, collision material\n");
    for (n, &id) in used.iter().enumerate() {
        let (cx, cy) = (n % columns * 20 + 2, n / columns * 20 + 2);
        let attrs = assets
            .data
            .metatiles
            .get(usize::from(id))
            .copied()
            .unwrap_or_default();
        for (quarter, attr) in attrs.iter().enumerate() {
            let tile = &assets.tiles[usize::from(attr & 0x7ff)];
            let palette = &assets.palettes[usize::from(attr >> 13 & 3)];
            for y in 0..8 {
                for x in 0..8 {
                    let sx = if attr & 0x800 != 0 { 7 - x } else { x };
                    let sy = if attr & 0x1000 != 0 { 7 - y } else { y };
                    let byte = tile.0[sy * 4 + sx / 2];
                    let value = usize::from(if sx % 2 == 0 { byte >> 4 } else { byte & 15 });
                    if value == 0 {
                        continue;
                    }
                    let (px, py) = (cx + quarter % 2 * 8 + x, cy + quarter / 2 * 8 + y);
                    let at = (py * width + px) * 4;
                    let c = palette[value];
                    sheet[at..at + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
        list += &format!(
            "{id:02X} {} {} {}\n",
            n % columns,
            n / columns,
            assets.data.collision[usize::from(id)] & 15
        );
    }
    write("metatiles.png", width.max(1), height.max(1), &sheet)?;
    std::fs::write(directory.join("metatiles.txt"), list)
        .map_err(|e| format!("{}: {e}", directory.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palettes() -> [Palette; 4] {
        let mut lines = [[[0u8; 3]; 16]; 4];
        lines[1][1] = [255, 0, 0];
        lines[1][2] = [0, 0, 255];
        lines[2][1] = [0, 255, 0];
        lines[2][3] = [255, 0, 0];
        lines
    }

    #[test]
    fn tiles_take_the_first_line_holding_all_their_colours() {
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let clear = [9, 9, 9, 0];
        let mut pixels = vec![clear; 64];
        pixels[0] = red;
        pixels[9] = green;
        let (pattern, line) = tile(&pixels, &palettes()).unwrap().unwrap();
        assert_eq!(line, 2, "red and green share line 2 only");
        assert_eq!(
            pattern.0[0], 0x30,
            "red is line 2 colour 3; then transparent"
        );
        assert_eq!(pattern.0[4], 0x01, "row 1: transparent, then green");
        // A colour picker's rounding still reaches the Mega Drive level.
        pixels[9] = [250, 4, 2, 255];
        assert_eq!(tile(&pixels, &palettes()).unwrap().unwrap().1, 1);
    }

    #[test]
    fn tiles_reject_colours_outside_one_line() {
        let mut pixels = vec![[0, 0, 0, 0]; 64];
        assert!(tile(&pixels, &palettes()).unwrap().is_none());
        pixels[0] = [255, 255, 0, 255];
        let error = tile(&pixels, &palettes()).unwrap_err();
        assert!(error.contains("#FFFF00"), "{error}");
        pixels[0] = [0, 0, 255, 255];
        pixels[1] = [0, 255, 0, 255];
        let error = tile(&pixels, &palettes()).unwrap_err();
        assert!(error.contains("one palette line"), "{error}");
    }
}
