//! Native sprite batching in painter's order, with explicit line masks.
//!
//! Sprite counts and pixel widths do not limit rendering. The cartridge's
//! X-zero masks still hide the HUD area and laser rooms, and recorded sprites
//! retain their plane priority and stage-specific line cuts.
use super::{Color, Flip, Frame, Palette, PaletteRef, Tile};
use crate::HEIGHT;
#[cfg(test)]
use crate::WIDTH;
use crate::art_pack::Replacement;
use std::ops::Range;
use std::rc::Rc;

/// The routine that writes a sprite into the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SatWriter {
    /// `Object_DrawSprite` (`$14CA`): a piece whose X comes out 0 is
    /// written at X 1 (`$1560`), so an object never masks by accident.
    Object,
    /// Written at its X, so at X 0 it masks: the window's entries 0..5
    /// (`Data_00B754`) and objects with `obj_tile` `$FFFF` (`$14E0` keeps
    /// X 0 for them: the 3-2 laser rooms' `$118`).
    Exact,
    /// `$39A02` scenery: linked after the objects, behind their sprites.
    Scenery,
}

/// One hardware sprite (a SAT entry): `width` x `height` cells with the
/// top left corner at screen (`x`, `y`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HwSprite {
    pub x: i32,
    pub y: i32,
    pub width: u8,
    pub height: u8,
    pub writer: SatWriter,
}
impl HwSprite {
    /// The nine-bit X the table holds (128 = the display's left edge).
    pub fn sat_x(self) -> u16 {
        let x = ((self.x + 128) & 0x1ff) as u16;
        if x == 0 && self.writer == SatWriter::Object {
            1
        } else {
            x
        }
    }
    fn lines(self) -> Range<i32> {
        self.y..self.y + i32::from(self.height) * 8
    }
}

/// One cell of a hardware sprite, at its screen position.
#[derive(Clone, Copy)]
pub struct SpriteCell<'a> {
    pub tile: &'a Tile,
    pub x: i32,
    pub y: i32,
    pub flip: Flip,
}

/// Explicit X-zero masking in link order on one display line.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SpriteMask {
    armed: bool,
    masked: bool,
}
impl SpriteMask {
    /// Whether the next sprite is visible after applying its mask command.
    pub(crate) fn next(&mut self, sat_x: u16) -> bool {
        if sat_x != 0 {
            self.armed = true;
        } else if self.armed {
            self.masked = true;
        }
        !self.masked
    }
}

struct Entry {
    sprite: HwSprite,
    colors: Palette,
    cram: Option<u8>,
    /// Drawn as a high-priority sprite ([`Frame::mark_high_sprites`]).
    high: bool,
    /// The pixel index writes stop at ([`Frame::clip_lines`]).
    end: usize,
    cells: Range<usize>,
    /// Start in `Sat::rows` of the visible rows, for a masked sprite.
    rows: Option<usize>,
}

struct Cell {
    tile: Tile,
    x: i32,
    y: i32,
    flip: Flip,
    art: Option<Rc<Replacement>>,
}

/// A native marker drawn among the sprites ([`Frame::overlay_rect`]).
struct Overlay {
    /// Drawn after this many entries.
    after: usize,
    rect: (i32, i32, i32, i32),
    color: Color,
}

/// The frame's recorded sprite table, in painter's order.
#[derive(Default)]
pub(super) struct Sat {
    recording: bool,
    entries: Vec<Entry>,
    cells: Vec<Cell>,
    rows: Vec<u8>,
    overlays: Vec<Overlay>,
}
impl Sat {
    pub(super) fn clear(&mut self) {
        self.recording = false;
        self.entries.clear();
        self.cells.clear();
        self.rows.clear();
        self.overlays.clear();
    }

    /// Link order: the painter's order reversed, then the scenery.
    fn link_order(&self) -> Vec<usize> {
        let (tail, head): (Vec<usize>, Vec<usize>) = (0..self.entries.len())
            .rev()
            .partition(|&index| self.entries[index].sprite.writer == SatWriter::Scenery);
        head.into_iter().chain(tail).collect()
    }

    /// Apply explicit masks without hardware count or pixel budgets.
    fn apply_masks(&mut self, width: usize) {
        let mut masks = [SpriteMask::default(); HEIGHT];
        for index in self.link_order() {
            let entry = &mut self.entries[index];
            let sprite = entry.sprite;
            let end = (entry.end / width) as i32;
            for line in sprite.y.max(0)..sprite.lines().end.min(end) {
                if !masks[line as usize].next(sprite.sat_x()) {
                    let rows = *entry.rows.get_or_insert_with(|| {
                        let rows = self.rows.len();
                        self.rows
                            .resize(rows + usize::from(sprite.height) * 8, u8::MAX);
                        rows
                    });
                    self.rows[rows + (line - sprite.y) as usize] = 0;
                }
            }
        }
    }

    /// Screen rows (bit `dy`) of a cell at `y` that `entry` shows.
    fn shown_rows(&self, entry: &Entry, y: i32) -> u8 {
        let Some(rows) = entry.rows else {
            return u8::MAX;
        };
        let mut shown = 0;
        for dy in 0..8 {
            let row = y + dy - entry.sprite.y;
            let visible = usize::try_from(row)
                .ok()
                .and_then(|row| self.rows.get(rows + row))
                .map_or(u8::MAX, |&visible| visible);
            if visible != 0 {
                shown |= 1 << dy;
            }
        }
        shown
    }
}

impl Frame {
    /// Record the sprites drawn from here on ([`Frame::sprite`]) as the
    /// frame's sprite table instead of drawing them.
    pub fn begin_sprite_table(&mut self) {
        self.sat.clear();
        self.sat.recording = true;
    }

    /// Sprites recorded so far.
    pub fn sprite_count(&self) -> usize {
        self.sat.entries.len()
    }

    /// The recorded sprites in link order, including scenery at the end.
    #[cfg(test)]
    pub(crate) fn sprite_table(&self) -> Vec<HwSprite> {
        self.sat
            .link_order()
            .into_iter()
            .map(|index| self.sat.entries[index].sprite)
            .collect()
    }

    /// Stop recording and apply explicit line masks.
    pub fn end_sprite_table(&mut self) {
        self.sat.recording = false;
        self.sat.apply_masks(self.width);
    }

    /// Draw recorded sprites `range` in painter's order, with their masks,
    /// priority marking and line clip.
    pub fn draw_sprite_table(&mut self, range: Range<usize>) {
        let sat = std::mem::take(&mut self.sat);
        let (marking, end) = (self.marking, self.end);
        let overlays = |frame: &mut Frame, after: usize| {
            for overlay in sat.overlays.iter().filter(|o| o.after == after) {
                let (x, y, w, h) = overlay.rect;
                frame.rect(x, y, w, h, overlay.color);
            }
        };
        if range.start == 0 {
            overlays(self, 0);
        }
        for (index, entry) in sat.entries[range.clone()].iter().enumerate() {
            let slot = self.slot(match entry.cram {
                Some(line) => PaletteRef::Cram(line, &entry.colors),
                None => PaletteRef::Fixed(&entry.colors),
            });
            self.marking = entry.high;
            self.end = entry.end;
            for cell in &sat.cells[entry.cells.clone()] {
                let rows = sat.shown_rows(entry, cell.y);
                if rows != 0 {
                    self.tile_rows_with_art(
                        &cell.tile,
                        slot,
                        cell.x,
                        cell.y,
                        cell.flip,
                        rows,
                        cell.art.clone(),
                    );
                }
            }
            (self.marking, self.end) = (marking, end);
            overlays(self, range.start + index + 1);
        }
        self.sat = sat;
    }

    /// A native marker rectangle (an outline standing in for unported
    /// art): while a sprite table is recorded it is drawn in its place
    /// among the sprites, unaffected by line masks.
    pub fn overlay_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        if self.sat.recording {
            self.sat.overlays.push(Overlay {
                after: self.sat.entries.len(),
                rect: (x, y, w, h),
                color,
            });
        } else {
            self.rect(x, y, w, h, color);
        }
    }

    /// One hardware sprite. While a sprite table is recorded it becomes
    /// the table's next entry; otherwise its cells are drawn at once.
    pub fn sprite<'a>(
        &mut self,
        sprite: HwSprite,
        palette: impl Into<PaletteRef<'a>>,
        cells: impl IntoIterator<Item = SpriteCell<'a>>,
    ) {
        self.sprite_art(sprite, palette, cells.into_iter().map(|cell| (cell, None)));
    }

    pub(crate) fn sprite_art<'a>(
        &mut self,
        sprite: HwSprite,
        palette: impl Into<PaletteRef<'a>>,
        cells: impl IntoIterator<Item = (SpriteCell<'a>, Option<Rc<Replacement>>)>,
    ) {
        let palette = palette.into();
        if !self.sat.recording {
            let slot = self.slot(palette);
            for (cell, art) in cells {
                self.tile_rows_with_art(cell.tile, slot, cell.x, cell.y, cell.flip, u8::MAX, art);
            }
            return;
        }
        let start = self.sat.cells.len();
        self.sat
            .cells
            .extend(cells.into_iter().map(|(cell, art)| Cell {
                tile: *cell.tile,
                x: cell.x,
                y: cell.y,
                flip: cell.flip,
                art,
            }));
        let (colors, cram) = match palette {
            PaletteRef::Cram(line, colors) => (*colors, Some(line)),
            PaletteRef::Fixed(colors) => (*colors, None),
        };
        self.sat.entries.push(Entry {
            sprite,
            colors,
            cram,
            high: self.marking,
            end: self.end,
            cells: start..self.sat.cells.len(),
            rows: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIT: Palette = {
        let mut palette = [[0; 3]; 16];
        palette[1] = [255, 255, 255];
        palette
    };
    const SOLID: Tile = Tile([0x11; 32]);

    fn object(x: i32, y: i32, width: u8, height: u8) -> HwSprite {
        HwSprite {
            x,
            y,
            width,
            height,
            writer: SatWriter::Object,
        }
    }

    /// A solid `width` x `height` sprite.
    fn solid(frame: &mut Frame, sprite: HwSprite) {
        let cells: Vec<SpriteCell> = (0..i32::from(sprite.width))
            .flat_map(|col| {
                (0..i32::from(sprite.height)).map(move |row| SpriteCell {
                    tile: &SOLID,
                    x: sprite.x + col * 8,
                    y: sprite.y + row * 8,
                    flip: Flip::default(),
                })
            })
            .collect();
        frame.sprite(sprite, &LIT, cells);
    }

    /// Lit pixels of display line `y`.
    fn lit(frame: &Frame, y: usize) -> Vec<usize> {
        let width = frame.width();
        let row = &frame.rgb()[y * width * 3..(y + 1) * width * 3];
        (0..width).filter(|&x| row[x * 3] == 255).collect()
    }

    #[test]
    fn widescreen_sprite_cells_keep_original_masks_and_line_cuts() {
        let mut frame = Frame::new(crate::WIDE_WIDTH);
        frame.begin_sprite_table();
        frame.clip_lines(Some(34));
        solid(&mut frame, object(-39, 24, 2, 2));
        solid(&mut frame, object(343, 24, 2, 2));
        crate::hud::window_masks(&mut frame);
        frame.clip_lines(None);
        frame.end_sprite_table();
        frame.draw_sprite_table(0..frame.sprite_count());
        assert!(lit(&frame, 24).is_empty());
        assert_eq!(lit(&frame, 25), (0..16).chain(382..398).collect::<Vec<_>>());
        assert_eq!(lit(&frame, 33).len(), 32);
        assert!(lit(&frame, 34).is_empty());
    }

    /// Record `sprites` in link order (so painted in reverse) and draw.
    fn table(sprites: &[HwSprite]) -> Frame {
        let mut frame = Frame::default();
        frame.clear([0; 3]);
        frame.begin_sprite_table();
        for &sprite in sprites.iter().rev() {
            solid(&mut frame, sprite);
        }
        frame.end_sprite_table();
        frame.draw_sprite_table(0..frame.sprite_count());
        frame
    }

    #[test]
    fn sprites_are_not_limited_by_scanline_count_or_pixel_width() {
        // The 21st sprite remains visible on the crowded line as well.
        let mut sprites: Vec<HwSprite> = (0..20).map(|n| object(n * 8, 50, 1, 1)).collect();
        sprites.push(object(200, 50, 1, 2));
        let frame = table(&sprites);
        let mut expected: Vec<usize> = (0..160).collect();
        expected.extend(200..208);
        assert_eq!(lit(&frame, 50), expected);
        assert_eq!(lit(&frame, 58), (200..208).collect::<Vec<_>>());
        // Off-screen pieces do not make the right edge disappear.
        let mut sprites: Vec<HwSprite> = (0..9).map(|n| object(n * 32, 50, 4, 1)).collect();
        sprites.insert(0, object(-40, 50, 2, 1)); // 16 px, none visible
        sprites.push(object(290, 50, 4, 1)); // fully visible to the screen edge
        sprites.push(object(300, 50, 1, 1));
        let frame = table(&sprites);
        let mut expected: Vec<usize> = (0..288).collect();
        expected.extend(290..320);
        assert_eq!(lit(&frame, 50), expected);
    }

    #[test]
    fn a_sprite_at_x_zero_masks_the_rest_of_its_lines() {
        let mask = HwSprite {
            writer: SatWriter::Exact,
            ..object(-128, 100, 1, 2)
        };
        // Armed by a sprite with a nonzero X on the line (the window's X 1
        // entry, or the laser room's X 1 record): lower sprites are hidden
        // on the mask's lines only.
        let armed = HwSprite {
            writer: SatWriter::Exact,
            ..object(-127, 100, 1, 1)
        };
        let frame = table(&[armed, mask, object(40, 96, 2, 4)]);
        assert_eq!(lit(&frame, 99), (40..56).collect::<Vec<_>>());
        assert!(lit(&frame, 100).is_empty() && lit(&frame, 107).is_empty());
        // Line 108 has no armed sprite before the mask.
        assert_eq!(lit(&frame, 108), (40..56).collect::<Vec<_>>());
        // An object's piece at X 0 is written at X 1 and masks nothing.
        let frame = table(&[armed, object(-128, 100, 1, 1), object(40, 96, 2, 4)]);
        assert_eq!(lit(&frame, 100), (40..56).collect::<Vec<_>>());
        // A crowded line cannot arm a mask on the next line.
        let mut sprites: Vec<HwSprite> = (0..10).map(|n| object(n * 32, 92, 4, 1)).collect();
        sprites.push(HwSprite { y: 100, ..mask });
        sprites.push(object(8, 96, 1, 8));
        let frame = table(&sprites);
        assert_eq!(lit(&frame, 99).len(), 320);
        assert_eq!(lit(&frame, 100), (8..16).collect::<Vec<_>>());
    }

    #[test]
    fn the_table_keeps_more_than_80_entries_with_scenery_at_its_end() {
        // Scenery stays behind every object without evicting any pieces.
        let mut frame = Frame::default();
        frame.clear([0; 3]);
        frame.begin_sprite_table();
        for n in 0..4 {
            solid(
                &mut frame,
                HwSprite {
                    writer: SatWriter::Scenery,
                    ..object(n * 8, 200, 1, 1)
                },
            );
        }
        for n in (0..78).rev() {
            solid(&mut frame, object(n % 10 * 8, 100 + n / 10 * 8, 1, 1));
        }
        frame.end_sprite_table();
        frame.draw_sprite_table(0..frame.sprite_count());
        assert_eq!(lit(&frame, 200), (0..32).collect::<Vec<_>>());
        assert_eq!(lit(&frame, 100), (0..80).collect::<Vec<_>>());
        assert_eq!(lit(&frame, 156), (0..64).collect::<Vec<_>>());
        let sprites = frame.sprite_table();
        assert_eq!(sprites.len(), 82);
        assert!(sprites[78..].iter().all(|s| s.writer == SatWriter::Scenery));
    }

    #[test]
    fn recorded_sprites_keep_their_clip_and_priority() {
        let mut frame = Frame::default();
        frame.clear([0; 3]);
        // Outside a table sprites draw at once, without limits.
        for n in 0..24 {
            solid(&mut frame, object(n * 8, 10, 1, 1));
        }
        assert_eq!(lit(&frame, 10).len(), 192);
        frame.begin_sprite_table();
        // Painted first (last in link order): ten 4-cell sprites on line 60.
        for n in 0..10 {
            solid(&mut frame, object(n * 32, 60, 4, 1));
        }
        let low = frame.sprite_count();
        solid(&mut frame, object(8, 48, 1, 1));
        // First in link order, high priority and cut at line 60 (the 4-1
        // H-interrupt): on the lines below the cut it spends nothing.
        frame.clip_lines(Some(60));
        frame.mark_high_sprites(true);
        solid(&mut frame, object(0, 48, 1, 2));
        frame.mark_high_sprites(false);
        frame.clip_lines(None);
        frame.end_sprite_table();
        frame.draw_sprite_table(0..low);
        assert_eq!(lit(&frame, 60).len(), 320);
        frame.draw_sprite_table(low..low + 1);
        // A high-priority plane pass covers the low sprite, not the high.
        frame.draw_sprite_table(low + 1..frame.sprite_count());
        let mut blue = [[0; 3]; 16];
        blue[1] = [0, 0, 255];
        frame.behind_high_sprites(true);
        frame.tile(&SOLID, &blue, 0, 48, Flip::default());
        frame.tile(&SOLID, &blue, 8, 48, Flip::default());
        frame.behind_high_sprites(false);
        assert_eq!(lit(&frame, 48), (0..8).collect::<Vec<_>>());
        assert_eq!(lit(&frame, 59), (0..8).collect::<Vec<_>>());
        assert_eq!(lit(&frame, 60).len(), 320);
        assert!(!frame.marking && frame.end == WIDTH * HEIGHT);
    }

    #[test]
    fn crowded_sprites_obey_masks_and_overlap_without_reservations() {
        let mut frame = Frame::default();
        frame.begin_sprite_table();
        solid(&mut frame, object(0, 100, 4, 2));
        // Cosmetic sprites use up the line even though they are off screen.
        for _ in 0..10 {
            solid(&mut frame, object(320, 100, 4, 2));
        }
        // A front sprite still covers the player's first eight pixels.
        let blue = [[0, 0, 255]; 16];
        frame.sprite(
            object(0, 100, 1, 1),
            &blue,
            [SpriteCell {
                tile: &SOLID,
                x: 0,
                y: 100,
                flip: Flip::default(),
            }],
        );
        // Explicit window masks must still hide the player.
        for x in [-128, -127] {
            solid(
                &mut frame,
                HwSprite {
                    writer: SatWriter::Exact,
                    ..object(x, 108, 1, 1)
                },
            );
        }
        frame.end_sprite_table();
        frame.draw_sprite_table(0..frame.sprite_count());
        assert_eq!(lit(&frame, 100), (8..32).collect::<Vec<_>>());
        assert!(lit(&frame, 108).is_empty());

        frame.clear([0; 3]);
        frame.begin_sprite_table();
        solid(&mut frame, object(200, 200, 1, 1));
        for n in 0..80 {
            solid(&mut frame, object(n % 10 * 8, n / 10 * 8, 1, 1));
        }
        frame.end_sprite_table();
        frame.draw_sprite_table(0..frame.sprite_count());
        assert_eq!(frame.sprite_table().len(), 81);
        assert_eq!(lit(&frame, 200), (200..208).collect::<Vec<_>>());
    }
}
