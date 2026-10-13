//! The `$1D5A` scroll routines as data, the `$1A64` camera limits and the
//! round 1-1 parallax rounding.
use super::{
    flags::{ModeFlags, RuntimeFlags},
    id::{StageId, round_count, substage_count},
    sections::Section,
};
use crate::{
    raster::SineWave,
    rom::{Result, Sine, signed, word},
};

/// Decoded `$1D5A` scroll routine of one part, without loading its art.
pub fn scroll_model(rom: &[u8], id: StageId) -> Result<ScrollModel> {
    let (r, s) = (id.round_index(), id.substage_index());
    if r >= round_count(rom)? || s >= substage_count(rom, id.round)? {
        return Err(format!("{id}: no such substage"));
    }
    ScrollModel::for_routine(word(rom, 0x1d5a + r * 8 + s * 2)?, id.part_index())
}

/// `$1A0C + (flags & 3) * 16`, copied to `$FF0148` by `$1A64`, with the
/// record's maximum Y (word 8) and nonzero maximum-X override (word 11).
/// Words 4..7 of each `$1A0C` record are the `$3854` collision X and Y
/// limits, the layout stride and a VRAM shift.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CameraBounds {
    pub x: (i32, i32),
    pub y: (i32, i32),
    /// Layout bytes per streamed column (horizontal) or row (vertical).
    pub stride: u16,
}

impl CameraBounds {
    /// `$1A64`: the `$1A0C + (flags & 3) * 16` limits with the record's
    /// maximum Y (word 8), nonzero maximum X (word 11) and, in boss arenas,
    /// the word-12 Y lock.
    pub fn for_section(rom: &[u8], section: &Section) -> Result<Self> {
        let raw = section.raw;
        let mode = ModeFlags(raw[7]);
        let flags = RuntimeFlags::from_record(raw[4], raw[7]);
        let dims = 0x1a0c + usize::from(flags.0 & 3) * 16;
        let arena_y = (mode.boss_arena() && raw[12] != 0).then(|| i32::from(raw[12] as i16));
        Ok(Self {
            x: (
                signed(rom, dims)?,
                if raw[11] != 0 {
                    i32::from(raw[11] as i16)
                } else {
                    signed(rom, dims + 2)?
                },
            ),
            y: (
                match arena_y {
                    Some(y) => y,
                    None => signed(rom, dims + 4)?,
                },
                arena_y.unwrap_or(i32::from(raw[8] as i16)),
            ),
            stride: word(rom, dims + 12)?,
        })
    }
}

/// Source of an hscroll value. Offsets are positive sample offsets, i.e.
/// the negation of the VDP value. `PlaneBX` is the separate background
/// position `$FFE010`; the viewer approximates it with camera X.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollSource {
    CameraX,
    PlaneBX,
    /// Frame counter advanced by `step` (16.16) every frame.
    Counter {
        step: i32,
    },
    Zero,
    /// Overwritten after the routine by boss code that positions a
    /// boss-drawn plane (`stage::BossRaster`); the band has no value of its
    /// own and the model falls back to camera X.
    Boss,
}
/// `rows` eight-line rows use `source * num / den`.
///
/// The shared helpers (`$1D96`, `$1DC4`, `$1DE0`) negate the 16.16 source
/// before shifting it and take the high word, so the offset's magnitude
/// rounds up from the whole fraction (`rounds_up`). `$1F68` and `$2582`
/// shift the camera's pixel word first and negate after: rounded down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Band {
    pub rows: u8,
    pub source: ScrollSource,
    pub num: i32,
    pub den: i32,
    pub rounds_up: bool,
}
const fn band(rows: u8, source: ScrollSource, num: i32, den: i32) -> Band {
    Band {
        rows,
        source,
        num,
        den,
        rounds_up: true,
    }
}
const fn band_down(rows: u8, source: ScrollSource, num: i32, den: i32) -> Band {
    Band {
        rounds_up: false,
        ..band(rows, source, num, den)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VScroll {
    CameraY,
    /// Camera Y plus the `$3326` shake/bob offset `$FF16C4`.
    CameraYShake,
    Fixed(i32),
    /// `(bY >> shift) + offset`.
    PlaneBY {
        shift: u8,
        offset: i32,
    },
    /// VDP register `$0B` bit 2: one word per two-cell column, written left
    /// to right by `$384C`/`$384E`/`$3850` (three, two or one columns).
    Columns(&'static [Column]),
    /// This routine does not write it.
    NotWritten,
}
/// What a per-column VSRAM word follows. `$1D96` negates its 16.16 input
/// and returns 7/8 .. 1/8 of it in d1 .. d7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnSource {
    /// `$FFE014`, the background Y (16.16).
    BackgroundY,
    /// The counter `$FF01B8`, advanced by `step` (16.16) every frame.
    Counter { step: i32 },
    /// `$261C`'s rise `$FF01BC`, whole pixels (`RasterState::rise`).
    Rise,
}
/// `count` two-cell columns showing `source * eighths / 8`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub count: u8,
    pub source: ColumnSource,
    pub eighths: i32,
}
const fn col(count: u8, source: ColumnSource, eighths: i32) -> Column {
    Column {
        count,
        source,
        eighths,
    }
}
/// Visible two-cell columns in H40 mode.
pub const COLUMNS: usize = 20;
/// `$1F84` (2-2 tower): d2 x3, d3, d4 x2, d5, d6, d7, d7 x2, d7, d6, d5,
/// d4 x2, d3, d2 x3 of the background Y.
const TOWER: &[Column] = {
    use self::ColumnSource::BackgroundY as Y;
    &[
        col(3, Y, 6),
        col(1, Y, 5),
        col(2, Y, 4),
        col(1, Y, 3),
        col(1, Y, 2),
        col(4, Y, 1),
        col(1, Y, 2),
        col(1, Y, 3),
        col(2, Y, 4),
        col(1, Y, 5),
        col(3, Y, 6),
    ]
};
/// `$261C` (6-1): d1 x2, d2 x2, d3 .. d7, the rise x2, d7 .. d3, d2 x2,
/// d1 x2 of the `$FF01B8` counter, which falls 8 px a frame.
const CLIFF: &[Column] = {
    const C: ColumnSource = ColumnSource::Counter { step: -8 << 16 };
    &[
        col(2, C, 7),
        col(2, C, 6),
        col(1, C, 5),
        col(1, C, 4),
        col(1, C, 3),
        col(1, C, 2),
        col(1, C, 1),
        col(2, ColumnSource::Rise, 8),
        col(1, C, 1),
        col(1, C, 2),
        col(1, C, 3),
        col(1, C, 4),
        col(1, C, 5),
        col(2, C, 6),
        col(2, C, 7),
    ]
};
/// The scroll routines' RAM that the camera does not determine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterState {
    /// `$FF01BC` (16.16): `$261C` part 0 adds `$A00` a frame up to `$120`
    /// px and saves it in `$FF0086`; later parts show the saved value.
    pub rise: i32,
    /// `$FF3A00`: `$207C` adds `$10` a frame to its sine byte offset.
    pub wave: u16,
    /// `$FF01C8`: the `$33C8` bob phase, +2 a frame.
    pub bob_phase: u16,
    /// `$FF16C0` (16.16): the bob position, the sum of its sine steps.
    pub bob: i32,
    /// Transient `$3326` jolt, supplied by the boss before rendering.
    pub jolt_offset: i32,
}
impl RasterState {
    /// One frame of the routine `model` of part `part`.
    pub(crate) fn step(&mut self, model: &ScrollModel, part: usize, sine: &Sine) {
        match model.routine {
            0x261c if part == 0 => self.rise = (self.rise + 0xa00).min(0x120_0000),
            0x207c => self.wave = self.wave.wrapping_add(0x10),
            0x27aa => {
                // $33C8: sine word ((-phase) & $1FF) * 4 bytes, >> 2, * 2.
                self.bob_phase = self.bob_phase.wrapping_add(2);
                let index = usize::from(self.bob_phase.wrapping_neg() & 0x1ff) * 2;
                self.bob = self.bob.wrapping_add((i32::from(sine.at(index)) >> 2) * 2);
            }
            _ => {}
        }
    }
    /// `Stage_VScrollOffset` (`$FF16C4`): the bob's whole pixels. Plane A
    /// (`CameraYShake`) and every object's screen Y add it; the `$3326`
    /// shake is supplied separately by the boss.
    pub fn vscroll_offset(&self) -> i32 {
        i32::from((self.bob >> 16) as i16) + self.jolt_offset
    }
}
/// What the scroll routines read in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollInputs {
    pub camera_y: i32,
    /// `$FFE014` in 16.16: the camera Y, except that vertical autoscroll
    /// moves it twice as fast (`$1C0E`).
    pub background_y: i64,
    /// Frames since the substage loaded (the `$FF01B8` counters).
    pub tick: u64,
    pub raster: RasterState,
}
/// The VSRAM words of one frame, as plane rows shown at scanline 0 per
/// two-cell screen column. Full-screen mode repeats one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vsram {
    pub plane_a: [i32; COLUMNS],
    pub plane_b: [i32; COLUMNS],
}
impl Vsram {
    /// Whether every column shows the same plane row.
    pub fn uniform(values: &[i32; COLUMNS]) -> bool {
        values.iter().all(|v| *v == values[0])
    }
}
/// Scroll model of one `$1D5A` routine for one part. Band lists cover the
/// 28 visible eight-line rows; an empty list means the routine does not
/// write that plane's hscroll or uses a per-line table described in `note`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollModel {
    pub routine: u16,
    pub plane_a: &'static [Band],
    pub plane_b: &'static [Band],
    pub plane_a_v: VScroll,
    pub plane_b_v: VScroll,
    pub forced_camera_x: Option<i32>,
    pub forced_camera_y: Option<i32>,
    /// `$2386` (4-1): while the player's state is below `$40`, its
    /// H-interrupt `$249A` fires after line 199 and writes 0 to VRAM
    /// `$BC02`, sprite 0's size/link word in the SAT at `$BC00`. The list
    /// then ends at sprite 0 (a hidden dummy), so no sprite shows below
    /// this line.
    pub sprite_cut: Option<usize>,
    /// Pixels the routine adds to the background Y `$FFE014` every frame.
    pub background_drift: i32,
    /// The routine ends in `Func_00342E`, the screen clamp that `$1D4C`
    /// otherwise runs only for runtime flags bit 6 (Genesis Plus GX
    /// coverage: 2-2 parts 1-2, 3-3, 4-3, 6-1, 7-1 part 1, 7-2 part 3,
    /// 7-3; the mounted 2-1 and 4-1 keep `crate::special`'s limits).
    pub screen_clamp: bool,
    pub note: &'static str,
}

use self::ScrollSource::{CameraX as CX, PlaneBX as BX, Zero as Z};
const fn ctr(step: i32) -> ScrollSource {
    ScrollSource::Counter { step }
}
const FULL_A: &[Band] = &[band(28, CX, 1, 1)];
const ZERO: &[Band] = &[band(28, Z, 0, 1)];
const B_1E18: &[Band] = &[
    band(9, BX, 3, 8),
    band(11, BX, 1, 4),
    band(2, BX, 3, 8),
    band(2, BX, 1, 2),
    band(2, BX, 5, 8),
    band(2, BX, 3, 4),
];
const B_1E88: &[Band] = &[
    band(7, BX, 3, 8),
    band(5, BX, 1, 8),
    band(10, BX, 3, 8),
    band(1, BX, 1, 2),
    band(1, BX, 5, 8),
    band(4, BX, 3, 4),
];
const HORSE: i32 = 6 << 16;
const A_1ED4: &[Band] = &[band(17, ctr(HORSE), 1, 16), band(11, ctr(HORSE), 1, 1)];
const B_1ED4: &[Band] = &[
    band(12, ctr(HORSE), 1, 64),
    band(1, ctr(HORSE), 1, 32),
    band(4, ctr(HORSE), 1, 8),
    band(3, ctr(HORSE), 1, 2),
    band(8, ctr(HORSE), 3, 2),
];
const B_1F68: &[Band] = &[band_down(28, BX, 1, 8)];
const B_202C: &[Band] = &[band(7, BX, 1, 2), band(10, BX, 1, 4), band(11, BX, 1, 2)];
const B_207C: &[Band] = &[band(10, BX, 1, 2), band(12, BX, -1, 8), band(6, BX, 1, 2)];
/// `$23C4`: `$FF0190` = 8 (16.16) is added to the fast counter `$FF01B8`
/// and `$8000` to the slow counter `$FF01BC` every frame.
const SURF_FAST: i32 = 8 << 16;
const SURF_SLOW: i32 = 0x8000;
/// `$2386`/`$23C0` plane A (high word of each `$36C2` long). `$1DC4` and
/// `$1D96` negate their input, so every band moves left (positive offset).
/// The tail `neg.w d0` / `add.w d0,d4` / `swap d0` / `move.w d4,d0` puts
/// `-fast/2 - fast` in plane A of the last six rows: fast x 3/2.
const A_SURF: &[Band] = &[
    band(14, ctr(SURF_SLOW), 1, 1),
    band(8, ctr(SURF_FAST), 1, 1),
    band(6, ctr(SURF_FAST), 3, 2),
];
/// `$23C0` (4-3) plane A as displayed: Boss4's `Func_02EE38` runs after
/// the routine every frame and rewrites the plane A word of rows 3..27
/// (`HScroll_Buffer+$60`, 25 rows) with the hull's X, so only rows 0..2
/// keep the slow band, under the HUD window (VDP register `$12` = 3).
/// Plane A then holds nothing but the hull (`stage::BossPlanes::Hull`).
const A_SURF_ARENA: &[Band] = &[
    band(3, ctr(SURF_SLOW), 1, 1),
    band(25, ScrollSource::Boss, 1, 1),
];
/// `$2386`/`$23C0` plane B: `$1DC4` (shift 3) slow eighths, `$1D96` fast
/// eighths with `d7` halved to 1/16, then the tail adds `-fast` to the 1/4
/// (`d6`) and 1/2 (`d4`) values: rows 22-27 are fast x 1, 5/4, 3/2.
const B_SURF: &[Band] = &[
    band(5, ctr(SURF_SLOW), 7, 8),
    band(2, ctr(SURF_SLOW), 6, 8),
    band(2, ctr(SURF_SLOW), 5, 8),
    band(5, ctr(SURF_SLOW), 4, 8),
    band(2, ctr(SURF_FAST), 1, 16),
    band(1, ctr(SURF_FAST), 1, 4),
    band(1, ctr(SURF_FAST), 3, 8),
    band(1, ctr(SURF_FAST), 1, 2),
    band(1, ctr(SURF_FAST), 5, 8),
    band(1, ctr(SURF_FAST), 3, 4),
    band(1, ctr(SURF_FAST), 7, 8),
    band(2, ctr(SURF_FAST), 1, 1),
    band(2, ctr(SURF_FAST), 5, 4),
    band(2, ctr(SURF_FAST), 3, 2),
];
const B_24AE: &[Band] = &[
    band(5, ctr(SURF_SLOW), 7, 8),
    band(2, ctr(SURF_SLOW), 6, 8),
    band(1, ctr(SURF_SLOW), 5, 8),
    band(1, ctr(SURF_SLOW), 4, 8),
    band(1, ctr(SURF_SLOW), 3, 8),
    band(14, BX, 1, 8),
    band(1, BX, 1, 4),
    band(1, BX, 3, 8),
    band(1, BX, 1, 2),
    band(1, BX, 5, 8),
];
const B_2582: &[Band] = &[band_down(28, BX, 1, 4)];
/// `$27C0` (7-1): `$FF01B8` rises by 4 px a frame and `$1D96` negates it,
/// so the cloud bands move left like the surf's.
const W71: i32 = 4 << 16;
const B_27F2: &[Band] = &[
    band(8, ctr(W71), 3, 4),
    band(6, ctr(W71), 5, 8),
    band(1, ctr(W71), 1, 2),
    band(2, ctr(W71), 3, 8),
    band(3, ctr(W71), 1, 4),
    band(1, ctr(W71), 1, 8),
    band(7, Z, 0, 1),
];
const B_283E: &[Band] = &[
    band(5, ctr(W71), 3, 4),
    band(4, ctr(W71), 5, 8),
    band(1, ctr(W71), 1, 2),
    band(3, ctr(W71), 3, 8),
    band(1, ctr(W71), 1, 4),
    band(3, ctr(W71), 1, 8),
    band(1, ctr(W71), 1, 4),
    band(3, ctr(W71), 3, 8),
    band(1, ctr(W71), 1, 2),
    band(4, ctr(W71), 5, 8),
    band(2, ctr(W71), 3, 4),
];
const B_28C8: &[Band] = &[
    band(10, Z, 0, 1),
    band(1, ctr(W71), 1, 8),
    band(3, ctr(W71), 1, 4),
    band(2, ctr(W71), 3, 8),
    band(1, ctr(W71), 1, 2),
    band(6, ctr(W71), 5, 8),
    band(5, ctr(W71), 3, 4),
];
const B_291C: &[Band] = &[band(28, BX, 1, 2)];

impl ScrollModel {
    const fn new(routine: u16, plane_a: &'static [Band], plane_b: &'static [Band]) -> Self {
        Self {
            routine,
            plane_a,
            plane_b,
            plane_a_v: VScroll::CameraY,
            // e.g. $1E18/$1E88 write the constant $FFE8 to $F482, the shadow
            // of plane B's VSRAM word: the forest's plane B never scrolls
            // vertically.
            plane_b_v: VScroll::Fixed(-24),
            forced_camera_x: None,
            forced_camera_y: None,
            sprite_cut: None,
            background_drift: 0,
            screen_clamp: false,
            note: "",
        }
    }
    pub fn for_routine(routine: u16, part: usize) -> Result<Self> {
        use self::VScroll::*;
        let m = Self::new(routine, FULL_A, &[]);
        Ok(match routine {
            0x1e18 => Self {
                plane_b: B_1E18,
                plane_a_v: Fixed(0x120),
                forced_camera_y: Some(0x120),
                note: "all rows 3/4 once $FF0199 bit 0 is set",
                ..m
            },
            0x1e88 => Self {
                plane_b: B_1E88,
                ..m
            },
            0x1ed4 => Self {
                plane_a: A_1ED4,
                plane_b: B_1ED4,
                plane_a_v: Fixed(0),
                plane_b_v: Fixed(0),
                forced_camera_y: Some(0),
                screen_clamp: true,
                note: "horse: counter $FF01B8 += 6 px per frame, no camera X term",
                ..m
            },
            0x1f52 if part == 0 => Self::for_routine(0x1f68, 0)?,
            0x1f52 | 0x1f84 => Self {
                plane_a: ZERO,
                plane_b: ZERO,
                plane_b_v: Columns(TOWER),
                forced_camera_x: Some(0),
                screen_clamp: true,
                note: "tower: plane B per-column vscroll 3/4..1/8..3/4 of bY",
                ..m
            },
            0x1f68 => Self {
                plane_b: B_1F68,
                plane_a_v: CameraYShake,
                plane_b_v: PlaneBY {
                    shift: 4,
                    offset: 0,
                },
                ..m
            },
            0x1f7e => Self {
                plane_a_v: CameraYShake,
                plane_b_v: NotWritten,
                note: "plane B scroll is not written by this routine",
                ..m
            },
            0x202c => Self {
                plane_b: B_202C,
                plane_a_v: if part == 0 { Fixed(0x120) } else { CameraY },
                forced_camera_y: (part == 0).then_some(0x120),
                ..m
            },
            0x207c => Self {
                plane_b: B_207C,
                note: "plane A rows add a +/-4 px sine wave ($FF3A00 phase, table $9748)",
                ..m
            },
            0x2366 => Self {
                plane_a: &[],
                plane_a_v: Fixed(0x118),
                plane_b_v: NotWritten,
                // Boss3 `.loc_02C086` clears Camera_X and sets the camera
                // bounds $FF014A/$FF014E to 0/$118 (#96).
                forced_camera_x: Some(0),
                forced_camera_y: Some(0x118),
                // Genesis Plus GX: Joe walking right stops at X 288.
                screen_clamp: true,
                note: "boss arena: hscroll and plane B vscroll written elsewhere",
                ..m
            },
            0x2386 => Self {
                plane_a: A_SURF,
                plane_b: B_SURF,
                plane_a_v: Fixed(0),
                plane_b_v: Fixed(32),
                forced_camera_y: Some(0),
                sprite_cut: Some(200),
                screen_clamp: true,
                note: "surf: counters +8 px and +0.5 px per frame; 4-1 installs an H-interrupt at line 200",
                ..m
            },
            // The `$100` VSRAM word is likewise replaced by Boss4.
            0x23c0 => Self {
                plane_a: A_SURF_ARENA,
                plane_b: B_SURF,
                plane_a_v: Fixed(0x100),
                plane_b_v: Fixed(32),
                forced_camera_y: Some(0x100),
                screen_clamp: true,
                note: "surf arena: plane B as 4-1; plane A rows 3..27 and VSRAM are Boss4's hull scroll",
                ..m
            },
            0x24ae => Self {
                plane_b: B_24AE,
                plane_b_v: Fixed(0),
                ..m
            },
            0x254a => Self {
                plane_b: B_2582,
                plane_a_v: CameraYShake,
                plane_b_v: if part < 3 {
                    PlaneBY {
                        shift: 3,
                        offset: 0x100,
                    }
                } else {
                    PlaneBY {
                        shift: 4,
                        offset: 0x70,
                    }
                },
                ..m
            },
            0x1e16 => Self {
                plane_a: &[],
                plane_a_v: NotWritten,
                plane_b_v: NotWritten,
                note: "rts: no scroll writes",
                ..m
            },
            0x261c => Self {
                plane_a: ZERO,
                plane_b: ZERO,
                plane_b_v: Columns(CLIFF),
                forced_camera_x: Some(0),
                forced_camera_y: (part != 0).then_some(0x20),
                screen_clamp: true,
                note: "column parallax: plane B per-column fractions of a -8 px/frame counter",
                ..m
            },
            0x2710 => Self {
                plane_a_v: CameraYShake,
                plane_b_v: PlaneBY {
                    shift: 2,
                    offset: 0,
                },
                forced_camera_y: [1, 1, 0, 0, 1, 1, 1, 0]
                    .get(part)
                    .is_some_and(|f| *f != 0)
                    .then_some(0x120),
                note: "per-line plane B: even lines +0.375, odd lines -0.5 px per frame (plane_b_line_offset)",
                ..m
            },
            0x2776 => Self {
                plane_a: ZERO,
                plane_b: ZERO,
                plane_a_v: Fixed(-24),
                plane_b_v: Fixed(0x100),
                forced_camera_x: Some(0),
                forced_camera_y: Some(-24),
                // The kabuki room occupies one screen. Keep the native
                // player inside its ledges instead of entering the unused
                // columns of the 512-pixel layout and falling forever.
                screen_clamp: true,
                ..m
            },
            0x27aa => match part {
                0 => Self {
                    plane_b: B_27F2,
                    plane_a_v: CameraYShake,
                    forced_camera_y: Some(0x120),
                    note: "counter +4 px per frame; $33C8 sine bob",
                    ..m
                },
                1 => Self {
                    plane_a: ZERO,
                    plane_b: B_283E,
                    plane_a_v: CameraYShake,
                    forced_camera_x: Some(0),
                    screen_clamp: true,
                    note: "counter +4 px per frame; $33C8 sine bob",
                    ..m
                },
                _ => Self {
                    plane_b: B_28C8,
                    plane_a_v: CameraYShake,
                    plane_b_v: Fixed(0),
                    forced_camera_y: Some(0x120),
                    note: "counter +4 px per frame; $33C8 sine bob",
                    ..m
                },
            },
            0x291c => Self {
                plane_b: B_291C,
                plane_a_v: CameraYShake,
                plane_b_v: PlaneBY {
                    shift: 2,
                    offset: 0,
                },
                // Data_002914: part 3 pins camera X and runs Func_00342E.
                forced_camera_x: (part == 3).then_some(0),
                screen_clamp: part == 3,
                // subq.w #1,$FFE014 before plane B reads it.
                background_drift: -1,
                note: "plane B Y decreases by one each frame",
                ..m
            },
            0x295e => Self {
                plane_a: ZERO,
                plane_b: ZERO,
                plane_a_v: Fixed(0),
                plane_b_v: Fixed(0),
                forced_camera_x: Some(0),
                forced_camera_y: Some(0x110),
                screen_clamp: true,
                note: "final boss: sine wobble only when $FF005F = 7",
                ..m
            },
            _ => return Err(format!("unknown scroll routine ${routine:04x}")),
        })
    }
    fn offset(bands: &[Band], scanline: usize, camera_x: i32, tick: u64) -> Option<i32> {
        let mut row = scanline / 8;
        for b in bands {
            if row < usize::from(b.rows) {
                // 16.16; the native camera has no fraction.
                let source = match b.source {
                    ScrollSource::CameraX | ScrollSource::PlaneBX => i64::from(camera_x) << 16,
                    ScrollSource::Counter { step } => tick as i64 * i64::from(step),
                    ScrollSource::Zero => 0,
                    ScrollSource::Boss => return None,
                };
                let den = i64::from(b.den);
                let offset = if b.rounds_up {
                    let scaled = source * i64::from(b.num.abs());
                    -(-scaled).div_euclid(den << 16) * i64::from(b.num.signum())
                } else {
                    ((source >> 16) * i64::from(b.num)).div_euclid(den)
                };
                return Some(offset as i32);
            }
            row -= usize::from(b.rows);
        }
        None
    }
    /// Positive plane B sample offset for a VDP scanline (0 when unmodeled).
    pub fn plane_b_offset(&self, scanline: usize, camera_x: i32, tick: u64) -> i32 {
        Self::offset(self.plane_b, scanline, camera_x, tick).unwrap_or(0)
    }
    /// Plane B sample offset of a VDP scanline including per-line tables.
    /// `$2710` (6-2): `$3598` advances 256 16.16 counters at `$FF01B8` every
    /// frame, even entries by +0.375 px and odd ones by -0.5 px; `$3516`
    /// writes entry `(vscroll + 16 + line) & 255` as each line's HScroll, so
    /// alternate plane B rows drift apart. Other routines use their bands.
    pub fn plane_b_line_offset(
        &self,
        scanline: usize,
        vscroll: i32,
        camera_x: i32,
        tick: u64,
    ) -> i32 {
        if self.routine != 0x2710 {
            return self.plane_b_offset(scanline, camera_x, tick);
        }
        let tick = tick as i64;
        let counter = if (vscroll + 16 + scanline as i32).rem_euclid(2) == 0 {
            tick * 0x6000
        } else {
            -tick * 0x8000
        };
        -((counter >> 16) as i32)
    }
    pub fn plane_a_offset(&self, scanline: usize, camera_x: i32, tick: u64) -> i32 {
        Self::offset(self.plane_a, scanline, camera_x, tick).unwrap_or(camera_x)
    }
    /// Plane A sample offset of a VDP scanline including the `$207C` (3-2)
    /// wave: each eight-line row adds `sine >> 12` (±4 px) to the HScroll,
    /// sampling `$9748` at byte `$FF3A00 + row * $20`. Other routines use
    /// their bands.
    pub(crate) fn plane_a_line_offset(
        &self,
        scanline: usize,
        camera_x: i32,
        tick: u64,
        raster: &RasterState,
        sine: &Sine,
    ) -> i32 {
        if self.routine != 0x207c || scanline >= 28 * 8 {
            return self.plane_a_offset(scanline, camera_x, tick);
        }
        camera_x - SineWave::shifted(raster.wave, 0x20, 12).offset(scanline / 8, sine)
    }
    /// `$295E`: H-scroll pairs for lines 24..223; both phases wrap as words.
    pub(crate) fn final_boss_hscroll(line: usize, tick: u16, sine: &Sine) -> (i32, i32) {
        let Some(i) = line.checked_sub(24).filter(|&i| i < 200) else {
            return (0, 0);
        };
        let a = SineWave::shifted(tick.wrapping_mul(0x40), 0x80, 12);
        let b = SineWave::shifted(tick.wrapping_mul(4), 0x13, 9);
        let c = SineWave::shifted(tick.wrapping_mul(0x23), 0x20, 9);
        (
            a.offset(i / 8, sine) + 4,
            b.offset(i, sine) + c.offset(i, sine),
        )
    }
    /// `$2A1C`: plane A has twenty sine-scrolled columns; plane B stays at zero.
    pub(crate) fn final_boss_vsram(tick: u16, sine: &Sine) -> Vsram {
        Vsram {
            plane_a: std::array::from_fn(|column| {
                SineWave::shifted(tick.wrapping_mul(0x40), 0x80, 12).offset(column, sine) + 4
            }),
            plane_b: [0; COLUMNS],
        }
    }

    /// Full-screen VSRAM value (plane row shown at scanline 0), with the
    /// camera standing in for the background Y; 0 for per-column or
    /// unwritten models (see `vsram`).
    pub fn vscroll(v: VScroll, camera_y: i32) -> i32 {
        match v {
            VScroll::CameraY | VScroll::CameraYShake => camera_y,
            VScroll::Fixed(v) => v,
            VScroll::PlaneBY { shift, offset } => (camera_y >> shift) + offset,
            VScroll::Columns(_) | VScroll::NotWritten => 0,
        }
    }
    /// This frame's VSRAM words for both planes.
    pub fn vsram(&self, inputs: &ScrollInputs) -> Vsram {
        Vsram {
            plane_a: Self::columns(self.plane_a_v, inputs),
            plane_b: Self::columns(self.plane_b_v, inputs),
        }
    }
    fn columns(v: VScroll, inputs: &ScrollInputs) -> [i32; COLUMNS] {
        let VScroll::Columns(list) = v else {
            let background_y = (inputs.background_y >> 16) as i32;
            return [match v {
                VScroll::CameraYShake => inputs.camera_y + inputs.raster.vscroll_offset(),
                VScroll::PlaneBY { shift, offset } => (background_y >> shift) + offset,
                _ => Self::vscroll(v, inputs.camera_y),
            }; COLUMNS];
        };
        let mut out = [0; COLUMNS];
        let mut next = out.iter_mut();
        for column in list {
            let value = match column.source {
                // $1D96 negates d0; the tower negates $FFE014 first.
                ColumnSource::BackgroundY => inputs.background_y,
                ColumnSource::Counter { step } => -(inputs.tick as i64 * i64::from(step)),
                ColumnSource::Rise => i64::from(inputs.raster.rise),
            };
            // Arithmetic shifts of 16.16 values: floor of the fraction.
            let word = (value * i64::from(column.eighths)).div_euclid(8 << 16) as i32;
            for slot in next.by_ref().take(usize::from(column.count)) {
                *slot = word;
            }
        }
        out
    }
}

/// $1E18, the round 1-1 scroll routine selected through $1D5A, writes $0120 to
/// $E004 and $F480 every frame, so the displayed camera Y never follows the
/// player in 1-1 (the emulator shows plane A VSRAM fixed at $0120).
/// The generic runtime reads it as `ScrollModel::forced_camera_y` of `$1E18`.
/// $1D92/$1E18/$3698: 28 eight-scanline rows, with independently shifted A/B.
/// `scanline` is the VDP line (0 = top of the display), not a viewport row.
/// Coordinates are positive sample offsets (the VDP stores their negatives).
/// $1D92 scales the negated 16.16 camera, so the result is ceil(camera*ratio).
/// The native camera is whole pixels (no fraction), which equals the original
/// whenever its camera fraction is zero. Plane B and the trunk sprites must
/// both use this function so they round identically.
pub fn forest_background_x(camera_x: i32, scanline: usize) -> i32 {
    let (n, d) = match scanline / 8 {
        0..=8 => (3, 8),
        9..=19 => (1, 4),
        20..=21 => (3, 8),
        22..=23 => (1, 2),
        24..=25 => (5, 8),
        _ => (3, 4),
    };
    // Arithmetic shifts of a negative 16.16 coordinate round away from zero.
    (camera_x * n).div_euclid(d) + i32::from((camera_x * n).rem_euclid(d) != 0)
}
