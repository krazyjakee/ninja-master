//! Versioned PNG/JSON art packs. Coordinates and gameplay stay in native pixels.
use crate::{
    pack::{Kind, Member, Source},
    render::{Color, Palette, Tile},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Cursor},
    path::{Path, PathBuf},
    rc::Rc,
};

mod player;
mod template;
pub use crate::pack::{Library, PackEntry, resolve_selection, selection_value};
pub use player::PlayerSheet;
pub use template::export_template;

pub const MANIFEST: &str = "pack.json";
pub const MAX_SCALE: usize = 10;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PACK_BYTES: usize = 1024 * 1024 * 1024;
const MAX_PNG_BYTES: usize = MAX_IMAGE_BYTES * 2 + 1024 * 1024;

pub static KIND: Kind = Kind {
    manifest: MANIFEST,
    folder: "art-packs",
    env: "NINJA_MASTER_ART_PACKS",
    max_bytes: MAX_PACK_BYTES,
    name: |source| Ok(manifest(source)?.name),
};

pub fn library_directory() -> PathBuf {
    crate::pack::library_directory(&KIND)
}

/// Discovery validates references without decoding potentially large PNGs.
fn manifest(source: &mut Source) -> Result<Manifest, String> {
    let manifest: Manifest = serde_json::from_slice(&source.manifest_bytes()?)
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
    if manifest.version != 1 {
        return Err(format!("unsupported art pack version {}", manifest.version));
    }
    if manifest.name.contains('\0') {
        return Err("pack name cannot contain a null character".into());
    }
    for sheet in &manifest.sheets {
        source.location(&sheet.file)?;
    }
    for sheet in &manifest.player_frames {
        source.location(&sheet.file)?;
    }
    Ok(manifest)
}

fn image(source: &mut Source, name: &str) -> Result<(usize, usize, Vec<[u8; 4]>), String> {
    match source.member(name, MAX_PNG_BYTES)? {
        Member::File(path) => read_png(&path),
        Member::Bytes(bytes) => decode_png(Cursor::new(bytes)),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub sheets: Vec<Sheet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub player_frames: Vec<PlayerSheet>,
}

/// Rectangles use the original sheet's coordinates. Enlarge the entire PNG
/// by an integer factor without changing any JSON coordinates.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub file: String,
    pub width: usize,
    pub height: usize,
    pub palettes: [Palette; 4],
    pub cells: Vec<Cell>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    /// All 32 source bytes, hex encoded; independent of recycled VRAM slots.
    pub tile: String,
    pub line: u8,
    pub x: usize,
    pub y: usize,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
    /// An assembled frame can contain overlapping pieces. Their original
    /// composite crop, in canonical orientation, identifies untouched copies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Vec<[u8; 4]>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Key(pub [u8; 32], pub u8);

#[derive(PartialEq, Eq)]
pub(crate) struct Replacement {
    pub scale: usize,
    pub pixels: Vec<[u8; 4]>,
    pub palette: Palette,
}

pub struct ArtPack {
    pub name: String,
    pub scale: usize,
    tiles: HashMap<Key, Rc<Replacement>>,
    player: player::PlayerArt,
}

impl ArtPack {
    /// A pack directory, pack.json, or a ZIP with one root/wrapped manifest.
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut source = Source::open(path, &KIND)?;
        let manifest = manifest(&mut source)?;
        let mut pack = Self {
            name: manifest.name,
            scale: 1,
            tiles: HashMap::new(),
            player: player::PlayerArt::default(),
        };
        let mut total = 0;
        for sheet in manifest.sheets {
            let (width, height, image) = image(&mut source, &sheet.file)?;
            total += image.len() * 4;
            if total > MAX_PACK_BYTES {
                return Err("art pack exceeds 1 GiB of decoded sheets".into());
            }
            let scale = width.checked_div(sheet.width).unwrap_or(0);
            if !(1..=MAX_SCALE).contains(&scale)
                || sheet.width.checked_mul(scale) != Some(width)
                || sheet.height.checked_mul(scale) != Some(height)
            {
                return Err(format!(
                    "{}: PNG must be 1..10 times the manifest dimensions in both axes",
                    sheet.file
                ));
            }
            pack.scale = pack.scale.max(scale);
            for cell in sheet.cells {
                let bytes = decode_tile(&cell.tile)?;
                if cell.line > 3
                    || cell.x.checked_add(8).is_none_or(|x| x > sheet.width)
                    || cell.y.checked_add(8).is_none_or(|y| y > sheet.height)
                {
                    return Err(format!(
                        "{}: cell outside sheet or invalid palette line",
                        sheet.file
                    ));
                }
                let palette = sheet.palettes[usize::from(cell.line)];
                if cell.baseline.as_ref().is_some_and(|b| b.len() != 64) {
                    return Err("cell baseline must contain 64 RGBA pixels".into());
                }
                let side = 8 * scale;
                let mut pixels = Vec::with_capacity(side * side);
                let mut changed = false;
                for y in 0..side {
                    for x in 0..side {
                        let sx = if cell.flip_x { side - 1 - x } else { x };
                        let sy = if cell.flip_y { side - 1 - y } else { y };
                        let pixel = image[(cell.y * scale + sy) * width + cell.x * scale + sx];
                        let index = tile_index(&bytes, x / scale, y / scale);
                        let expected = cell.baseline.as_ref().map_or_else(
                            || rgba(palette[index as usize], index != 0),
                            |b| b[y / scale * 8 + x / scale],
                        );
                        changed |= pixel[3] != expected[3]
                            || (pixel[3] != 0 && pixel[..3] != expected[..3]);
                        pixels.push(pixel);
                    }
                }
                // Untouched template cells keep the ROM's palette effects and
                // never compete with an edited occurrence of a shared pattern.
                if !changed {
                    continue;
                }
                let replacement = Rc::new(Replacement {
                    scale,
                    pixels,
                    palette,
                });
                let key = Key(bytes, cell.line);
                if let Some(previous) = pack.tiles.get(&key) {
                    if previous != &replacement {
                        return Err(format!(
                            "conflicting edits of shared tile {} on line {}; edit one occurrence or keep copies identical",
                            cell.tile, cell.line
                        ));
                    }
                } else {
                    pack.tiles.insert(key, replacement);
                }
            }
        }
        pack.player = player::PlayerArt::load(&mut source, manifest.player_frames, &mut total)?;
        pack.scale = pack.scale.max(pack.player.scale());
        Ok(pack)
    }

    pub fn replacement_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn player_frame_count(&self) -> usize {
        self.player.len()
    }

    pub(crate) fn player_cells(
        &self,
        id: usize,
        cells: impl Iterator<Item = (i32, i32, crate::render::Flip)>,
    ) -> Option<Rc<Vec<Rc<Replacement>>>> {
        self.player.cells(id, cells)
    }

    pub(crate) fn replacement(&self, tile: &Tile, line: Option<u8>) -> Option<Rc<Replacement>> {
        self.tiles.get(&Key(tile.0, line?)).cloned()
    }
}

fn decode_tile(hex: &str) -> Result<[u8; 32], String> {
    if hex.len() != 64 || !hex.is_ascii() {
        return Err("tile must contain 64 hexadecimal characters".into());
    }
    let mut bytes = [0; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| "invalid tile hex")?;
    }
    Ok(bytes)
}

fn tile_index(tile: &[u8; 32], x: usize, y: usize) -> u8 {
    let byte = tile[y * 4 + x / 2];
    if x.is_multiple_of(2) {
        byte >> 4
    } else {
        byte & 15
    }
}

fn rgba(color: Color, opaque: bool) -> [u8; 4] {
    [color[0], color[1], color[2], if opaque { 255 } else { 0 }]
}

fn read_png(path: &Path) -> Result<(usize, usize, Vec<[u8; 4]>), String> {
    decode_png(BufReader::new(
        fs::File::open(path).map_err(|e| e.to_string())?,
    ))
}

pub(crate) fn decode_png(
    reader: impl BufRead + std::io::Seek,
) -> Result<(usize, usize, Vec<[u8; 4]>), String> {
    let mut decoder = png::Decoder::new_with_limits(
        reader,
        png::Limits {
            bytes: MAX_IMAGE_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    let (width, height) = (info.width as usize, info.height as usize);
    if width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .is_none_or(|n| n > MAX_IMAGE_BYTES)
    {
        return Err("PNG exceeds 64 MiB decoded RGBA size".into());
    }
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let channels = info.color_type.samples();
    let pixels = buffer[..info.buffer_size()]
        .chunks_exact(channels)
        .map(|p| match info.color_type {
            png::ColorType::Rgba => [p[0], p[1], p[2], p[3]],
            png::ColorType::Rgb => [p[0], p[1], p[2], 255],
            png::ColorType::GrayscaleAlpha => [p[0], p[0], p[0], p[1]],
            png::ColorType::Grayscale => [p[0], p[0], p[0], 255],
            png::ColorType::Indexed => unreachable!("EXPAND expands indexed PNGs"),
        })
        .collect();
    Ok((width, height, pixels))
}

pub(crate) fn write_png(
    path: &Path,
    width: usize,
    height: usize,
    pixels: &[u8],
) -> Result<(), String> {
    let file = fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(pixels).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}
