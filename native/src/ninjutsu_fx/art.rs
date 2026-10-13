//! The patterns and mappings the techniques and the shield draw with.
use crate::{
    assets::musashi,
    combat::draw_pieces,
    ninjutsu::Kind,
    render::{Frame, Lines, Tile},
    rom::{Result, SpritePiece, pointer, sprite_mapping, word},
};
use std::collections::BTreeMap;

/// Player frame table: mapping word, DMA list offset (`$23ADE`).
const FRAME_TABLE: usize = 0x23ade;
/// `$2454A`: DMA list of the shield frames (`$A1088` to tiles `$2B0..$2CF`).
const SHIELD_DMA: usize = 0x2454a;

/// The patterns and mappings one technique draws with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Art {
    pub(super) tiles: BTreeMap<usize, Tile>,
    pub(super) mappings: BTreeMap<u16, Vec<SpritePiece>>,
}
impl Art {
    pub(super) fn load(rom: &[u8], kind: Kind) -> Result<Self> {
        let mut art = Self::default();
        let ids: Vec<u16> = match kind {
            Kind::Ikazuchi => {
                // Sparks: resident block 1 ($540..$54F). Bolt: $21918 with
                // d1 = $10 at cursor $0D00 ($2B8..$2FF).
                art.resident(rom, 1)?;
                art.root(rom, 0x10)?;
                (0x365..=0x368).chain([0xe1]).collect()
            }
            Kind::Kariu => {
                // $21918, d1 = 8 at $0D00: dragon and flames ($2CC..$2FF).
                art.root(rom, 8)?;
                (0xdd..=0xe0).collect()
            }
            Kind::Fushin => {
                // Player frame 125's DMA holds the doubles' poses: mapping
                // $49 ($280..), frame 28's art at $2B0 and frame 29's at $2CB.
                art.player_frame(rom, 125)?;
                vec![0x49, 8, 7]
            }
            Kind::Mijin => {
                // $21918, d1 = $14 at $0500 (debris, $2BB..$2FF); player
                // frame 124's DMA (fireballs, $28C..$29B); resident block 2
                // (smoke, $536..$53F).
                art.root(rom, 0x14)?;
                art.player_frame(rom, 124)?;
                art.resident(rom, 2)?;
                (0xe2..=0xe5)
                    .chain(0xeb..=0xed)
                    .chain(0x35f..=0x362)
                    .collect()
            }
        };
        for id in ids {
            art.mappings.insert(id, sprite_mapping(rom, id)?.1);
        }
        Ok(art)
    }
    /// A Musashi block: first tile, count - 1, data.
    fn compressed(&mut self, rom: &[u8], source: usize) -> Result<()> {
        let first = usize::from(word(rom, source)?);
        let count = usize::from(word(rom, source + 2)?) + 1;
        let (data, _) = musashi(rom, source + 4, 65536)?;
        let data = data.get(..count * 32).ok_or("truncated ninjutsu art")?;
        for (i, bytes) in data.as_chunks::<32>().0.iter().enumerate() {
            self.tiles.insert(first + i, Tile(*bytes));
        }
        Ok(())
    }
    /// One of the eight resident `$8001C` blocks.
    fn resident(&mut self, rom: &[u8], block: usize) -> Result<()> {
        let source = pointer(rom, pointer(rom, 0x8001c)? + block * 4)?;
        self.compressed(rom, source)
    }
    /// `Player_LoadRootAsset` (`$21918`): entry `d1` of the `$80000` root.
    fn root(&mut self, rom: &[u8], offset: usize) -> Result<()> {
        let source = pointer(rom, pointer(rom, 0x80000)? + offset)?;
        self.compressed(rom, source)
    }
    /// A player DMA list: little-endian word count, `$97`, 23-bit word
    /// source, VRAM byte destination, last flag.
    fn dma(&mut self, rom: &[u8], mut cursor: usize) -> Result<()> {
        for _ in 0..16 {
            let count = rom
                .get(cursor..cursor + 3)
                .filter(|b| b[2] == 0x97)
                .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])) * 2)
                .ok_or("invalid ninjutsu DMA record")?;
            let high = usize::from(*rom.get(cursor + 3).ok_or("truncated DMA record")?);
            let source = ((high << 16) | usize::from(word(rom, cursor + 4)?)) * 2;
            let destination = usize::from(word(rom, cursor + 6)?);
            let data = rom
                .get(source..source + count)
                .ok_or("ninjutsu DMA outside ROM")?;
            let mut address = destination;
            let mut remaining = data;
            while !remaining.is_empty() {
                let offset = address % 32;
                let count = remaining.len().min(32 - offset);
                let tile = self.tiles.entry(address / 32).or_insert(Tile([0; 32]));
                tile.0[offset..offset + count].copy_from_slice(&remaining[..count]);
                address += count;
                remaining = &remaining[count..];
            }
            let last = word(rom, cursor + 8)? != 0;
            cursor += 10;
            if last {
                return Ok(());
            }
        }
        Err("unterminated ninjutsu DMA list".into())
    }
    fn player_frame(&mut self, rom: &[u8], frame: usize) -> Result<()> {
        let offset = word(rom, FRAME_TABLE + frame * 4 + 2)?;
        self.dma(rom, FRAME_TABLE + usize::from(offset))
    }
    /// Mapping `id` at `origin`: `tile` is the record's `obj_tile` (index
    /// and palette added to every piece; bit 11 mirrors the whole sprite).
    /// False when the art has no such mapping.
    pub(super) fn draw(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        id: u16,
        tile: u16,
        origin: (i32, i32),
    ) -> bool {
        let Some(pieces) = self.mappings.get(&id) else {
            return false;
        };
        let mirror = tile & 0x800 != 0;
        draw_pieces(
            frame,
            pieces,
            palettes,
            tile & 0x67ff,
            origin,
            mirror,
            crate::render::SatWriter::Object,
            |index| self.tiles.get(&index),
        );
        true
    }
}

/// The Ikazuchi shield's mappings (`shield_mapping`) with the `$2454A` art,
/// decoded once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shield(Art);
impl Shield {
    pub fn load(rom: &[u8]) -> Result<Self> {
        let mut art = Art::default();
        art.dma(rom, SHIELD_DMA)?;
        for id in [0x62, 0x63, 0x86, 0x87] {
            art.mappings.insert(id, sprite_mapping(rom, id)?.1);
        }
        Ok(Self(art))
    }
    /// Draws shield mapping `id` at the player's origin (screen pixels);
    /// false for an id that is not a shield frame.
    pub fn draw(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        id: u16,
        origin: (i32, i32),
        facing_left: bool,
    ) -> bool {
        let tile = if facing_left { 0x800 } else { 0 };
        self.0.draw(frame, palettes, id, tile, origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dma_preserves_partial_tiles_and_applies_overlapping_transfers_in_order() {
        let mut rom = vec![0; 108];
        rom[..10].copy_from_slice(&[20, 0, 0x97, 0, 0, 32, 0, 30, 0, 0]);
        rom[10..20].copy_from_slice(&[2, 0, 0x97, 0, 0, 52, 0, 32, 0, 1]);
        rom[64..104].copy_from_slice(&std::array::from_fn::<_, 40, _>(|i| i as u8));
        rom[104..].copy_from_slice(&[200, 201, 202, 203]);
        let mut art = Art::default();
        art.tiles.insert(0, Tile([0xaa; 32]));

        art.dma(&rom, 0).unwrap();

        assert_eq!(art.tiles.len(), 3);
        assert_eq!(&art.tiles[&0].0[..30], &[0xaa; 30]);
        assert_eq!(&art.tiles[&0].0[30..], &[0, 1]);
        assert_eq!(&art.tiles[&1].0[..4], &[200, 201, 202, 203]);
        assert_eq!(&art.tiles[&1].0[4..], &rom[70..98]);
        assert_eq!(&art.tiles[&2].0[..6], &rom[98..104]);
        assert_eq!(&art.tiles[&2].0[6..], &[0; 26]);
    }
}
