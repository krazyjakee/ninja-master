use crate::art_pack::{ArtPack, Replacement};
use crate::{HEIGHT, WIDTH};
use std::{
    cell::OnceCell,
    collections::HashMap,
    io::{self, Write},
    rc::Rc,
};

mod art;
mod sat;
pub(crate) use sat::SpriteMask;
pub use sat::{HwSprite, SatWriter, SpriteCell};

pub type Color = [u8; 3];
pub type Palette = [Color; 16];

/// A Mega Drive CRAM word (`0000 BBB0 GGG0 RRR0`) as RGB, 255 / 7 per level.
pub fn md_color(word: u16) -> Color {
    [
        (((word >> 1) & 7) * 255 / 7) as u8,
        (((word >> 5) & 7) * 255 / 7) as u8,
        (((word >> 9) & 7) * 255 / 7) as u8,
    ]
}

/// CRAM words as the four stage palettes.
pub fn cram_palettes(words: &[[u16; 16]; 4]) -> [Palette; 4] {
    words.map(|line| line.map(md_color))
}

/// CRAM nibble steps in a complete fade: colours use the even values 0..=14.
pub const FADE_STEPS: u8 = 14;

/// Palette fades as `$5EA` (in) and `$67A` (out) run them: each update moves
/// every CRAM nibble one step, so darker channels reach black first on the
/// way out and reach their colour first on the way in. `Mode_Game` fades once
/// per frame (`$FF16E8` = 0); the round card fades every fourth frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fade {
    /// `step` decrements applied (0 = unchanged, `FADE_STEPS` = black).
    Out(u8),
    /// `step` increments from black toward the palette.
    In(u8),
}
impl Fade {
    /// One more fade update, stopping at `FADE_STEPS`.
    pub fn advance(self) -> Self {
        match self {
            Self::Out(step) => Self::Out((step + 1).min(FADE_STEPS)),
            Self::In(step) => Self::In((step + 1).min(FADE_STEPS)),
        }
    }
    pub fn finished(self) -> bool {
        matches!(self, Self::Out(step) | Self::In(step) if step >= FADE_STEPS)
    }
    /// One 8-bit channel after the fade. Channels are mapped back to their
    /// 3-bit CRAM level (nibble 2 * level); the hardware ignores nibble bit 0.
    pub fn channel(self, value: u8) -> u8 {
        let nibble = ((u32::from(value) * 7 + 127) / 255) * 2;
        let faded = match self {
            Self::Out(step) => nibble.saturating_sub(u32::from(step)),
            Self::In(step) => nibble.min(u32::from(step)),
        };
        ((faded / 2) * 255 / 7) as u8
    }
}

/// Genesis-format tile: 8x8 pixels, two four-bit palette indices per byte.
/// This decodes an already located/decompressed tile, not arbitrary ROM data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile(pub [u8; 32]);

#[derive(Clone, Copy, Default)]
pub struct Flip {
    pub x: bool,
    pub y: bool,
}

/// A palette as a draw call names it. The VDP keeps a CRAM index per pixel
/// and looks its colour up as the line is scanned out, so a CRAM write
/// part-way down the screen recolours what uses that line from there on.
/// The frame keeps the same distinction: see [`Frame::change_cram_line`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteRef<'a> {
    /// CRAM line `line` (0..4) holding these colours at the top of the
    /// frame: its pixels follow raster changes to the line.
    Cram(u8, &'a Palette),
    /// A palette of the native port's own (a hit-flash copy of a line, the
    /// HUD's window line, tool and demo screens): no CRAM change reaches it.
    Fixed(&'a Palette),
}
impl<'a> PaletteRef<'a> {
    pub fn colors(self) -> &'a Palette {
        match self {
            Self::Cram(_, colors) | Self::Fixed(colors) => colors,
        }
    }
}
impl<'a> From<&'a Palette> for PaletteRef<'a> {
    fn from(colors: &'a Palette) -> Self {
        Self::Fixed(colors)
    }
}

impl<'a> From<&PaletteRef<'a>> for PaletteRef<'a> {
    fn from(palette: &PaletteRef<'a>) -> Self {
        *palette
    }
}

/// The four palettes attribute bits 13-14 select.
pub type Lines<'a> = [PaletteRef<'a>; 4];

/// CRAM lines 0..4 holding `palettes`.
pub fn cram_lines(palettes: &[Palette; 4]) -> Lines<'_> {
    std::array::from_fn(|line| PaletteRef::Cram(line as u8, &palettes[line]))
}

/// A CRAM line written during the frame (an H-interrupt handler's, Boss4's
/// `Func_02EF80`): from display line `line` down, CRAM line `cram_line`
/// holds `colors` until the frame ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CramChange {
    pub line: usize,
    pub cram_line: u8,
    pub colors: Palette,
}

/// A palette interned in the frame ([`Frame::slot`]); its pixels store the
/// slot and the colour index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot(u32);

#[derive(Clone, Copy)]
struct SlotPalette {
    colors: Palette,
    /// The CRAM line it stands for (`None`: a fixed palette).
    cram: Option<u8>,
}

/// A pixel entry holding an RGB colour of its own (`clear`, `rect`,
/// `pixel`, and everything once the frame is baked) rather than a slot's
/// colour index (`slot << 4 | index`).
const DIRECT: u32 = 1 << 31;
const BLEND: u32 = 1 << 30;
const TINT: u32 = 1 << 29;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PixelTint {
    color: Color,
    reference: Color,
    entry: u32,
}

struct Hd {
    scale: usize,
    pixels: Vec<u32>,
    high: Vec<bool>,
    rgb: OnceCell<Vec<u8>>,
}

type ArtEntries = Rc<EncodedArt>;

struct EncodedArt {
    scale: usize,
    pixels: Vec<(u32, u8)>,
    values: Vec<u32>,
    row_alpha: Vec<u8>,
}
impl std::ops::Deref for EncodedArt {
    type Target = [(u32, u8)];
    fn deref(&self) -> &Self::Target {
        &self.pixels
    }
}

struct PixelBlend {
    entries: [u32; 4],
    weights: [f32; 4],
}

#[inline(always)]
fn resolve_entry(entry: u32, table: &[Color], blends: &[PixelBlend], tints: &[Color]) -> Color {
    if entry & TINT != 0 && entry & (DIRECT | BLEND) == 0 {
        tints[(entry & !TINT) as usize]
    } else if entry & DIRECT != 0 {
        [(entry >> 16) as u8, (entry >> 8) as u8, entry as u8]
    } else if entry & BLEND != 0 {
        let blend = &blends[(entry & !BLEND) as usize];
        let mut color = [0.0; 3];
        for (&sample, &weight) in blend.entries.iter().zip(&blend.weights) {
            if weight == 0.0 {
                continue;
            }
            let rgb = resolve_entry(sample, table, blends, tints);
            for channel in 0..3 {
                color[channel] += f32::from(rgb[channel]) * weight;
            }
        }
        color.map(|value| value.round().clamp(0.0, 255.0) as u8)
    } else {
        table[entry as usize]
    }
}

fn resolve_tint(tint: &PixelTint, table: &[Color]) -> Color {
    // Cached tiles from an earlier frame may refer to slots absent in this
    // frame. Those entries are not drawn, but still occupy the tint cache.
    let live = table.get(tint.entry as usize).copied().unwrap_or([0; 3]);
    let total = tint.reference.iter().map(|&c| u32::from(c)).sum::<u32>();
    let live_total = live.iter().map(|&c| u32::from(c)).sum::<u32>();
    std::array::from_fn(|i| {
        let (value, reference) = if tint.reference[i] != 0 {
            (u32::from(live[i]), u32::from(tint.reference[i]))
        } else {
            (live_total, total.max(1))
        };
        (u32::from(tint.color[i]) * value / reference).min(255) as u8
    })
}

const ART_CACHE_BYTES: usize = 64 * 1024 * 1024;
/// More distinct palettes than this in one frame bake the frame first (a
/// fade with more than half of them bakes instead of copying them).
const SLOT_LIMIT: usize = 256;

fn direct(color: Color) -> u32 {
    DIRECT | u32::from(color[0]) << 16 | u32::from(color[1]) << 8 | u32::from(color[2])
}

/// The frame as the VDP composes it: every pixel is a palette slot and
/// colour index (or a colour of its own) until [`Frame::rgb`] resolves each
/// display line through the palettes in effect on that line.
pub struct Frame {
    /// Desktop presentation preference; headless ROM comparisons keep it off.
    pub smooth_wave_effects: bool,
    width: usize,
    /// Where the original screen's X=0 lands in the expanded frame.
    origin_x: i32,
    pixels: Vec<u32>,
    blends: Vec<PixelBlend>,
    slots: Vec<SlotPalette>,
    /// Mid-frame CRAM line writes, in display line order.
    changes: Vec<CramChange>,
    /// Pixels drawn by high-priority sprites since the last `clear`.
    high: Vec<bool>,
    /// Writes mark `high` (a high-priority sprite is being drawn).
    marking: bool,
    /// Writes skip `high` pixels (a high-priority plane pass is drawn).
    behind: bool,
    /// Writes at or past this pixel index are dropped (`clip_lines`).
    end: usize,
    /// The resolved RGB24 frame, until the next write.
    rgb: OnceCell<Vec<u8>>,
    /// The sprite table being recorded ([`Frame::begin_sprite_table`]).
    sat: sat::Sat,
    art_pack: Option<Rc<ArtPack>>,
    hd: Option<Hd>,
    tints: Vec<PixelTint>,
    tint_entries: HashMap<PixelTint, u32>,
    art_entries: HashMap<(usize, u32, usize), ArtEntries>,
    art_cache_bytes: usize,
    art_cache_dirty: bool,
}

impl Default for Frame {
    fn default() -> Self {
        Self::new(WIDTH)
    }
}

impl Frame {
    /// A square-pixel frame with room for a wider world view. UI coordinates
    /// initially retain their original 320-pixel canvas, centred in it.
    pub fn new(width: usize) -> Self {
        assert!(width >= WIDTH && width <= i32::MAX as usize);
        Self {
            smooth_wave_effects: false,
            width,
            origin_x: ((width - WIDTH) / 2) as i32,
            pixels: vec![direct([0; 3]); width * HEIGHT],
            blends: Vec::new(),
            slots: Vec::new(),
            changes: Vec::new(),
            high: vec![false; width * HEIGHT],
            marking: false,
            behind: false,
            end: width * HEIGHT,
            rgb: OnceCell::new(),
            sat: sat::Sat::default(),
            art_pack: None,
            hd: None,
            tints: Vec::new(),
            tint_entries: HashMap::new(),
            art_entries: HashMap::new(),
            art_cache_bytes: 0,
            art_cache_dirty: false,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn art_pack(&self) -> Option<Rc<ArtPack>> {
        self.art_pack.clone()
    }

    pub fn output_scale(&self) -> usize {
        self.hd.as_ref().map_or(1, |hd| hd.scale)
    }
    pub fn output_width(&self) -> usize {
        self.width * self.output_scale()
    }
    pub fn output_height(&self) -> usize {
        HEIGHT * self.output_scale()
    }

    /// Desktop/capture pixels. `rgb` retains native resolution for game tooling.
    pub fn presentation_rgb(&self) -> &[u8] {
        self.hd.as_ref().map_or_else(
            || self.rgb(),
            |hd| {
                hd.rgb
                    .get_or_init(|| self.resolve_pixels(&hd.pixels, self.output_width(), hd.scale))
            },
        )
    }

    fn invalidate(&mut self) {
        self.rgb.take();
        if let Some(hd) = &mut self.hd {
            hd.rgb.take();
        }
    }

    /// Visible X coordinates in the original screen's coordinate space.
    pub fn view_left(&self) -> i32 {
        -self.origin_x
    }

    pub fn view_right(&self) -> i32 {
        self.width as i32 - self.origin_x
    }

    pub fn center_content(&mut self) {
        self.origin_x = ((self.width - WIDTH) / 2) as i32;
    }

    /// Visible screen coordinates after clamping to the layout. Layouts
    /// narrower than the expanded frame have equal space on either side.
    pub(crate) fn world_view(width: usize, camera_x: i32, world_width: i32) -> (i32, i32) {
        let width = width as i32;
        let left = if width == WIDTH as i32 {
            0
        } else if world_width < width {
            -((width - world_width) / 2) - camera_x
        } else {
            (camera_x - (width - WIDTH as i32) / 2).clamp(0, world_width - width) - camera_x
        };
        (left, left + width)
    }

    /// Expand around the gameplay camera, keeping the view inside the layout
    /// at either end. This changes drawing only, never simulation coordinates.
    pub fn align_world(&mut self, camera_x: i32, world_width: i32) {
        self.origin_x = -Self::world_view(self.width, camera_x, world_width).0;
    }

    /// Difference between the original follow camera and the camera at the
    /// centre of the expanded view. Parallax follows the latter at level edges.
    pub(crate) fn world_camera_delta(&self) -> i32 {
        self.view_left() + ((self.width - WIDTH) / 2) as i32
    }

    /// The RGB24 frame: each display line's pixels through the palettes in
    /// effect on that line.
    pub fn rgb(&self) -> &[u8] {
        self.rgb.get_or_init(|| self.resolve())
    }

    fn resolve(&self) -> Vec<u8> {
        self.resolve_pixels(&self.pixels, self.width, 1)
    }

    fn resolve_pixels(&self, pixels: &[u32], width: usize, scale: usize) -> Vec<u8> {
        let mut table: Vec<Color> = self.slots.iter().flat_map(|slot| slot.colors).collect();
        let mut tints: Vec<Color> = self.tints.iter().map(|t| resolve_tint(t, &table)).collect();
        let mut changes = self.changes.iter().peekable();
        let mut rgb = vec![0; pixels.len() * 3];
        let rows = pixels.chunks_exact(width);
        let lines = rgb.chunks_exact_mut(width * 3);
        for (y, (row, out)) in rows.zip(lines).enumerate() {
            let mut changed = false;
            while let Some(change) = changes.next_if(|change| change.line <= y / scale) {
                changed = true;
                for (slot, colors) in self.slots.iter().zip(table.as_chunks_mut::<16>().0) {
                    if slot.cram == Some(change.cram_line) {
                        *colors = change.colors;
                    }
                }
            }
            if changed {
                for (tint, color) in self.tints.iter().zip(&mut tints) {
                    *color = resolve_tint(tint, &table);
                }
            }
            for (&entry, out) in row.iter().zip(out.as_chunks_mut::<3>().0) {
                *out = resolve_entry(entry, &table, &self.blends, &tints);
            }
        }
        rgb
    }

    /// Replace every pixel by its resolved colour, with `map` applied.
    fn bake(&mut self, map: impl Fn(u8) -> u8) {
        let hd_rgb = self.hd.as_ref().map(|_| self.presentation_rgb().to_vec());
        let rgb = self.rgb.take().unwrap_or_else(|| self.resolve());
        for (entry, color) in self.pixels.iter_mut().zip(rgb.as_chunks::<3>().0) {
            *entry = direct(color.map(&map));
        }
        if let (Some(hd), Some(rgb)) = (&mut self.hd, hd_rgb) {
            hd.rgb.take();
            for (entry, color) in hd.pixels.iter_mut().zip(rgb.as_chunks::<3>().0) {
                *entry = direct(color.map(&map));
            }
        }
        self.slots.clear();
        self.blends.clear();
        self.reset_art_cache();
    }

    pub fn clear(&mut self, color: Color) {
        self.invalidate();
        self.pixels.fill(direct(color));
        self.slots.clear();
        self.blends.clear();
        self.changes.clear();
        self.high.fill(false);
        self.sat.clear();
        if self.art_cache_dirty || self.art_cache_bytes + self.tints.len() * 48 > ART_CACHE_BYTES {
            self.reset_art_cache();
        }
        if let Some(hd) = &mut self.hd {
            hd.pixels.fill(direct(color));
            hd.high.fill(false);
        }
    }

    /// [`Frame::clear`] to the VDP backdrop: colour `index` of `palette`,
    /// the CRAM entry register `$07` selects. As a CRAM index it follows
    /// the frame's CRAM changes and fades.
    pub fn clear_backdrop(&mut self, palette: PaletteRef<'_>, index: u8) {
        self.clear([0; 3]);
        let slot = self.slot(palette);
        self.pixels.fill(slot.0 << 4 | u32::from(index & 15));
        if let Some(hd) = &mut self.hd {
            hd.pixels.fill(slot.0 << 4 | u32::from(index & 15));
        }
    }

    /// VDP priority for layers painted in order: while `on`, what is drawn
    /// is a high-priority sprite, and a later `behind_high_sprites` pass
    /// (the high-priority plane cells) leaves its pixels alone.
    pub fn mark_high_sprites(&mut self, on: bool) {
        self.marking = on;
    }

    /// While `on`, writes skip pixels of high-priority sprites.
    pub fn behind_high_sprites(&mut self, on: bool) {
        self.behind = on;
    }

    /// Drop writes to lines `end` and below until reset with `None` (the
    /// 4-1 H-interrupt ends the sprite list after line 200).
    pub fn clip_lines(&mut self, end: Option<usize>) {
        self.end = end.map_or(self.width * HEIGHT, |line| line.min(HEIGHT) * self.width);
    }

    /// A CRAM line write during this frame: from `change.line` down, the
    /// pixels drawn with that CRAM line ([`PaletteRef::Cram`]), before or
    /// after this call, show `change.colors` at their colour index.
    /// `clear` starts the next frame with none.
    pub fn change_cram_line(&mut self, change: CramChange) {
        self.invalidate();
        let at = self.changes.partition_point(|c| c.line <= change.line);
        self.changes.insert(at, change);
    }

    /// The frame slot of `palette`, for drawing its colour indices with
    /// [`Frame::index_pixel`].
    pub fn slot(&mut self, palette: PaletteRef<'_>) -> Slot {
        let (colors, cram) = match palette {
            PaletteRef::Cram(line, colors) => (colors, Some(line)),
            PaletteRef::Fixed(colors) => (colors, None),
        };
        let found = self
            .slots
            .iter()
            .rposition(|slot| slot.cram == cram && slot.colors == *colors);
        if let Some(at) = found {
            return Slot(at as u32);
        }
        if self.slots.len() >= SLOT_LIMIT {
            self.bake(|channel| channel);
        }
        self.slots.push(SlotPalette {
            colors: *colors,
            cram,
        });
        Slot(self.slots.len() as u32 - 1)
    }

    fn put_native(&mut self, index: usize, entry: u32) {
        if index >= self.end || (self.behind && self.high[index]) {
            return;
        }
        self.pixels[index] = entry;
        // Sprites paint back to front, so the pixel keeps the priority of
        // the frontmost sprite: the VDP shows the first opaque sprite pixel
        // in link order with that sprite's priority bit, so a low sprite in
        // front of a high one falls behind the high plane cells with it.
        self.high[index] = self.marking;
    }

    fn put(&mut self, index: usize, entry: u32) {
        self.put_native(index, entry);
        if let Some(hd) = &mut self.hd {
            if index >= self.end {
                return;
            }
            let width = self.width * hd.scale;
            let (x, y) = (index % self.width * hd.scale, index / self.width * hd.scale);
            for dy in 0..hd.scale {
                if !self.behind {
                    let at = (y + dy) * width + x;
                    hd.pixels[at..at + hd.scale].fill(entry);
                    hd.high[at..at + hd.scale].fill(self.marking);
                    continue;
                }
                for dx in 0..hd.scale {
                    let at = (y + dy) * width + x + dx;
                    if !self.behind || !hd.high[at] {
                        hd.pixels[at] = entry;
                        hd.high[at] = self.marking;
                    }
                }
            }
        }
    }

    pub fn pixel(&mut self, x: i32, y: i32, color: Color) {
        let x = x.saturating_add(self.origin_x);
        if (0..self.width as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            self.invalidate();
            self.put(y as usize * self.width + x as usize, direct(color));
        }
    }

    /// Colour `index` of the `slot` palette (0 is drawn, not transparent).
    pub fn index_pixel(&mut self, x: i32, y: i32, slot: Slot, index: u8) {
        let x = x.saturating_add(self.origin_x);
        if (0..self.width as i32).contains(&x) && (0..HEIGHT as i32).contains(&y) {
            self.invalidate();
            self.put(
                y as usize * self.width + x as usize,
                slot.0 << 4 | u32::from(index & 15),
            );
        }
    }

    /// Bilinear plane sampling. Transparent taps retain the underlying
    /// pixel; palette indices remain live through raster writes and fades.
    pub(crate) fn filtered_pixel(
        &mut self,
        x: i32,
        y: i32,
        samples: [Option<(Slot, u8)>; 4],
        weights: [f32; 4],
    ) {
        let x = x.saturating_add(self.origin_x);
        if !(0..self.width as i32).contains(&x) || !(0..HEIGHT as i32).contains(&y) {
            return;
        }
        let at = y as usize * self.width + x as usize;
        if at >= self.end
            || (self.behind && self.high[at])
            || !samples
                .iter()
                .zip(weights)
                .any(|(sample, weight)| sample.is_some() && weight > 0.0)
        {
            return;
        }
        let entries = samples.map(|sample| {
            sample.map_or(self.pixels[at], |(slot, index)| {
                slot.0 << 4 | u32::from(index & 15)
            })
        });
        self.invalidate();
        let entry = BLEND | self.blends.len() as u32;
        self.blends.push(PixelBlend { entries, weights });
        self.put(at, entry);
    }

    /// A native horizontal wave span. The nine samples cover one tile row
    /// and the first pixel of its neighbour; transparent taps keep the pixel
    /// below. Coordinates are clipped once for the whole run.
    pub(crate) fn filtered_horizontal_span(
        &mut self,
        screen: (i32, i32),
        offset: f32,
        source_x: i32,
        samples: &[Option<(Slot, u8)>; 9],
        run: usize,
    ) {
        debug_assert!(self.hd.is_none());
        let x = screen.0 + self.origin_x;
        let y = screen.1;
        if !(0..HEIGHT as i32).contains(&y) {
            return;
        }
        let left = x.saturating_neg().clamp(0, run as i32) as usize;
        let end_x = self
            .end
            .saturating_sub(y as usize * self.width)
            .min(self.width);
        let right = run.min(end_x.saturating_sub(x.max(0) as usize) + left);
        if left >= right || x >= self.width as i32 {
            return;
        }
        self.invalidate();
        let output = y as usize * self.width + (x + left as i32) as usize;
        for dx in left..right {
            let at = output + dx - left;
            if self.behind && self.high[at] {
                continue;
            }
            // Keep the scalar sampler's addition order: rounding can change
            // the fractional part as a span crosses a power of two.
            let wx = (screen.0 + dx as i32) as f32 + offset;
            let integer = wx as i32;
            let integer = integer - i32::from(wx < integer as f32);
            let fraction = wx - integer as f32;
            let px = (integer - source_x) as usize;
            let a = samples[px];
            let b = if fraction != 0.0 {
                samples[px + 1]
            } else {
                None
            };
            if a.is_none() && b.is_none() {
                continue;
            }
            let under = self.pixels[at];
            let encode = |sample: Option<(Slot, u8)>| {
                sample.map_or(under, |(slot, index)| slot.0 << 4 | u32::from(index))
            };
            let a = encode(a);
            let b = encode(b);
            let entry = if fraction == 0.0 || a == b {
                a
            } else {
                let entry = BLEND | self.blends.len() as u32;
                self.blends.push(PixelBlend {
                    entries: [a, b, 0, 0],
                    weights: [1.0 - fraction, fraction, 0.0, 0.0],
                });
                entry
            };
            self.put_native(at, entry);
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: Color) {
        self.invalidate();
        let x = x.saturating_add(self.origin_x);
        let left = x.clamp(0, self.width as i32) as usize;
        let right = (x.saturating_add(w).clamp(0, self.width as i32) as usize).max(left);
        for py in y.max(0)..y.saturating_add(h).min(HEIGHT as i32) {
            let row = py as usize * self.width;
            self.pixels[row + left..row + right].fill(direct(color));
            if let Some(hd) = &mut self.hd {
                let width = self.width * hd.scale;
                for dy in 0..hd.scale {
                    let row = (py as usize * hd.scale + dy) * width;
                    hd.pixels[row + left * hd.scale..row + right * hd.scale].fill(direct(color));
                }
            }
        }
    }

    /// Palette entry zero is transparent; callers establish layer priority
    /// through draw order. The tile is clipped to the frame once.
    pub fn tile<'a>(
        &mut self,
        tile: &Tile,
        palette: impl Into<PaletteRef<'a>>,
        x: i32,
        y: i32,
        flip: Flip,
    ) {
        let slot = self.slot(palette.into());
        self.tile_slot(tile, slot, x, y, flip);
    }

    /// [`Frame::tile`] with an interned palette.
    pub fn tile_slot(&mut self, tile: &Tile, slot: Slot, x: i32, y: i32, flip: Flip) {
        self.tile_rows(tile, slot, x, y, flip, u8::MAX);
    }

    /// [`Frame::tile_slot`] on the screen rows `dy` whose bit is set in
    /// `rows` (explicit masks hide the others).
    fn tile_rows(&mut self, tile: &Tile, slot: Slot, x: i32, y: i32, flip: Flip, rows: u8) {
        self.tile_rows_with_art(tile, slot, x, y, flip, rows, None);
    }

    #[allow(clippy::too_many_arguments)]
    fn tile_rows_with_art(
        &mut self,
        tile: &Tile,
        slot: Slot,
        x: i32,
        y: i32,
        flip: Flip,
        rows: u8,
        art: Option<Rc<Replacement>>,
    ) {
        let replacement = art.or_else(|| self.replacement(tile, slot));
        if let Some(art) = &replacement {
            self.art_tile_rows(art, slot, x, y, flip, rows);
        }
        let x = x.saturating_add(self.origin_x);
        let columns =
            x.saturating_neg().clamp(0, 8)..(self.width as i32).saturating_sub(x).clamp(0, 8);
        let lines = y.saturating_neg().clamp(0, 8)..(HEIGHT as i32).saturating_sub(y).clamp(0, 8);
        if columns.is_empty() || lines.is_empty() {
            return;
        }
        self.invalidate();
        let base = slot.0 << 4;
        for dy in lines.filter(|dy| rows & (1 << dy) != 0) {
            let sy = (if flip.y { 7 - dy } else { dy }) as usize;
            let row = (y + dy) as usize * self.width;
            for dx in columns.clone() {
                let sx = (if flip.x { 7 - dx } else { dx }) as usize;
                let packed = tile.0[sy * 4 + sx / 2];
                let index = if sx.is_multiple_of(2) {
                    packed >> 4
                } else {
                    packed & 15
                };
                if index != 0 {
                    let at = row + (x + dx) as usize;
                    if replacement.is_some() {
                        self.put_native(at, base | u32::from(index));
                    } else {
                        self.put(at, base | u32::from(index));
                    }
                }
            }
        }
    }

    /// Apply a palette fade to everything drawn so far; what is drawn
    /// later is not faded. On the VDP a fade is CRAM contents: pixels keep
    /// their CRAM line and index and take the faded colours of their
    /// palette (a faded copy of each slot), so a [`CramChange`] still
    /// replaces them from its display line down, with its own colours,
    /// whether it is added before or after the fade. Direct colours are
    /// faded in place. `In(FADE_STEPS)` and `Out(0)` leave the frame
    /// untouched.
    pub fn fade(&mut self, fade: Fade) {
        if matches!(fade, Fade::In(FADE_STEPS) | Fade::Out(0)) {
            return;
        }
        let table: [u8; 256] = std::array::from_fn(|value| fade.channel(value as u8));
        let first = self.slots.len();
        if first * 2 > SLOT_LIMIT {
            self.bake(|channel| table[usize::from(channel)]);
            return;
        }
        self.invalidate();
        self.art_entries.clear();
        self.tint_entries.clear();
        self.art_cache_dirty = true;
        for slot in 0..first {
            let SlotPalette { colors, cram } = self.slots[slot];
            self.slots.push(SlotPalette {
                colors: colors.map(|color| color.map(|channel| table[usize::from(channel)])),
                cram,
            });
        }
        let shift = (first as u32) << 4;
        let faded = |entry: u32| {
            if entry & DIRECT != 0 {
                let [_, r, g, b] = entry.to_be_bytes();
                direct([r, g, b].map(|channel| table[usize::from(channel)]))
            } else if entry & (BLEND | TINT) != 0 {
                entry
            } else {
                entry + shift
            }
        };
        for tint in &mut self.tints {
            tint.entry = faded(tint.entry);
        }
        for blend in &mut self.blends {
            blend.entries = blend.entries.map(faded);
        }
        for entry in &mut self.pixels {
            *entry = faded(*entry);
        }
        if let Some(hd) = &mut self.hd {
            for entry in &mut hd.pixels {
                *entry = faded(*entry);
            }
        }
    }

    /// Portable framebuffer capture, usable by both desktop and headless runs.
    pub fn write_ppm(&self, mut output: impl Write) -> io::Result<()> {
        write!(
            output,
            "P6\n{} {}\n255\n",
            self.output_width(),
            self.output_height()
        )?;
        output.write_all(self.presentation_rgb())
    }

    pub fn checksum(&self) -> u64 {
        self.rgb().iter().fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    }
}

/// `$6E8`: move each of the three color nibbles one step toward `target`.
pub(crate) fn fade_step(current: u16, target: u16) -> u16 {
    let mut out = 0;
    for mask in [0xf00u16, 0xf0, 0xf] {
        let unit = mask.isolate_lowest_one();
        let (c, t) = (current & mask, target & mask);
        out |= match c.cmp(&t) {
            std::cmp::Ordering::Less => c + unit,
            std::cmp::Ordering::Greater => c - unit,
            std::cmp::Ordering::Equal => c,
        };
    }
    out
}

#[cfg(test)]
mod tests;
