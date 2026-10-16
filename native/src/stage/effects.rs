//! Dynamic presentation of one stage part and the round 1-1 part-two event.
use super::{
    LIMIT,
    id::{StageId, StageRoutines, stage_routines},
    palette_fx::PaletteEffect,
    scenery::{Scenery, scenery_list},
    tile_anim::TileAnimation,
};
use crate::assets::musashi;
use crate::rom::{Result, Sine, byte, words};

/// Dynamic presentation of one stage part, interpreted from its routine
/// table entries (`StageRoutines`): the `$894` palette routine, the
/// `$390AE` tile animation routine and the `$39BE4` scenery sprite list.
/// Round 1-1 additionally carries its hard-wired `$39642` part-two event.
#[derive(Clone, Debug)]
pub struct StageEffects {
    pub routines: StageRoutines,
    pub palette: PaletteEffect,
    /// `None` for `$390E0` (`rts`) and in boss arenas, where `$390E2`
    /// returns while `$FF39F7` bit 0 is set.
    pub animation: Option<TileAnimation>,
    pub scenery: Vec<Scenery>,
    pub event: Option<ForestEvent>,
    /// `$9748`, read by the `$207C` plane A wave.
    pub(crate) sine: Sine,
}
impl StageEffects {
    pub fn load(rom: &[u8], id: StageId, boss_arena: bool) -> Result<Self> {
        let routines = stage_routines(rom, id.round_index(), id.substage_index())?;
        Ok(Self {
            palette: PaletteEffect::load(rom, routines.palette)?,
            animation: if boss_arena {
                None
            } else {
                TileAnimation::load(rom, routines.animation, id.part_index())?
            },
            scenery: scenery_list(rom, routines.scenery)?,
            event: if id.is_forest() {
                Some(ForestEvent::load(rom)?)
            } else {
                None
            },
            routines,
            sine: Sine::load(rom)?,
        })
    }
}

/// `$39642` triggers the round 1-1 event object (`$3F`, states `$370F4`)
/// in part 2 once the local camera X reaches `$100`.
pub const EVENT_PART: usize = 2;
pub const EVENT_CAMERA_X: i32 = 0x100;
/// Event tick of `$37144`: the event background and the `$FF0199` flag
/// (all background rows at 3/4, vegetation state `$3922C`).
pub const EVENT_FLAG_TICK: u64 = 2;
/// `$3714C`, in the same `$37116` state as the flag: the camera's minimum X
/// (`$FF0148`, loaded from the part record by `$1A64`) becomes `$100`. The
/// forest entrance, where the `$BC8` ramp would otherwise lighten line 2
/// again, can no longer scroll back into view under the cave background;
/// `$1D4C` runs `$342E` in this part, which holds the player on screen. Nothing lowers it again until a part load re-runs `$1A64`. The
/// `$FF11B8` latch of `$39642` re-arms the trigger only below `$100`, so the
/// floor also keeps the event from running twice.
pub const EVENT_CAMERA_FLOOR: i32 = 0x100;
/// Event tick of `$371D8`/`$371E4`: palette selector 15 is loaded and the
/// scenery list is cleared (`$39C5E`). Native play shows line 2 at once
/// and starts the `$3721A` flash countdown here, omitting the `$371F0`
/// fade of that line.
pub const EVENT_PALETTE_TICK: u64 = 6;

/// Round 1-1 part-two event content.
#[derive(Clone, Debug)]
pub struct ForestEvent {
    /// `$37116`: 16x25 bytes over base attribute `$4080`, repeated horizontally.
    pub background: Vec<u16>,
    pub background_range: std::ops::Range<usize>,
    /// Palette selector 15 (`$4307E`), the `$FF16FA` fade targets. Only
    /// line 2 reaches CRAM (`$371F0`).
    pub palettes: [[u16; 16]; 4],
}
impl ForestEvent {
    pub fn load(rom: &[u8]) -> Result<Self> {
        let (bytes, end) = musashi(rom, crate::rom::regions::source(rom, 0x3e450), LIMIT)?;
        let background = bytes
            .get(..400)
            .ok_or("short event background")?
            .iter()
            .map(|v| 0x4080 + u16::from(*v))
            .collect();
        let mut palettes = [[0; 16]; 4];
        for (bank, row) in palettes.iter_mut().enumerate() {
            *row = words(rom, 0x42062 + usize::from(byte(rom, 0x4307e + bank)?) * 32)?;
        }
        Ok(Self {
            background,
            background_range: 0x3e432..end,
            palettes,
        })
    }
}
