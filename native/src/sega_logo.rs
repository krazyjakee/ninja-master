//! The SEGA logo (#65): `Mode_SegaLogo` (`$69A8`) repeated as VRAM, CRAM,
//! sprite table and scroll writes into a `FrontVdp`, frame for frame
//!
//! `SegaLogo_S00` builds the art (`Func_006DAA`): sixteen solid tiles, 256
//! two-colour half tiles and the four letters S, E, G, A (32 x 32 pixels
//! each, Kosinski `Data_007452`) as 1x tiles and 2x-scaled tiles. Then one
//! of two animations runs, chosen by bit 2 of the random word `$FF001C`:
//!
//! - `Variant::Zoom` (`loc_006EE4`): each letter shrinks in five steps, as
//!   16x, 8x and 4x pictures drawn cell by cell on the planes, then 2x and
//!   1x sprites, until the four letters stand in a row;
//! - `Variant::Slide` (`loc_0071A0`): with 128 x 32 cell planes, the
//!   letters drawn one cell per pixel scroll in behind a black bar, then
//!   the 1x sprites slide in from the left and wobble into place.
//!
//! Both end the same way: 16 frames, a shine that whitens CRAM entries 3 to
//! 14 one per frame (`SegaLogo_S0C`; entry 14 stays white), a 64-frame
//! hold, a fade out and `Mode_Intro`; Start from the hold on fades out to
//! `Mode_Title`.
//!
//! Handler calls that run through several frames (the load, the plane
//! clears, the slide's first drawing) are modelled as holds with the
//! lengths measured in Genesis Plus GX. The interrupt still runs in those
//! frames and copies the sprite, scroll and palette shadows (the slide's
//! scroll, set early in its long call, is on screen in them), except for
//! the one the load loses.

use crate::{
    assets::kosinski,
    front_vdp::{FONT_ART, FrontVdp, one_bpp, palette_set},
    rom::{Result, words},
};

/// `Data_007452`: the letters, Kosinski, 4 x 32 rows of 32 pixels.
const LETTERS: usize = 0x7452;
const LETTER_BYTES: usize = 0x200;
const ROW_BYTES: usize = 16;
/// `Data_00734A`: the slide's four sprites; `Data_0073E0`: its wobble.
const SLIDE_SPRITES: usize = 0x734a;
const WOBBLE: usize = 0x73e0;
/// `Func_0008F0` palette set 1.
const PALETTE_SET: usize = 1;
/// Solid tiles `$F0..$FF` (VRAM `$1E00`) and half tiles `$100..$1FF`.
const SOLID_TILE: usize = 0xf0;
const HALF_TILE: usize = 0x100;
/// 1x letters from tile `$40` (16 tiles each), 2x from tile `$200` (64
/// each), both in sprite (column) order.
const SMALL_TILE: usize = 0x40;
const LARGE_TILE: usize = 0x200;
/// `Data_006B90[Sys_Flags2 & 2]`: CRAM line 0 colour 15, white on an
/// overseas console (bit 1 of the version port), black on a Japanese one.
/// The port is the overseas version.
const COLOR_15: u16 = 0x0eee;
/// `SegaLogo_S08` / `SegaLogo_S0C`: the shine colour.
const SHINE: u16 = 0x0eee;
/// `SegaLogo_S10`: frames held before the fade out.
const HOLD: u16 = 0x40;

/// Handler calls that run through several frames, measured in Genesis Plus
/// GX (`$50A` after the call): the frames after the call starts.
/// `SegaLogo_S00` runs frames 18..28, and the interrupt of frame 27 is
/// lost (`Frame_Counter` stays `$0009`).
const LOAD_HOLD: u32 = 10;
const LOAD_LOST: u32 = 2;
/// `SegaLogo_S08`'s plane clears: frames 141..143 (Slide: 179..181).
const CLEAR_HOLD: u32 = 2;
/// The Slide's first call (planes, bar and letters): frames 29..31.
const SLIDE_DRAW_HOLD: u32 = 2;

/// Native ticks of the Zoom logo without input: emulator frames 18..233,
/// the last call installing `Mode_Intro` (tick 216).
pub const ZOOM_TICKS: u32 = 216;
/// The Slide logo: frames 18..271.
pub const SLIDE_TICKS: u32 = 254;
/// The first tick whose Start is honoured (`Mode_Routine` = `$10`,
/// `Func_006B6C`): frame 156 (Zoom) or 194 (Slide).
pub const ZOOM_START_TICK: u32 = 139;
pub const SLIDE_START_TICK: u32 = 177;

/// `$FF00FA`: the animation `SegaLogo_S04` runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// 0: `loc_006EE4`.
    Zoom,
    /// 4: `loc_0071A0`.
    Slide,
}
impl Variant {
    /// `SegaLogo_S00`: `$FF001C & 4`.
    pub fn from_random(random: u16) -> Self {
        if random & 4 == 0 {
            Self::Zoom
        } else {
            Self::Slide
        }
    }
}

/// The handler the logo installs when it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// `SegaLogo_S10`: `Mode_Intro`.
    Intro,
    /// `SegaLogo_S14` (Start): `Mode_Title`.
    Title,
}

/// `Mode_SegaLogo`'s state: the VDP and the RAM it works on.
#[derive(Clone)]
pub struct Logo {
    pub v: FrontVdp,
    pub variant: Variant,
    /// `Mode_Routine`, `$FF00F6`, `$FF00F8`.
    pub routine: u16,
    pub f6: u16,
    pub f8: u16,
    /// `$FF9C10`: the letters as decompressed, 16 bytes a row.
    letters: Vec<u8>,
    /// The font's first tile and patterns, palette set 1.
    font: (usize, Vec<u8>),
    palette: [u16; 64],
    slide_sprites: [[u16; 4]; 4],
    wobble: [u8; 32],
    /// Frames left in the current multi-frame call.
    hold: u32,
    /// The `hold` value whose interrupt is lost.
    lost: Option<u32>,
}

impl Logo {
    /// The state `Mode_SegaLogo` starts from, with what `SegaLogo_S00`
    /// reads from the ROM; it runs on the first tick.
    pub fn new(rom: &[u8], variant: Variant, frame_counter: u16) -> Result<Self> {
        let (letters, _) = kosinski(rom, LETTERS, 0x10000)?;
        if letters.len() < 4 * LETTER_BYTES {
            return Err("SEGA logo letters truncated".into());
        }
        let mut slide_sprites = [[0; 4]; 4];
        for (i, sprite) in slide_sprites.iter_mut().enumerate() {
            *sprite = words(rom, SLIDE_SPRITES + i * 8)?;
        }
        let wobble = rom
            .get(WOBBLE..WOBBLE + 32)
            .and_then(|b| b.try_into().ok())
            .ok_or("wobble table truncated")?;
        Ok(Self {
            v: FrontVdp::new(frame_counter),
            variant,
            routine: 0,
            f6: 0,
            f8: 0,
            letters,
            font: one_bpp(rom, FONT_ART, 0, 1)?,
            palette: palette_set(rom, PALETTE_SET)?,
            slide_sprites,
            wobble,
            hold: 0,
            lost: None,
        })
    }

    /// `SegaLogo_S00`: planes cleared, `VDP_InitRegisters` (display off
    /// until the next interrupt), `$FF16E8` = 0, the font with colour 1 on
    /// 0 (`Func_00A24C`), palette set 1, the art and the placeholder
    /// sprites. The call runs through frame 28.
    fn load(&mut self) {
        let v = &mut self.v;
        v.clear_plane_a();
        v.clear_plane_b();
        v.vdp.display = false;
        v.fade_mask = 0;
        v.vdp.write_tiles(self.font.0, &self.font.1);
        v.palette = self.palette;
        v.palette_dirty = true;
        self.build_art();
        // Seven 4 x 4 placeholders linked 1..7, the eighth ends the list.
        for (i, sprite) in self.v.sprites[..8].iter_mut().enumerate() {
            let link = if i == 7 { 0 } else { i as u16 + 1 };
            *sprite = [0, 0x0f00 | link, 0, 1];
        }
        self.f6 = 0;
        self.f8 = 1;
        self.routine = 4;
        self.hold = LOAD_HOLD;
        self.lost = Some(LOAD_LOST);
    }

    /// `Func_006DAA`.
    fn build_art(&mut self) {
        let vdp = &mut self.v.vdp;
        for color in 0..16u8 {
            vdp.write_tiles(SOLID_TILE + usize::from(color), &[color * 0x11; 32]);
        }
        // Rows 0-3: colour `left` in pixels 0-3, `right` in 4-7.
        for left in 0..16u8 {
            for right in 0..16u8 {
                let mut tile = [0; 32];
                for row in tile[..16].chunks_mut(4) {
                    row.copy_from_slice(&[left * 0x11, left * 0x11, right * 0x11, right * 0x11]);
                }
                vdp.write_tiles(
                    HALF_TILE + usize::from(left) * 16 + usize::from(right),
                    &tile,
                );
            }
        }
        // 1x: each 32-row band of 32 pixels as four columns of four tiles.
        // The loader sends $40 + 1 tiles; the last is RAM past the buffer
        // (zero here, and nothing shows it).
        let small = columns(&self.letters[..4 * LETTER_BYTES], 4);
        vdp.write_tiles(SMALL_TILE, &small);
        // 2x: every pixel doubled across and down (64 x 256), then eight
        // columns of four tiles per 32-row band.
        let mut large = Vec::with_capacity(0x2000);
        for row in self.letters[..4 * LETTER_BYTES].chunks(ROW_BYTES) {
            let wide: Vec<u8> = row
                .iter()
                .flat_map(|&b| [(b >> 4) * 0x11, (b & 15) * 0x11])
                .collect();
            large.extend_from_slice(&wide);
            large.extend_from_slice(&wide);
        }
        vdp.write_tiles(LARGE_TILE, &columns(&large, 8));
    }

    /// One frame: the interrupt, then the handler call unless a long call
    /// is still running; the handler the logo leaves to when it ends.
    pub fn tick(&mut self, start: bool) -> Option<Exit> {
        if self.hold > 0 {
            if self.lost == Some(self.hold) {
                self.lost = None;
            } else {
                self.vblank();
            }
            self.hold -= 1;
            return None;
        }
        self.vblank();
        // Func_006B6C.
        if start && self.routine > 0x0c {
            self.routine = 0x14;
        }
        match self.routine {
            0 => {
                self.load();
                None
            }
            4 => {
                match self.variant {
                    Variant::Zoom => self.zoom(),
                    Variant::Slide => self.slide(),
                }
                None
            }
            8 => {
                self.f6 = self.f6.wrapping_sub(1);
                if self.f6 == 0 {
                    self.v.clear_plane_a();
                    self.v.clear_plane_b();
                    self.f8 = self.v.palette[2];
                    self.v.palette[2] = SHINE;
                    self.routine = 0x0c;
                    self.hold = CLEAR_HOLD;
                }
                None
            }
            0x0c => {
                let v = &mut self.v;
                v.palette_dirty = true;
                let at = 2 + usize::from(self.f6);
                v.palette[at] = self.f8;
                self.f8 = v.palette[at + 1];
                v.palette[at + 1] = SHINE;
                self.f6 += 1;
                if self.f6 == 12 {
                    self.f6 = HOLD;
                    self.routine = 0x10;
                    return self.hold_then_fade();
                }
                None
            }
            0x10 => self.hold_then_fade(),
            _ => (!self.v.fade_out()).then_some(Exit::Title),
        }
    }

    /// The interrupt: shadows copied, display back on (`$8164`), frame
    /// counted.
    fn vblank(&mut self) {
        self.v.vblank();
        self.v.vdp.display = true;
    }

    /// `SegaLogo_S10`.
    fn hold_then_fade(&mut self) -> Option<Exit> {
        self.f6 = self.f6.wrapping_sub(1);
        if (self.f6 as i16) >= 0 || self.v.fade_out() {
            return None;
        }
        Some(Exit::Intro)
    }

    /// `loc_006EE4`: a step every `$FF00F8` frames, through the 21 routines
    /// `Func_006EFE..Func_006F4E`.
    fn zoom(&mut self) {
        self.f8 = self.f8.wrapping_sub(1);
        if self.f8 != 0 {
            return;
        }
        self.f6 += 4;
        let step = usize::from(self.f6 / 4) - 1;
        if step == 20 {
            self.zoom_done();
            return;
        }
        let (letter, size) = (step / 5, step % 5);
        let at = letter * LETTER_BYTES;
        self.f8 = 4;
        match size {
            0..=2 => {
                // The planes; no sprites: sprite 0 ends the list.
                self.v.sprites[0][..2].fill(0);
                match size {
                    0 => {
                        // Func_006F6E: 16x, rows 8-23.
                        if step == 0 {
                            self.color_15();
                        }
                        self.f8 = 8;
                        self.set_scroll(-0x60, [0x18, 0x14]);
                        self.zoom16(at + 8 * ROW_BYTES);
                    }
                    1 => {
                        // Func_006FC2: 8x.
                        self.set_scroll(-0x60, [0x18, 0x14]);
                        self.cells(at, 0xc018, 0x80);
                    }
                    _ => {
                        // Func_007022: 4x.
                        self.set_scroll(-0x60 + 0x10 * letter as i16, [0x118, 0x114]);
                        self.halves(at);
                    }
                }
            }
            3 => {
                // Func_00707C: 2x, four 4 x 4 sprites.
                self.set_scroll(0xa0, [0x118, 0x114]);
                let tile = (LARGE_TILE + letter * 0x40) as u16;
                let x = [0xe0, 0xf0, 0x110, 0x120][letter];
                for (i, sprite) in self.v.sprites[..4].iter_mut().enumerate() {
                    let i = i as u16;
                    let y = if i < 2 { 0xc8 } else { 0xe8 };
                    *sprite = [y, 0x0f01 + i, tile + 0x10 * i, x + 0x20 * (i & 1)];
                }
            }
            _ => {
                // Func_00710E: 1x, sprite 4 + letter; the 2x sprites leave
                // (Y 0).
                self.set_scroll(0xa0, [0x118, 0x114]);
                let x = [0xef, 0x105, 0x11b, 0x135][letter];
                let sprites = &mut self.v.sprites;
                let sprite = &mut sprites[4 + letter];
                sprite[0] = 0xd8;
                sprite[2] = (SMALL_TILE + letter * 0x10) as u16;
                sprite[3] = x;
                for sprite in &mut sprites[..4] {
                    sprite[0] = 0;
                }
            }
        }
    }

    /// `Func_007154`: sprites 0-3 take the 1x letters (keeping their
    /// links), then `SegaLogo_S08` in 16 frames.
    fn zoom_done(&mut self) {
        let sprites = &mut self.v.sprites;
        for i in 0..4 {
            let [y, size, tile, x] = sprites[i + 4];
            sprites[i] = [y, (size & 0xff00) | (sprites[i][1] & 0xff), tile, x];
        }
        self.f6 = 0x10;
        self.routine = 8;
    }

    /// `loc_0071A0`, dispatched on `$FF00F6`.
    fn slide(&mut self) {
        match self.f6 {
            0 => {
                // $9003: 128 x 32 planes; $8124: display off.
                let vdp = &mut self.v.vdp;
                vdp.set_plane_size(0x03);
                vdp.display = false;
                self.bar(0xa0f1);
                self.f6 = 4;
                self.f8 = 0x20;
                self.v.hscroll = [0xf8; 2];
                self.v.vscroll = [0x18; 2];
                for (letter, address) in [0xc000, 0xe02c, 0xc058, 0xe08c].into_iter().enumerate() {
                    self.cells(letter * LETTER_BYTES, address, 0x100);
                }
                self.hold = SLIDE_DRAW_HOLD;
            }
            4 | 0x0c => {
                self.scroll_left();
                self.f8 -= 1;
                if self.f8 == 0 {
                    self.f6 += 4;
                }
            }
            8 => {
                self.bar(0);
                self.scroll_left();
                self.f6 += 4;
                self.f8 = 0x20;
            }
            0x10 => {
                self.color_15();
                self.v.vdp.display = false;
                self.v.clear_plane_a();
                self.v.clear_plane_b();
                self.v.sprites[..4].copy_from_slice(&self.slide_sprites);
                self.f6 += 4;
                self.f8 = 0x20;
            }
            0x14 => {
                for sprite in &mut self.v.sprites[..4] {
                    sprite[3] = sprite[3].wrapping_add(8);
                }
                self.f8 -= 1;
                if self.f8 == 0 {
                    self.f6 += 4;
                    self.f8 = 0x21;
                }
            }
            0x18 => {
                self.f8 -= 1;
                if self.f8 == 0 {
                    self.f6 += 4;
                } else if self.f8 & 3 == 0 {
                    let at = usize::from(self.f8 & 0x1c);
                    for (sprite, &dx) in self.v.sprites[..4].iter_mut().zip(&self.wobble[at..]) {
                        sprite[3] = sprite[3].wrapping_add(i16::from(dx as i8) as u16);
                    }
                }
            }
            _ => {
                // loc_007400: sprites 4-7 copy 0-3 (keeping their links)
                // and sprite 3 links on to them.
                let sprites = &mut self.v.sprites;
                for i in 0..4 {
                    let [y, size, tile, x] = sprites[i];
                    sprites[i + 4] = [y, (size & 0xff00) | (sprites[i + 4][1] & 0xff), tile, x];
                }
                sprites[3][1] = (sprites[3][1] & 0xff00) | 4;
                self.f6 = 0x10;
                self.routine = 8;
            }
        }
    }

    /// `Func_006B94`: CRAM line 0 colour 15.
    fn color_15(&mut self) {
        self.v.palette[15] = COLOR_15;
        self.v.palette_dirty = true;
    }

    fn set_scroll(&mut self, h: i16, v: [i16; 2]) {
        self.v.hscroll = [h; 2];
        self.v.vscroll = v;
    }

    fn scroll_left(&mut self) {
        for h in &mut self.v.hscroll {
            *h = h.wrapping_sub(0x10);
        }
    }

    /// The slide's bar: 8 cells of `cell` from plane A column 95, 32 rows.
    fn bar(&mut self, cell: u16) {
        for row in 0..32 {
            for col in 0..8 {
                self.v.vdp.write_word(0xc0be + row * 0x100 + col * 2, cell);
            }
        }
    }

    /// Pixels 0..32 of a letter row, colour 0..15.
    fn row_pixels(&self, at: usize) -> impl Iterator<Item = u16> + '_ {
        self.letters[at..at + ROW_BYTES]
            .iter()
            .flat_map(|&b| [u16::from(b >> 4), u16::from(b & 15)])
    }

    /// `Func_006CA6`: 16 rows from `at`, each pixel 2 x 2 solid cells, 64
    /// cells a row across plane A.
    fn zoom16(&mut self, at: usize) {
        for row in 0..16 {
            let cells: Vec<u16> = self
                .row_pixels(at + row * ROW_BYTES)
                .flat_map(|p| [0x80f0 | p; 2])
                .collect();
            for line in 0..2 {
                for (col, &cell) in cells.iter().enumerate() {
                    self.v
                        .vdp
                        .write_word(0xc000 + (row * 2 + line) * 0x80 + col * 2, cell);
                }
            }
        }
    }

    /// `loc_006C20`: a letter one solid cell per pixel, four blank cells
    /// either side, 32 rows `stride` bytes apart from `address`.
    fn cells(&mut self, at: usize, address: usize, stride: usize) {
        for row in 0..32 {
            let cells: Vec<u16> = [0; 4]
                .into_iter()
                .chain(self.row_pixels(at + row * ROW_BYTES).map(|p| 0x80f0 | p))
                .chain([0; 4])
                .collect();
            for (col, &cell) in cells.iter().enumerate() {
                self.v
                    .vdp
                    .write_word(address + row * stride + col * 2, cell);
            }
        }
    }

    /// `Func_006BAE`: a letter two pixels per half tile, even rows on plane
    /// A from `$D42A`, odd rows on plane B from `$F42A`, 16 rows (plane B
    /// is scrolled 4 lines lower).
    fn halves(&mut self, at: usize) {
        for row in 0..16 {
            for (plane, address) in [0xd42a, 0xf42a].into_iter().enumerate() {
                let from = at + (row * 2 + plane) * ROW_BYTES;
                for i in 0..ROW_BYTES {
                    let cell = 0x8100 | u16::from(self.letters[from + i]);
                    self.v.vdp.write_word(address + row * 0x80 + i * 2, cell);
                }
            }
        }
    }
}

/// A linear bitmap `tiles_wide` tiles across cut into sprite-order tiles:
/// each 32-line band becomes `tiles_wide` columns of four tiles.
fn columns(bitmap: &[u8], tiles_wide: usize) -> Vec<u8> {
    let row_bytes = tiles_wide * 4;
    let mut out = vec![0; bitmap.len()];
    for (line, row) in bitmap.chunks(row_bytes).enumerate() {
        let (band, y) = (line / 32, line % 32);
        for (col, bytes) in row.chunks(4).enumerate() {
            let at = band * row_bytes * 32 + col * 128 + y * 4;
            out[at..at + 4].copy_from_slice(bytes);
        }
    }
    out
}
