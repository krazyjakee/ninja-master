//! The blinking "GO" markers that point the way on (list-1 type `$3E`).
//!
//! `Func_03771A` runs on every stage load (`Mode_StageLoad`, after
//! `Objects_InitWorkspace`) and on every streamed part change
//! (`SectionChange_S30`, which keeps list 1). Outside boss arenas
//! (`Stage_InhibitFlags` bit 0) it reads one long for the part from
//! `Data_0376FA`: `$3764A` + a word per round and substage + 4 per part.
//! `$FFFF` places nothing. Otherwise `Func_03775E` allocates a marker with
//! the high nibble of the first word selecting mapping `$3A4 + n` and the
//! low twelve bits giving X, and the second word giving Y. `$C0` calls
//! `Func_03775E` directly for the 3-1 collapse's marker. The tile word is
//! `$8000`: the `$3C570` art at tiles `$30..$3C`, loaded by `$A32E` on
//! every stage that is not mounted. `Func_037796` shows the mapping while
//! `Frame_Counter & $C` is not 0 and frees the marker once `Game_Requests`
//! has any of bits 0-3 set.
use crate::{
    rom::{Result, word},
    stage::StageId,
};

/// `Data_0376FA`: the part table base, then the offset words.
const TABLE: usize = 0x376fa;
const FIRST_MAPPING: u16 = 0x3a4;
/// High priority, palette line 0, absolute tiles.
pub(crate) const TILE: u16 = 0x8000;
/// List-1 records (`Object_ListParams+5`).
const LIST1_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Marker {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) mapping: u16,
}

impl Marker {
    /// `Func_03775E`'s decoding of `d6` (nibble and X) and `d5` (Y).
    pub(crate) fn from_words(d6: u16, d5: u16) -> Self {
        Self {
            x: i32::from(d6 & 0xfff),
            y: i32::from(d5),
            mapping: FIRST_MAPPING + (d6 >> 12),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Markers {
    list: Vec<Marker>,
}

impl Markers {
    /// `Func_03771A`'s entry for `id`, or `None` for `$FFFF` and arenas.
    pub(crate) fn entry(rom: &[u8], id: StageId, arena: bool) -> Result<Option<Marker>> {
        if arena {
            return Ok(None);
        }
        let base = crate::rom::pointer(rom, TABLE)?;
        let slot = id.round_index() * 4 + id.substage_index() * 2;
        let at = base + usize::from(word(rom, TABLE + 4 + slot)?) + id.part_index() * 4;
        let d6 = word(rom, at)?;
        if d6 == 0xffff {
            return Ok(None);
        }
        Ok(Some(Marker::from_words(d6, word(rom, at + 2)?)))
    }

    /// `Objects_InitWorkspace` cleared list 1.
    pub(crate) fn clear(&mut self) {
        self.list.clear();
    }

    /// `Func_03775E`; a full list 1 drops the marker.
    pub(crate) fn add(&mut self, marker: Marker) {
        if self.list.len() < LIST1_CAPACITY {
            self.list.push(marker);
        }
    }

    /// `Func_037796` for every marker: a pending request frees them.
    pub(crate) fn tick(&mut self, requests: u8) {
        if requests & 0xf != 0 {
            self.list.clear();
        }
    }

    /// The markers shown on this `Frame_Counter`.
    pub(crate) fn visible(&self, frame_counter: u64) -> impl Iterator<Item = &Marker> {
        let shown = frame_counter & 0xc != 0;
        self.list.iter().filter(move |_| shown)
    }
}
