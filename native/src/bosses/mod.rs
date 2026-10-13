//! Boss layer (issues #41, #52).
//!
//! `Mode_StageLoad` calls `Boss_Setup` (`$2A3AE`): in a boss arena
//! (`Stage_InhibitFlags` bit 0, the part record's `ModeFlags::boss_arena`)
//! `Boss_Spawn` (`$2A3C4`) allocates the list-8 record from
//! `Boss_Records[Round_Index]` (`$2A32E`, 16 bytes per round: X, Y, BCD
//! score, damage byte, health byte, `obj_dx`, `obj_dy`) and `Boss_LoadArt`
//! decompresses the round's art. `Boss_Update` (`$2A4A4`) then jumps every
//! frame through `Boss_Jumps` (`$2A4D4`, one `jmp.l` per round; round 8
//! repeats `Boss7`) with a0 = boss and a3 = player.
//!
//! Natively the list-8 record is an [`Enemy`] of the pseudo type
//! [`BOSS_KIND`] appended to `Combat::enemies` at load, so weapons, contact
//! damage, effects and drawing reuse the enemy paths and it runs from the
//! existing `Combat::tick`. Each round's handler is a [`Behavior`] in its own
//! module (`boss1` .. `boss7`), so the seven bosses can be ported in
//! parallel; a boss's list-5 parts (`$78` hydra, `$7C` mech) are registered
//! from the same module (`PART_KINDS`, `register_parts`) and allocated with
//! `Ctx::spawn`. A finished death sequence calls `Ctx::next_substage`
//! (`$FF0029` bit 1, then `Mode_RoundClear`), one of the `combat::Signal`s
//! the game drains after the pass.
//!
//! All seven rounds are ported; a handler whose art the renderer cannot
//! draw yet reports `Behavior::stand_in` and is outlined.
//!
//! Not modelled here: `Boss_LoadArt` is `stage.rs`'s concern (its VRAM
//! overlay order is pinned by the emulator tile goldens there).
use crate::{
    combat::{Enemy, EnemyState, Spawn},
    enemies::{Behavior, Ctx},
    rom::{Result, byte, pointer, signed, signed_byte, word},
    stage::StageId,
};
use std::sync::Arc;

pub(crate) mod boss1;
pub(crate) mod boss2;
pub(crate) mod boss3;
pub(crate) mod boss4;
pub(crate) mod boss5;
pub(crate) mod boss6;
pub(crate) mod boss7;
mod contact;
mod palette;

pub(crate) use palette::{Flasher, HealthPalette, fade_lines, fade_white_lines};

/// Pseudo type offset of the list-8 boss record (list-5 types are
/// `$00..$FC`; `Boss_Spawn` allocates list 8 with d0 = 0).
pub const BOSS_KIND: u16 = 0x100;
/// `Boss_Records`: 16 bytes per round.
pub const BOSS_RECORDS: usize = 0x2a32e;
/// `Boss_Jumps` + 2: the first `jmp.l` (`$4EF9`) of the per-round table.
pub const BOSS_JUMPS: usize = 0x2a4d4;

/// One `Boss_Records` entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BossRecord {
    /// 0-based `Round_Index`.
    pub round_index: usize,
    /// ROM address of the entry.
    pub source: usize,
    pub x: i32,
    pub y: i32,
    pub score_bcd: u16,
    pub damage: u8,
    pub health: u8,
    /// Initial `obj_dx`/`obj_dy` words.
    pub dx: i16,
    pub dy: i16,
    /// `Boss_Jumps` target: the round's handler.
    pub handler: usize,
}
impl BossRecord {
    pub fn load(rom: &[u8], round_index: usize) -> Result<Self> {
        if round_index >= 8 {
            return Err(format!("no boss record for round index {round_index}"));
        }
        let source = BOSS_RECORDS + round_index * 16;
        let jump = BOSS_JUMPS + round_index * 6;
        if word(rom, jump)? != 0x4ef9 {
            return Err(format!("Boss_Jumps entry at ${jump:05x} is not jmp.l"));
        }
        let stats = word(rom, source + 6)?;
        Ok(Self {
            round_index,
            source,
            x: signed(rom, source)?,
            y: signed(rom, source + 2)?,
            score_bcd: word(rom, source + 4)?,
            damage: (stats >> 8) as u8,
            health: stats as u8,
            dx: word(rom, source + 8)? as i16,
            dy: word(rom, source + 10)? as i16,
            handler: pointer(rom, jump + 2)?,
        })
    }
}

/// `Boss_Setup`: the part's boss record, or `None` outside boss arenas.
pub fn record(rom: &[u8], stage: StageId) -> Result<Option<BossRecord>> {
    if !crate::combat::boss_arena(rom, stage)? {
        return Ok(None);
    }
    BossRecord::load(rom, stage.round_index()).map(Some)
}

/// `Boss_Spawn`: the list-8 record as a synthetic spawn plus its object,
/// live from the first frame (no proximity activation).
pub(crate) fn spawn(record: &BossRecord, index: usize) -> (Spawn, Enemy) {
    let spawn = Spawn {
        source: record.source,
        x: record.x,
        y: record.y,
        flags: 0x8000,
        kind: BOSS_KIND,
        handler: record.handler,
        score_bcd: record.score_bcd,
        contact_damage: record.damage,
        health_counter: record.health,
        placeholder_mapping: 0,
    };
    let mut enemy = Enemy::new(index, record.x, record.y, 0, u16::from(record.health) + 1);
    // $2A40A: obj_tile $2000 (palette line 1), routine 0.
    enemy.state = EnemyState::Waiting;
    enemy.tile = Some(0x2000);
    (spawn, enemy)
}

/// List-5 types spawned and owned by a boss module (e.g. the `$78` hydra
/// parts, the `$7C` mech parts), whether or not their round is an arena.
pub fn part_kinds() -> impl Iterator<Item = u16> {
    [
        boss1::PART_KINDS,
        boss2::PART_KINDS,
        boss3::PART_KINDS,
        boss4::PART_KINDS,
        boss5::PART_KINDS,
        boss6::PART_KINDS,
        boss7::PART_KINDS,
    ]
    .into_iter()
    .flatten()
    .copied()
}
/// Register every boss module's list-5 part types.
pub(crate) fn register_parts(rom: &[u8], roster: &mut crate::enemies::Roster) -> Result<()> {
    boss1::register_parts(rom, roster)?;
    boss2::register_parts(rom, roster)?;
    boss3::register_parts(rom, roster)?;
    boss4::register_parts(rom, roster)?;
    boss5::register_parts(rom, roster)?;
    boss6::register_parts(rom, roster)?;
    boss7::register_parts(rom, roster)
}

/// `Data_009748` sine of a 2048-step angle (word offset `angle & $7FE`).
pub(super) fn sine(cx: &Ctx, angle: i32) -> i32 {
    i32::from(cx.shared.sine.word(angle as u16))
}

/// `Func_034B04`: clamp both velocities to +-`cap`, clearing that axis's
/// acceleration when it is exceeded.
pub(super) fn cap_speed(e: &mut Enemy, cap: i32) {
    if e.motion.vx.abs() > cap {
        e.motion.ax = 0;
        e.motion.vx = cap * e.motion.vx.signum();
    }
    if e.motion.vy.abs() > cap {
        e.motion.gravity = 0;
        e.motion.vy = cap * e.motion.vy.signum();
    }
}

/// `Func_034B70`'s 13 explosion offsets (`$34B56`, signed byte pairs).
pub(super) fn blast_offsets(rom: &[u8]) -> Result<Vec<(i32, i32)>> {
    (0..13)
        .map(|i| {
            Ok((
                signed_byte(rom, 0x34b56 + i * 2)?,
                signed_byte(rom, 0x34b57 + i * 2)?,
            ))
        })
        .collect()
}

/// `Data_034A00`: 64 (cos, sin) pairs in 4.12 over half a turn.
pub(super) fn half_turn(rom: &[u8]) -> Result<Vec<(i32, i32)>> {
    (0..64)
        .map(|i| Ok((signed(rom, 0x34a00 + i * 4)?, signed(rom, 0x34a02 + i * 4)?)))
        .collect()
}
/// `Func_01F820`: 16.16 unit vector of a 128-step `angle` (counter-clockwise
/// from the right, up negative) from the [`half_turn`] table.
pub(super) fn unit_vector(half_turn: &[(i32, i32)], angle: u8) -> (i32, i32) {
    let (c, s) = half_turn
        .get(usize::from(angle & 63))
        .copied()
        .unwrap_or((0x1000, 0));
    let (x, y) = (c << 4, s << 4);
    if angle & 0x40 != 0 { (-x, y) } else { (x, -y) }
}

/// `Data_01F68E`: [`aim_angle`]'s 64 slope bytes, the angle (128 per turn)
/// of `16 * dy / dx` below `$40`.
pub(super) fn slopes(rom: &[u8]) -> Result<[u8; 64]> {
    let mut out = [0; 64];
    for (i, value) in out.iter_mut().enumerate() {
        *value = byte(rom, 0x1f68e + i)?;
    }
    Ok(out)
}
/// `Func_01F5DE`: the 128-step angle ([`unit_vector`]'s convention) from
/// `from` (`$20/$24(a0)`) to `to` (the player's `$20`, `$24 - $10`).
/// `16 * |dy| / |dx|` (`divu.w`) indexes the [`slopes`] table below `$40`,
/// then steps through `$1B..$1F` at `$51/$6C/$A3/$100`; the quadrant is
/// folded in by signed word compares. Straight right with the target
/// level is `$80`, not 0, as in the ROM.
pub(super) fn aim_angle(slopes: &[u8; 64], from: (i32, i32), to: (i32, i32)) -> u8 {
    let (x, y) = (from.0 as i16, from.1 as i16);
    let (tx, ty) = (to.0 as i16, to.1 as i16);
    let dx = tx.wrapping_sub(x).unsigned_abs();
    let dy = ty.wrapping_sub(y).unsigned_abs();
    let base: u8 = if dx == 0 {
        0x20
    } else if dy == 0 {
        0
    } else {
        // ext.l / asl.l #4 / divu.w: on overflow d1 keeps its low word.
        let dividend = ((dy as i16 as i32) << 4) as u32;
        let quotient = dividend / u32::from(dx);
        let slope = if quotient > 0xffff {
            dividend & 0xffff
        } else {
            quotient
        };
        match slope {
            0..0x40 => slopes[slope as usize],
            0x40..0x51 => 0x1b,
            0x51..0x6c => 0x1c,
            0x6c..0xa3 => 0x1d,
            0xa3..0x100 => 0x1e,
            _ => 0x1f,
        }
    };
    match (ty < y, tx < x) {
        (true, false) => base,
        (true, true) => 0x40 - base,
        (false, true) => base + 0x40,
        (false, false) => 0x80 - base,
    }
}

/// `Boss_Jumps[round_index]`: the round's handler (round 8 repeats round 7).
pub(crate) fn fight(rom: &[u8], round_index: usize) -> Result<Arc<dyn Behavior>> {
    match round_index {
        0 => boss1::fight(rom),
        1 => boss2::fight(rom),
        2 => boss3::fight(rom),
        3 => boss4::fight(rom),
        4 => boss5::fight(rom),
        5 => boss6::fight(rom),
        6 | 7 => boss7::fight(rom),
        _ => Err(format!("no boss handler for round index {round_index}")),
    }
}

// Round `round`'s arena (substage 3, part 0) and its boss record's index.
