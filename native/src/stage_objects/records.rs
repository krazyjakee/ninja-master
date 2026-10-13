//! The `$27178` spawn table and the part rules that pick a list.
use crate::{
    rom::{Result, pointer, signed, word},
    stage::StageId,
};

/// `$27178`: one long per round + substage * 8 (`$26F94..$26FB2`), each
/// pointing at a list of part pointers.
const TABLE: usize = 0x27178;
/// Bytes per record (`$26FEA`, `$26FFA`).
const RECORD_SIZE: usize = 16;
/// `$FF3808` is a long and `btst d7,d3` on a data register takes the bit
/// number modulo 32, so a part holds at most 32 distinct records.
pub(super) const MAX_RECORDS: usize = 32;
/// Table index 9 (2-2): `$26FBC` subtracts 36 from the byte offset and,
/// when that leaves zero, reads the first part pointer for every part.
const SHARED_LIST: usize = 9;

/// One `$27178` record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// ROM address of the record.
    pub source: usize,
    /// List-6 type: the index into the `$27062` spawn and `$270AA` update
    /// jump tables.
    pub kind: u16,
    /// Words 1-4: the window the camera must be in for the record to spawn.
    pub x_window: (i32, i32),
    pub y_window: (i32, i32),
    /// Words 5-6: where the object spawns.
    pub x: i32,
    pub y: i32,
    /// Word 7 (`+$34` of the object): type-specific; the item box keeps
    /// its contents in bits 0-2.
    pub param: u16,
}
impl Record {
    /// `$27006..$27024`: the camera at `$FFE000`/`$FFE004` is inside the
    /// window widened by 384/320 pixels before and 65 after.
    pub fn active_at(&self, camera: (i32, i32)) -> bool {
        self.active_in_view(camera, (0, crate::WIDTH as i32))
    }

    pub(super) fn active_in_view(&self, camera: (i32, i32), view: (i32, i32)) -> bool {
        camera.0 + view.1 >= self.x_window.0 - 64
            && camera.1 >= self.y_window.0 - 320
            && camera.0 + view.0 <= self.x_window.1 + 65
            && camera.1 <= self.y_window.1 + 65
    }
}

/// The table list of a part, as `$26F94..$26FCE` finds it. Boss arenas
/// never read it (see [`stage_records`]); their slots point into unrelated
/// data, so this returns an error or nonsense for them.
fn table_records(rom: &[u8], id: StageId) -> Result<Vec<Record>> {
    let index = id.round_index() + id.substage_index() * 8;
    let list = pointer(rom, TABLE + index * 4)?;
    // $26FB6: an empty long ends the scan.
    if pointer(rom, list)? == 0 {
        return Ok(Vec::new());
    }
    let part = if index == SHARED_LIST {
        0
    } else {
        id.part_index()
    };
    let start = pointer(rom, list + part * 4)?;
    let mut records = Vec::new();
    for i in 0..=MAX_RECORDS {
        let p = start + i * RECORD_SIZE;
        let kind = word(rom, p)?;
        // $26FDE: a negative type word ends the list.
        if kind & 0x8000 != 0 {
            return Ok(records);
        }
        if i == MAX_RECORDS {
            break;
        }
        records.push(Record {
            source: p,
            kind,
            x_window: (signed(rom, p + 2)?, signed(rom, p + 4)?),
            y_window: (signed(rom, p + 6)?, signed(rom, p + 8)?),
            x: signed(rom, p + 10)?,
            y: signed(rom, p + 12)?,
            param: word(rom, p + 14)?,
        });
    }
    Err(format!(
        "{id}: more than {MAX_RECORDS} stage objects at ${start:05X}"
    ))
}

/// The records a part spawns. `$26F7C` skips the scan while
/// `Stage_InhibitFlags` (`$FF39F7`) bit 0 is set, which the part record's
/// mode word sets in boss arenas, so they spawn none.
pub(super) fn stage_records(rom: &[u8], id: StageId) -> Result<Vec<Record>> {
    if crate::combat::boss_arena(rom, id)? {
        return Ok(Vec::new());
    }
    table_records(rom, id)
}

/// Whether moving into part `id` keeps the list-6 state of the previous
/// part. `$26EE6` clears `$FF3808` and frees list 6 on every stage or part
/// load, except when the new part is 2-2 part 1 or 2 or 7-1 part 1: those
/// parts share the previous part's list (a lift carries the player across).
pub(super) fn keeps_state(id: StageId) -> bool {
    matches!(
        (id.round_index(), id.substage_index(), id.part_index()),
        (1, 1, 1..) | (6, 0, 1)
    )
}
