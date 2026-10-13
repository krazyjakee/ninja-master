//! Spawn records (`$41EFA`) of every stage part.
use super::Mapping;
use crate::{
    enemies,
    rom::{Result, pointer, word},
    stage::StageId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    pub source: usize,
    pub x: i32,
    pub y: i32,
    pub flags: u16,
    /// Byte offset into $12E9A and $68A8, not a sequential type number.
    pub kind: u16,
    pub handler: usize,
    pub score_bcd: u16,
    pub contact_damage: u8,
    pub health_counter: u8,
    /// Pose for an unported type (issue #51): the first
    /// `move.w #imm,obj_frame(An)` in its handler, or 0. Drawn with the
    /// unported outline; 0 for ported types.
    pub placeholder_mapping: u16,
}
impl Spawn {
    pub fn active_at(&self, camera: (i32, i32), tick: u64) -> bool {
        self.active_in_view(camera, tick, (0, crate::WIDTH as i32))
    }

    pub(crate) fn active_in_view(&self, camera: (i32, i32), tick: u64, view: (i32, i32)) -> bool {
        self.flags & 0x8000 == 0
            && self.x > camera.0 + view.0 - 64
            && self.x < camera.0 + view.1 + 64
            && (camera.1 + 112 - self.y).abs() < 144
            && (self.flags & 0x1000 == 0 || tick & 63 == 0)
    }
}

/// Static placeholder pose: the first `MOVE.W #imm,$0A(An)` (`obj_frame`)
/// within the handler's first kilobyte whose mapping is below `$3C0` and
/// loads. A heuristic for display only, not a recovered state machine; it
/// is removed when the last type is ported.
pub(crate) fn placeholder_mapping(rom: &[u8], handler: usize) -> u16 {
    for at in (handler..handler + 0x400).step_by(2) {
        let (Ok(op), Ok(imm), Ok(offset)) = (word(rom, at), word(rom, at + 2), word(rom, at + 4))
        else {
            break;
        };
        if op & 0xf1ff == 0x317c && offset == 0x000a && imm != 0 && imm < 0x3c0 {
            return if Mapping::load(rom, imm).is_ok() {
                imm
            } else {
                0
            };
        }
    }
    0
}

/// Spawn records a part activates. `Objects_UpdateEnemies` (`$125D2`) skips
/// `Spawn_ActivateNearby` while `Stage_InhibitFlags` bit 0 is set, so a boss
/// arena activates none: `Spawn_RoundTable` gives the third substage the
/// second substage's part table, and those records are never used (#52).
pub fn stage_spawns(rom: &[u8], id: StageId) -> Result<Vec<Spawn>> {
    if boss_arena(rom, id)? {
        return Ok(Vec::new());
    }
    spawn_records(rom, id)
}
/// The part record's mode word (`$11754`).
pub(crate) fn mode_flags(rom: &[u8], id: StageId) -> Result<crate::stage::ModeFlags> {
    let all = crate::stage::sections(rom, id.round_index(), id.substage_index())?;
    let section = all
        .get(id.part_index())
        .ok_or_else(|| format!("{id}: combat part outside recovered sections"))?;
    Ok(crate::stage::ModeFlags(section.mode_flags))
}
/// Whether the part record's mode word selects a boss arena (`$FF39F7` bit 0).
pub fn boss_arena(rom: &[u8], id: StageId) -> Result<bool> {
    Ok(mode_flags(rom, id)?.boss_arena())
}
/// The ROM spawn list of any part, whether or not the part activates it:
/// `$41EFA -> round -> substage -> part` (`$129F8`), after the 32-byte
/// graphics/palette prefix.
pub fn spawn_records(rom: &[u8], id: StageId) -> Result<Vec<Spawn>> {
    let start = object_selection(rom, id)? + 32;
    let mut result = Vec::new();
    for i in 0..32 {
        let p = start + i * 8;
        let x = word(rom, p)? as i16;
        if x < 0 {
            return Ok(result);
        }
        let kind = word(rom, p + 6)?;
        // $12E9A holds 64 handlers: types are byte offsets $00..$FC.
        if !kind.is_multiple_of(4) || kind >= 0x100 {
            return Err(format!("unsupported object type ${kind:x}"));
        }
        let stats = word(rom, 0x68aa + usize::from(kind))?;
        let handler = pointer(rom, 0x12e9a + usize::from(kind))?;
        result.push(Spawn {
            source: p,
            x: i32::from(x),
            y: i32::from(word(rom, p + 2)? as i16),
            flags: word(rom, p + 4)?,
            kind,
            handler,
            score_bcd: word(rom, 0x68a8 + usize::from(kind))?,
            contact_damage: (stats >> 8) as u8,
            health_counter: stats as u8,
            placeholder_mapping: if enemies::ported(kind) {
                0
            } else {
                placeholder_mapping(rom, handler)
            },
        });
    }
    Err("unterminated spawn list".into())
}

/// Object selection (32-byte prefix, then spawns) of one part.
pub(super) fn object_selection(rom: &[u8], id: StageId) -> Result<usize> {
    let all = crate::stage::sections(rom, id.round_index(), id.substage_index())?;
    all.get(id.part_index())
        .map(|section| section.object_selection)
        .ok_or_else(|| format!("{id}: combat part outside recovered sections"))
}
