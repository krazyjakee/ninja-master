//! The loaders and palette routines the front-end handlers share (#64, #65),
//! repeated as writes into a `crate::vdp::Vdp`, and the RAM shadows they
//! work on: the CRAM copy at `$FFF000`, the fade target `$FF16FA`, the fade
//! frame mask `$FF16E8` and `Frame_Counter`.
//!
//! The cartridge writes tiles and plane maps straight into VRAM, but keeps
//! the palette, the sprite table and the scroll values in RAM and copies
//! them in the vertical interrupt that starts the next frame: sprites and
//! scroll every frame, the palette only when `Sys_Flags` bit 1 asks for it.
//! `FrontVdp::vblank` repeats that copy, so a handler's VRAM writes show in
//! the frame it runs and its sprite, scroll and palette changes one frame
//! later, as in Genesis Plus GX. A handler call that runs through several
//! frames (a long load) delays the next call; `FrontVdp::lag` counts such a
//! frame without the copies.

use crate::{
    assets::{kosinski, musashi},
    render::fade_step,
    rom::{Result, byte, pointer, word, words},
    vdp::{SPRITES, Vdp, command_address},
};

/// `$80000`: the table of art tables.
pub const ART_TABLES: usize = 0x80000;
/// `Func_000900` / `Func_0008F0`: palette set -> four `Data_042062` lines.
const PALETTE_SETS: usize = 0x41f1a;
const PALETTES: usize = 0x42062;
/// `Data_03A000`: the 1bpp font (first tile 0, 58 tiles).
pub const FONT_ART: usize = 0x3a000;
/// `Data_0048E2`: font tile per ASCII byte.
pub const FONT_MAP: usize = 0x48e2;

/// Entry `entry` (a byte offset) of the art table at `$80000` + `table`.
pub fn art(rom: &[u8], table: usize, entry: usize) -> Result<usize> {
    pointer(rom, pointer(rom, ART_TABLES + table)? + entry)
}

/// The four CRAM lines of a `Func_000900` palette set.
pub fn palette_set(rom: &[u8], set: usize) -> Result<[u16; 64]> {
    let mut out = [0; 64];
    for line in 0..4 {
        let id = usize::from(byte(rom, PALETTE_SETS + set * 4 + line)?);
        let colors: [u16; 16] = words(rom, PALETTES + id * 32)?;
        out[line * 16..line * 16 + 16].copy_from_slice(&colors);
    }
    Ok(out)
}

/// `Func_0046A8`/`Func_0046B4`: 1bpp tiles (first tile, count - 1, eight
/// bytes per tile) expanded to `zero` behind `one`, leftmost pixel in bit 7.
pub fn one_bpp(rom: &[u8], at: usize, zero: u8, one: u8) -> Result<(usize, Vec<u8>)> {
    let first = usize::from(word(rom, at)?);
    let count = usize::from(word(rom, at + 2)?) + 1;
    let bits = rom
        .get(at + 4..at + 4 + count * 8)
        .ok_or_else(|| format!("1bpp art ${at:06x} truncated"))?;
    let mut out = Vec::with_capacity(count * 32);
    for &row in bits {
        for pair in 0..4 {
            let pixel = |x: u32| if row & (0x80 >> x) != 0 { one } else { zero };
            out.push(pixel(pair * 2) << 4 | pixel(pair * 2 + 1));
        }
    }
    Ok((first, out))
}

/// Decompressed tiles cut (or padded) to the `count - 1` the loader sends.
pub fn tiles(mut data: Vec<u8>, count_minus_one: u16) -> Vec<u8> {
    data.resize((usize::from(count_minus_one) + 1) * 32, 0);
    data
}

/// A `Plane_ReadMapHeader` header: VRAM address, base, width, height.
pub fn map_header(rom: &[u8], at: usize) -> Result<(usize, u16, usize, usize)> {
    Ok((
        command_address(pointer(rom, at)? as u32),
        word(rom, at + 4)?,
        usize::from(word(rom, at + 6)?) + 1,
        usize::from(word(rom, at + 8)?) + 1,
    ))
}

/// A VDP state and the RAM shadows of the front-end handlers.
#[derive(Clone)]
pub struct FrontVdp {
    pub vdp: Vdp,
    /// `$FFF000`: the CRAM copy the handlers change.
    pub palette: [u16; 64],
    /// `Sys_Flags` bit 1: copy `palette` into CRAM at the next interrupt.
    pub palette_dirty: bool,
    /// `$FF16FA`: the palette `Func_0005EA` fades toward.
    pub target: [u16; 64],
    /// `$FF16E8`: fades update only on frames where
    /// `Frame_Counter & fade_mask` is zero.
    pub fade_mask: u16,
    /// `Frame_Counter` (`$FF0022`), counted by the vertical interrupt.
    pub frame_counter: u16,
    /// `Sprite_Table` (`$FFF500`), copied to the VDP every frame.
    pub sprites: [[u16; 4]; SPRITES],
    /// `HScroll_Buffer` (`$FFF080`): the full-screen horizontal scroll of
    /// planes A and B (register `$0B` = `$00`).
    pub hscroll: [i16; 2],
    /// `VScroll_PlaneA` / `VScroll_PlaneB` (`$FFF480`).
    pub vscroll: [i16; 2],
}
impl FrontVdp {
    pub fn new(frame_counter: u16) -> Self {
        Self {
            vdp: Vdp::default(),
            palette: [0; 64],
            palette_dirty: false,
            target: [0; 64],
            fade_mask: 0,
            frame_counter,
            sprites: [[0; 4]; SPRITES],
            hscroll: [0; 2],
            vscroll: [0; 2],
        }
    }

    /// The vertical interrupt ahead of a main-loop frame: the sprite table
    /// and scroll copies, the palette copy when it was asked for, and the
    /// frame counter.
    pub fn vblank(&mut self) {
        self.vdp.sprites = self.sprites;
        self.vdp.hscroll.fill(self.hscroll);
        self.vdp.vscroll = self.vscroll;
        if self.palette_dirty {
            self.vdp.cram = self.palette;
            self.palette_dirty = false;
        }
        self.frame_counter = self.frame_counter.wrapping_add(1);
    }
    /// A frame inside a handler call that runs through several: the
    /// interrupt counts it (unless `counted` is false: a few long loads
    /// run through an interrupt with interrupts masked, and the cartridge
    /// loses that count), but what the call has written so far is copied
    /// only after it returns (measured for the loads of the front end).
    pub fn lag(&mut self, counted: bool) {
        if counted {
            self.frame_counter = self.frame_counter.wrapping_add(1);
        }
    }

    /// `Func_000900`: a palette set as the fade target.
    pub fn target_set(&mut self, rom: &[u8], set: usize) -> Result<()> {
        self.target = palette_set(rom, set)?;
        Ok(())
    }
    /// `Func_0008F0`: a palette set straight into the CRAM copy.
    pub fn palette_set(&mut self, rom: &[u8], set: usize) -> Result<()> {
        self.palette = palette_set(rom, set)?;
        self.palette_dirty = true;
        Ok(())
    }

    /// `Func_0005EA`: every CRAM copy nibble one step up toward the target,
    /// on the frames `$FF16E8` allows; true while the fade still runs.
    pub fn fade_in(&mut self) -> bool {
        if self.frame_counter & self.fade_mask != 0 {
            return true;
        }
        self.palette_dirty = true;
        let mut changed = false;
        for (color, &target) in self.palette.iter_mut().zip(&self.target) {
            let mut out = 0;
            for mask in [0xf00u16, 0xf0, 0xf] {
                let unit = mask.isolate_lowest_one();
                let (c, t) = (*color & mask, target & mask);
                out |= if c == t {
                    c
                } else {
                    changed = true;
                    c.wrapping_add(unit) & mask
                };
            }
            *color = out;
        }
        changed
    }
    /// `Func_00067A`: every nibble one step down toward black, on the
    /// frames `$FF16E8` allows; true while the fade still runs.
    pub fn fade_out(&mut self) -> bool {
        if self.frame_counter & self.fade_mask != 0 {
            return true;
        }
        self.palette_dirty = true;
        let mut changed = false;
        for color in &mut self.palette {
            changed |= *color & 0x0fff != 0;
            *color = fade_step(*color, 0);
        }
        changed
    }
    /// `Func_0006E8`: CRAM copy entries from `first` one step toward
    /// `target`, whatever the frame.
    pub fn fade_toward(&mut self, first: usize, target: &[u16]) {
        self.palette_dirty = true;
        for (color, &target) in self.palette[first..].iter_mut().zip(target) {
            *color = fade_step(*color, target);
        }
    }
    /// `Func_00075C`: sixteen CRAM copy entries from `first` one step
    /// toward `$FFF` (white); true while any changed.
    pub fn brighten(&mut self, first: usize) -> bool {
        self.palette_dirty = true;
        let mut changed = false;
        for color in &mut self.palette[first..first + 16] {
            let before = *color;
            *color = fade_step(*color, 0x0fff);
            changed |= *color != before;
        }
        changed
    }

    /// `Func_00459C` / `Func_0045AA`: clear plane A (`$C000`) or B
    /// (`$E000`), `$2000` bytes each.
    pub fn clear_plane_a(&mut self) {
        self.vdp.write_bytes(0xc000, &[0; 0x2000]);
    }
    pub fn clear_plane_b(&mut self) {
        self.vdp.write_bytes(0xe000, &[0; 0x2000]);
    }

    /// `Func_0046FA`: Kosinski tiles after their tile and count words.
    pub fn kosinski_tiles(&mut self, rom: &[u8], at: usize) -> Result<()> {
        self.kosinski_tiles_at(rom, at, usize::from(word(rom, at)?))
    }
    /// `Func_0046FE`: the same art loaded at another tile.
    pub fn kosinski_tiles_at(&mut self, rom: &[u8], at: usize, tile: usize) -> Result<()> {
        let (data, _) = kosinski(rom, at + 4, 0x10000)?;
        self.vdp.write_tiles(tile, &tiles(data, word(rom, at + 2)?));
        Ok(())
    }
    /// `Func_0046DA`: LZ tiles after their tile and count words.
    pub fn lz_tiles(&mut self, rom: &[u8], at: usize) -> Result<()> {
        self.lz_tiles_at(rom, at, usize::from(word(rom, at)?))
    }
    /// `Func_0046DE`: the same art loaded at another tile.
    pub fn lz_tiles_at(&mut self, rom: &[u8], at: usize, tile: usize) -> Result<()> {
        let (data, _) = musashi(rom, at + 4, 0x10000)?;
        self.vdp.write_tiles(tile, &tiles(data, word(rom, at + 2)?));
        Ok(())
    }
    /// `Func_00465E`: 1bpp art expanded with `zero` behind `one`; with a
    /// nonzero `zero` the first tile is cleared.
    pub fn one_bpp_tiles(&mut self, rom: &[u8], at: usize, zero: u8, one: u8) -> Result<()> {
        let (first, mut data) = one_bpp(rom, at, zero, one)?;
        if zero != 0 {
            data[..32].fill(0);
        }
        self.vdp.write_tiles(first, &data);
        Ok(())
    }

    /// `Plane_DrawByteMapHeader`: a header and one byte per cell.
    pub fn byte_map(&mut self, rom: &[u8], at: usize) -> Result<()> {
        let (address, base, width, height) = map_header(rom, at)?;
        let cells: Vec<u16> = rom
            .get(at + 10..at + 10 + width * height)
            .ok_or_else(|| format!("byte map ${at:06x} truncated"))?
            .iter()
            .map(|&b| u16::from(b))
            .collect();
        self.vdp.write_map(address, width, base, &cells);
        Ok(())
    }
    /// `Plane_DrawLZByteMap`: a header, then LZ bytes.
    pub fn lz_byte_map(&mut self, rom: &[u8], at: usize) -> Result<()> {
        let (address, base, width, height) = map_header(rom, at)?;
        let (data, _) = musashi(rom, at + 10, 0x10000)?;
        let cells: Vec<u16> = (0..width * height)
            .map(|i| u16::from(data.get(i).copied().unwrap_or(0)))
            .collect();
        self.vdp.write_map(address, width, base, &cells);
        Ok(())
    }
    /// `Plane_DrawLZWordMap`: a header, then LZ words.
    pub fn lz_word_map(&mut self, rom: &[u8], at: usize) -> Result<()> {
        let (address, base, width, height) = map_header(rom, at)?;
        self.lz_words_to(rom, at + 10, address, base, width, height)
    }
    /// LZ words drawn at `address` with `base` added, `width` cells a row.
    pub fn lz_words_to(
        &mut self,
        rom: &[u8],
        at: usize,
        address: usize,
        base: u16,
        width: usize,
        height: usize,
    ) -> Result<()> {
        let (data, _) = musashi(rom, at, 0x10000)?;
        let cells: Vec<u16> = (0..width * height)
            .map(|i| match data.get(i * 2..i * 2 + 2) {
                Some(b) => u16::from_be_bytes([b[0], b[1]]),
                None => 0,
            })
            .collect();
        self.vdp.write_map(address, width, base, &cells);
        Ok(())
    }
}

/// `Func_0040FA` / `Func_004100`: text bytes through the font map, plus
/// `attribute`, row by row from `address`.
pub fn text_cells(rom: &[u8], text: &[u8]) -> Result<Vec<u16>> {
    let map = rom
        .get(FONT_MAP..FONT_MAP + 256)
        .ok_or("font map truncated")?;
    Ok(text
        .iter()
        .map(|&b| u16::from(map[usize::from(b)]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fades_follow_the_cartridge() {
        let mut v = FrontVdp::new(0);
        v.target[0] = 0x0e42;
        let mut steps = 0;
        while v.fade_in() {
            steps += 1;
        }
        assert_eq!((steps, v.palette[0]), (14, 0x0e42));
        assert!(v.palette_dirty);
        v.vblank();
        assert_eq!(v.vdp.cram[0], 0x0e42);
        v.fade_mask = 3;
        assert!(v.fade_out()); // frame 1: skipped
        assert_eq!(v.palette[0], 0x0e42);
        v.frame_counter = 4;
        assert!(v.fade_out());
        assert_eq!(v.palette[0], 0x0d31);
        v.palette.fill(0);
        v.palette[0] = 0xf000; // Bits outside CRAM do not prolong a fade.
        assert!(!v.fade_out());
        assert_eq!(v.palette[0], 0);
        v.palette[0] = 0x0eec;
        assert!(v.brighten(0));
        assert_eq!(v.palette[0], 0x0ffd);
    }
}
