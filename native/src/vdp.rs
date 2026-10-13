//! A small model of the Mega Drive VDP for the screens the cartridge draws
//! as whole plane maps and hardware sprites (#64): the title picture and the
//! options backdrop.
//!
//! Gameplay renders its layers natively (`stage`, `game`); these screens
//! instead are built by the cartridge as a series of VRAM, CRAM and sprite
//! table writes, so the port repeats those writes into a `Vdp` and renders
//! it. Only what the front end uses is modelled: planes A and B of 64 x 64
//! or 128 x 32 cells at fixed addresses, the display enable bit,
//! full-screen vertical scroll, per-line horizontal
//! scroll, the sprite table as the cartridge keeps it in RAM (`Sprite_Table`,
//! copied to VRAM every frame), explicit sprite masks
//! (`render::SpriteMask`) and shadow/highlight.
//! Layer merging follows Genesis Plus GX (`vdp_render.c`, `make_lut_bg`,
//! `make_lut_bgobj` and their `_ste` variants).

use crate::{
    HEIGHT, WIDTH,
    render::{Color, Fade, Frame, SpriteMask},
};

/// Bytes of VRAM.
pub const VRAM_SIZE: usize = 0x10000;
/// Plane size in cells (VDP register `$10` = `$11` on these screens).
pub const PLANE_CELLS: usize = 64;
/// One plane row of the map writers: the `$800000` they add to a VDP
/// command, whatever the plane size.
pub const PLANE_ROW_BYTES: usize = PLANE_CELLS * 2;
/// Entries of the H40 sprite table.
pub const SPRITES: usize = 80;

/// The VRAM address of a VDP write command long (`$4000_0003` = `$C000`):
/// address bits 0-13 in the high word, bits 14-15 in bits 0-1.
pub fn command_address(command: u32) -> usize {
    (((command >> 16) & 0x3fff) | ((command & 3) << 14)) as usize
}

/// A CRAM word with a palette fade applied as `$5EA` (in, from black) and
/// `$67A` (out) leave it after `step` updates.
pub fn faded(word: u16, fade: Fade) -> u16 {
    let mut out = 0;
    for shift in [0, 4, 8] {
        let nibble = (word >> shift) & 15;
        let value = match fade {
            Fade::In(step) => nibble.min(u16::from(step)),
            Fade::Out(step) => nibble.saturating_sub(u16::from(step)),
        };
        out |= value << shift;
    }
    out
}

/// One background layer pixel: CRAM index (palette * 16 + colour) and the
/// cell's priority bit.
#[derive(Clone, Copy, Default)]
struct Pixel {
    color: u8,
    priority: bool,
}
impl Pixel {
    fn opaque(self) -> bool {
        self.color & 15 != 0
    }
}

/// Displayed intensity under shadow/highlight.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Intensity {
    Shadow,
    Normal,
    Highlight,
}

#[derive(Clone)]
pub struct Vdp {
    pub vram: Vec<u8>,
    pub cram: [u16; 64],
    /// Register 7: the backdrop's CRAM index.
    pub backdrop: u8,
    /// Name table addresses (registers 2 and 4).
    pub plane_a: usize,
    pub plane_b: usize,
    /// Horizontal scroll per screen line, planes A and B.
    pub hscroll: Vec<[i16; 2]>,
    /// Full-screen vertical scroll (VSRAM words 0 and 1).
    pub vscroll: [i16; 2],
    /// `Sprite_Table` words (Y, size and link, tile, X).
    pub sprites: [[u16; 4]; SPRITES],
    /// Register `$0C` bit 3.
    pub shadow_highlight: bool,
    /// Register `$10`: plane width and height in cells (`$11` = 64 x 64,
    /// `$03` = 128 x 32).
    pub plane_size: (usize, usize),
    /// Register 1 bit 6. Off, every line shows the backdrop colour.
    pub display: bool,
}
impl Default for Vdp {
    fn default() -> Self {
        Self {
            vram: vec![0; VRAM_SIZE],
            cram: [0; 64],
            backdrop: 0,
            plane_a: 0xc000,
            plane_b: 0xe000,
            hscroll: vec![[0; 2]; HEIGHT],
            vscroll: [0; 2],
            sprites: [[0; 4]; SPRITES],
            shadow_highlight: false,
            plane_size: (PLANE_CELLS, PLANE_CELLS),
            display: true,
        }
    }
}
impl Vdp {
    /// Bytes at a VRAM address (wrapping at 64 KiB).
    pub fn write_bytes(&mut self, address: usize, bytes: &[u8]) {
        for (i, &byte) in bytes.iter().enumerate() {
            self.vram[(address + i) % VRAM_SIZE] = byte;
        }
    }
    /// Patterns from tile `first` (`Func_004612`: 32 bytes per tile).
    pub fn write_tiles(&mut self, first: usize, bytes: &[u8]) {
        self.write_bytes(first * 32, bytes);
    }
    pub fn write_word(&mut self, address: usize, word: u16) {
        self.write_bytes(address & !1, &word.to_be_bytes());
    }
    pub fn word(&self, address: usize) -> u16 {
        let at = address & (VRAM_SIZE - 2);
        u16::from_be_bytes([self.vram[at], self.vram[at + 1]])
    }
    /// `Plane_DrawWordMap` / `Plane_DrawByteMap`: `cells` row by row from
    /// `address`, `width` cells per row, each plus `base`.
    pub fn write_map(&mut self, address: usize, width: usize, base: u16, cells: &[u16]) {
        for (i, &cell) in cells.iter().enumerate() {
            let at = address + (i / width) * PLANE_ROW_BYTES + (i % width) * 2;
            self.write_word(at, cell.wrapping_add(base));
        }
    }
    /// Every line's horizontal scroll in 8-line strips (register `$0B` =
    /// `$02`): `strips[n]` covers lines `8n..8n+8`.
    pub fn set_strips(&mut self, strips: &[[i16; 2]]) {
        for (line, scroll) in self.hscroll.iter_mut().enumerate() {
            if let Some(&value) = strips.get(line / 8) {
                *scroll = value;
            }
        }
    }

    /// Register `$10`: `$03` gives 128 x 32 cells, `$11` 64 x 64.
    pub fn set_plane_size(&mut self, register: u8) {
        let cells = |bits: u8| match bits & 3 {
            0 => 32,
            1 => 64,
            _ => 128,
        };
        self.plane_size = (cells(register), cells(register >> 4));
    }

    /// One plane's pixels on screen line `y`, a cell at a time.
    fn plane_line(&self, base: usize, plane: usize, y: i32, line: &mut [Pixel; WIDTH]) {
        let (width, height) = self.plane_size;
        let py = (y + i32::from(self.vscroll[plane])).rem_euclid(height as i32 * 8) as usize;
        let row = base + (py / 8) * width * 2;
        let scroll = i32::from(self.hscroll[y as usize][plane]);
        let mut x = 0;
        while x < WIDTH {
            let px = (x as i32 - scroll).rem_euclid(width as i32 * 8) as usize;
            let cell = self.word(row + (px / 8) * 2);
            let ty = if cell & 0x1000 != 0 {
                7 - py % 8
            } else {
                py % 8
            };
            let palette = ((cell >> 9) & 0x30) as u8;
            let priority = cell & 0x8000 != 0;
            let run = (8 - px % 8).min(WIDTH - x);
            for (i, pixel) in line[x..x + run].iter_mut().enumerate() {
                let tx = px % 8 + i;
                let tx = if cell & 0x0800 != 0 { 7 - tx } else { tx };
                *pixel = Pixel {
                    color: palette | self.pattern(cell, tx, ty),
                    priority,
                };
            }
            x += run;
        }
    }
    /// Colour 0..15 of a pattern pixel.
    fn pattern(&self, cell: u16, x: usize, y: usize) -> u8 {
        let byte = self.vram[(usize::from(cell & 0x7ff) * 32 + y * 4 + x / 2) % VRAM_SIZE];
        if x.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 15
        }
    }

    /// The sprite layer of one line: the first opaque pixel in link order
    /// wins, with explicit X-zero masks and no scanline budgets.
    fn sprite_line(&self, y: i32, line: &mut [Option<Pixel>; WIDTH]) {
        line.fill(None);
        let mut mask = SpriteMask::default();
        let mut index = 0usize;
        for _ in 0..SPRITES {
            let [ypos, size, tile, xpos] = self.sprites[index];
            let top = i32::from(ypos & 0x1ff) - 128;
            let height = i32::from((size >> 8) & 3) + 1;
            let width = i32::from((size >> 10) & 3) + 1;
            if (top..top + height * 8).contains(&y) {
                if !mask.next(xpos & 0x1ff) {
                    break;
                }
                let left = i32::from(xpos & 0x1ff) - 128;
                let mut row = y - top;
                if tile & 0x1000 != 0 {
                    row = height * 8 - 1 - row;
                }
                for dx in 0..width * 8 {
                    let x = left + dx;
                    if !(0..WIDTH as i32).contains(&x) || line[x as usize].is_some() {
                        continue;
                    }
                    let column = if tile & 0x0800 != 0 {
                        width * 8 - 1 - dx
                    } else {
                        dx
                    };
                    let name = (tile & 0x7ff) + ((column / 8) * height + row / 8) as u16;
                    let color = self.pattern(name, (column % 8) as usize, (row % 8) as usize);
                    if color != 0 {
                        line[x as usize] = Some(Pixel {
                            color: ((tile >> 9) & 0x30) as u8 | color,
                            priority: tile & 0x8000 != 0,
                        });
                    }
                }
            }
            index = usize::from(size & 0x7f);
            if index == 0 || index >= SPRITES {
                break;
            }
        }
    }

    /// Render with a palette fade applied to CRAM first, as the cartridge
    /// fades CRAM itself.
    pub fn render(&self, frame: &mut Frame, fade: Fade) {
        let colors: [[Color; 3]; 64] = std::array::from_fn(|i| {
            let word = faded(self.cram[i], fade);
            let level = |shift: u16| u32::from((word >> shift) & 7);
            [Intensity::Shadow, Intensity::Normal, Intensity::Highlight].map(|intensity| {
                [1, 5, 9].map(|shift| {
                    let level = level(shift);
                    (match intensity {
                        Intensity::Shadow => level * 255 / 14,
                        Intensity::Normal => level * 255 / 7,
                        Intensity::Highlight => (level + 7) * 255 / 14,
                    }) as u8
                })
            })
        });
        // Front-end art retains its original centred canvas. Clear the
        // surrounding area too, so a mode change cannot leave stage pixels
        // behind in the wider frame's margins.
        frame.clear(colors[usize::from(self.backdrop & 63)][Intensity::Normal as usize]);
        if !self.display {
            return;
        }
        let mut sprites = [None; WIDTH];
        let (mut a, mut b) = ([Pixel::default(); WIDTH], [Pixel::default(); WIDTH]);
        for y in 0..HEIGHT as i32 {
            self.sprite_line(y, &mut sprites);
            self.plane_line(self.plane_b, 1, y, &mut b);
            self.plane_line(self.plane_a, 0, y, &mut a);
            for x in 0..WIDTH {
                let (color, intensity) = self.merge(a[x], b[x], sprites[x]);
                let color = if color & 15 == 0 {
                    self.backdrop
                } else {
                    color
                };
                frame.pixel(
                    x as i32,
                    y,
                    colors[usize::from(color & 63)][intensity as usize],
                );
            }
        }
    }

    /// `make_lut_bg` then `make_lut_bgobj` (or the shadow/highlight
    /// variants): the visible CRAM index and its intensity.
    fn merge(&self, a: Pixel, b: Pixel, sprite: Option<Pixel>) -> (u8, Intensity) {
        let mut bg = if a.priority {
            if a.opaque() { a } else { b }
        } else if b.priority {
            if b.opaque() { b } else { a }
        } else if a.opaque() {
            a
        } else {
            b
        };
        if !bg.opaque() {
            bg = Pixel::default();
        }
        if !self.shadow_highlight {
            return match sprite {
                Some(s) if s.priority || !(bg.priority && bg.opaque()) => {
                    (s.color, Intensity::Normal)
                }
                _ => (bg.color, Intensity::Normal),
            };
        }
        // Background at normal intensity when either plane cell has priority.
        let lit = if a.priority || b.priority {
            Intensity::Normal
        } else {
            Intensity::Shadow
        };
        let Some(s) = sprite else {
            return (bg.color, lit);
        };
        if !s.priority && bg.priority && bg.opaque() {
            return (bg.color, lit);
        }
        match s.color {
            // Palette 3 colour 15: shadow operator; colour 14: highlight.
            0x3f => (bg.color, Intensity::Shadow),
            0x3e => (
                bg.color,
                if lit == Intensity::Normal {
                    Intensity::Highlight
                } else {
                    Intensity::Normal
                },
            ),
            0x0e | 0x1e | 0x2e => (s.color, Intensity::Normal),
            color => (color, if s.priority { Intensity::Normal } else { lit }),
        }
    }
}
