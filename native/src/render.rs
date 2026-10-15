use crate::art_pack::{ArtPack, Key, Replacement};
use crate::{HEIGHT, WIDTH};
use std::{
    cell::OnceCell,
    collections::HashMap,
    io::{self, Write},
    rc::Rc,
};

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

struct SampledArt {
    key: Key,
    slot: u32,
    entries: Option<ArtEntries>,
    scale: usize,
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
    sampled_art: Option<SampledArt>,
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
            sampled_art: None,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// Keep simulation coordinates native while resolving actual HD texels.
    pub fn set_art_pack(&mut self, pack: Option<Rc<ArtPack>>) {
        self.reset_art_cache();
        self.art_pack = pack;
        self.hd = self.art_pack.as_ref().map(|pack| {
            let size = self.width * HEIGHT * pack.scale * pack.scale;
            Hd {
                scale: pack.scale,
                pixels: vec![direct([0; 3]); size],
                high: vec![false; size],
                rgb: OnceCell::new(),
            }
        });
        self.clear([0; 3]);
    }

    /// Sample HD sources at the displayed integer scale. Captures default to
    /// the pack's full resolution unless the desktop sets a smaller viewport.
    pub fn set_output_scale(&mut self, scale: usize) -> bool {
        let Some(hd) = &mut self.hd else {
            return false;
        };
        let scale = scale.clamp(1, self.art_pack.as_ref().unwrap().scale);
        if hd.scale == scale {
            return false;
        }
        hd.scale = scale;
        hd.pixels
            .resize(self.width * HEIGHT * scale * scale, direct([0; 3]));
        hd.high.resize(hd.pixels.len(), false);
        self.clear([0; 3]);
        true
    }

    fn reset_art_cache(&mut self) {
        self.tints.clear();
        self.tint_entries.clear();
        self.art_entries.clear();
        self.art_cache_bytes = 0;
        self.art_cache_dirty = false;
        self.sampled_art = None;
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
        self.sampled_art = None;
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

    fn replacement(&self, tile: &Tile, slot: Slot) -> Option<Rc<Replacement>> {
        self.art_pack
            .as_ref()?
            .replacement(tile, self.slots[slot.0 as usize].cram)
    }

    fn replacement_entries(&mut self, slot: Slot, art: &Replacement, scale: usize) -> ArtEntries {
        // ArtPack owns each immutable replacement for its whole lifetime;
        // switching packs clears this cache before any address can be reused.
        let key = (art as *const Replacement as usize, slot.0, scale);
        if let Some(entries) = self.art_entries.get(&key) {
            return entries.clone();
        }
        let mut colors = HashMap::new();
        let side = 8 * scale;
        let pixels: Vec<_> = (0..side * side)
            .map(|at| {
                let x = at % side * art.scale / scale;
                let y = at / side * art.scale / scale;
                let pixel = art.pixels[y * 8 * art.scale + x];
                let entry = *colors.entry(pixel).or_insert_with(|| {
                    if pixel[3] == 0 {
                        return 0;
                    }
                    let color = [pixel[0], pixel[1], pixel[2]];
                    let nearest = (1..16)
                        .filter(|&i| art.palette[i] != [0; 3])
                        .min_by_key(|&i| {
                            color
                                .iter()
                                .zip(art.palette[i])
                                .map(|(&a, b)| (i32::from(a) - i32::from(b)).pow(2))
                                .sum::<i32>()
                        })
                        .unwrap_or(1);
                    let entry = slot.0 << 4 | nearest as u32;
                    if color == art.palette[nearest] {
                        return entry;
                    }
                    let tint = PixelTint {
                        color,
                        reference: art.palette[nearest],
                        entry,
                    };
                    *self.tint_entries.entry(tint).or_insert_with(|| {
                        let entry = TINT | self.tints.len() as u32;
                        self.tints.push(tint);
                        entry
                    })
                });
                (entry, pixel[3])
            })
            .collect();
        let row_alpha = pixels
            .chunks_exact(side)
            .map(|row| {
                if row.iter().all(|p| p.1 == 255) {
                    255
                } else if row.iter().all(|p| p.1 == 0) {
                    0
                } else {
                    128
                }
            })
            .collect();
        let values = pixels.iter().map(|p| p.0).collect();
        let entries = Rc::new(EncodedArt {
            scale,
            pixels,
            values,
            row_alpha,
        });
        self.art_entries.insert(key, entries.clone());
        self.art_cache_bytes +=
            entries.len() * (std::mem::size_of::<(u32, u8)>() + 4) + entries.row_alpha.len();
        entries
    }

    #[allow(clippy::too_many_arguments)]
    fn art_tile_rows(
        &mut self,
        art: &Replacement,
        slot: Slot,
        x: i32,
        y: i32,
        flip: Flip,
        rows: u8,
    ) {
        let scale = self.output_scale();
        let entries = self.replacement_entries(slot, art, scale);
        let side = 8 * scale;
        self.invalidate();
        let x = x.saturating_add(self.origin_x);
        for dy in 0..side {
            let ly = dy / scale;
            let py = y.saturating_mul(scale as i32).saturating_add(dy as i32);
            if rows & (1 << ly) == 0 || !(0..self.output_height() as i32).contains(&py) {
                continue;
            }
            for dx in 0..side {
                let px = x.saturating_mul(scale as i32).saturating_add(dx as i32);
                if !(0..self.output_width() as i32).contains(&px) {
                    continue;
                }
                let sx = if flip.x { side - 1 - dx } else { dx };
                let sy = if flip.y { side - 1 - dy } else { dy };
                let (entry, alpha) = entries[sy * 8 * entries.scale + sx];
                self.put_hd(px as usize, py as usize, entry, alpha);
            }
        }
    }

    fn put_hd(&mut self, x: usize, y: usize, entry: u32, alpha: u8) {
        let Some(hd) = &self.hd else {
            return;
        };
        if y / hd.scale * self.width + x / hd.scale >= self.end {
            return;
        }
        let at = y * self.width * hd.scale + x;
        self.put_hd_at(at, entry, alpha);
    }

    /// A texel in a run already clipped to the native sprite line limit.
    #[inline(always)]
    fn put_hd_at(&mut self, at: usize, entry: u32, alpha: u8) {
        if alpha == 0 {
            return;
        }
        let hd = self.hd.as_mut().unwrap();
        if self.behind && hd.high[at] {
            return;
        }
        let entry = if alpha == 255 {
            entry
        } else {
            let blend = BLEND | self.blends.len() as u32;
            let weight = f32::from(alpha) / 255.0;
            self.blends.push(PixelBlend {
                entries: [entry, hd.pixels[at], 0, 0],
                weights: [weight, 1.0 - weight, 0.0, 0.0],
            });
            blend
        };
        hd.pixels[at] = entry;
        hd.high[at] = self.marking;
    }

    fn copy_hd_opaque(&mut self, output: usize, entries: &[u32]) {
        let hd = self.hd.as_mut().unwrap();
        let end = output + entries.len();
        if !self.behind || !hd.high[output..end].iter().any(|&v| v) {
            hd.pixels[output..end].copy_from_slice(entries);
            hd.high[output..end].fill(self.marking);
        } else {
            for (dx, &entry) in entries.iter().enumerate() {
                if !hd.high[output + dx] {
                    hd.pixels[output + dx] = entry;
                    hd.high[output + dx] = self.marking;
                }
            }
        }
    }

    /// One native scanline run; unchanged art expands with the ordinary
    /// indexed write, while replacement art samples each HD texel.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hd_tile_run(
        &mut self,
        tile: &Tile,
        slot: Slot,
        screen: (i32, i32),
        first: usize,
        row: usize,
        run: usize,
        flip: Flip,
    ) {
        let art = self.replacement(tile, slot);
        let entries = art
            .as_ref()
            .map(|art| self.replacement_entries(slot, art, art.scale));
        let scale = self.output_scale();
        self.invalidate();
        let x = screen.0.saturating_add(self.origin_x);
        let y = screen.1;
        if !(0..HEIGHT as i32).contains(&y) {
            return;
        }
        let left = x.saturating_neg().clamp(0, run as i32) as usize;
        let right = run.min((self.width as i32 - x).max(0) as usize).min(
            self.end
                .saturating_sub(y as usize * self.width)
                .saturating_sub(x.max(0) as usize)
                + left,
        );
        if left >= right {
            return;
        }
        for dx in left..right {
            let px = x + dx as i32;
            if !(0..self.width as i32).contains(&px) {
                continue;
            }
            let tx = if flip.x { 7 - first - dx } else { first + dx };
            let ty = if flip.y { 7 - row } else { row };
            let packed = tile.0[ty * 4 + tx / 2];
            let index = if tx.is_multiple_of(2) {
                packed >> 4
            } else {
                packed & 15
            };
            let at = y as usize * self.width + px as usize;
            if art.is_some() {
                if index != 0 {
                    self.put_native(at, slot.0 << 4 | u32::from(index));
                }
            } else if index != 0 {
                self.put(at, slot.0 << 4 | u32::from(index));
            }
        }
        if let (Some(art), Some(entries)) = (&art, &entries) {
            let output_width = self.width * scale;
            let source_side = art.scale * 8;
            let first_x = (first + left) * scale;
            let count = (right - left) * scale;
            for sy in 0..scale {
                let ty = row * scale + sy;
                let ty = if flip.y { 8 * scale - 1 - ty } else { ty };
                let source_row = ty * art.scale / scale * source_side;
                let output =
                    (y as usize * scale + sy) * output_width + (x + left as i32) as usize * scale;
                if entries.row_alpha[source_row / source_side] == 0 {
                    continue;
                }
                if art.scale == scale {
                    if !flip.x && entries.row_alpha[source_row / source_side] == 255 {
                        self.copy_hd_opaque(
                            output,
                            &entries.values[source_row + first_x..source_row + first_x + count],
                        );
                        continue;
                    }
                    for dx in 0..count {
                        let tx = first_x + dx;
                        let tx = if flip.x { source_side - 1 - tx } else { tx };
                        let (entry, alpha) = entries[source_row + tx];
                        self.put_hd_at(output + dx, entry, alpha);
                    }
                } else {
                    for dx in 0..count {
                        let tx = first_x + dx;
                        let tx = if flip.x { 8 * scale - 1 - tx } else { tx };
                        let (entry, alpha) = entries[source_row + tx * art.scale / scale];
                        self.put_hd_at(output + dx, entry, alpha);
                    }
                }
            }
        }
    }

    /// A scrolled plane sample at a physical output pixel. UV stays in
    /// original 8x8 tile units, so fractional wave motion keeps HD detail.
    pub(crate) fn hd_tile_sample(
        &mut self,
        tile: &Tile,
        slot: Slot,
        screen: (i32, i32),
        uv: (f32, f32),
        flip: Flip,
    ) {
        let scale = self.output_scale();
        let x = screen.0.saturating_add(self.origin_x * scale as i32);
        let y = screen.1;
        if !(0..self.output_width() as i32).contains(&x)
            || !(0..self.output_height() as i32).contains(&y)
        {
            return;
        }
        self.invalidate();
        let key = Key(tile.0, self.slots[slot.0 as usize].cram.unwrap());
        if self
            .sampled_art
            .as_ref()
            .is_none_or(|cached| cached.key != key || cached.slot != slot.0)
        {
            let art = self.replacement(tile, slot);
            let scale = art.as_ref().map_or(1, |a| a.scale);
            let entries = art.map(|a| self.replacement_entries(slot, &a, a.scale));
            self.sampled_art = Some(SampledArt {
                key,
                slot: slot.0,
                entries,
                scale,
            });
        }
        let art = self.sampled_art.as_ref().unwrap();
        if let Some(entries) = &art.entries {
            let side = 8 * art.scale;
            let px = (uv.0 * art.scale as f32)
                .floor()
                .clamp(0.0, (side - 1) as f32) as usize;
            let py = (uv.1 * art.scale as f32)
                .floor()
                .clamp(0.0, (side - 1) as f32) as usize;
            let px = if flip.x { side - 1 - px } else { px };
            let py = if flip.y { side - 1 - py } else { py };
            let (entry, alpha) = entries[py * side + px];
            self.put_hd(x as usize, y as usize, entry, alpha);
        } else {
            let px = uv.0.floor().clamp(0.0, 7.0) as usize;
            let py = uv.1.floor().clamp(0.0, 7.0) as usize;
            let px = if flip.x { 7 - px } else { px };
            let py = if flip.y { 7 - py } else { py };
            let byte = tile.0[py * 4 + px / 2];
            let index = if px.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 15
            };
            if index != 0 {
                self.put_hd(x as usize, y as usize, slot.0 << 4 | u32::from(index), 255);
            }
        }
        // Native-resolution tooling remains usable with packs installed.
        if screen.0.rem_euclid(scale as i32) == 0 && screen.1.rem_euclid(scale as i32) == 0 {
            let px = uv.0.floor().clamp(0.0, 7.0) as usize;
            let py = uv.1.floor().clamp(0.0, 7.0) as usize;
            let px = if flip.x { 7 - px } else { px };
            let py = if flip.y { 7 - py } else { py };
            let byte = tile.0[py * 4 + px / 2];
            let index = if px.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 15
            };
            if index != 0 {
                self.put_native(
                    y as usize / scale * self.width + x as usize / scale,
                    slot.0 << 4 | u32::from(index),
                );
            }
        }
    }

    /// One HD scanline span with a constant tile and vertical scroll. The
    /// plane splits spans at tile and VSRAM column boundaries before calling.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hd_tile_span(
        &mut self,
        tile: &Tile,
        slot: Slot,
        screen: (i32, i32),
        source: (f32, f32),
        run: usize,
        flip: Flip,
    ) {
        let scale = self.output_scale();
        let width = self.output_width();
        let (x, y) = (screen.0 + self.origin_x * scale as i32, screen.1);
        if !(0..self.output_height() as i32).contains(&y) {
            return;
        }
        let left = x.saturating_neg().clamp(0, run as i32) as usize;
        let native_row = y as usize / scale * self.width;
        let end_x = self.end.saturating_sub(native_row).min(self.width) * scale;
        let right = run.min(end_x.saturating_sub(x.max(0) as usize) + left);
        if left >= right || x >= width as i32 {
            return;
        }
        self.invalidate();
        let art = self.replacement(tile, slot);
        let entries = art
            .as_ref()
            .map(|art| self.replacement_entries(slot, art, art.scale));
        let base = slot.0 << 4;
        let native_y = source.1.floor().clamp(0.0, 7.0) as usize;
        let native_y = if flip.y { 7 - native_y } else { native_y };
        let output = y as usize * width + (x + left as i32) as usize;
        let side = art.as_ref().map_or(8, |art| art.scale * 8);
        let source_scale = art.as_ref().map_or(1, |art| art.scale);
        let source_y = (source.1 * source_scale as f32)
            .floor()
            .clamp(0.0, (side - 1) as f32) as usize;
        let source_y = if flip.y {
            side - 1 - source_y
        } else {
            source_y
        };
        for dx in left..right {
            let physical_x = screen.0 + dx as i32;
            let u = (physical_x as f32 / scale as f32 + source.0).rem_euclid(8.0);
            let tx = u.floor().clamp(0.0, 7.0) as usize;
            let tx = if flip.x { 7 - tx } else { tx };
            let byte = tile.0[native_y * 4 + tx / 2];
            let index = if tx.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 15
            };
            let (entry, alpha) = if let Some(entries) = &entries {
                let sx = (u * source_scale as f32)
                    .floor()
                    .clamp(0.0, (side - 1) as f32) as usize;
                let sx = if flip.x { side - 1 - sx } else { sx };
                entries[source_y * side + sx]
            } else {
                (base | u32::from(index), if index == 0 { 0 } else { 255 })
            };
            self.put_hd_at(output + dx - left, entry, alpha);
            if physical_x.rem_euclid(scale as i32) == 0
                && y.rem_euclid(scale as i32) == 0
                && index != 0
            {
                self.put_native(
                    native_row + (x + dx as i32) as usize / scale,
                    base | u32::from(index),
                );
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
        self.sampled_art = None;
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
mod tests {
    use super::*;

    #[test]
    fn filtered_wave_pixels_preserve_transparency_priority_cram_and_fades() {
        let mut frame = Frame::default();
        frame.clear([0, 0, 255]);
        let mut palette = [[0; 3]; 16];
        palette[1] = [255, 0, 0];
        let slot = frame.slot(PaletteRef::Cram(0, &palette));
        let taps = [Some((slot, 1)), None, None, None];
        let weights = [0.5, 0.5, 0.0, 0.0];
        frame.filtered_pixel(0, 30, taps, weights);
        assert_eq!(pixel(&frame, 0, 30), [128, 0, 128]);
        palette[1] = [0, 255, 0];
        frame.change_cram_line(CramChange {
            line: 30,
            cram_line: 0,
            colors: palette,
        });
        assert_eq!(pixel(&frame, 0, 30), [0, 128, 128]);
        frame.fade(Fade::Out(FADE_STEPS));
        assert_eq!(pixel(&frame, 0, 30), [0, 128, 0]);
        frame.mark_high_sprites(true);
        frame.pixel(1, 30, [255; 3]);
        frame.mark_high_sprites(false);
        frame.behind_high_sprites(true);
        frame.filtered_pixel(1, 30, taps, weights);
        assert_eq!(pixel(&frame, 1, 30), [255; 3]);
        frame.clear([0; 3]);
        assert_eq!(pixel(&frame, 0, 30), [0; 3]);
    }

    #[test]
    fn widescreen_pixels_tiles_and_raster_changes_cover_both_sides() {
        for width in [crate::WIDE_WIDTH, 535, 573, 796, 853] {
            let mut frame = Frame::new(width);
            let mut palette = [[0; 3]; 16];
            palette[1] = [255, 0, 0];
            let tile = Tile([0x11; 32]);
            let left = frame.view_left();
            let right = frame.view_right();
            frame.clear_backdrop(PaletteRef::Cram(0, &palette), 1);
            frame.tile(
                &tile,
                PaletteRef::Cram(0, &palette),
                left - 4,
                30,
                Flip::default(),
            );
            frame.tile(
                &tile,
                PaletteRef::Cram(0, &palette),
                right - 4,
                30,
                Flip::default(),
            );
            palette[1] = [0, 255, 0];
            frame.change_cram_line(CramChange {
                line: 32,
                cram_line: 0,
                colors: palette,
            });
            frame.clip_lines(Some(34));
            frame.pixel(left, 33, [0, 0, 255]);
            frame.pixel(right - 1, 34, [0, 0, 255]);
            let pixel = |x, y| {
                let at = (y * frame.width() + x) * 3;
                &frame.rgb()[at..at + 3]
            };
            for x in [0, frame.width() - 1] {
                assert_eq!(pixel(x, 31), [255, 0, 0]);
                assert_eq!(pixel(x, 32), [0, 255, 0]);
                assert_eq!(pixel(x, 34), [0, 255, 0]);
            }
            assert_eq!(pixel(0, 33), [0, 0, 255]);
            let mut ppm = Vec::new();
            frame.write_ppm(&mut ppm).unwrap();
            assert!(ppm.starts_with(format!("P6\n{width} 224\n255\n").as_bytes()));
            assert_eq!(frame.rgb().len(), width * HEIGHT * 3);
        }
    }

    #[test]
    fn widescreen_view_stays_inside_layout_and_centres_ui() {
        let mut frame = Frame::new(crate::WIDE_WIDTH);
        frame.align_world(0, 4096);
        assert_eq!((frame.view_left(), frame.view_right()), (0, 398));
        frame.align_world(1024, 4096);
        assert_eq!((frame.view_left(), frame.view_right()), (-39, 359));
        frame.align_world(3776, 4096);
        assert_eq!((frame.view_left(), frame.view_right()), (-78, 320));
        assert_eq!(frame.world_camera_delta(), -39);
        frame.align_world(0, 320);
        assert_eq!((frame.view_left(), frame.view_right()), (-39, 359));
        frame.align_world(16, 256);
        assert_eq!((frame.view_left(), frame.view_right()), (-87, 311));
        frame.center_content();
        assert_eq!((frame.view_left(), frame.view_right()), (-39, 359));
        let mut original = Frame::default();
        original.align_world(0, 4096);
        assert_eq!((original.view_left(), original.view_right()), (0, 320));
    }

    fn pixel(frame: &Frame, x: usize, y: usize) -> &[u8] {
        let start = (y * WIDTH + x) * 3;
        &frame.rgb()[start..start + 3]
    }

    #[test]
    fn packed_tile_transparency_flip_and_clipping() {
        let mut tile = Tile([0; 32]);
        tile.0[0] = 0x12;
        let mut palette = [[0; 3]; 16];
        palette[1] = [255, 0, 0];
        palette[2] = [0, 255, 0];
        let mut frame = Frame::default();
        frame.clear([1, 2, 3]);
        frame.tile(&tile, &palette, -1, 0, Flip::default());
        assert_eq!(pixel(&frame, 0, 0), palette[2]);
        assert_eq!(pixel(&frame, 1, 0), [1, 2, 3]);
        frame.tile(&tile, &palette, 0, 0, Flip { x: true, y: true });
        assert_eq!(pixel(&frame, 7, 7), palette[1]);
        assert_eq!(pixel(&frame, 6, 7), palette[2]);
        // The bottom-right corner clips the rest of the tile.
        frame.tile(
            &tile,
            &palette,
            WIDTH as i32 - 1,
            HEIGHT as i32 - 1,
            Flip::default(),
        );
        assert_eq!(pixel(&frame, WIDTH - 1, HEIGHT - 1), palette[1]);
        // Fully off-screen tiles are safe and do not wrap to another row.
        let before = frame.checksum();
        frame.tile(
            &tile,
            &palette,
            WIDTH as i32,
            HEIGHT as i32,
            Flip::default(),
        );
        assert_eq!(frame.checksum(), before);
    }

    #[test]
    fn high_plane_passes_cover_low_sprites_but_not_high_ones() {
        let mut tile = Tile([0; 32]);
        tile.0.fill(0x11);
        let mut red = [[0; 3]; 16];
        red[1] = [255, 0, 0];
        let mut blue = red;
        blue[1] = [0, 0, 255];
        let mut frame = Frame::default();
        frame.clear([1, 2, 3]);
        frame.tile(&tile, &red, 0, 0, Flip::default()); // a low sprite
        frame.mark_high_sprites(true);
        frame.tile(&tile, &red, 8, 0, Flip::default()); // a high sprite
        frame.mark_high_sprites(false);
        frame.behind_high_sprites(true);
        for x in [0, 8] {
            frame.tile(&tile, &blue, x, 0, Flip::default()); // high plane cells
        }
        frame.behind_high_sprites(false);
        assert_eq!(pixel(&frame, 0, 0), [0, 0, 255]);
        assert_eq!(pixel(&frame, 8, 0), [255, 0, 0]);
        // A low sprite in front of a high one takes its pixels behind the
        // plane cells too.
        frame.mark_high_sprites(true);
        frame.tile(&tile, &red, 16, 0, Flip::default());
        frame.mark_high_sprites(false);
        frame.tile(&tile, &red, 16, 0, Flip::default());
        frame.behind_high_sprites(true);
        frame.tile(&tile, &blue, 16, 0, Flip::default());
        frame.behind_high_sprites(false);
        assert_eq!(pixel(&frame, 16, 0), [0, 0, 255]);
        // A new frame forgets the sprites.
        frame.clear([1, 2, 3]);
        frame.behind_high_sprites(true);
        frame.tile(&tile, &blue, 8, 0, Flip::default());
        assert_eq!(pixel(&frame, 8, 0), [0, 0, 255]);
    }

    #[test]
    fn clipped_lines_drop_writes_until_reset() {
        let mut tile = Tile([0; 32]);
        tile.0.fill(0x11);
        let mut red = [[0; 3]; 16];
        red[1] = [255, 0, 0];
        let mut frame = Frame::default();
        frame.clear([1, 2, 3]);
        frame.clip_lines(Some(201));
        frame.tile(&tile, &red, 0, 196, Flip::default());
        frame.pixel(20, 201, [9, 9, 9]);
        assert_eq!(pixel(&frame, 0, 200), [255, 0, 0]);
        assert_eq!(pixel(&frame, 0, 201), [1, 2, 3]);
        assert_eq!(pixel(&frame, 20, 201), [1, 2, 3]);
        frame.clip_lines(None);
        frame.pixel(20, 201, [9, 9, 9]);
        assert_eq!(pixel(&frame, 20, 201), [9, 9, 9]);
    }

    #[test]
    fn a_palette_split_recolours_its_line_below_the_split_only() {
        let mut tile = Tile([0; 32]);
        tile.0.fill(0x11);
        let mut normal = [[0; 3]; 16];
        normal[1] = [255, 0, 0];
        let mut other = normal;
        other[1] = [0, 255, 0];
        let mut reloaded = normal;
        reloaded[1] = [0, 0, 255];
        let mut frame = Frame::default();
        frame.clear([1, 2, 3]);
        frame.tile(&tile, PaletteRef::Cram(1, &normal), 0, 196, Flip::default());
        // The change reaches pixels drawn before and after it.
        frame.change_cram_line(CramChange {
            line: 200,
            cram_line: 1,
            colors: reloaded,
        });
        frame.tile(&tile, PaletteRef::Cram(2, &other), 8, 196, Flip::default());
        frame.tile(
            &tile,
            PaletteRef::Cram(1, &normal),
            16,
            196,
            Flip::default(),
        );
        assert_eq!(pixel(&frame, 0, 199), [255, 0, 0]);
        assert_eq!(pixel(&frame, 0, 200), [0, 0, 255]);
        assert_eq!(pixel(&frame, 16, 199), [255, 0, 0]);
        assert_eq!(pixel(&frame, 16, 203), [0, 0, 255]);
        assert_eq!(pixel(&frame, 8, 200), [0, 255, 0]);
        // The next frame starts with the line as the CRAM copy has it.
        frame.clear([1, 2, 3]);
        frame.tile(
            &tile,
            PaletteRef::Cram(1, &normal),
            16,
            196,
            Flip::default(),
        );
        assert_eq!(pixel(&frame, 16, 203), [255, 0, 0]);
    }

    /// The VDP looks each pixel's CRAM index up, not its colour: another
    /// line and a native copy holding exactly the changed line's colours
    /// keep them below the change.
    #[test]
    fn a_cram_change_leaves_lines_and_copies_that_share_its_colours() {
        let mut tile = Tile([0; 32]);
        tile.0.fill(0x12);
        let mut normal = [[0; 3]; 16];
        normal[1] = [255, 0, 0];
        normal[2] = [36, 72, 109];
        let mut reloaded = normal;
        reloaded[1] = [0, 0, 255];
        reloaded[2] = [0, 0, 0];
        let mut frame = Frame::default();
        frame.clear([1, 2, 3]);
        frame.change_cram_line(CramChange {
            line: 200,
            cram_line: 1,
            colors: reloaded,
        });
        let lines = [normal; 4];
        let cram = cram_lines(&lines);
        frame.tile(&tile, cram[1], 0, 196, Flip::default());
        frame.tile(&tile, cram[2], 8, 196, Flip::default());
        frame.tile(&tile, &normal, 16, 196, Flip::default());
        frame.pixel(24, 200, normal[1]);
        frame.rect(25, 200, 1, 1, normal[2]);
        assert_eq!(pixel(&frame, 0, 199), [255, 0, 0]);
        assert_eq!(pixel(&frame, 0, 200), [0, 0, 255]);
        assert_eq!(pixel(&frame, 1, 200), [0, 0, 0]);
        for x in [8, 16] {
            assert_eq!(pixel(&frame, x, 200), [255, 0, 0]);
            assert_eq!(pixel(&frame, x + 1, 203), [36, 72, 109]);
        }
        assert_eq!(pixel(&frame, 24, 200), [255, 0, 0]);
        assert_eq!(pixel(&frame, 25, 200), [36, 72, 109]);
    }

    /// A fade is CRAM contents: a CRAM change replaces the faded colours of
    /// its line from its display line down, added before or after the
    /// fade, and leaves other lines, native copies and direct colours
    /// faded.
    #[test]
    fn a_cram_change_replaces_the_faded_colours_of_its_line() {
        let mut tile = Tile([0; 32]);
        tile.0.fill(0x11);
        let mut normal = [[0; 3]; 16];
        normal[1] = [255, 255, 255];
        let mut reloaded = normal;
        reloaded[1] = [0, 0, 255];
        let change = CramChange {
            line: 200,
            cram_line: 1,
            colors: reloaded,
        };
        for before in [false, true] {
            let mut frame = Frame::default();
            frame.clear([0; 3]);
            if before {
                frame.change_cram_line(change);
            }
            frame.tile(&tile, PaletteRef::Cram(1, &normal), 0, 196, Flip::default());
            frame.tile(&tile, PaletteRef::Cram(2, &normal), 8, 196, Flip::default());
            frame.tile(&tile, &normal, 16, 196, Flip::default());
            frame.pixel(24, 200, [255, 255, 255]);
            frame.fade(Fade::Out(FADE_STEPS));
            if !before {
                frame.change_cram_line(change);
            }
            // Drawn after the fade: not faded, and changed below line 200.
            frame.tile(
                &tile,
                PaletteRef::Cram(1, &normal),
                32,
                196,
                Flip::default(),
            );
            assert_eq!(pixel(&frame, 0, 199), [0, 0, 0]);
            assert_eq!(
                pixel(&frame, 0, 200),
                [0, 0, 255],
                "change before: {before}"
            );
            for x in [8, 16, 24] {
                assert_eq!(pixel(&frame, x, 200), [0, 0, 0]);
            }
            assert_eq!(pixel(&frame, 32, 199), [255, 255, 255]);
            assert_eq!(pixel(&frame, 32, 200), [0, 0, 255]);
        }
        // Part-way through a fade the faded line keeps its CRAM line.
        let mut frame = Frame::default();
        frame.clear([0; 3]);
        frame.tile(&tile, PaletteRef::Cram(1, &normal), 0, 196, Flip::default());
        frame.fade(Fade::Out(4));
        frame.change_cram_line(change);
        let dimmed = Fade::Out(4).channel(255);
        assert_eq!(pixel(&frame, 0, 199), [dimmed; 3]);
        assert_eq!(pixel(&frame, 0, 200), [0, 0, 255]);
    }

    /// The backdrop is the CRAM entry register $07 selects: it follows CRAM
    /// changes and fades like any pixel of that line.
    #[test]
    fn the_backdrop_is_a_cram_entry() {
        let mut lines = [[[0; 3]; 16]; 4];
        lines[0][0] = [255, 0, 0];
        lines[2][0] = [0, 255, 0];
        let cram = cram_lines(&lines);
        let mut frame = Frame::default();
        frame.clear_backdrop(cram[0], 0);
        let mut changed = lines[0];
        changed[0] = [0, 0, 255];
        frame.change_cram_line(CramChange {
            line: 100,
            cram_line: 0,
            colors: changed,
        });
        assert_eq!(pixel(&frame, 5, 99), [255, 0, 0]);
        assert_eq!(pixel(&frame, 5, 100), [0, 0, 255]);
        frame.fade(Fade::Out(FADE_STEPS));
        assert_eq!(pixel(&frame, 5, 99), [0, 0, 0]);
        assert_eq!(pixel(&frame, 5, 100), [0, 0, 255]);
        // Register $07 = $20: line 2, colour 0.
        let mut frame = Frame::default();
        frame.clear_backdrop(cram[2], 0);
        assert_eq!(pixel(&frame, 319, 223), [0, 255, 0]);
        // A new frame drops the backdrop's palette with the rest.
        frame.clear([1, 2, 3]);
        assert_eq!(pixel(&frame, 0, 0), [1, 2, 3]);
    }

    /// A fade keeps the colours the frame shows when it runs; what is drawn
    /// later is not faded. Many distinct palettes in one frame still draw.
    #[test]
    fn fades_reach_what_is_drawn_so_far_and_slots_overflow_safely() {
        let mut tile = Tile([0; 32]);
        tile.0.fill(0x11);
        let mut white = [[0; 3]; 16];
        white[1] = [255, 255, 255];
        let mut frame = Frame::default();
        frame.clear([0; 3]);
        frame.tile(&tile, PaletteRef::Cram(0, &white), 0, 0, Flip::default());
        frame.fade(Fade::Out(FADE_STEPS));
        frame.tile(&tile, PaletteRef::Cram(0, &white), 8, 0, Flip::default());
        assert_eq!(pixel(&frame, 0, 0), [0, 0, 0]);
        assert_eq!(pixel(&frame, 8, 0), [255, 255, 255]);
        for n in 0..SLOT_LIMIT + 8 {
            let mut palette = [[0; 3]; 16];
            palette[1] = [n as u8, (n >> 8) as u8, 7];
            frame.tile(
                &tile,
                &palette,
                (n % 40) as i32 * 8,
                8 + (n / 40) as i32 * 8,
                Flip::default(),
            );
        }
        for n in [0, SLOT_LIMIT - 1, SLOT_LIMIT, SLOT_LIMIT + 7] {
            let (x, y) = (n % 40 * 8, 8 + n / 40 * 8);
            assert_eq!(pixel(&frame, x, y), [n as u8, (n >> 8) as u8, 7]);
        }
        assert_eq!(pixel(&frame, 8, 0), [255, 255, 255]);
    }

    #[test]
    fn fades_step_cram_nibbles_like_the_cartridge() {
        let white = md_color(0x0eee);
        assert_eq!(white, [255, 255, 255]);
        let grey = md_color(0x0444)[0]; // level 2
        // $67A: one nibble decrement per update; level 2 is black after 4.
        // CRAM keeps only bits 1-3, so odd shadow nibbles show the level
        // below: white is level 1 after 12 and black from 13.
        assert_eq!(Fade::Out(0).channel(grey), grey);
        assert_eq!(Fade::Out(1).channel(grey), md_color(0x0222)[0]);
        assert_eq!(Fade::Out(4).channel(grey), 0);
        assert_eq!(Fade::Out(12).channel(255), md_color(0x0222)[0]);
        assert_eq!(Fade::Out(13).channel(255), 0);
        assert_eq!(Fade::Out(FADE_STEPS).channel(255), 0);
        // $5EA: increments toward the colour; darker channels arrive first.
        assert_eq!(Fade::In(0).channel(255), 0);
        assert_eq!(Fade::In(4).channel(grey), grey);
        assert_eq!(Fade::In(4).channel(255), md_color(0x0444)[0]);
        assert_eq!(Fade::In(FADE_STEPS).channel(255), 255);
        let mut fade = Fade::Out(0);
        for _ in 0..20 {
            fade = fade.advance();
        }
        assert_eq!(fade, Fade::Out(FADE_STEPS));
        assert!(fade.finished() && !Fade::In(3).finished());
        let mut frame = Frame::default();
        frame.clear([255, 255, 255]);
        let before = frame.checksum();
        frame.fade(Fade::In(FADE_STEPS));
        assert_eq!(frame.checksum(), before);
        frame.fade(Fade::Out(FADE_STEPS));
        assert!(frame.rgb().iter().all(|&c| c == 0));
    }
}
