//! Stage identity, the round/substage/part catalog and the per-substage
//! routine and music tables.
use super::{flags::ModeFlags, sections::sections};
use crate::rom::{Result, byte, pointer, word};
use std::fmt;

/// A playable section. `round` and `substage` are 1-based as displayed by the
/// game (1-1 .. 7-3); `part` is the 0-based index into the part records.
/// The ROM stores 0-based indices in `$FF37A6`, `$FF37A8` and `$FF37AA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StageId {
    pub round: u8,
    pub substage: u8,
    pub part: u8,
}
impl StageId {
    pub const FOREST: Self = Self::new(1, 1, 0);
    pub const fn new(round: u8, substage: u8, part: u8) -> Self {
        Self {
            round,
            substage,
            part,
        }
    }
    pub fn round_index(self) -> usize {
        usize::from(self.round).wrapping_sub(1)
    }
    pub fn substage_index(self) -> usize {
        usize::from(self.substage).wrapping_sub(1)
    }
    pub fn part_index(self) -> usize {
        usize::from(self.part)
    }
    pub fn with_part(self, part: u8) -> Self {
        Self { part, ..self }
    }
    /// Round 1-1 has its own dynamic forest effects and native play slice.
    pub fn is_forest(self) -> bool {
        self.round == 1 && self.substage == 1
    }
    /// Parse "R-S" (1-based), optionally followed by ":P" for the part.
    pub fn parse(value: &str) -> Result<Self> {
        let (stage, part) = value.split_once(':').unwrap_or((value, "0"));
        let (round, substage) = stage.split_once('-').ok_or("stage must be R-S")?;
        let parse = |v: &str| {
            v.parse::<u8>()
                .map_err(|_| format!("invalid stage {value}"))
        };
        Ok(Self::new(parse(round)?, parse(substage)?, parse(part)?))
    }
}
impl fmt::Display for StageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{} part {}", self.round, self.substage, self.part)
    }
}

/// `$688A`: number of rounds.
pub(super) fn round_count(rom: &[u8]) -> Result<usize> {
    Ok(usize::from(word(rom, 0x688a)?))
}
/// `$6284..$687C` substage tables (count word, then pointers).
pub(super) fn substage_count(rom: &[u8], round: u8) -> Result<usize> {
    let index = usize::from(round).wrapping_sub(1);
    if index >= round_count(rom)? {
        return Err(format!("round {round} does not exist"));
    }
    Ok(usize::from(word(rom, pointer(rom, 0x688c + index * 4)?)?))
}
pub fn part_count(rom: &[u8], id: StageId) -> Result<usize> {
    Ok(sections(rom, id.round_index(), id.substage_index())?.len())
}
/// Every substage (part 0), in ROM order.
pub fn all_substages(rom: &[u8]) -> Result<Vec<StageId>> {
    let mut out = Vec::new();
    for round in 1..=round_count(rom)? as u8 {
        for substage in 1..=substage_count(rom, round)? as u8 {
            out.push(StageId::new(round, substage, 0));
        }
    }
    Ok(out)
}

/// `Mode_Game` state `$20` (`$B0C6..$B148`) after a `$FF0029` bit 1 request:
/// part 0 of the next substage, else substage 1 of the next round (through
/// `Mode_RoundClear`), or `None` past the last round (`Mode_Ending`).
pub fn next_substage(rom: &[u8], id: StageId) -> Result<Option<StageId>> {
    if usize::from(id.substage) < substage_count(rom, id.round)? {
        return Ok(Some(StageId::new(id.round, id.substage + 1, 0)));
    }
    if usize::from(id.round) < round_count(rom)? {
        return Ok(Some(StageId::new(id.round + 1, 1, 0)));
    }
    Ok(None)
}
/// `$4AC8`, called by the stage load `$119F2`: boss arenas (`$FF39F7` bit 0)
/// take `$6156 + round`, other parts `$60D6 + round*16 + (substage&1)*8 +
/// part`. `$4A36` then plays the first byte of the `$5986 + id*16` sound-test
/// record (1-1 -> `$93` JAPONESQUE, 1-2 -> `$81` HE RUNS).
pub fn stage_music(rom: &[u8], id: StageId) -> Result<u8> {
    let (r, s, p) = (id.round_index(), id.substage_index(), id.part_index());
    let all = sections(rom, r, s)?;
    let section = all
        .get(p)
        .ok_or_else(|| format!("{id}: substage has {} parts", all.len()))?;
    let index = if ModeFlags(section.mode_flags).boss_arena() {
        byte(rom, 0x6156 + r)?
    } else {
        byte(rom, 0x60d6 + r * 16 + (s & 1) * 8 + (p & 7))?
    };
    byte(rom, 0x5986 + usize::from(index) * 16)
}
/// `$ACC8..$ACCA`: the final-exit camera lock plays sound-test record 4
/// (`$85` HASSOU!) when the record has a word-12 Y target.
pub fn camera_lock_music(rom: &[u8]) -> Result<u8> {
    byte(rom, 0x5986 + 4 * 16)
}

/// Per-substage routine pointers. `animation` comes from `$39602 +
/// round*8 + substage*4`: consecutive rounds' entries overlap, so for example
/// 1-3 and 2-1 share one slot. `$390E2` skips it in boss arenas.
/// `StageEffects` interprets `palette`, `animation` and `scenery`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageRoutines {
    /// `$1D5A` per-frame scroll routine (`$1D0A`).
    pub scroll: u16,
    /// `$2FC8` plane B setup (`$2FA6`).
    pub plane_b: u16,
    /// `$9C0` palette effect routine (`$894`); `$0A00` is `rts`.
    pub palette: u16,
    /// `$39602` tile animation / stage event routine; `$390E0` is `rts`.
    pub animation: usize,
    /// `$39CD0 + round*16 + substage*4` scenery sprite list (`$39BE4`).
    pub scenery: usize,
}

/// Routine table entries for 0-based round and substage indices.
pub(super) fn stage_routines(rom: &[u8], r: usize, s: usize) -> Result<StageRoutines> {
    Ok(StageRoutines {
        scroll: word(rom, 0x1d5a + r * 8 + s * 2)?,
        plane_b: word(rom, 0x2fc8 + r * 8 + s * 2)?,
        palette: word(rom, 0x9c0 + r * 8 + s * 2)?,
        animation: pointer(rom, 0x39602 + r * 8 + s * 4)?,
        scenery: pointer(rom, 0x39cd0 + r * 16 + s * 4)?,
    })
}
