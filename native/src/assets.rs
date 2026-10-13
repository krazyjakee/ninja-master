//! Decompressors for the ROM's stream formats and `StageAssets`, the
//! presentation state (animated tiles, palettes and effects) of one stage
//! part loaded through `stage::StageData`. Addresses and formats are
use crate::render::{Palette, Tile, cram_palettes};
use crate::rom::Result;
use crate::stage::{Orientation, StageData, StageEffects, StageId, UploadKind};
use std::ops::Range;

#[path = "player_assets.rs"]
mod player_assets;
pub use player_assets::{PlayerAssets, SoundAssets};
#[path = "player_animation.rs"]
mod player_animation;
pub use player_animation::{
    PlayerAnimation, hang_phase_next, hang_steps, player_body_top, sequence, sequences, step_cursor,
};

/// Byte cursor of the stream decoders.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8> {
        let value = *self
            .data
            .get(self.pos)
            .ok_or_else(|| format!("truncated data at ${:06x}", self.pos))?;
        self.pos += 1;
        Ok(value)
    }
    fn word(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes([self.byte()?, self.byte()?]))
    }
}

fn copy_back(out: &mut Vec<u8>, distance: usize, length: usize, limit: usize) -> Result<()> {
    if distance == 0 || distance > out.len() {
        return Err("back-reference precedes decoded data".into());
    }
    if out.len() + length > limit {
        return Err("decoded data exceeds output limit".into());
    }
    for _ in 0..length {
        out.push(out[out.len() - distance]);
    }
    Ok(())
}
fn literal(out: &mut Vec<u8>, value: u8, limit: usize) -> Result<()> {
    if out.len() >= limit {
        return Err("decoded data exceeds output limit".into());
    }
    out.push(value);
    Ok(())
}

/// ROM routine $3f8a. Header selects 1..4 length bits and a count of eight-token groups.
/// Padding tokens can produce more bytes than a caller's tile transfer requires.
pub fn musashi(data: &[u8], start: usize, limit: usize) -> Result<(Vec<u8>, usize)> {
    let mut r = Reader { data, pos: start };
    let header = r.word()?;
    let bits = 4 - (header >> 14);
    let mask = (1 << bits) - 1;
    let mut out = Vec::new();
    for _ in 0..=(header & 0x3fff) {
        let flags = r.byte()?;
        for bit in (0..8).rev() {
            let token = u16::from(r.byte()?);
            if flags & (1 << bit) == 0 {
                literal(&mut out, token as u8, limit)?;
            } else {
                copy_back(
                    &mut out,
                    usize::from((token >> bits) + 1),
                    usize::from((token & mask) + 2),
                    limit,
                )?;
            }
        }
    }
    Ok((out, r.pos))
}

struct Bits<'a> {
    r: Reader<'a>,
    flags: u16,
    remaining: u8,
}
impl<'a> Bits<'a> {
    fn new(data: &'a [u8], start: usize) -> Result<Self> {
        let mut this = Self {
            r: Reader { data, pos: start },
            flags: 0,
            remaining: 0,
        };
        this.refill()?;
        Ok(this)
    }
    fn refill(&mut self) -> Result<()> {
        self.flags = u16::from_le_bytes([self.r.byte()?, self.r.byte()?]);
        self.remaining = 16;
        Ok(())
    }
    fn bit(&mut self) -> Result<bool> {
        let bit = self.flags & 1 != 0;
        self.flags >>= 1;
        self.remaining -= 1;
        // The 68k routine prefetches immediately, BEFORE the next token byte.
        if self.remaining == 0 {
            self.refill()?;
        }
        Ok(bit)
    }
}

/// ROM routine $3fe2 (Kosinski), including overlapping copies and eager flag refill.
pub fn kosinski(data: &[u8], start: usize, limit: usize) -> Result<(Vec<u8>, usize)> {
    let mut b = Bits::new(data, start)?;
    let mut out = Vec::new();
    loop {
        if b.bit()? {
            literal(&mut out, b.r.byte()?, limit)?;
        } else {
            let (distance, length) = if !b.bit()? {
                let length = usize::from(b.bit()?) * 2 + usize::from(b.bit()?) + 2;
                (256 - usize::from(b.r.byte()?), length)
            } else {
                let low = usize::from(b.r.byte()?);
                let high = usize::from(b.r.byte()?);
                let distance = 8192 - (((high & 0xf8) << 5) | low);
                let length = if high & 7 != 0 {
                    (high & 7) + 2
                } else {
                    match b.r.byte()? {
                        0 => return Ok((out, b.r.pos)),
                        1 => continue,
                        length => usize::from(length) + 1,
                    }
                };
                (distance, length)
            };
            copy_back(&mut out, distance, length, limit)?;
        }
    }
}

/// Slice-by-8 CRC-32 (IEEE) tables: `CRC_TABLES[0]` is each byte value's
/// CRC, `CRC_TABLES[k]` the same byte followed by `k` zero bytes.
const CRC_TABLES: [[u32; 256]; 8] = {
    let mut tables = [[0; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
            bit += 1;
        }
        tables[0][i] = crc;
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            let previous = tables[k - 1][i];
            tables[k][i] = (previous >> 8) ^ tables[0][(previous & 0xff) as usize];
            i += 1;
        }
        k += 1;
    }
    tables
};

/// CRC-32 (IEEE). Stage and player loads check the whole ROM with it.
pub fn crc32(data: &[u8]) -> u32 {
    let t = &CRC_TABLES;
    let (blocks, rest) = data.as_chunks::<8>();
    let mut crc = !0u32;
    for block in blocks {
        let low = crc ^ u32::from_le_bytes([block[0], block[1], block[2], block[3]]);
        crc = t[7][(low & 0xff) as usize]
            ^ t[6][((low >> 8) & 0xff) as usize]
            ^ t[5][((low >> 16) & 0xff) as usize]
            ^ t[4][(low >> 24) as usize]
            ^ t[3][usize::from(block[4])]
            ^ t[2][usize::from(block[5])]
            ^ t[1][usize::from(block[6])]
            ^ t[0][usize::from(block[7])];
    }
    for &byte in rest {
        crc = (crc >> 8) ^ t[0][usize::from(crc as u8 ^ byte)];
    }
    !crc
}

/// Bind user-supplied content and prepared metadata to the canonical US image.
pub(crate) fn verified(rom: &[u8]) -> Result<()> {
    if rom.len() != 0x100000 || crc32(rom) != 0x5381506f {
        Err("asset extraction requires the verified US ROM (CRC32 5381506f)".into())
    } else {
        Ok(())
    }
}

/// One stage part as the viewer and the game present it: the ROM data plus
/// the state its effects change every frame.
#[derive(Clone)]
pub struct StageAssets {
    pub data: StageData,
    /// Palette routine, tile animation, scenery sprites and the 1-1 event.
    pub effects: StageEffects,
    /// VRAM patterns as currently animated (`data.tiles` at load).
    pub tiles: Vec<Tile>,
    /// CRAM words as currently changed by the palette routine, and as RGB.
    pub palette_words: [[u16; 16]; 4],
    pub palettes: [Palette; 4],
    /// Pattern ranges of the viewer's sheets.
    pub foreground_range: Range<usize>,
    pub background_range: Range<usize>,
    pub object_range: Range<usize>,
}

impl StageAssets {
    /// Any round/substage/part, with the effects of its routine tables.
    pub fn load_stage(rom: &[u8], id: StageId) -> Result<Self> {
        verified(rom)?;
        let data = StageData::load(rom, id)?;
        let effects = StageEffects::load(rom, id, data.mode.boss_arena())?;
        let mut tiles = data.tiles.clone();
        tiles[0] = Tile([0; 32]); // Blank pattern established by the game's VRAM clear.
        let mut foreground_range = data.upload_range(UploadKind::Foreground);
        // $39112/$391BC: 1-1 starts with its vegetation primed; each four-tile
        // patch has two frames and the descending transfers leave the even one.
        if let Some(animation) = effects.animation.as_ref().filter(|a| a.prime.is_some()) {
            animation.prime(&mut tiles);
            for (dest, patch) in animation.channels.iter().flat_map(|c| &c.frames) {
                foreground_range.end = foreground_range.end.max(dest + patch.len());
            }
        }
        let object_range = {
            let objects = data.upload_range(UploadKind::Object);
            let boss = data.upload_range(UploadKind::Boss);
            if boss.is_empty() {
                objects
            } else if objects.is_empty() {
                boss
            } else {
                objects.start.min(boss.start)..objects.end.max(boss.end)
            }
        };
        Ok(Self {
            effects,
            tiles,
            palette_words: data.palette_words,
            palettes: cram_palettes(&data.palette_words),
            foreground_range,
            background_range: data.upload_range(UploadKind::Background),
            object_range,
            data,
        })
    }

    /// `$3854` D2 bit 15 for an object whose sprite priority bit is clear:
    /// the material is in the `$3906` high word `$55BE` (1-5, 7, 8, 10, 12, 14).
    /// This is the generic `BMI` test of ordinary objects, not the player's
    /// collision; `collision_at` and `movement::Material` give the rest.
    pub fn solid_at(&self, x: i32, y: i32) -> bool {
        crate::movement::Material(self.collision_at(x, y)).object_solid(false)
    }

    /// `$3854`: the material code (low nibble) after quadrant exclusion, 0
    /// outside the layout.
    pub fn collision_at(&self, x: i32, y: i32) -> u8 {
        let (width, height) = self.data.pixel_size();
        let vertical = self.data.orientation == Orientation::Vertical;
        if x < 0 || y < 0 || (vertical && x >= width) || (!vertical && y >= height) {
            return 0;
        }
        let Some(id) = self.data.metatile_at(x / 16, y / 16) else {
            return 0;
        };
        let value = self.data.collision[usize::from(id)];
        let quadrant = ((y as usize & 8) >> 2) | ((x as usize & 8) >> 3);
        if value & (0x80 >> quadrant) != 0 {
            0
        } else {
            value & 15
        }
    }

    pub fn report(&self) -> String {
        self.data.report()
    }
}
