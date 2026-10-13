//! Cartridge HUD window plane (`Hud_Init`, `$A550`).
//!
//! The native framebuffer has no VDP window plane, so this module first builds
//! the same 40 x 3 name table and then rasterizes its cells over the scene.
//! Table construction and value formatting stay independent of SDL.
use crate::{
    assets::musashi,
    front_vdp::{FONT_ART, FONT_MAP},
    render::{Frame, HwSprite, Palette, SatWriter, Tile, md_color},
    rom::{byte, pointer, word},
};

pub const WIDTH: usize = 40;
pub const HEIGHT: usize = 3;
pub const CELLS: usize = WIDTH * HEIGHT;
/// Native bar height: an odd number of pixels centres icons and counts exactly.
pub const PIXEL_HEIGHT: usize = HEIGHT * 8 + 1;

const HUD_ART: usize = 0x3bffa;
const HUD_PALETTE: usize = 0x42122;
const NINJUTSU_ICONS: usize = 0x3c88a;
const FRAME: usize = 0xa34a;
const LIVES_ICON: usize = 0xa3cc;
const SHURIKEN_ICON: usize = 0xa3e8;
const NINJUTSU_BOX: usize = 0xa404;
const LIVES_BOX: usize = 0xa420;
const AMMO_BACKGROUND: usize = 0xa442;
const LIFE_LABEL: usize = 0xa47c;
const PTS_LABEL: usize = 0xa48a;
const FRAME_DICTIONARY: usize = 0xa498;
const DIGIT_TILES: usize = 0xaa2a;
const HEALTH_COLORS: usize = 0xa87e;
/// Separate visible items, in columns of the cartridge window table. The
/// equals signs travel with their counts; labels and bars centre independently.
const ITEMS: [(usize, usize); 10] = [
    (1, 3),
    (3, 5),
    (5, 9),
    (9, 17),
    (18, 20),
    (20, 24),
    (25, 27),
    (27, 29),
    (30, 37),
    (37, 40),
];

/// `Data_00B754`: sprite table entries 0..5 (Y, size and link, tile, X) as
/// `Func_00B784` installs them with the window. Each is a 1 x 4 sprite on
/// lines -8..23 and they alternate X 1 and X 0, so every later sprite is
/// masked on the window's lines: no object shows over the HUD.
pub const WINDOW_MASKS: [[u16; 4]; 6] = {
    let mut masks = [[0; 4]; 6];
    let mut i = 0;
    while i < masks.len() {
        masks[i] = [0x78, 0x301 + i as u16, 0, (i as u16 + 1) % 2];
        i += 1;
    }
    masks
};

/// Record [`WINDOW_MASKS`] as the head of the frame's sprite table: they
/// are painted last, so they come first in link order. Shift them down one
/// pixel to cover the native bar's additional line.
pub fn window_masks(frame: &mut Frame) {
    for [y, size, _, x] in WINDOW_MASKS.into_iter().rev() {
        let sprite = HwSprite {
            x: i32::from(x) - 128,
            y: i32::from(y) - 128 + (PIXEL_HEIGHT - HEIGHT * 8) as i32,
            width: ((size >> 10) & 3) as u8 + 1,
            height: ((size >> 8) & 3) as u8 + 1,
            writer: SatWriter::Exact,
        };
        frame.sprite(sprite, &[[0; 3]; 16], []);
    }
}

/// Live values consumed by the HUD, corresponding to `$FF37B6..$FF37E9`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Values {
    pub health: u8,
    pub max_health: u8,
    pub lives: u8,
    pub ammo: u16,
    pub ninjutsu_uses: u8,
    pub ninjutsu_selected: u8,
    /// Decimal equivalent of `Score_Display`, not the real score.
    pub score_display: u32,
    /// `Game_Options` bit 7.
    pub infinite_shurikens: bool,
}

/// ROM-backed HUD art plus the immutable part of the window name table.
#[derive(Clone)]
pub struct Renderer {
    tile_first: usize,
    tiles: Vec<Tile>,
    /// Font tiles below `tile_first`, such as the blank `$2E` filling the
    /// gaps between the HUD items.
    font: Vec<Option<Tile>>,
    icons: Vec<Vec<Tile>>,
    palette: Palette,
    health_colors: [u16; 17],
    digits: [[u8; 3]; 10],
    lives_boxes: [u16; 2],
    border: [u16; CELLS],
    base: [u16; CELLS],
}

impl Renderer {
    pub fn load(rom: &[u8]) -> Result<Self, String> {
        let tile_first = usize::from(word(rom, HUD_ART)?);
        let tile_count = usize::from(word(rom, HUD_ART + 2)?) + 1;
        let (decoded, _) = musashi(rom, HUD_ART + 4, 65536)?;
        let tile_bytes = decoded.get(..tile_count * 32).ok_or("truncated HUD art")?;
        let tiles = tile_bytes
            .as_chunks::<32>()
            .0
            .iter()
            .copied()
            .map(Tile)
            .collect::<Vec<_>>();
        if tile_first > 0x40 || tile_first + tiles.len() <= 0x71 {
            return Err("HUD art does not cover tiles $40..$71".into());
        }

        let font = font_tiles(rom, tile_first)?;

        let icons = (0..4)
            .map(|index| -> Result<Vec<Tile>, String> {
                let source = pointer(rom, NINJUTSU_ICONS + index * 4)?;
                let first = word(rom, source)?;
                let count = usize::from(word(rom, source + 2)?) + 1;
                if first != 0x72 || count != 6 {
                    return Err(format!(
                        "ninjutsu icon {index} is tiles ${first:02x}.., count {count}"
                    ));
                }
                let (decoded, _) = musashi(rom, source + 4, 65536)?;
                let tile_bytes = decoded.get(..count * 32).ok_or("truncated ninjutsu icon")?;
                Ok(tile_bytes
                    .as_chunks::<32>()
                    .0
                    .iter()
                    .copied()
                    .map(Tile)
                    .collect())
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut palette = [[0; 3]; 16];
        for (index, color) in palette.iter_mut().enumerate() {
            *color = md_color(word(rom, HUD_PALETTE + index * 2)?);
        }
        let mut health_colors = [0; 17];
        for (index, color) in health_colors.iter_mut().enumerate() {
            *color = word(rom, HEALTH_COLORS + index * 2)?;
        }
        let mut digits = [[0; 3]; 10];
        for (index, digit) in digits.iter_mut().enumerate() {
            digit.copy_from_slice(bytes(rom, DIGIT_TILES + index * 4, 3)?);
        }
        let (lives_base, lives_width, lives_height) = header(rom, LIVES_BOX)?;
        if (lives_width, lives_height) != (1, 1) {
            return Err("HUD lives box is not 1x1".into());
        }
        let lives_boxes = [
            lives_base.wrapping_add(word(rom, LIVES_BOX + 10)?),
            lives_base.wrapping_add(word(rom, LIVES_BOX + 12)?),
        ];

        let mut base = [0; CELLS];
        draw_frame_table(rom, &mut base)?;
        let border = base;
        draw_table(rom, &mut base, LIVES_ICON, (1, 0), (3, 3), 2)?;
        draw_table(rom, &mut base, SHURIKEN_ICON, (18, 0), (3, 3), 2)?;
        draw_table(rom, &mut base, NINJUTSU_BOX, (25, 0), (3, 3), 2)?;
        draw_table(rom, &mut base, AMMO_BACKGROUND, (21, 0), (3, 3), 2)?;
        draw_table(rom, &mut base, LIFE_LABEL, (5, 1), (4, 1), 1)?;
        draw_table(rom, &mut base, PTS_LABEL, (37, 1), (3, 1), 1)?;

        Ok(Self {
            tile_first,
            tiles,
            font,
            icons,
            palette,
            health_colors,
            digits,
            lives_boxes,
            border,
            base,
        })
    }

    /// Build the 40 x 3 VDP window cells exactly as `Hud_Init`/`Hud_Update`.
    pub fn tilemap(&self, values: Values) -> [u16; CELLS] {
        let mut cells = self.base;

        // Hud_DrawLivesBox / Hud_DrawNineLives: more than nine spares uses
        // the alternate left edge, while the displayed digit remains nine.
        cells[WIDTH + 3] = self.lives_boxes[usize::from(values.lives > 9)];
        self.draw_digit(&mut cells, 4, usize::from(values.lives.min(9)));

        let ammo = if values.infinite_shurikens {
            999
        } else {
            values.ammo.min(999)
        };
        self.draw_ammo(&mut cells, ammo);
        self.draw_digit(&mut cells, 28, usize::from(values.ninjutsu_uses.min(9)));
        // Hud_DrawScore always leaves five leading cells blank at zero and
        // draws the final two digits; nonzero leading zeroes remain untouched.
        self.draw_number(&mut cells, 30, 7, values.score_display.min(9_999_999), 2);

        let maximum = usize::from(values.max_health.min(16));
        let health = usize::from(values.health.min(values.max_health).min(16));
        for col in 9..17 {
            cells[WIDTH + col] = 0x802e;
        }
        for cell in 0..maximum / 2 {
            cells[WIDTH + 9 + cell] = 0x806d;
        }
        if maximum % 2 != 0 {
            cells[WIDTH + 9 + maximum / 2] = 0x8071;
        }
        for cell in 0..health / 2 {
            cells[WIDTH + 9 + cell] = 0x806f;
        }
        if health % 2 != 0 {
            let at = WIDTH + 9 + health / 2;
            cells[at] = if cells[at] & 0x7ff == 0x6d {
                0x806e
            } else {
                0x8070
            };
        }
        cells
    }

    /// Rasterize the window at screen Y=0, centring each item's visible art
    /// between the border lines. Cartridge tilemap/value construction stays
    /// intact; its tile padding does not determine native vertical placement.
    pub fn draw(&self, frame: &mut Frame, values: Values) {
        let cells = self.tilemap(values);
        let mut palette = self.palette;
        palette[15] = md_color(self.health_colors[usize::from(values.health.min(16))]);
        let left = frame.view_left();
        let extra = frame.width() as i32 - (WIDTH * 8) as i32;
        let slot = frame.slot((&palette).into());
        for y in 0..PIXEL_HEIGHT {
            // Repeat the last interior line, moving only the bottom border.
            let source_y = if y >= HEIGHT * 8 - 1 { y - 1 } else { y };
            for x in 0..frame.width() {
                let source_x = if x < 18 * 8 {
                    x
                } else if x < 18 * 8 + extra as usize {
                    17 * 8 + (x - 18 * 8) % 8
                } else {
                    x - extra as usize
                };
                let index = self.cell_pixel(
                    self.border[source_y / 8 * WIDTH + source_x / 8],
                    values.ninjutsu_selected,
                    source_x % 8,
                    source_y % 8,
                );
                frame.index_pixel(left + x as i32, y as i32, slot, index);
            }
        }
        for (first, end) in ITEMS {
            let Some((top, bottom)) =
                self.item_bounds(&cells, &palette, values.ninjutsu_selected, first, end)
            else {
                continue;
            };
            // Centre white glyph faces rather than their lower black shadows.
            let text_offset = usize::from(matches!(first, 3 | 20 | 27 | 30));
            let target_top = 1 + (PIXEL_HEIGHT - 2 - (bottom - top + 1)).div_ceil(2) + text_offset;
            for y in top..=bottom {
                for x in first * 8..end * 8 {
                    let index = self.cell_pixel(
                        cells[y / 8 * WIDTH + x / 8],
                        values.ninjutsu_selected,
                        x % 8,
                        y % 8,
                    );
                    let screen_x = left + x as i32 + if x >= 18 * 8 { extra } else { 0 };
                    frame.index_pixel(screen_x, (y - top + target_top) as i32, slot, index);
                }
            }
        }
    }

    fn tile(&self, cell: u16, selected: u8) -> Option<&Tile> {
        let index = usize::from(cell & 0x07ff);
        if (0x72..=0x77).contains(&index) {
            self.icons
                .get(usize::from(selected.min(3)))?
                .get(index - 0x72)
        } else if index < self.tile_first {
            self.font.get(index)?.as_ref()
        } else {
            self.tiles.get(index - self.tile_first)
        }
    }

    fn cell_pixel(&self, cell: u16, selected: u8, x: usize, y: usize) -> u8 {
        let Some(tile) = self.tile(cell, selected) else {
            return 0;
        };
        let x = if cell & 0x0800 != 0 { 7 - x } else { x };
        let y = if cell & 0x1000 != 0 { 7 - y } else { y };
        let packed = tile.0[y * 4 + x / 2];
        if x.is_multiple_of(2) {
            packed >> 4
        } else {
            packed & 15
        }
    }

    fn item_pixel(
        &self,
        cells: &[u16; CELLS],
        palette: &Palette,
        selected: u8,
        x: usize,
        y: usize,
    ) -> Option<u8> {
        let at = y / 8 * WIDTH + x / 8;
        let index = self.cell_pixel(cells[at], selected, x % 8, y % 8);
        // Do not move the frame's white lines, its grey background, or a
        // transparent pixel. Black glyph shadows are part of the artwork.
        (index != 0
            && palette[usize::from(index)] != palette[9]
            && index != self.cell_pixel(self.border[at], selected, x % 8, y % 8))
        .then_some(index)
    }

    fn item_bounds(
        &self,
        cells: &[u16; CELLS],
        palette: &Palette,
        selected: u8,
        first: usize,
        end: usize,
    ) -> Option<(usize, usize)> {
        let occupied = |y| {
            (first * 8..end * 8).any(|x| self.item_pixel(cells, palette, selected, x, y).is_some())
        };
        let top = (0..HEIGHT * 8).find(|&y| occupied(y))?;
        let bottom = (top..HEIGHT * 8).rfind(|&y| occupied(y))?;
        Some((top, bottom))
    }

    fn draw_digit(&self, cells: &mut [u16; CELLS], col: usize, digit: usize) {
        for (row, tile) in self.digits[digit].into_iter().enumerate() {
            cells[row * WIDTH + col] = 0x8000 | u16::from(tile);
        }
    }

    fn draw_number(
        &self,
        cells: &mut [u16; CELLS],
        col: usize,
        width: usize,
        value: u32,
        minimum_digits: usize,
    ) {
        let mut divisor = 10u32.pow((width - 1) as u32);
        let mut started = false;
        for offset in 0..width {
            let digit = (value / divisor) % 10;
            let remaining = width - offset;
            if digit != 0 || started || remaining <= minimum_digits {
                self.draw_digit(cells, col + offset, digit as usize);
                started = true;
            }
            divisor = (divisor / 10).max(1);
        }
    }

    /// `Hud_DrawAmmo` advances the VDP destination only when it draws a
    /// digit, so a one- or two-digit count is left-aligned in the box.
    fn draw_ammo(&self, cells: &mut [u16; CELLS], ammo: u16) {
        let digits = [ammo / 100, ammo / 10 % 10, ammo % 10];
        let mut col = 21;
        let mut started = false;
        for (index, digit) in digits.into_iter().enumerate() {
            if digit != 0 || started || index == 2 {
                self.draw_digit(cells, col, usize::from(digit));
                col += 1;
                started = true;
            }
        }
    }
}

/// `Func_00A254`: the resident 1bpp font below the HUD art, expanded with
/// colour 11 on colour 9. Tile 0 stays empty.
fn font_tiles(rom: &[u8], tile_first: usize) -> Result<Vec<Option<Tile>>, String> {
    let first = usize::from(word(rom, FONT_ART)?);
    let count = usize::from(word(rom, FONT_ART + 2)?) + 1;
    let art = bytes(rom, FONT_ART + 4, count * 8)?;
    let mut tiles = vec![None; tile_first];
    for (index, rows) in art.as_chunks::<8>().0.iter().enumerate() {
        let Some(slot) = tiles.get_mut(first + index).filter(|_| first + index != 0) else {
            continue;
        };
        let mut tile = [0; 32];
        for (y, row) in rows.iter().enumerate() {
            for x in 0..8 {
                let color = if row & (0x80 >> x) != 0 { 11 } else { 9 };
                tile[y * 4 + x / 2] |= if x % 2 == 0 { color << 4 } else { color };
            }
        }
        *slot = Some(Tile(tile));
    }
    Ok(tiles)
}

fn draw_frame_table(rom: &[u8], cells: &mut [u16; CELLS]) -> Result<(), String> {
    let (base, width, height) = header(rom, FRAME)?;
    if (width, height) != (WIDTH, HEIGHT) {
        return Err(format!("HUD frame is {width}x{height}, expected 40x3"));
    }
    for (at, encoded) in bytes(rom, FRAME + 10, CELLS)?.iter().copied().enumerate() {
        let tile = if encoded >= 0x61 {
            word(rom, FRAME_DICTIONARY + usize::from(encoded - 0x61) * 2)?
        } else {
            u16::from(byte(rom, FONT_MAP + usize::from(encoded))?)
        };
        cells[at] = base.wrapping_add(tile);
    }
    Ok(())
}

/// A mapped table of `cell_bytes`-byte tile cells (words or bytes) drawn
/// at cell (col, row); its header must give the expected size.
fn draw_table(
    rom: &[u8],
    cells: &mut [u16; CELLS],
    table: usize,
    (col, row): (usize, usize),
    expected: (usize, usize),
    cell_bytes: usize,
) -> Result<(), String> {
    let (base, width, height) = header(rom, table)?;
    if (width, height) != expected {
        return Err(format!(
            "HUD table ${table:05x} is {width}x{height}, expected {}x{}",
            expected.0, expected.1
        ));
    }
    for y in 0..height {
        for x in 0..width {
            let at = table + 10 + (y * width + x) * cell_bytes;
            let tile = if cell_bytes == 2 {
                word(rom, at)?
            } else {
                u16::from(byte(rom, at)?)
            };
            cells[(row + y) * WIDTH + col + x] = base.wrapping_add(tile);
        }
    }
    Ok(())
}

/// Map headers store a VDP command long, tile attributes, width-1, height-1.
fn header(rom: &[u8], at: usize) -> Result<(u16, usize, usize), String> {
    Ok((
        word(rom, at + 4)?,
        usize::from(word(rom, at + 6)?) + 1,
        usize::from(word(rom, at + 8)?) + 1,
    ))
}

fn bytes(rom: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    rom.get(at..at + len)
        .ok_or_else(|| format!("HUD data ${at:05x}.. outside ROM"))
}
