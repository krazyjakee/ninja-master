//! Big-endian readers for the cartridge image and tables shared by several
//! object families.

pub type Result<T> = std::result::Result<T, String>;
#[path = "rom_regions.rs"]
pub mod regions;
#[path = "rom_regions_data.rs"]
mod regions_data;

pub(crate) fn byte(rom: &[u8], p: usize) -> Result<u8> {
    rom.get(p)
        .copied()
        .ok_or_else(|| format!("truncated ROM byte at ${p:06x}"))
}
/// A signed byte, e.g. the operand of `moveq #imm,dn`.
pub(crate) fn signed_byte(rom: &[u8], p: usize) -> Result<i32> {
    Ok(i32::from(byte(rom, p)? as i8))
}
pub(crate) fn word(rom: &[u8], p: usize) -> Result<u16> {
    rom.get(p..p + 2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("truncated ROM word at ${p:06x}"))
}
/// An unsigned word widened for 16.16 arithmetic.
pub(crate) fn word_i(rom: &[u8], p: usize) -> Result<i32> {
    Ok(i32::from(word(rom, p)?))
}
pub(crate) fn signed(rom: &[u8], p: usize) -> Result<i32> {
    Ok(i32::from(word(rom, p)? as i16))
}
pub(crate) fn pointer(rom: &[u8], p: usize) -> Result<usize> {
    let source = regions::source(rom, p);
    Ok(regions::pointer(rom, raw_pointer(rom, source)?))
}
pub(crate) fn raw_pointer(rom: &[u8], p: usize) -> Result<usize> {
    Ok((usize::from(word(rom, p)?) << 16) | usize::from(word(rom, p + 2)?))
}
/// A 16.16 long immediate, e.g. the operand of `move.l #$FFFD8000,+$1C(a0)`.
pub(crate) fn long(rom: &[u8], p: usize) -> Result<i32> {
    Ok(raw_pointer(rom, p)? as u32 as i32)
}
pub(crate) fn words<const N: usize>(rom: &[u8], p: usize) -> Result<[u16; N]> {
    let mut out = [0; N];
    for (i, value) in out.iter_mut().enumerate() {
        *value = word(rom, p + i * 2)?;
    }
    Ok(out)
}
/// Mapping word list ended by the ROM's negative ($FFFF) terminator.
pub(crate) fn sequence(rom: &[u8], p: usize) -> Result<Vec<u16>> {
    let mut out = Vec::new();
    for i in 0..64 {
        let value = word(rom, p + i * 2)?;
        if value & 0x8000 != 0 {
            return Ok(out);
        }
        out.push(value);
    }
    Err(format!("unterminated ROM sequence at ${p:06x}"))
}

/// `$9748`: the 1024-step sine table (signed, 1.0 = `$4000`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sine(Vec<i16>);
impl Sine {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        (0..1024)
            .map(|i| Ok(word(rom, 0x9748 + i * 2)? as i16))
            .collect::<Result<_>>()
            .map(Self)
    }
    /// Entry `index` modulo the table length.
    pub(crate) fn at(&self, index: usize) -> i16 {
        self.0[index & 0x3ff]
    }
    /// The word at byte offset `angle & $7FE` (a 2048-step angle, as the
    /// raster wobbles and boss handlers index it).
    pub(crate) fn word(&self, angle: u16) -> i16 {
        self.at(usize::from(angle & 0x7fe) >> 1)
    }
    /// `$94A0`: (cos * 17/16, sin) of a 1024-step angle in 16.16 pixels.
    pub(crate) fn polar(&self, angle: u16) -> (i32, i32) {
        let index = usize::from(angle);
        let sin = i32::from(self.at(index)) << 2;
        let cos = i32::from(self.at(index + 256)) << 2;
        (cos + (cos >> 4), sin)
    }
}

/// `Sprite_Mappings` (`$43222`): per id, a hitbox word pair then 6-byte
/// pieces, the last with a nonzero final byte.
pub(crate) const SPRITE_MAPPINGS: usize = 0x43222;
/// The table holds 959 offsets: the first record follows at `$439A0`.
pub(crate) const SPRITE_MAPPING_COUNT: u16 = ((0x439a0 - SPRITE_MAPPINGS) / 2) as u16;

/// One `Sprite_Mappings` piece: offset from the object origin, VDP attribute
/// word and size in cells (patterns advance down each column first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpritePiece {
    pub x: i32,
    pub y: i32,
    pub attr: u16,
    pub width: usize,
    pub height: usize,
}

/// Mapping `id`'s hitbox bytes (signed center x and y, unsigned half width
/// and height) and its pieces in SAT order (`$39ABA`; `$14F0` rejects
/// `>= $3C0`).
pub fn sprite_mapping(rom: &[u8], id: u16) -> Result<([u8; 4], Vec<SpritePiece>)> {
    if id >= 0x3c0 {
        return Err(format!("sprite mapping ${id:03x} outside the table"));
    }
    let mut p = SPRITE_MAPPINGS + usize::from(word(rom, SPRITE_MAPPINGS + usize::from(id) * 2)?);
    let hitbox: [u8; 4] = rom
        .get(p..p + 4)
        .and_then(|b| b.try_into().ok())
        .ok_or("truncated sprite mapping")?;
    p += 4;
    let mut pieces = Vec::new();
    for _ in 0..80 {
        let data = rom.get(p..p + 6).ok_or("truncated sprite mapping")?;
        let size = data[4] >> 1;
        pieces.push(SpritePiece {
            x: i32::from(data[0] as i8),
            y: i32::from(data[1] as i8),
            attr: u16::from_be_bytes([data[2], data[3]]),
            width: usize::from((size >> 2) & 3) + 1,
            height: usize::from(size & 3) + 1,
        });
        p += 6;
        if data[5] != 0 {
            return Ok((hitbox, pieces));
        }
    }
    Err(format!("unterminated sprite mapping ${id:03x}"))
}
