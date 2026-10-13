//! Plane-map text shared by every front-end screen (#48).
//!
//! `Func_0040FA` reads a 10-byte header with `Plane_ReadMapHeader` (`$40DE`:
//! VDP write command, attribute word, width - 1, height - 1) and writes one
//! cell per following byte, translated to a tile through the font map
//! `Data_0048E2`. The font itself is the 1bpp `Data_03A000`, expanded to two
//! colours by `Func_0046A8`. The pause window (`Func_00B9F2`) uses the same
//! map, except that bytes from 'a' select frame tiles through `Data_00A498`.
//! Screens are data (ROM text blocks) drawn through `Font`; the native port
//! draws cells straight into the frame instead of building plane maps.

use crate::{
    assets::musashi,
    front_vdp::{FONT_ART, FONT_MAP},
    render::{Color, Frame, md_color},
    rom::{pointer, word},
};

/// Resident HUD art (`$3BFFA`, tiles `$40..$71`) holding the frame tiles.
const HUD_ART: usize = 0x3bffa;
/// `Data_00A498`: tile words for the window bytes 'a'..'h' (index = byte & 15).
const FRAME_TABLE: usize = 0xa498;
/// HUD palette line 0 (`$42122`). `Func_00A254` expands the gameplay font
/// with colour 9 behind colour 11.
const HUD_PALETTE: usize = 0x42122;
/// Tiles a window or font map can address.
const TILES: usize = 0x80;
/// Tile word flip bits.
const FLIP_X: u16 = 0x0800;
const FLIP_Y: u16 = 0x1000;

/// Plane A (VDP register 2 = `$30`) and the window plane (register 3 =
/// `$2C`) in the gameplay and front-end captures. Both are 64 cells wide.
pub const PLANE_A: u16 = 0xc000;
pub const WINDOW: u16 = 0xb000;
const PLANE_ROW_BYTES: u16 = 128;

/// Colours and size of drawn cells. `background: None` leaves the glyph's
/// clear pixels transparent, as the front-end screens expand the font with
/// colour 0 (`Func_00A258` with d0 = 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub foreground: Color,
    pub background: Option<Color>,
    /// Pixel multiple; 2 stands in for the 2x2-cell title font (`Func_0041AC`).
    pub scale: i32,
}
impl Style {
    /// `foreground` over transparent cells.
    pub fn plain(foreground: Color, scale: i32) -> Self {
        Self {
            foreground,
            background: None,
            scale,
        }
    }
}

/// A `Func_0040FA` text block: header and cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextBlock {
    /// VRAM address of the header's VDP write command.
    pub vram: u16,
    pub width: usize,
    pub height: usize,
    pub text: Vec<u8>,
}
impl TextBlock {
    /// `Plane_ReadMapHeader` (`$40DE`) and the `width * height` bytes after it.
    pub fn read(rom: &[u8], address: usize) -> Result<Self, String> {
        let command = pointer(rom, address)?;
        let width = usize::from(word(rom, address + 6)?) + 1;
        let height = usize::from(word(rom, address + 8)?) + 1;
        let text = rom
            .get(address + 10..address + 10 + width * height)
            .ok_or_else(|| format!("text block ${address:05X} truncated"))?
            .to_vec();
        Ok(Self {
            vram: crate::vdp::command_address(command as u32) as u16,
            width,
            height,
            text,
        })
    }
    /// Top-left cell (column, row) in a 64-cell plane starting at `plane`.
    pub fn cell(&self, plane: u16) -> (i32, i32) {
        let offset = self.vram.wrapping_sub(plane);
        (
            i32::from(offset % PLANE_ROW_BYTES / 2),
            i32::from(offset / PLANE_ROW_BYTES),
        )
    }
}

/// `value` in `digits` decimal digits with leading zeros blanked, as the
/// score writers `Func_00B87C`/`$B8BA` do (zero is all blanks).
pub fn blank_zeros(value: u32, digits: usize) -> String {
    let full = format!("{value:0digits$}");
    let text = &full[full.len() - digits..];
    let first = text.find(|c| c != '0').unwrap_or(digits);
    " ".repeat(first) + &text[first..]
}

/// The cartridge font and window frame as 1bpp cells.
#[derive(Clone, Debug)]
pub struct Font {
    map: [u8; 128],
    /// Rows of each addressable tile, most significant bit on the left.
    glyphs: Vec<Option<[u8; 8]>>,
    /// `Data_00A498` words 0..=8.
    frame: [u16; 9],
    /// HUD palette colours 9 and 11: the gameplay window's colours.
    pub background: Color,
    pub foreground: Color,
}
impl Font {
    pub fn load(rom: &[u8]) -> Result<Self, String> {
        let mut map = [0; 128];
        map.copy_from_slice(
            rom.get(FONT_MAP..FONT_MAP + 128)
                .ok_or("font map outside ROM")?,
        );
        let mut glyphs = vec![None; TILES];
        let first = usize::from(word(rom, FONT_ART)?);
        let count = usize::from(word(rom, FONT_ART + 2)?) + 1;
        let art = rom
            .get(FONT_ART + 4..FONT_ART + 4 + count * 8)
            .ok_or("font art truncated")?;
        for (i, rows) in art.as_chunks::<8>().0.iter().enumerate() {
            if let Some(slot) = glyphs.get_mut(first + i) {
                *slot = Some(*rows);
            }
        }
        // $465E clears tile 0 whenever the background colour is not 0: the
        // cell unmapped bytes select stays transparent.
        glyphs[0] = None;
        // The frame tiles $57..$59 are 4bpp HUD art in colours 9 and 11.
        let hud_first = usize::from(word(rom, HUD_ART)?);
        let hud_count = usize::from(word(rom, HUD_ART + 2)?) + 1;
        let (hud, _) = musashi(rom, HUD_ART + 4, 65536)?;
        let hud = hud.get(..hud_count * 32).ok_or("HUD art truncated")?;
        for (glyph, tile) in glyphs[0x57..=0x59].iter_mut().zip(0x57..) {
            let data = hud
                .as_chunks::<32>()
                .0
                .get(tile - hud_first)
                .ok_or("frame tile outside HUD art")?;
            let mut rows = [0u8; 8];
            for (y, row) in rows.iter_mut().enumerate() {
                for x in 0..8 {
                    let packed = data[y * 4 + x / 2];
                    let index = if x % 2 == 0 { packed >> 4 } else { packed & 15 };
                    if index == 11 {
                        *row |= 0x80 >> x;
                    }
                }
            }
            *glyph = Some(rows);
        }
        let mut frame = [0; 9];
        for (i, entry) in frame.iter_mut().enumerate() {
            *entry = word(rom, FRAME_TABLE + i * 2)?;
        }
        Ok(Self {
            map,
            glyphs,
            frame,
            background: md_color(word(rom, HUD_PALETTE + 9 * 2)?),
            foreground: md_color(word(rom, HUD_PALETTE + 11 * 2)?),
        })
    }
    /// The gameplay window style: white on the dark window colour.
    pub fn window_style(&self) -> Style {
        Style {
            foreground: self.foreground,
            background: Some(self.background),
            scale: 1,
        }
    }
    /// `Data_0048E2`: the font tile for `byte`.
    pub fn tile(&self, byte: u8) -> u16 {
        u16::from(self.map.get(usize::from(byte)).copied().unwrap_or(0))
    }
    /// `Func_00B9F2`: window bytes from 'a' (`$61`) are frame tiles.
    pub fn window_tile(&self, byte: u8) -> u16 {
        if byte >= 0x61 {
            self.frame[usize::from(byte & 15).min(8)]
        } else {
            self.tile(byte)
        }
    }
    /// One cell. Tile 0 and tiles outside the font draw nothing; the flip bits
    /// of a frame-table word are honoured.
    pub fn draw_tile(&self, frame: &mut Frame, word: u16, x: i32, y: i32, style: Style) {
        let index = usize::from(word & 0x7ff);
        let Some(Some(rows)) = self.glyphs.get(index) else {
            return;
        };
        let scale = style.scale.max(1);
        for (dy, row) in rows.iter().enumerate() {
            let sy = (if word & FLIP_Y != 0 { 7 - dy } else { dy }) as i32;
            for dx in 0..8 {
                let sx = if word & FLIP_X != 0 { 7 - dx } else { dx };
                let color = if row & (0x80 >> dx) != 0 {
                    style.foreground
                } else if let Some(background) = style.background {
                    background
                } else {
                    continue;
                };
                frame.rect(x + sx * scale, y + sy * scale, scale, scale, color);
            }
        }
    }
    /// `Func_004100`: `cells` as rows of `width`, from pixel (x, y).
    pub fn draw_cells(
        &self,
        frame: &mut Frame,
        x: i32,
        y: i32,
        width: usize,
        cells: &[u8],
        style: Style,
    ) {
        self.draw_mapped(frame, x, y, width, cells, style, |byte| self.tile(byte));
    }
    /// `Func_00B9F2`: window rows, where 'a'..'h' draw the frame.
    pub fn draw_window(
        &self,
        frame: &mut Frame,
        x: i32,
        y: i32,
        width: usize,
        cells: &[u8],
        style: Style,
    ) {
        self.draw_mapped(frame, x, y, width, cells, style, |byte| {
            self.window_tile(byte)
        });
    }
    /// One line of text.
    pub fn draw_text(&self, frame: &mut Frame, x: i32, y: i32, text: &str, style: Style) {
        self.draw_cells(frame, x, y, text.len().max(1), text.as_bytes(), style);
    }
    /// A ROM text block at its header's position in a plane at scroll 0.
    pub fn draw_block(&self, frame: &mut Frame, block: &TextBlock, plane: u16, style: Style) {
        let (col, row) = block.cell(plane);
        self.draw_cells(frame, col * 8, row * 8, block.width, &block.text, style);
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_mapped(
        &self,
        frame: &mut Frame,
        x: i32,
        y: i32,
        width: usize,
        cells: &[u8],
        style: Style,
        tile: impl Fn(u8) -> u16,
    ) {
        let step = 8 * style.scale.max(1);
        for (i, &byte) in cells.iter().enumerate() {
            let (col, row) = ((i % width.max(1)) as i32, (i / width.max(1)) as i32);
            self.draw_tile(frame, tile(byte), x + col * step, y + row * step, style);
        }
    }
}
