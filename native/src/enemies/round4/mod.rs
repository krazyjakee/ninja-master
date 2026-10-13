//! Round 4 enemy types (issue #51), ported from their handlers:
//!
//! | Type | Handler | Where | Native model |
//! | --- | --- | --- | --- |
//! | `$18` | `Enemy18` `$13F3A` | 4-1 | [`Guard`]: the jet-ski end guard: bombs and missile volleys |
//! | `$30` | `Enemy30` `$16558` | 4-2, 6-2 | [`Flamer`]: hovers and breathes flame bursts |
//! | `$38` | `Enemy38` `$166D6` | 4-2, 6-1, 6-2 | [`Circler`]: circles its post, throws aimed stars |
//! | `$3C` | `Enemy3C` `$168DA` | 4-1 | [`Flotsam`]: invisible floating-debris spawner |
//! | `$60` | `Enemy60` `$17BB6` | 4-1 (from `$6C`) | [`Jet`]: machine-gunner passes |
//! | `$68` | `Enemy68` `$1703C` | 4-2, 7-1 | [`Vent`]: timed flame jets |
//! | `$6C` | `Enemy6C` `$173B2` | 4-1 | [`Squadron`]: invisible `$60`/`$70` spawner |
//! | `$70` | `Enemy70` `$17474` | 4-1 (from `$6C`) | [`Jet`]: bomber passes |
//! | `$E4` | `EnemyE4` `$1AA48` | 4-2 | [`Slasher`]: steps in, slashes a wave, steps back |
//!
//! Each routine is the byte offset into the handler's `bra.w` table; the
//! comments cite the routine's address. Immediates and tables are read from
//! the ROM at load (each type's `tables_match_rom` test pins them).
//!
//! List-4 objects these types allocate become [`Projectile`]s. The ones
//! whose ROM handler changes them after launch are driven from their
//! thrower's update through `Projectile::owner` (the `$30`/`$68` flame
//! animations, the `$18` missiles) or idempotently by every round-4 object
//! ([`settle`]: a list-4 `$2C` bomb that reaches the water line becomes a
//! floating mine, `Func_0177B4`). List-9 sprite parts drawn with their
//! parent (the `$60`/`$70` wake and thruster) are one-frame effects.
//!
//! The wake splashes these types allocate (`Func_0176C6` in list 9,
//! `Func_0176DC` in list 1: type `$24`, routine `$10`, `+$28` = the life
//! in `d7`) are [`splash`]: the `$18` guard's glide, bob and death sprays,
//! the `$60`/`$70` `$1D6` thruster's, and a `$2C` bomb's or debris's as it
//! reaches the water ([`settle`], [`Flotsam`]).
//!
//! The `$18`'s list-9 jet-skis and glide flame (type `$3C`, `Func_0144E8`,
//! `Func_01452C`) are [`escort`] effects its update drives.
//!
//! Simplifications, documented rather than recovered: the `$18` player freeze
//! (`plr_event` 6), the `$68` layout damage
//! (`Func_017124`) and smoke trails. Player weapons do not destroy list-4
//! `$2C` bombs, mines or debris in the ROM either: only `Player_CheckDamage`
//! calls `Object_CollideL4`, and `Func_01776E` explodes the object
//! (`$36D9A`, [`Projectile::contact_blast`]) once that body contact set its
//! collision bit.
//! `$FF0190` (the water layer speed) is `crate::special::WATER_FLOW`.
use super::{Ctx, Roster, shot_damage};
use crate::{
    combat::{Enemy, Motion, Projectile, advance, ambient, spawn_records},
    rom::{Result, byte, signed, word, word_i},
    stage::{all_substages, part_count},
};
use std::{collections::BTreeSet, sync::Arc};

mod circler;
mod escort;
mod flamer;
mod flotsam;
mod guard;
mod jets;
mod slasher;
mod vent;
use circler::Circler;
use flamer::Flamer;
use flotsam::Flotsam;
use guard::Guard;
use jets::{Jet, Squadron};
use slasher::Slasher;
use vent::Vent;

pub(crate) const KINDS: &[u16] = &[0x18, 0x30, 0x38, 0x3c, 0x60, 0x68, 0x6c, 0x70, 0xe4];

pub(crate) fn register(rom: &[u8], roster: &mut Roster) -> Result<()> {
    let mines = Mines::load(rom)?;
    roster.add(0x18, Arc::new(Guard::load(rom, mines)?));
    roster.add(0x30, Arc::new(Flamer::load(rom)?));
    roster.add(0x38, Arc::new(Circler::load(rom)?));
    roster.add(0x3c, Arc::new(Flotsam::load(rom, mines)?));
    roster.add(0x60, Arc::new(Jet::load(rom, true, mines)?));
    roster.add(0x68, Arc::new(Vent::load(rom)?));
    roster.add(0x6c, Arc::new(Squadron::load(rom, mines)?));
    roster.add(0x70, Arc::new(Jet::load(rom, false, mines)?));
    roster.add(0xe4, Arc::new(Slasher::load(rom)?));
    Ok(())
}

/// `$FF0190`, the fast water layer's speed in whole pixels per tick.
fn water_speed() -> i32 {
    -crate::special::WATER_FLOW
}
/// `add.l d0,$20(a0)` / `add.l d0,$24(a0)`: 16.16 position nudges.
fn nudge_x(e: &mut Enemy, delta: i32) {
    advance(&mut e.x, &mut e.motion.fx, delta);
}
fn nudge_y(e: &mut Enemy, delta: i32) {
    advance(&mut e.y, &mut e.motion.fy, delta);
}
/// `subq.w #1,counter; bpl`: true while the counter has not gone negative.
fn count_down(counter: &mut i32) -> bool {
    *counter -= 1;
    *counter >= 0
}
/// Spawn records of `kind` in round `round` (1-based), for handlers that
/// test `Round_Index`.
pub(super) fn round_sources(rom: &[u8], round: u8, kind: u16) -> Result<BTreeSet<usize>> {
    let mut out = BTreeSet::new();
    for id in all_substages(rom)? {
        if id.round != round {
            continue;
        }
        for part in 0..part_count(rom, id)? {
            if let Ok(spawns) = spawn_records(rom, id.with_part(part as u8)) {
                out.extend(
                    spawns
                        .iter()
                        .filter(|spawn| spawn.kind == kind)
                        .map(|spawn| spawn.source),
                );
            }
        }
    }
    Ok(out)
}
/// A `(mapping, count)` list ending at a negative mapping word.
fn pairs(rom: &[u8], p: usize) -> Result<Vec<(u16, i32)>> {
    let mut out = Vec::new();
    for i in 0..32 {
        let mapping = word(rom, p + i * 4)?;
        if mapping & 0x8000 != 0 {
            return Ok(out);
        }
        out.push((mapping, signed(rom, p + i * 4 + 2)?));
    }
    Err("unterminated pair list".into())
}

// ---------------------------------------------------------------------------
// List-4 type $2C (`Func_01776E`): bombs, mines and floating debris.

/// A bomb (mapping `bomb`) falls until `obj_y >= line`, then shows the next
/// mapping and floats left with the water layer (`Func_0177B4`), leaving a
/// list-9 wake splash with `+$28` = `splash_life` at its position plus
/// `splash_offset` (`$17814`, `Func_0176A4`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Mines {
    bomb: u16,
    line: i32,
    damage: u8,
    splash_life: i32,
    splash_offset: (i32, i32),
}
impl Mines {
    fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            // $1773C: move.w #$1D9,$A(a6) in Func_0176F2.
            bomb: word(rom, 0x1773e)?,
            // $177B8: cmpi.w #$B8,d0.
            line: word_i(rom, 0x177ba)?,
            damage: shot_damage(rom, 0x2c)?,
            // $17814: moveq #$20,d7.
            splash_life: i32::from(byte(rom, 0x17815)?),
            // $176B0 / $176BC: addi.w #0,d0.
            splash_offset: (signed(rom, 0x176b2)?, signed(rom, 0x176be)?),
        })
    }
    /// `Func_0177B4` reaching the water at `(x, y)`: `$17814`'s splash.
    fn splash(&self, cx: &mut Ctx, (x, y): (i32, i32)) {
        let at = (x + self.splash_offset.0, y + self.splash_offset.1);
        splash(cx, at, self.splash_life, true);
    }
    /// `Object_AllocL4($2C)` at (x, y): a bomb with the given launch.
    fn throw(&self, cx: &mut Ctx, x: i32, y: i32, motion: Motion) -> bool {
        let mut bomb = Projectile::new(x, y, 0, true, self.bomb, self.damage).with_motion(motion);
        bomb.passes_terrain = true;
        bomb.left_edge_only = true;
        bomb.contact_blast = Some(cx.shared.wide_blast);
        bomb.owner = Some(cx.current);
        cx.shoot(bomb)
    }
}
/// `Func_0177B4` for every list-4 `$2C` bomb in flight: at the water line
/// the mapping advances, the motion becomes the water flow and a wake
/// splash goes up where it landed. Idempotent, so every round-4 object may
/// run it.
fn settle(cx: &mut Ctx, mines: &Mines) {
    let mut landed = Vec::new();
    for shot in cx.projectiles.iter_mut() {
        if shot.enemy && shot.alive && shot.mapping == mines.bomb && shot.y >= mines.line {
            shot.mapping = mines.bomb + 1;
            shot.motion = Motion::default();
            shot.velocity_x = -water_speed();
            landed.push((shot.x, shot.y));
        }
    }
    for at in landed {
        mines.splash(cx, at);
    }
}
/// `Func_0176C6` (list 9, `back`) or `Func_0176DC` (list 1): a type-`$24`
/// record with routine `$10` and `+$28` = `life` at `at`.
/// Routine `$10` is Boss4's wake splash ([`ambient::wake`]): it gets
/// its first update in the frame that allocates it.
fn splash(cx: &mut Ctx, at: (i32, i32), life: i32, back: bool) {
    // `$FF0190`, the water layer's speed (16.16).
    let water = water_speed() << 16;
    let effect = ambient::wake(&cx.shared.ambient, at, life, water, back);
    cx.effect(effect);
}

// 4-1 part 0, the jet-ski stage.
