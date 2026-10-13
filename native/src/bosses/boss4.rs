//! Round 4 boss: `Boss4` (`$2D9E8`), called through `Boss_Jumps`.
//!
//! 4-3 at sea, fought from the surfboard. The boss is two machines drawn in
//! plane A (`Func_00429A` tile maps `Data_02F416` 25x9 and `Data_02F566`
//! 18x11, base `$A320`, each after `Func_00459C` clears the plane),
//! positioned by the scroll registers from the record's X/Y, with list-1
//! sprite parts chained through `+$3C`. `Func_02EE38`, which every routine
//! returns through, writes `X - Camera_X - (+$40)` to the plane A word of
//! H-scroll rows 3..27 (`HScroll_Buffer+$60`, VDP register `$0B` = 2: one
//! word per eight lines) and `-(Y - Camera_Y - (+$42))` to `VScroll_PlaneA`,
//! over the surf values `$23C0` wrote earlier in the frame. Rows 0..2 keep
//! `$23C0`'s slow band under the HUD window (register `$12` = 3). Natively
//! the hull is a [`crate::stage::BossPlanes::Hull`] plane scrolled by
//! [`PlaneOp::Scroll`]:
//!
//! - Phase 1, the gunboat (routines `$00..$28`): it surges up out of the
//!   water (`Func_02EE38` bob, amplitude `$60`), moves in to X `$140` and
//!   fires two volleys of three cannon shells (list-4 `$34` routine 0,
//!   `$2A6`, 8 px/tick, damage 2), sinking 32 px between them; backs off and
//!   launches three homing missiles (`Data_02DE78`, list-4 `$34` routine 8:
//!   an elliptic launch arc, a hover that turns toward the player, then
//!   4 px/tick straight); then either repeats or lunges (routine `$18`,
//!   X sway `$20` then `$5F`), charges at X `$150` behind steam for 128
//!   ticks and surges. Every hit on its hull (`$F8083820`) subtracts the
//!   weapon's damage from `+$2D` = 30 with four ticks of immunity; below
//!   zero it sinks for 512 ticks (routine `$2C`).
//! - Phase 2, the flying machine (routines `$30..$80`), health `+$2D` = 28,
//!   rises out of the water (routine `$30`: its first two parts are pulled
//!   up from the sea on chains of sixteen segments, then take their roles
//!   and poses). Its two lower parts must be destroyed first: the gun (part
//!   2, role `$0C`, `$2FE4C`, 13) and then the arm (part 1, role `$10`,
//!   `$2FF94`, 13, hittable only once the gun is gone); only then do hits
//!   on the hull (`$FC082C1C`) count (`$2DA74`). Its attack cycle
//!   (`Data_02E5A4`): with both parts, routines `$34..$80`: diagonal shells
//!   from the gun (list-4 routine `$18`, `$2A0`, -8/+4 px/tick, damage 2),
//!   six homing missiles (`Data_02E776`), a sweep to the left over the
//!   water, six more from the other side (`Data_02E888`), bombs while
//!   returning (list-4 routine `$10`), and a four-shell spread (list-4
//!   routine `$28`/`$30`, damage 3). With the gun gone, routines
//!   `$64..$78`: the arm's reach and a machine-gun strafe along the water.
//!   With the arm gone, the cycle starts at `$44`.
//! - Death (routines `$84`, `$88`): the machine drifts off to the left
//!   through explosions and smoke, then `$FF0029` bit 1
//!   ([`Ctx::next_substage`]).
//!
//! The sprite parts are list-1 type-`$2B` records ([`parts`], drawn as
//! [`crate::combat::EffectKind::BossPart`] effects): the gunboat's cannon
//! (recoil), stowed ram and bow glow, the flying machine's arm, gun, two
//! hull pieces and five fires. Natively the list-8 record is one
//! [`Enemy`]: its collision box is the hull or, in phase 2, the weak part's
//! own box where the part is while it takes hits (the hull turns weapons
//! away meanwhile). `Enemy::health` is held at a sentinel and the ROM
//! counters are kept in the record and the parts, so only the last hull
//! hit is lethal (and scored) centrally.
//!
//! Every update starts at `$2D9E8` by pointing the H-interrupt at
//! `IRQ_Default` or, before routine `$88`, at `Func_02EF80` with VDP
//! register `$0A` = `$C7` and the interrupt enabled (`$8014`). After line
//! 199 that handler waits `(d0 & $3F) + 9` short loops and writes
//! `Data_0303BE` over CRAM line 1, and sets `Sys_Flags` bit 1 so V-blank
//! uploads the `$FFF000` copy again: the hull's rows from line 200 down
//! turn to the blue of the water around them ([`PaletteOp::HInt`],
//! `Game` applies the ROM's other conditions).
//!
//! The water around the hull is type-`$24` splash objects
//! ([`crate::combat::ambient`]). `Func_02EE38` leaves a wake splash
//! (routine `$10`) every fourth update while either machine's Y is in
//! `$170..$200`, at X or X + 64 on the water line ([`Splashes`]); they are
//! high-priority sprites on palette line 2, so they cover the hull's high
//! plane A cells and the line-199 reload does not recolour them. The
//! sinking gunboat throws up plumes (routine 8) below Y `$200`
//! ([`Plumes`]).
//!
//! The sinking gunboat smokes and sheds burning debris (`Func_02F146`,
//! `Func_02F178`, [`Sinking`]); the dying machine smokes too.
//!
//! The charge's steam, palette flash and fade, the plane-A cells beside
//! the bow and the bow wave are in [`effects`].
//!
//! Remaining differences: missile turn-rate/launch timing and exhaust sparks;
//! the strafe's flash, casings, smoke and stationary water hazard (currently
//! a damaging moving tracer); projectile trails and spread-launcher contact;
//! and weapons meeting fixed parts/chain segments. Hull and fixed-part
//! player contacts are handled by `touch`; the arm separately deals 2.
//! Emulator evidence: `obj/emu/states/r4-3.state` with the player's health
use super::sine;
use crate::{
    combat::{
        Blast, CramReload, Effect, EffectKind, Enemy, EnemyState, Hitbox, Motion, PaletteOp, Plane,
        PlaneOp, Projectile, Reaction, ShotDown, Spawn, SpriteList, ambient,
    },
    enemies::{Behavior, Ctx, HitResponse, Roster},
    game::Terrain,
    movement::PlayerEvent,
    rom::{Result, byte, pointer, signed, word, words},
    sound_events::Table,
    stage::{FLYER_MAP, GUNBOAT_MAP},
};
use parts::{ARM_PART, CANNON, GLOW, GUN_PART, Parts, RAM_PART, destroyed, part, step_toward};
use std::sync::Arc;

/// `Boss_Jumps` target of this round.
pub(crate) const HANDLER: usize = 0x2d9e8;

pub(crate) fn fight(rom: &[u8]) -> Result<Arc<dyn Behavior>> {
    Ok(Arc::new(Boss4::load(rom)?))
}

/// List-5 types this boss spawns and owns (registered by [`register_parts`]).
/// `Boss4`'s parts and shots are list-1, list-4 (`$34`) and list-9 objects.
pub(crate) const PART_KINDS: &[u16] = &[];

pub(crate) fn register_parts(_rom: &[u8], _roster: &mut Roster) -> Result<()> {
    Ok(())
}

/// `+$28` timer, `+$2A` sub-counter, the `Func_02EE38` waves (`+$44..$4B`:
/// step and amplitude bytes packed, angle words), `+$2E` hit immunity (in
/// phase 2 the weak part's) and the death drift (`+$1A`); `+$30..$34` of
/// the part that reaches ([`parts`]) and whether the weak part takes hits.
/// `Enemy::home_x`/`launch_y` are `+$22`/`+$26`, `Enemy::step_x` the
/// current ROM health counter, `Enemy::phase` the stage.
const T: usize = 0;
const S: usize = 1;
const X_WAVE: usize = 2;
const X_ANGLE: usize = 3;
const Y_WAVE: usize = 4;
const Y_ANGLE: usize = 5;
const COOL: usize = 6;
const DRIFT: usize = 7;
const P30: usize = 8;
const P32: usize = 9;
const P34: usize = 10;
const OPEN: usize = 11;

/// `Enemy::phase`: the gunboat, then the flying machine while its gun, its
/// arm and at last its hull take the hits.
const GUNBOAT: u8 = 0;
const PART_C: u8 = 1;
const ARM: u8 = 2;
const HULL: u8 = 3;

/// `Enemy::health` outside the final stage: hits are measured against it.
const SENTINEL: u16 = 0x4000;
/// Marks this boss's missiles in `Projectile::life` (table << 2 | entry).
const MISSILE_TAG: u16 = 0x8000;
/// The water line where shells and bombs end (`$2F7C4`, `$2FA9E`).
const WATER: i32 = 0x1c8;

/// `Func_02EE38`'s waterline splashes (`$2EEAE..$2EF0C`): while the
/// machine's Y is in `rows`, every update with `+$3A & period` = 0
/// allocates a type-`$24` wake splash (routine `$10`, `+$28` = `life`) at
/// X + ((`+$3A` & `spread`) << `shift`) on the water at `water` +
/// (Y & `y_mask`); in list 9 while `+$3A` has `back` set, else list 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Splashes {
    rows: (i32, i32),
    period: u16,
    back: u16,
    spread: u16,
    shift: u32,
    y_mask: i32,
    water: i32,
    life: i32,
}
impl Splashes {
    fn read(rom: &[u8]) -> Result<Self> {
        // cmpi.w #,d0 / andi.w #,d0 / moveq #$24,d0 / btst #,$3B(a0) /
        // move.w #,4(a6) / lsl.w #,d0 / addi.w #,d0 / move.w #,$28(a6).
        let opcodes = [
            (0x2eeae, 0x0c40),
            (0x2eeb4, 0x0c40),
            (0x2eebe, 0x0240),
            (0x2eec4, 0x7024),
            (0x2eec6, 0x0828),
            (0x2eede, 0x3d7c),
            (0x2eee8, 0x0240),
            (0x2eefa, 0x0240),
            (0x2eefe, 0x0640),
            (0x2ef06, 0x3d7c),
        ];
        for (at, opcode) in opcodes {
            if word(rom, at)? != opcode {
                return Err(format!("Func_02EE38's splash code differs at ${at:05x}"));
            }
        }
        // The routine word and the list-9 test's byte (`$3B`, the low byte
        // of `+$3A`).
        if word(rom, 0x2eee0)? != 0x10 || word(rom, 0x2eeca)? != 0x3b {
            return Err("Func_02EE38 allocates another splash".into());
        }
        let lsl = word(rom, 0x2eeec)?;
        if lsl & 0xf1ff != 0xe148 {
            return Err(format!("${lsl:04x} at $2EEEC is not lsl.w #n,d0"));
        }
        Ok(Self {
            rows: (signed(rom, 0x2eeb0)?, signed(rom, 0x2eeb6)?),
            period: word(rom, 0x2eec0)?,
            back: 1 << word(rom, 0x2eec8)?,
            spread: word(rom, 0x2eeea)?,
            // A count of 0 encodes 8.
            shift: match (lsl >> 9) & 7 {
                0 => 8,
                count => u32::from(count),
            },
            y_mask: signed(rom, 0x2eefc)?,
            water: signed(rom, 0x2ef00)?,
            life: signed(rom, 0x2ef08)?,
        })
    }
    /// The splash this update allocates, if any: position and list 9.
    fn at(&self, counter: u16, (x, y): (i32, i32)) -> Option<((i32, i32), bool)> {
        // cmpi.w/bcs, cmpi.w/bcc: an unsigned Y range.
        let row = y as u16 as i32;
        if !(self.rows.0..self.rows.1).contains(&row) || counter & self.period != 0 {
            return None;
        }
        let dx = i32::from((counter & self.spread) << self.shift);
        Some((
            (x + dx, self.water + (y & self.y_mask)),
            counter & self.back != 0,
        ))
    }
}

/// The sinking gunboat's plumes (`$2E312..$2E362`): with its Y at `below`
/// or more and `+$28` still at `ticks` or more, every update with
/// `+$28 & period` = 0 requests effect `sound` and allocates a list-1
/// type-`$24` plume (routine 8) at X + (`$FF001C` byte & `spread`) on the
/// water at `water`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Plumes {
    below: i32,
    ticks: i32,
    period: i32,
    sound: u8,
    spread: i32,
    water: i32,
}
impl Plumes {
    fn read(rom: &[u8]) -> Result<Self> {
        // cmpi.w #,$24(a0) / cmpi.w #,$28(a0) / andi.w #,d0 / moveq #,d0 /
        // moveq #$24,d0 / move.w #8,4(a6) / andi.w #,d0 / move.w #,$24(a6).
        let opcodes = [
            (0x2e312, 0x0c68),
            (0x2e31a, 0x0c68),
            (0x2e326, 0x0240),
            (0x2e33c, 0x7024),
            (0x2e346, 0x3d7c),
            (0x2e352, 0x0240),
            (0x2e35e, 0x3d7c),
        ];
        for (at, opcode) in opcodes {
            if word(rom, at)? != opcode {
                return Err(format!("Func_02E1EA's plume code differs at ${at:05x}"));
            }
        }
        if word(rom, 0x2e330)? >> 8 != 0x70 || word(rom, 0x2e348)? != 8 {
            return Err("Func_02E1EA allocates another splash".into());
        }
        Ok(Self {
            below: signed(rom, 0x2e314)?,
            ticks: signed(rom, 0x2e31c)?,
            period: signed(rom, 0x2e328)?,
            sound: byte(rom, 0x2e331)?,
            spread: signed(rom, 0x2e354)?,
            water: signed(rom, 0x2e360)?,
        })
    }
    /// Where this update's plume goes, if any: `timer` is `+$28` after
    /// the update's `subq`, `random` the `$FF001C` word (not advanced).
    fn at(&self, timer: i32, (x, y): (i32, i32), random: u16) -> Option<(i32, i32)> {
        // Both cmpi.w/bcs tests are unsigned.
        let low = y as u16 >= self.below as u16;
        if !low || (timer as u16) < self.ticks as u16 || timer & self.period != 0 {
            return None;
        }
        Some((x + (i32::from(random >> 8) & self.spread), self.water))
    }
}

/// `Func_02F146`/`Func_02F3B4` (smoke) and `Func_02F178` (debris): each
/// placed at the boss plus the `$FF001C` bytes shifted right (the high
/// byte by 1 for X, the low byte by 2 for Y) without advancing it.
/// Smoke is a type-`$1E` record at routine 4 (`$36FBC`: `Data_036F46`, a
/// frame every other update), tile `$8000`, drifting with the water
/// (`-$FF0190`) and rising 1 px per frame, in list 9 when the high byte
/// is odd. Debris is a list-9 `$2B` record at routine `$14` (a `+$28`
/// countdown), mapping `$EE` with tile `$A3B0` + (r & `$F`), half the
/// water's drift and gravity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sinking {
    smoke: (u16, u16),
    rise: i32,
    lift: i32,
    debris: (u16, u16),
    gravity: i32,
    life: i32,
    /// `$2E36E`: the plume's smoke is `$80` higher.
    plume_smoke: i32,
}
impl Sinking {
    fn read(rom: &[u8]) -> Result<Self> {
        // moveq #$1E,d0 / moveq #$2B,d0 / move.w #$FFFF,$1C(a6) /
        // subi.w #$80,$24(a6).
        if word(rom, 0x2f3b4)? != 0x701e
            || word(rom, 0x2f198)? != 0x702b
            || word(rom, 0x2f3e8)? != 0x3d7c
            || word(rom, 0x2e36e)? != 0x046e
        {
            return Err("Boss4's smoke and debris code differs".into());
        }
        Ok(Self {
            smoke: (word(rom, 0x2f3f0)?, word(rom, 0x2f3d8)?),
            rise: signed(rom, 0x2f3ea)?,
            lift: i32::from(byte(rom, 0x2f3f5)?),
            debris: (word(rom, 0x2f1aa)?, word(rom, 0x2f1ba)?),
            gravity: pointer(rom, 0x2f1dc)? as i32,
            life: signed(rom, 0x2f1e4)?,
            plume_smoke: signed(rom, 0x2e370)?,
        })
    }
    fn at(random: u16, (x, y): (i32, i32)) -> (i32, i32) {
        (
            x + (i32::from((random >> 8) as u8 as i8) >> 1),
            y + (i32::from(random as u8 as i8) >> 2),
        )
    }
}

/// One `Func_02F0F6` record: launch offset, arc angle and step (2048 per
/// turn), arc radii, arc and hover ticks, centre drift (px/tick).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Missile {
    offset: (i32, i32),
    angle: i32,
    step: i32,
    radii: (i32, i32),
    arc: i32,
    hover: i32,
    drift: (i32, i32),
}
impl Missile {
    fn read(rom: &[u8], p: usize) -> Result<Self> {
        let s = |i: usize| signed(rom, p + i * 2);
        Ok(Self {
            offset: (s(0)?, s(1)?),
            angle: s(2)?,
            step: s(3)?,
            radii: (s(4)?, s(5)?),
            arc: s(6)?,
            hover: s(7)?,
            drift: (s(8)?, s(9)?),
        })
    }
}

/// A `Data_02FC4A` spread shell: offset, speed (whole px) and acceleration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spread {
    offset: (i32, i32),
    speed: (i32, i32),
    accel: (i32, i32),
}

pub(crate) struct Boss4 {
    parts: Parts,
    fx: effects::Effects,
    /// `$2E4E6`: the arm's health, set when the intro hands the arm and
    /// the gun their roles, with effect `$2C` (`$2E4FE`).
    arm_health: i32,
    handover: u8,
    /// `$2EF3A`: `Func_02EE38`'s item: kind, X and `Data_02EF78`'s rows.
    item: (u8, i32, [i32; 4]),
    /// `$2DDD2`: the cannon's muzzle flash, mapping and offset.
    muzzle: (u16, i32, i32),
    /// `Func_02F08E`'s charge end (`$2E054`): effect `$37`, and the
    /// sixteen wakes from X - `$100` on the water (`$2E0B0`).
    charge_end: u8,
    /// `Func_02EF80`'s CRAM reload below register `$0A`'s line.
    reload: CramReload,
    /// Func_02F250 restores twelve player colours and the hull's full line.
    hit_restore: ([u16; 12], [u16; 16]),
    splashes: Splashes,
    plumes: Plumes,
    sinking: Sinking,
    hulls: [Hitbox; 2],
    /// `Func_02F1EA`'s damage (`move.b #1,7(a3)`) below and from routine
    /// `$30`.
    touch_damage: [u8; 2],
    /// `+$40`/`+$42` of each hull: its plane A scroll offsets.
    scroll_offsets: [(i32, i32); 2],
    starts: [(i32, i32); 2],
    /// `$44`/`$48` longs of each phase: (step, amplitude, angle) x2.
    waves: [[(i32, i32, i32); 2]; 2],
    health: [i32; 2],
    rise_ticks: i32,
    intro_sound: u8,
    shell: (u16, i32, u8),
    shell2: (u16, (i32, i32), (i32, i32), u8),
    bomb: (u16, i32, u8),
    spread: Vec<Spread>,
    spread_mapping: u16,
    charge_frames: [u16; 14],
    missiles: [[Missile; 3]; 3],
    missile_damage: u8,
    headings: [u16; 16],
    heading_flips: [u16; 4],
    tracer: u16,
    voice: u8,
    sounds: Sounds,
}
/// `$4A4E` effect IDs this boss requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sounds {
    cannon: u8,
    missile: u8,
    shell: u8,
    bomb: u8,
    spread: u8,
    charge: u8,
    gun: u8,
    sinking: u8,
    blast: u8,
    lunge: u8,
    surge: u8,
}

fn hitbox(long: u32) -> Hitbox {
    Hitbox {
        x: i32::from((long >> 24) as u8 as i8),
        y: i32::from((long >> 16) as u8 as i8),
        half_width: i32::from((long >> 8) as u8),
        half_height: i32::from(long as u8),
    }
}
fn wave(long: u32) -> [(i32, i32, i32); 2] {
    let b = |shift: u32| i32::from((long >> shift) as u8 as i8);
    [(b(24), b(16), (long & 0xffff) as i32), (0, 0, 0)]
}

/// `$2DA2A`: `move.w #$8AC7,(VDP_CTRL)`, register `$0A`.
const LINE_COUNTER: usize = 0x2da2c;
/// `Func_02EF80`: `lea Data_0303BE(pc),a0` and the CRAM write command.
const RELOAD_TABLE: usize = 0x2ef8a;
const RELOAD_COMMAND: usize = 0x2ef98;

/// `Func_02EF80` as Boss4 installs it: the interrupt comes after line
/// `$0A` (199), so the eight `move.l` writes land from line 200.
fn cram_reload(rom: &[u8]) -> Result<CramReload> {
    let counter = word(rom, LINE_COUNTER)?;
    if counter >> 8 != 0x8a
        || word(rom, RELOAD_TABLE)? != 0x41fa
        || word(rom, RELOAD_COMMAND)? != 0x237c
    {
        return Err("Func_02EF80 is not where Boss4 installs it".into());
    }
    let table = (RELOAD_TABLE as i64 + 2 + i64::from(signed(rom, RELOAD_TABLE + 2)?)) as usize;
    let command = pointer(rom, RELOAD_COMMAND + 2)? as u32;
    // A CRAM write (CD = 3): the address picks the line.
    if command & 0xc000_00f0 != 0xc000_0000 {
        return Err(format!("${command:08x} is not a CRAM write"));
    }
    let address = (command >> 16) & 0x3fff;
    Ok(CramReload {
        line: (counter & 0xff) + 1,
        cram_line: (address / 32) as u8,
        words: words(rom, table)?,
    })
}

impl Boss4 {
    fn load(rom: &[u8]) -> Result<Self> {
        let l = |p: usize| -> Result<u32> { Ok(pointer(rom, p)? as u32) };
        let s = |p: usize| signed(rom, p);
        let effect = |p: usize| -> Result<u8> { byte(rom, p) };
        let table = |p: usize| -> Result<[Missile; 3]> {
            Ok([
                Missile::read(rom, p)?,
                Missile::read(rom, p + 20)?,
                Missile::read(rom, p + 40)?,
            ])
        };
        // $2DC68 / $2DC70: the $44 and $48 longs.
        let w1 = [wave(l(0x2dc6a)?)[0], wave(l(0x2dc72)?)[0]];
        let w2 = [wave(l(0x2e266)?)[0], wave(l(0x2e26e)?)[0]];
        let mut spread = Vec::new();
        for i in 0..4 {
            let p = 0x2fc4a + i * 12;
            spread.push(Spread {
                offset: (s(p)?, s(p + 2)?),
                speed: (s(p + 4)?, s(p + 6)?),
                accel: (s(p + 8)?, s(p + 10)?),
            });
        }
        let mut headings = [0; 16];
        for (i, heading) in headings.iter_mut().enumerate() {
            *heading = word(rom, 0x2f8e2 + i * 2)?;
        }
        let mut heading_flips = [0; 4];
        for (i, flip) in heading_flips.iter_mut().enumerate() {
            *flip = word(rom, 0x2f8da + i * 2)?;
        }
        // `move.l #imm,$40(a0)` after each `Func_00429A` call.
        let offsets = |call: u32| -> Result<(i32, i32)> {
            let at = call as usize + 26;
            if word(rom, at)? != 0x217c || word(rom, at + 6)? != 0x40 {
                return Err(format!("no hull offsets at ${at:06x}"));
            }
            Ok((s(at + 2)?, s(at + 4)?))
        };
        if word(rom, 0x2e4e6)? != 0x337c || word(rom, 0x2e058)? >> 8 != 0x70 {
            return Err("Boss4's intro hand-over or charge end moved".into());
        }
        Ok(Self {
            parts: Parts::load(rom)?,
            fx: effects::Effects::load(rom)?,
            arm_health: s(0x2e4e8)?,
            handover: effect(0x2e4ff)?,
            muzzle: (word(rom, 0x2ddda)?, -s(0x2ddea)?, -s(0x2ddf6)?),
            item: (
                byte(rom, 0x2ef51)?,
                s(0x2ef54)?,
                [s(0x2ef78)?, s(0x2ef7a)?, s(0x2ef7c)?, s(0x2ef7e)?],
            ),
            charge_end: effect(0x2e059)?,
            reload: cram_reload(rom)?,
            hit_restore: (
                words(rom, 0x42122)?,
                words(rom, 0x42062 + usize::from(byte(rom, 0x4320f)?) * 32)?,
            ),
            splashes: Splashes::read(rom)?,
            plumes: Plumes::read(rom)?,
            sinking: Sinking::read(rom)?,
            hulls: [hitbox(l(0x2dc56)?), hitbox(l(0x2e252)?)],
            touch_damage: [byte(rom, 0x2f22f)?, byte(rom, 0x2f237)?],
            scroll_offsets: [offsets(GUNBOAT_MAP)?, offsets(FLYER_MAP)?],
            starts: [(s(0x2dc5e)?, s(0x2dc64)?), (s(0x2e25a)?, s(0x2e260)?)],
            waves: [w1, w2],
            health: [
                i32::from(byte(rom, 0x2dc83)?),
                i32::from(byte(rom, 0x2e277)?),
            ],
            // $2DC78: the $28 long's high word.
            rise_ticks: s(0x2dc7a)?,
            intro_sound: effect(0x2dc8b)?,
            // List-4 $34 routine 0 ($2F6D4): mapping, X speed, damage word.
            shell: (
                word(rom, 0x2f6d6)?,
                s(0x2f6f4)?,
                (word(rom, 0x2f6ee)? >> 8) as u8,
            ),
            // Routine $18 ($2FA12): mapping, offset, speed, damage.
            shell2: (
                word(rom, 0x2fa14)?,
                (-s(0x2fa1e)?, s(0x2fa24)?),
                (s(0x2fa2a)?, s(0x2fa30)?),
                (word(rom, 0x2fa3a)? >> 8) as u8,
            ),
            // Routine $10 ($2F974): mapping, gravity, damage.
            bomb: (
                word(rom, 0x2f976)?,
                s(0x2f986)?,
                (word(rom, 0x2f98c)? >> 8) as u8,
            ),
            spread,
            spread_mapping: word(rom, 0x2fba8)?,
            charge_frames: words(rom, 0x2fb70)?,
            missiles: [table(0x2de78)?, table(0x2e776)?, table(0x2e888)?],
            // $2F79A: move.w #$100,6(a0).
            missile_damage: (word(rom, 0x2f79c)? >> 8) as u8,
            headings,
            heading_flips,
            tracer: word(rom, 0x2eb8c)?,
            voice: Table::Voice.command_at(rom, 0x2e451)?,
            sounds: Sounds {
                cannon: effect(0x2dd9b)?,
                missile: effect(0x2f0fb)?,
                shell: effect(0x2e667)?,
                bomb: effect(0x2e91d)?,
                spread: effect(0x2fb91)?,
                charge: effect(0x2ec97)?,
                gun: effect(0x2eb2d)?,
                sinking: effect(0x2e2d9)?,
                blast: effect(0x2ed77)?,
                lunge: effect(0x2de41)?,
                surge: effect(0x2df69)?,
            },
        })
    }

    fn get_wave(e: &Enemy, var: usize) -> (i32, i32) {
        let v = e.vars[var];
        (i32::from((v >> 8) as u8 as i8), i32::from(v as u8 as i8))
    }
    fn set_wave(e: &mut Enemy, var: usize, step: i32, amp: i32) {
        e.vars[var] = ((step & 0xff) << 8) | (amp & 0xff);
    }
    fn set_waves(e: &mut Enemy, waves: &[(i32, i32, i32); 2]) {
        Self::set_wave(e, X_WAVE, waves[0].0, waves[0].1);
        e.vars[X_ANGLE] = waves[0].2;
        Self::set_wave(e, Y_WAVE, waves[1].0, waves[1].1);
        e.vars[Y_ANGLE] = waves[1].2;
    }
    /// `+$45` / `+$49` amplitude bytes.
    fn amp(e: &Enemy, var: usize) -> i32 {
        Self::get_wave(e, var).1
    }
    fn set_amp(e: &mut Enemy, var: usize, amp: i32) {
        let (step, _) = Self::get_wave(e, var);
        Self::set_wave(e, var, step, amp);
    }
    fn cosine(cx: &Ctx, angle: i32) -> i32 {
        sine(cx, angle + 0x200)
    }
    /// `Func_02EE38`: immunity countdown and the two sine waves about
    /// (`+$22`, `+$26`); X is clamped to `$1B0`. Then the waterline splash
    /// ([`Splashes`]).
    fn place(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[COOL] > 0 {
            e.vars[COOL] -= 1;
            if e.vars[COOL] == 0 && matches!(e.phase, GUNBOAT | HULL) {
                self.restore_hit_palette(cx);
            }
        }
        let (step, amp) = Self::get_wave(e, X_WAVE);
        e.vars[X_ANGLE] = (e.vars[X_ANGLE] + step) & 0xffff;
        let d = (sine(cx, e.vars[X_ANGLE]) * (amp * 2)) << 2 >> 16;
        e.x = (e.home_x + d).min(0x1b0);
        let (step, amp) = Self::get_wave(e, Y_WAVE);
        e.vars[Y_ANGLE] = (e.vars[Y_ANGLE] + step) & 0xffff;
        let d = (sine(cx, e.vars[Y_ANGLE]) * amp) << 2 >> 16;
        e.y = e.launch_y + d;
        // $2EF3A: every $200 updates an item (d7 = 0) is scheduled at X
        // $140 on one of Data_02EF78's rows (the high byte of `+$3A`).
        if e.counter & 0x1ff == 0 {
            let (kind, x, rows) = self.item;
            cx.signal(crate::combat::Signal::Drop(crate::stage_objects::Drop {
                kind,
                x,
                y: rows[usize::from((e.counter >> 8) & 6) >> 1],
                vx: self.parts.wreck.item_speed,
                high_priority: false,
            }));
        }
        if let Some((at, back)) = self.splashes.at(e.counter, (e.x, e.y)) {
            // `$FF0190`, the water layer's speed.
            let water = -crate::special::WATER_FLOW << 16;
            let rules = &cx.shared.ambient;
            cx.effect(ambient::wake(rules, at, self.splashes.life, water, back));
        }
    }
    /// `Func_00459C` then `Func_00429A`: the cleared plane A with a hull.
    fn draw_hull(cx: &mut Ctx, map: u32) {
        cx.plane_op(PlaneOp::Fill {
            vram: 0xc000,
            width: 64,
            height: 64,
            cell: 0,
        });
        cx.plane_op(PlaneOp::LzWordMap(map));
    }
    /// `Func_02EE38`'s scroll writes: H-scroll rows 3..27 and the plane A
    /// V-scroll, as VDP values. Rows 0..2, under the HUD window, carry the
    /// same value here instead of `$23C0`'s slow band.
    fn scroll(&self, cx: &Ctx, e: &Enemy) -> PlaneOp {
        self.hull_scroll(e.phase, (e.x, e.y), cx.camera)
    }
    fn hull_scroll(&self, phase: u8, (x, y): (i32, i32), camera: (i32, i32)) -> PlaneOp {
        let (dx, dy) = self.scroll_offsets[usize::from(phase != GUNBOAT)];
        let h = (x - camera.0 - dx) as i16;
        let v = -(y - camera.1 - dy) as i16;
        PlaneOp::Scroll {
            plane: Plane::A,
            bands: [h; 28],
            columns: [v; 20],
        }
    }
    /// `+$22`/`+$26` toward a target by `speed` (the `ext.l/swap/ori #1`
    /// idiom); true once there.
    fn toward(value: &mut i32, target: i32, speed: i32) -> bool {
        if *value == target {
            return true;
        }
        if (*value - target).abs() < speed {
            *value = target;
        } else {
            *value += if target > *value { speed } else { -speed };
        }
        false
    }
    /// `tst.w (Object_List4)`: no enemy shot is left.
    fn list4_empty(cx: &Ctx) -> bool {
        !cx.projectiles.iter().any(|shot| shot.enemy && shot.alive)
    }
    /// `Func_02F146(1, 2)` then `Func_02F3B4`: a smoke puff `dy` below
    /// its place.
    fn smoke(&self, cx: &mut Ctx, e: &Enemy, dy: i32) {
        let random = *cx.random;
        let (x, y) = Sinking::at(random, (e.x, e.y));
        let (mapping, tile) = self.sinking.smoke;
        let mut smoke = Effect::new(EffectKind::Debris, x, y + dy, mapping);
        parts::set_tile(&mut smoke, tile);
        // btst #0,($FF001C): list 9 when the high byte is odd.
        smoke.back = random & 0x100 != 0;
        smoke.motion = Motion {
            vx: (-crate::special::WATER_FLOW << 16).wrapping_neg(),
            vy: self.sinking.rise << 16,
            gravity: self.sinking.lift,
            ..Motion::default()
        };
        // Its first update (`$36FBC`) and `Object_Integrate` run in this
        // frame (the effect pass, which counts it to 2): `Data_036F46`'s
        // second entry is due in two updates.
        smoke.age = 1;
        cx.effect(smoke);
    }
    /// `Func_02F178(1, 2)`: a burning piece of the sinking gunboat.
    fn debris(&self, cx: &mut Ctx, e: &Enemy) {
        let random = *cx.random;
        let at = Sinking::at(random, (e.x, e.y));
        let (mapping, tile) = self.sinking.debris;
        let mut piece = Effect::new(EffectKind::Pieces, at.0, at.1, mapping);
        parts::set_tile(&mut piece, tile.wrapping_add(random & 0xf));
        piece.back = true;
        piece.motion = Motion {
            vx: (-crate::special::WATER_FLOW << 16).wrapping_neg() >> 1,
            gravity: self.sinking.gravity,
            ..Motion::default()
        };
        parts::countdown_life(&mut piece, self.sinking.life);
        cx.effect(piece);
    }
    fn blast(cx: &Ctx, sound: u8) -> Blast {
        Blast {
            sound: cx.shared.effects.command(sound).unwrap_or(0),
            ..cx.shared.blasts[0]
        }
    }
    /// `Func_02F0F6`: list-4 `$34` routine 8, `table` entry `index`.
    fn launch_missile(&self, cx: &mut Ctx, e: &Enemy, table: usize, index: usize) {
        let m = self.missiles[table][index];
        cx.effect_sound(self.sounds.missile);
        let (ox, oy) = Self::arc(cx, &m, m.angle);
        let (x, y) = (e.x + m.offset.0 + ox, e.y + m.offset.1 + oy);
        // `$2F108` leaves `obj_frame` 0 (only the cannon's `$2DDA8` sets it).
        let mut missile =
            Projectile::new(x, y, 0, true, self.headings[0], self.missile_damage).unset_frame();
        missile.life = MISSILE_TAG | ((table as u16) << 2) | index as u16;
        missile.passes_terrain = true;
        missile.owner = Some(cx.current);
        // `$2F7B2`/`$2F7EA`: a weapon frees the missile with `$2F3B4`
        // smoke; the only sound is the attacking weapon's hit response.
        missile.shot_down = Some(ShotDown {
            blast: None,
            first: self.sinking.smoke.0,
            frames: 0,
            period: 0,
            sound: None,
            smoke: Some((self.sinking.smoke.1, self.sinking.rise, self.sinking.lift)),
        });
        missile.contact_puff = true;
        missile.missile_heading = Some(0x600);
        self.aim_missile(cx, &mut missile);
        missile.x += m.drift.0;
        missile.y += m.drift.1;
        cx.shoot(missile);
    }
    /// The arc offset at `angle` (`$2F830`: `muls`, `asl.l #2`, `swap`).
    fn arc(cx: &Ctx, m: &Missile, angle: i32) -> (i32, i32) {
        (
            (Self::cosine(cx, angle) * m.radii.0) << 2 >> 16,
            (sine(cx, angle) * m.radii.1) << 2 >> 16,
        )
    }
    /// `$2F860`: the heading toward the player (2048 per turn, `$7F0`).
    fn heading(cx: &Ctx, shot: &Projectile) -> i32 {
        (i32::from(cx.shared.angle_to((shot.x, shot.y), cx.player)) << 3) & 0x7f0
    }
    fn aim_missile(&self, cx: &Ctx, shot: &mut Projectile) {
        let target = Self::heading(cx, shot);
        let current = i32::from(shot.missile_heading.unwrap_or(0x600)) & 0x7f0;
        let delta = (target - current) & 0x7ff;
        let heading = (current
            + match delta {
                0 => 0,
                0x400.. => -0x20,
                _ => 0x20,
            })
            & 0x7ff;
        shot.missile_heading = Some(heading as u16);
        shot.mapping = self.headings[((heading >> 6) & 15) as usize];
        let flip = self.heading_flips[((heading >> 9) & 3) as usize];
        shot.faces_left = (flip & 0x800 != 0) != shot.leftward();
        shot.base = (shot.base & !0x1000) | (flip & 0x1000);
    }
    /// List-4 `$34` routine `$0C` for every missile of this boss: the arc
    /// about its drifting centre, the hover, then the launch at 4 px/tick.
    fn steer_missiles(&self, cx: &mut Ctx) {
        let me = cx.current;
        let mut shots = std::mem::take(&mut *cx.projectiles);
        for shot in shots.iter_mut() {
            if shot.owner == Some(me) && shot.alive && shot.water.is_some() && shot.y >= WATER {
                shot.alive = false;
                if shot.water == Some(true) {
                    let water = -crate::special::WATER_FLOW << 16;
                    cx.effect(ambient::wake(
                        &cx.shared.ambient,
                        (shot.x, WATER),
                        32,
                        water,
                        false,
                    ));
                }
                continue;
            }
            if shot.owner != Some(me) || !shot.alive || shot.shot_down.is_none() {
                continue;
            }
            // `$2F7CE` runs before the arc/heading step and frees a missile
            // at the water line, leaving the same routine-8 plume as the
            // sinking hull. This also applies after its launch tag is gone.
            if shot.y >= WATER {
                shot.alive = false;
                let water = -crate::special::WATER_FLOW << 16;
                cx.effect(ambient::plume(&cx.shared.ambient, (shot.x, WATER), water));
                continue;
            }
            if shot.life & 0xff00 != MISSILE_TAG {
                self.missile_exhaust(cx, shot);
                continue;
            }
            let table = usize::from((shot.life >> 2) & 3).min(2);
            let m = self.missiles[table][usize::from(shot.life & 3).min(2)];
            let k = i32::from(shot.age);
            if k == 0 || shot.motion.vx != 0 || shot.motion.vy != 0 {
                continue;
            }
            if k < m.arc {
                let (x0, y0) = Self::arc(cx, &m, m.angle + m.step * (k - 1));
                let (x1, y1) = Self::arc(cx, &m, m.angle + m.step * k);
                shot.x += x1 - x0;
                shot.y += y1 - y0;
                self.missile_exhaust(cx, shot);
                self.aim_missile(cx, shot);
                shot.x += m.drift.0;
                shot.y += m.drift.1;
            } else if k < m.arc + m.hover - 1 {
                self.aim_missile(cx, shot);
                shot.x += m.drift.0;
                shot.y += m.drift.1;
            } else {
                let heading = i32::from(shot.missile_heading.unwrap_or(0x600));
                shot.motion.vx = Self::cosine(cx, heading) << 4;
                shot.motion.vy = sine(cx, heading) << 4;
                // Keep the heading's flip while the motion mirrors the art.
                let flip = self.heading_flips[((heading >> 9) & 3) as usize] & 0x800 != 0;
                shot.faces_left = flip != (shot.motion.vx < 0);
                shot.life = u16::MAX;
                self.missile_exhaust(cx, shot);
            }
        }
        *cx.projectiles = shots;
    }
    fn missile_exhaust(&self, cx: &mut Ctx, shot: &Projectile) {
        if (shot.age + 1) & 3 != 0 {
            return;
        }
        let heading = i32::from(shot.missile_heading.unwrap_or(0x600));
        let at = (
            shot.x + ((-Self::cosine(cx, heading) << 6) >> 16),
            shot.y + ((-sine(cx, heading) << 6) >> 16),
        );
        let mut spark = Effect::new(EffectKind::Dust, at.0, at.1, cx.shared.dust_seq[0]);
        spark.back = true;
        spark.blink = true;
        spark.high_priority = shot.high_priority;
        cx.effect(spark);
    }
    /// Removes the boss's hovering missiles (they have no life of their own).
    fn clear_missiles(cx: &mut Ctx) {
        let me = cx.current;
        for shot in cx.projectiles.iter_mut() {
            if shot.owner == Some(me) && shot.life & 0xff00 == MISSILE_TAG {
                shot.alive = false;
            }
        }
    }
    /// `$2DD82`: a cannon shell from the bow gun.
    /// The barrel recoils (`+$36` += 8, its `+$2A` = 8 walks it back) and,
    /// with the shell, a list-1 `$2B` muzzle flash shows for one frame
    /// (routine `$10`, `+$28` = 2).
    fn cannon(&self, cx: &mut Ctx, e: &Enemy) {
        if let Some(p) = part(cx.effects, CANNON) {
            p.origin.0 += self.parts.recoil;
            p.angle = 8;
        }
        cx.effect_sound(self.sounds.cannon);
        let (mapping, speed, damage) = self.shell;
        // `$2F6D4`: tile word 0 (not mirrored while it flies left) and no
        // frame before the first `Object_LoadHitbox`.
        let mut shell =
            Projectile::new(e.x - 0xa0, e.y - 0x10, speed, true, mapping, damage).unset_frame();
        shell.faces_left = true;
        shell.flip_period = Some(1);
        shell.trail = Some((4, 0));
        shell.passes_terrain = true;
        if cx.shoot(shell) {
            let (mapping, dx, dy) = self.muzzle;
            cx.overlay(e.x + dx, e.y + dy, mapping, 0, false);
        }
    }
    /// `$2E646`: the phase-2 diagonal shell.
    fn shell2(&self, cx: &mut Ctx, e: &Enemy) {
        cx.effect_sound(self.sounds.shell);
        let (mapping, offset, speed, damage) = self.shell2;
        let (x, y) = (e.x + offset.0, e.y + offset.1);
        let mut shell = Projectile::new(x, y, speed.0, true, mapping, damage)
            .with_motion(Motion {
                vy: speed.1 << 16,
                ..Motion::default()
            })
            .unset_frame();
        // $2FA18 clears obj_tile: this left-moving diagonal artwork is
        // already oriented toward its flight, with the tail at the gun.
        shell.faces_left = true;
        shell.passes_terrain = true;
        shell.owner = Some(cx.current);
        shell.water = Some(false);
        shell.trail = Some((1, 0));
        cx.shoot(shell);
    }
    /// `Func_02E902`: every eighth call a bomb lobbed toward the player.
    fn bombs(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.vars[T] = (e.vars[T] - 0x20) & 0xffff;
        if e.vars[T] & 0xff != 0 {
            return;
        }
        if (e.vars[T] >> 8) & 3 == 0 {
            cx.effect_sound(self.sounds.bomb);
        }
        let random = cx.random();
        let mut vx = if ((random >> 8) as u8 as i8) < 0 {
            -4
        } else {
            -2
        };
        let vy = if (random as i16) < 0 { -3 } else { -1 };
        if e.x < cx.player.0 {
            vx = -vx;
        }
        let (mapping, gravity, damage) = self.bomb;
        let y = e.y - 0x28;
        let mut bomb = Projectile::new(e.x, y, 0, true, mapping, damage)
            .with_motion(Motion {
                vx: vx << 16,
                vy: vy << 16,
                gravity,
                ..Motion::default()
            })
            .unset_frame();
        bomb.passes_terrain = true;
        bomb.owner = Some(cx.current);
        bomb.water = Some(true);
        bomb.flip_period = Some(1);
        cx.shoot(bomb);
    }
    /// List-4 routine `$30` (`Data_02FC4A`): four shells fanning out from
    /// the launcher at (-64, +16), all accelerating left.
    fn spread(&self, cx: &mut Ctx, e: &Enemy) {
        cx.effect_sound(self.sounds.spread);
        for shell in &self.spread {
            let (x, y) = (e.x - 64 + shell.offset.0, e.y + 16 + shell.offset.1);
            let mut shot = Projectile::new(x, y, 0, true, self.spread_mapping, 3)
                .with_motion(Motion {
                    vx: shell.speed.0 << 16,
                    vy: shell.speed.1 << 16,
                    ax: shell.accel.0,
                    gravity: shell.accel.1,
                    ..Motion::default()
                })
                .unset_frame();
            shot.passes_terrain = true;
            cx.shoot(shot);
        }
    }
    fn release_spread_launcher(&self, cx: &mut Ctx, e: &Enemy) {
        let at = cx
            .effects
            .iter_mut()
            .find(|fx| fx.kind == EffectKind::Boss4Launcher && fx.alive)
            .map(|fx| {
                fx.alive = false;
                (fx.x, fx.y)
            })
            .unwrap_or((e.x - 64, e.y + 16));
        let mut shot = Projectile::new(at.0, at.1, 0, true, self.spread_mapping, 3)
            .with_motion(Motion {
                ax: -0x5000,
                ..Motion::default()
            })
            .unset_frame();
        shot.owner = Some(cx.current);
        shot.passes_terrain = true;
        shot.trail = Some((8, self.spread_mapping));
        cx.shoot(shot);
    }
    /// `$2EC76` allocates the list-4 `$28` launcher when the attack starts.
    /// Its charge frames come from `Data_02FB70` and it stays briefly after
    /// spawning the four balls.
    fn start_spread_charge(&self, cx: &mut Ctx, e: &Enemy) {
        let mut launcher = Effect::new(
            EffectKind::Boss4Launcher,
            e.x - 64,
            e.y + 16,
            self.charge_frames[13],
        );
        launcher.list = SpriteList::EnemyShots;
        launcher.counter = 0x70;
        cx.effect(launcher);
    }
    fn spread_charge_spark(&self, cx: &mut Ctx, e: &Enemy) {
        let random = *cx.random;
        let mut spark = Effect::new(EffectKind::Boss4Charge, 0, 0, 0x368);
        spark.back = true;
        spark.origin = (e.x - 64, e.y + 16);
        spark.angle = random >> 4;
        spark.phase = (random & 15) as u8;
        spark.counter = 0x1f;
        cx.effect(spark);
    }
    /// The launcher follows the hull only until it fires; the released
    /// sprite stays where it launched and sheds its own trail in the ROM.
    fn update_spread_charge(&self, cx: &mut Ctx, e: &Enemy) {
        for fx in cx.effects.iter_mut() {
            if fx.kind == EffectKind::Boss4Charge {
                fx.origin = (e.x - 64, e.y + 16);
            } else if fx.kind == EffectKind::Boss4Launcher && fx.alive {
                if fx.counter > 0 && e.routine == 0x80 && e.vars[T] > 0x91 {
                    fx.x = e.x - 64;
                    fx.y = e.y + 16;
                    fx.counter = e.vars[T] - 0x91;
                    let left = fx.counter as usize;
                    fx.mapping = self.charge_frames[2 * (left / 16) + (left & 1)];
                } else if fx.counter > 0 && !(e.routine == 0x80 && e.vars[T] == 0x91) {
                    fx.counter -= 1;
                    fx.alive = fx.counter > 0;
                }
            }
        }
    }
    /// `$2EB12`: a machine-gun tracer from the lower gun toward the water.
    fn strafe(&self, cx: &mut Ctx, e: &Enemy) {
        cx.effect_sound(self.sounds.gun);
        let (x, y) = (e.home_x - 0x58, e.y + 0x24);
        let mut tracer = Effect::new(EffectKind::Pieces, x, y, self.tracer);
        tracer.base = 0x8000;
        tracer.high_priority = true;
        tracer.back = true;
        tracer.counter = 6;
        tracer.motion = Motion {
            vx: -11 << 16,
            vy: 3 << 16,
            ..Motion::default()
        };
        cx.effect(tracer);
        let mut hazard = Projectile::new(e.home_x - 320, WATER, 0, true, 0xd5, 1);
        hazard.box_load = crate::combat::ShotBox::Fixed(Hitbox {
            x: 0,
            y: -16,
            half_width: 16,
            half_height: 16,
        });
        hazard.hidden = true;
        hazard.passes_terrain = true;
        hazard.life = 16;
        hazard.stays = true;
        cx.shoot(hazard);
    }
    /// Func_02F250: restore the palette after a hull hit, including plane A.
    fn restore_hit_palette(&self, cx: &mut Ctx) {
        for (i, &color) in self.hit_restore.0.iter().enumerate() {
            cx.palette_op(PaletteOp::Color(0, i as u8, color));
        }
        cx.palette_op(PaletteOp::Load(1, self.hit_restore.1));
    }
    /// `$2DAA0`: a hit's damage against the current counter: the hull's,
    /// or in phase 2 the weak part's own (`loc_02FD56`).
    fn damage(&self, cx: &mut Ctx, e: &mut Enemy, damage: i32) {
        match e.phase {
            PART_C => return Self::hit_part(cx, GUN_PART, damage),
            ARM => return Self::hit_part(cx, ARM_PART, damage),
            _ => {}
        }
        e.vars[COOL] = 4;
        // Func_02F23C writes CRAM: the hull is plane A, not a sprite.
        cx.fill_palette(1, 0x0eee);
        e.step_x -= damage;
        if e.phase == HULL {
            // Func_02F2A2 after every hull hit of the flying machine.
            self.burn(cx, e);
        }
        if e.step_x >= 0 || e.phase != GUNBOAT {
            return;
        }
        // $2DAB6: restore the hull colours before it sinks.
        self.restore_hit_palette(cx);
        cx.palette_op(PaletteOp::Reload(2));
        cx.palette_op(PaletteOp::Reload(3));
        e.home_x = e.x;
        e.launch_y = e.y;
        // move.l d0,$44 / move.l d0,$48: both waves stop.
        Self::set_waves(e, &[(0, 0, 0), (0, 0, 0)]);
        e.routine = 0x2c;
        e.vars[T] = 0x200;
    }
    /// After the parts' update: a weak part under water hands the hits on
    /// (its `+$2A` destroyed flag, which the routines test), and the
    /// collision box is the weak part's while it takes hits, else the
    /// hull's.
    fn target(&self, cx: &Ctx, e: &mut Enemy) {
        if e.phase == PART_C && destroyed(cx.effects, GUN_PART) {
            e.phase = ARM;
        }
        if e.phase == ARM && destroyed(cx.effects, ARM_PART) {
            e.phase = HULL;
            e.step_x = self.health[1];
            // From now on hits are measured against the hull's counter.
            e.health = (e.step_x + 1) as u16;
            e.vars[COOL] = 0;
        }
        let weak = match e.phase {
            PART_C => Some(GUN_PART),
            ARM => Some(ARM_PART),
            _ => None,
        };
        e.vars[OPEN] = 0;
        e.body = Some(self.hulls[usize::from(e.phase != GUNBOAT)]);
        if let Some(n) = weak {
            if let Some(p) = parts::part_ref(cx.effects, n) {
                e.step_x = p.counter;
            }
            if let Some((area, immunity)) = self.weak_part(cx, e, n) {
                e.body = Some(area);
                e.vars[OPEN] = 1;
                e.vars[COOL] = i32::from(immunity);
            }
        }
    }
    /// `Data_02E5A4`: the next phase-2 attack.
    fn choose(cx: &mut Ctx, e: &mut Enemy) {
        let random = cx.random() & 1 != 0;
        e.routine = match e.phase {
            HULL => 0x44,
            ARM => {
                if random {
                    0x74
                } else {
                    0x64
                }
            }
            _ => 0x34,
        };
    }
    /// The phase-2 missile routines `$40`/`$50`: six shots, three per
    /// `$100` of `+$28` counted down by 8, skipping `$400`/`$300`.
    fn salvo(&self, cx: &mut Ctx, e: &mut Enemy, table: usize, next: u16) {
        if e.vars[T] == 0 {
            if Self::list4_empty(cx) {
                e.routine = next;
            }
            return;
        }
        e.vars[T] -= 8;
        if e.vars[T] & 0xff != 0 {
            return;
        }
        let high = e.vars[T] >> 8;
        let index = match high {
            3 | 4 => return,
            h if h > 4 => h - 5,
            h => h,
        };
        self.launch_missile(cx, e, table, index as usize);
    }
    fn routine(&self, cx: &mut Ctx, e: &mut Enemy) {
        loop {
            match e.routine {
                0 => {
                    // $2DBB4: the gunboat surges up out of the water.
                    Self::draw_hull(cx, GUNBOAT_MAP);
                    self.build_parts(cx, e, 0);
                    // ori.w #$800 on the second part: the ram stays stowed.
                    if let Some(p) = part(cx.effects, RAM_PART) {
                        p.flags |= parts::HIDDEN;
                    }
                    e.phase = GUNBOAT;
                    e.step_x = self.health[0];
                    (e.home_x, e.launch_y) = self.starts[0];
                    Self::set_waves(e, &self.waves[0]);
                    e.vars[T] = self.rise_ticks;
                    e.vars[S] = 0;
                    e.routine = 4;
                    cx.effect_sound(self.intro_sound);
                    return;
                }
                4 => {
                    // $2DCBE
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.launch_y = e.y;
                        Self::set_wave(e, Y_WAVE, 8, 8);
                        e.vars[Y_ANGLE] = 0;
                        e.routine = 8;
                    } else if e.vars[S] == 0 && e.vars[Y_ANGLE] >= 0x600 {
                        e.launch_y = e.y + (e.launch_y - e.y) / 2;
                        Self::set_wave(e, Y_WAVE, 0x0c, 0x30);
                        e.vars[S] = 1;
                    }
                    return;
                }
                8 => {
                    // $2DD18: in to X $140.
                    if e.home_x >= 0x140 {
                        e.routine = 0x0c;
                        e.vars[T] = 0x400;
                        e.vars[S] = 1;
                    } else {
                        e.home_x += 2;
                    }
                    return;
                }
                0x0c => {
                    // $2DD38: two volleys of three shells, sinking between.
                    if e.vars[S] != 0 {
                        e.vars[T] -= 0x10;
                        if e.vars[T] != 0 {
                            if e.vars[T] & 0xff == 0 {
                                self.cannon(cx, e);
                            }
                            return;
                        }
                        e.vars[T] = 0x20;
                        if e.vars[S] < 0 {
                            e.routine = 0x10;
                            return;
                        }
                        e.vars[S] = 0;
                    }
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x400;
                        e.vars[S] = -1;
                    }
                    e.launch_y += 1;
                    return;
                }
                0x10 => {
                    // $2DDFE
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x210;
                        e.routine = 0x14;
                    }
                    e.home_x -= 2;
                    e.launch_y -= 1;
                    return;
                }
                0x14 => {
                    // $2DE18: three homing missiles, then lunge or return.
                    if e.vars[T] == 0 {
                        if Self::list4_empty(cx) {
                            e.routine = if cx.random() & 1 != 0 {
                                Self::set_wave(e, X_WAVE, 0, 0x20);
                                e.vars[X_ANGLE] = 0;
                                cx.effect_sound(self.sounds.lunge);
                                0x18
                            } else {
                                8
                            };
                            e.vars[T] = 0x200;
                        }
                        return;
                    }
                    e.vars[T] -= 8;
                    if e.vars[T] & 0xff == 0 {
                        self.launch_missile(cx, e, 0, (e.vars[T] >> 8) as usize);
                    }
                    return;
                }
                0x18 => {
                    // $2DEB4: sway, then the long lunge.
                    if e.vars[T] >> 8 >= 2 {
                        e.vars[X_ANGLE] += 4;
                        if e.vars[X_ANGLE] >= 0x200 {
                            Self::set_wave(e, X_WAVE, 0, 0x5f);
                            e.vars[X_ANGLE] = 0x200;
                            e.home_x -= 0x7e;
                            e.vars[T] = 0x100;
                        }
                        return;
                    }
                    e.home_x += if e.vars[X_ANGLE] < 0x400 { -2 } else { 2 };
                    e.vars[X_ANGLE] += 0x10;
                    if e.vars[X_ANGLE] >= 0x780 {
                        e.home_x = e.x;
                        // move.l #0,$44: the sway stops.
                        Self::set_wave(e, X_WAVE, 0, 0);
                        e.vars[X_ANGLE] = 0;
                        e.routine = 0x20;
                    }
                    return;
                }
                0x1c => e.routine = 0x20,
                0x20 => {
                    // $2DF26: to X $150, then the charge.
                    if Self::toward(&mut e.home_x, 0x150, 2) {
                        self.charge_colors(cx);
                        e.vars[T] = 0x80;
                        e.routine = 0x24;
                        cx.effect_sound(self.sounds.surge);
                    }
                    return;
                }
                0x24 => {
                    // $2DF86: the ram slides out of the bow while the steam
                    // runs, and the bow glows at its end.
                    if let Some(p) = part(cx.effects, RAM_PART) {
                        p.flags &= !parts::HIDDEN;
                        step_toward(&mut p.origin.0, self.parts.ram.0);
                    }
                    e.vars[T] -= 1;
                    let t = e.vars[T];
                    if t == 0 {
                        self.charge_end(cx, e);
                        e.vars[T] = 0x28;
                        e.routine = 0x28;
                    } else if self.steam(cx, e, t) {
                        return;
                    } else if t < 0x40 && (t == 8 || t == 4) {
                        let glow = self.parts.glow[usize::from(t == 4)];
                        if let Some(p) = part(cx.effects, GLOW) {
                            p.mapping = glow;
                            if t == 8 {
                                p.origin.1 = self.parts.glow_y.0;
                            }
                        }
                    }
                    return;
                }
                0x28 => {
                    // $2E106: the surge, then the ram is stowed again and
                    // either loop follows.
                    if e.vars[T] != 0 {
                        e.vars[T] -= 1;
                        self.charge_fade(cx, e.vars[T]);
                        if e.vars[T] != 0 {
                            if e.vars[T] & 1 == 0 {
                                e.home_x += 1;
                            }
                            return;
                        }
                    }
                    if let Some(p) = part(cx.effects, RAM_PART) {
                        if step_toward(&mut p.origin.0, self.parts.ram.1) {
                            return;
                        }
                        p.flags |= parts::HIDDEN;
                    }
                    e.routine = if cx.random() & 1 != 0 { 8 } else { 0x20 };
                    return;
                }
                0x2c => {
                    // $2E1EA: sinking in smoke and debris, the player held
                    // (`move.b #6,$28(a3)`).
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[T] -= 1;
                    let t = e.vars[T];
                    if t > 0 {
                        if (e.y as u16) < 0x1f0 && t & 1 == 0 {
                            self.smoke(cx, e, 0);
                            if t & 0xf == 0 {
                                cx.effect_sound(self.sounds.sinking);
                            }
                        }
                        if t & 7 == 0 {
                            self.debris(cx, e);
                        }
                        if t & 2 == 0 {
                            e.launch_y += 1;
                            // move.b ($FF001C),d0 / andi.w #3,d0: read only.
                            e.home_x ^= i32::from((*cx.random >> 8) & 3);
                        }
                        // $2E312: plumes of water where it goes under, each
                        // with smoke $80 above.
                        if let Some(at) = self.plumes.at(t, (e.x, e.y), *cx.random) {
                            cx.effect_sound(self.plumes.sound);
                            let water = -crate::special::WATER_FLOW << 16;
                            cx.effect(ambient::plume(&cx.shared.ambient, at, water));
                            self.smoke(cx, e, -self.sinking.plume_smoke);
                        }
                        return;
                    }
                    // $2E202: the flying machine, its hull fire lit and
                    // palette line 3 white for the parts' hit flash.
                    Self::draw_hull(cx, FLYER_MAP);
                    self.build_parts(cx, e, 1);
                    Self::light_last(cx);
                    Self::clear_missiles(cx);
                    e.phase = PART_C;
                    e.step_x = self.parts.health;
                    (e.home_x, e.launch_y) = self.starts[1];
                    Self::set_waves(e, &self.waves[1]);
                    e.vars[T] = 0;
                    e.vars[S] = 0x50;
                    e.routine = 0x30;
                    cx.effect_sound(self.intro_sound);
                    cx.fill_palette(3, 0x0eee);
                    return;
                }
                0x30 => {
                    // $2E3C0 with +$28 as the sub-state.
                    match e.vars[T] {
                        0 => {
                            // $2E3E8: rising, then the first part is pulled
                            // up out of the sea.
                            e.vars[S] -= 1;
                            if e.vars[S] == 0 {
                                self.start_rise(cx, e, 0);
                                e.vars[T] = 4;
                            }
                            e.launch_y -= 2;
                        }
                        4 | 8 => {
                            // $2E41A / $2E484: bobbing with the rising part.
                            let n = usize::from(e.vars[T] == 8);
                            let sub = parts::part_ref(cx.effects, n).map_or(0, |p| p.angle);
                            if sub == 8 && e.counter & 3 == 0 {
                                e.launch_y -= 1;
                            }
                            if sub == 0x0c && e.counter & 1 == 0 {
                                e.launch_y += 1;
                            }
                            if sub == 0 {
                                cx.play(self.voice);
                                if n == 0 {
                                    self.start_rise(cx, e, 1);
                                }
                                e.vars[T] += 4;
                            }
                        }
                        0x0c => {
                            // $2E4D0: arm and gun into place, then they
                            // take their roles.
                            if !Self::approach(cx, &self.parts.poses.intro[..2]) {
                                if let Some(p) = part(cx.effects, ARM_PART) {
                                    p.phase = parts::ARM;
                                    p.counter = self.arm_health;
                                }
                                if let Some(p) = part(cx.effects, GUN_PART) {
                                    p.phase = parts::GUN;
                                }
                                e.vars[T] = 0x10;
                                cx.effect_sound(self.handover);
                            }
                        }
                        0x10 => {
                            // $2E50C
                            let intro = &self.parts.poses.intro;
                            if !Self::approach(cx, &intro[..3]) && !Self::approach(cx, intro) {
                                e.vars[T] = 0x14;
                            }
                        }
                        0x14 => {
                            // $2E53A: the sway widens and it settles at Y $140.
                            let mut moving = false;
                            let x_amp = Self::amp(e, X_WAVE);
                            if x_amp != 8 {
                                Self::set_amp(e, X_WAVE, x_amp + 1);
                                moving = true;
                            }
                            let y_amp = Self::amp(e, Y_WAVE);
                            if y_amp != 0x20 {
                                Self::set_amp(e, Y_WAVE, y_amp + 1);
                                moving = true;
                            }
                            if e.launch_y != 0x140 {
                                e.launch_y -= 1;
                                moving = true;
                            }
                            if !moving {
                                e.vars[T] = 0x18;
                            }
                        }
                        // $2E570: the attack starts on the next update.
                        _ => Self::choose(cx, e),
                    }
                    return;
                }
                0x34 => {
                    // $2E5B0: arm and gun aim while it settles to Y $150.
                    if !Self::list4_empty(cx) {
                        return;
                    }
                    if !Self::approach(cx, &self.parts.poses.aim) {
                        cx.play(self.voice);
                        e.vars[T] = 0x220;
                        e.routine = 0x38;
                    }
                    Self::double_step(&mut e.launch_y, 0x150);
                    return;
                }
                0x38 => {
                    // $2E600: shells at +$29 = $1C and $10 of each $A0,
                    // the arm and gun swinging between their poses.
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x40;
                        e.routine = 0x3c;
                        return;
                    }
                    let low = e.vars[T] & 0xff;
                    if low == 0 {
                        e.vars[T] = (e.vars[T] - 0x100) | 0xa0;
                    }
                    if low == 0x10 || low == 0x1c {
                        // $2E646: only from the gun at rest.
                        let Some(gun) = part(cx.effects, GUN_PART).filter(|p| p.angle == 4) else {
                            e.vars[T] = 0x40;
                            e.routine = 0x3c;
                            return;
                        };
                        gun.origin.0 += self.parts.gun_recoil.0;
                        gun.origin.1 += self.parts.gun_recoil.1;
                        self.shell2(cx, e);
                        return;
                    }
                    let pose = if (0x40..0xc0).contains(&low) {
                        &self.parts.poses.rest[0]
                    } else {
                        &self.parts.poses.aim
                    };
                    Self::approach(cx, pose);
                    return;
                }
                0x3c => {
                    // $2E6C8
                    let gun_down = usize::from(destroyed(cx.effects, GUN_PART));
                    if !Self::approach(cx, &self.parts.poses.rest[gun_down]) {
                        e.vars[T] -= 1;
                        if e.vars[T] == 0 {
                            e.vars[T] = 0x710;
                            e.routine = 0x40;
                        }
                    }
                    if Self::amp(e, X_WAVE) != 0 {
                        Self::set_amp(e, X_WAVE, Self::amp(e, X_WAVE) - 1);
                        Self::set_amp(e, Y_WAVE, Self::amp(e, Y_WAVE) - 1);
                    }
                    Self::double_step(&mut e.launch_y, 0x160);
                    return;
                }
                0x40 => {
                    // $2E732
                    self.salvo(cx, e, 1, 0x44);
                    return;
                }
                0x44 => {
                    // $2E7B2: down to the water and across to the left.
                    if !Self::list4_empty(cx) {
                        return;
                    }
                    if e.launch_y < 0x1c0 {
                        e.launch_y += 1;
                        return;
                    }
                    let gun_down = usize::from(destroyed(cx.effects, GUN_PART));
                    if Self::approach(cx, &self.parts.poses.low[gun_down]) {
                        return;
                    }
                    if e.home_x > 0x40 {
                        e.home_x -= 3;
                        return;
                    }
                    let y_amp = Self::amp(e, Y_WAVE);
                    if y_amp != 0x20 {
                        Self::set_amp(e, Y_WAVE, y_amp + (0x20 - y_amp).signum());
                        return;
                    }
                    Self::set_wave(e, Y_WAVE, 8, 0x20);
                    e.vars[T] = 0x710;
                    e.routine = 0x48;
                    return;
                }
                0x48 | 0x4c => e.routine = 0x50,
                0x50 => {
                    // $2E842
                    self.salvo(cx, e, 2, 0x5c);
                    return;
                }
                0x54 => e.routine = 0x58,
                0x58 => {
                    // $2E8C4: back to ($100, $130), dropping bombs on the
                    // way (`bne Func_02E902`).
                    let x = Self::toward(&mut e.home_x, 0x100, 1);
                    let y = Self::toward(&mut e.launch_y, 0x130, 1);
                    if x && y {
                        e.routine = 0x44;
                    } else {
                        self.bombs(cx, e);
                    }
                    return;
                }
                0x5c => {
                    // $2E988: up, then back to X $100 dropping bombs.
                    let gun_down = usize::from(destroyed(cx.effects, GUN_PART));
                    if Self::approach(cx, &self.parts.poses.rest[gun_down]) {
                        return;
                    }
                    if e.launch_y > 0x140 {
                        e.launch_y -= 2;
                    } else if e.home_x < 0x100 {
                        e.home_x += 2;
                        self.bombs(cx, e);
                    } else {
                        e.vars[T] = 0x20;
                        e.routine = 0x7c;
                    }
                    return;
                }
                0x60 => e.routine = 0x64,
                0x64 => {
                    // $2E9F6: to X $130 and Y $140, the arm drawn in.
                    let mut moving = false;
                    if Self::amp(e, X_WAVE) != 0 {
                        Self::set_amp(e, X_WAVE, Self::amp(e, X_WAVE) - 1);
                        moving = true;
                    }
                    if Self::amp(e, Y_WAVE) >= 0x10 {
                        Self::set_amp(e, Y_WAVE, Self::amp(e, Y_WAVE) - 1);
                        moving = true;
                    }
                    if e.home_x != 0x130 {
                        e.home_x += 1;
                        moving = true;
                    }
                    if e.launch_y != 0x140 {
                        e.launch_y -= 1;
                        moving = true;
                    }
                    if moving
                        || (!destroyed(cx.effects, ARM_PART)
                            && Self::approach(cx, &self.parts.poses.arm[0]))
                    {
                        return;
                    }
                    e.vars[T] = 0x140;
                    e.routine = 0x68;
                    return;
                }
                0x68 => {
                    // $2EA5A: at +$29 = 0 the arm reaches out (effect $3C).
                    e.vars[T] -= 1;
                    if destroyed(cx.effects, ARM_PART) || e.vars[T] == 0 {
                        e.vars[T] = 0x80;
                        e.routine = 0x6c;
                        return;
                    }
                    let low = e.vars[T] & 0xff;
                    if low == 0
                        && let Some(arm) = part(cx.effects, ARM_PART).filter(|p| p.angle == 0)
                    {
                        arm.origin = self.parts.poses.arm[1][0];
                        arm.angle = 4;
                        cx.effect_sound(self.parts.reach_sound);
                    }
                    if low < 0x20 {
                        Self::approach(cx, &self.parts.poses.arm[1]);
                    }
                    return;
                }
                0x6c => {
                    // $2EABC
                    if e.launch_y >= 0x140 {
                        e.launch_y -= 1;
                        return;
                    }
                    if Self::approach(cx, &self.parts.poses.arm[2]) {
                        return;
                    }
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 || destroyed(cx.effects, ARM_PART) {
                        e.vars[T] = 0xc0;
                        e.routine = 0x70;
                        return;
                    }
                    e.home_x += 3;
                    return;
                }
                0x70 => {
                    // $2EAFE: strafing the water while moving left, the
                    // arm shaking; a reaching arm ends it.
                    e.vars[T] -= 1;
                    let reaching =
                        parts::part_ref(cx.effects, ARM_PART).is_some_and(|p| p.angle != 0);
                    if e.vars[T] == 0 || reaching {
                        e.vars[T] = 0x80;
                        e.routine = 0x54;
                        return;
                    }
                    if e.vars[T] & 3 == 0 {
                        self.strafe(cx, e);
                    }
                    if e.vars[T] & 1 == 0
                        && let Some(arm) = part(cx.effects, ARM_PART)
                    {
                        // eori.l #$20001,$36(a1)
                        arm.origin.0 ^= 2;
                        arm.origin.1 ^= 1;
                    }
                    e.home_x -= 3;
                    return;
                }
                0x74 => e.routine = 0x78,
                0x78 => {
                    e.vars[T] = 0x80;
                    e.routine = 0x6c;
                    return;
                }
                0x7c => {
                    // $2EC60
                    if !Self::list4_empty(cx) {
                        return;
                    }
                    if e.launch_y < 0x1a0 {
                        e.launch_y += 1;
                        return;
                    }
                    if Self::amp(e, Y_WAVE) >= 0x10 {
                        Self::set_amp(e, Y_WAVE, Self::amp(e, Y_WAVE) - 1);
                    }
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x100;
                        e.routine = 0x80;
                        cx.effect_sound(self.sounds.charge);
                        self.start_spread_charge(cx, e);
                    }
                    return;
                }
                0x80 => {
                    // $2ECC4 with the list-4 routine $28 launcher: it charges
                    // $70 ticks, fires the spread and pulls the boss to $120.
                    e.vars[T] -= 1;
                    if e.vars[T] == 0x91 {
                        self.release_spread_launcher(cx, e);
                        self.spread(cx, e);
                    } else if (0x4f..0x8f).contains(&e.vars[T]) {
                        e.home_x += (0x120 - e.home_x) >> 4;
                    }
                    if e.vars[T] >= 0x90 && e.vars[T] & 3 == 0 {
                        self.spread_charge_spark(cx, e);
                    }
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x18;
                        e.routine = 0x30;
                    }
                    return;
                }
                0x84 => {
                    // $2ED22: drifting off through explosions.
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[T] -= 1;
                    if e.home_x < -72 {
                        cx.fill_palette(2, 0x0eee);
                        e.vars[T] = 0x80;
                        e.routine = 0x88;
                        return;
                    }
                    e.home_x += e.vars[DRIFT];
                    if e.vars[T] & 7 == 0 {
                        let dx = i32::from(cx.random() as u8 as i8) >> 1;
                        let blast = Self::blast(cx, self.sounds.blast);
                        cx.explode(e.x + dx, e.y, blast);
                    }
                    if e.vars[T] & 0xf == 0 {
                        e.vars[DRIFT] -= 1;
                        e.launch_y += 1;
                    }
                    if e.vars[T] & 3 == 0 {
                        // $2EDE6: smoke every fourth update.
                        self.smoke(cx, e, 0);
                    }
                    return;
                }
                _ => {
                    // $2EDF0: then the round clear request.
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[T] -= 1;
                    if e.vars[T] & 7 == 0 {
                        cx.restore_palette(2);
                    }
                    if e.vars[T] <= 0 {
                        Self::clear_missiles(cx);
                        cx.next_substage();
                        Ctx::remove(e);
                    }
                    return;
                }
            }
        }
    }
    /// `$2E3EE`/`$2E45C`: part `n` of the flying machine is pulled up from
    /// the sea below the boss (`+$32`/`+$34`), to show mapping `+$30`.
    fn start_rise(&self, cx: &mut Ctx, e: &mut Enemy, n: usize) {
        let (below, mapping) = self.parts.rises[n];
        if let Some(p) = part(cx.effects, n) {
            p.angle = 4;
        }
        e.vars[P32] = e.x;
        e.vars[P34] = e.y + below;
        e.vars[P30] = i32::from(mapping);
    }
    /// The `ext.l`/`swap`/`ori.w #1`/`add.w d0,d0` step: two toward
    /// `target` (an odd distance keeps it stepping over).
    fn double_step(value: &mut i32, target: i32) {
        if *value != target {
            *value += (target - *value).signum() * 2;
        }
    }
    /// `$2E054`: effect `$37`, the bow glow stowed, sixteen wakes from X -
    /// `$100` along the water (their tile `$800` flips them).
    fn charge_end(&self, cx: &mut Ctx, e: &Enemy) {
        cx.effect_sound(self.charge_end);
        self.charge_flash(cx);
        if let Some(p) = part(cx.effects, GLOW) {
            p.origin.1 = self.parts.glow_y.1;
        }
        let water = -crate::special::WATER_FLOW << 16;
        for i in (0..16).rev() {
            let at = (e.x + (i << 4) - 0x100, WATER);
            let mut wake = ambient::wake(&cx.shared.ambient, at, 0x20, water, false);
            wake.flip = true;
            cx.effect(wake);
        }
    }
}

/// Whether Boss4's and its parts' `Func_02F1EA` contacts can matter:
/// routines `$2C`, `$84` and `$88` set the record's `obj_flags` bit 13,
/// which turns every contact into damage, and hold the player with
/// `plr_hurt` = 2 (`$2E1F4`, `$2ED2C`, `$2EDFA`), so that damage is never
/// taken; routine 0 has not placed the machine yet.
fn touchable(e: &Enemy) -> bool {
    !matches!(e.routine, 0 | 0x2c) && e.routine < 0x84
}

impl Boss4 {
    /// `Func_02F1EA` after `Object_CollidePlayer` (`$2DA40`, `$2FD2E`)
    /// meets the player with `area`, for an object whose Y is `y`. A
    /// contact from above (`y` not below the player's feet, unsigned) with
    /// the player's `plr_flags` bits 3 and 4 clear drops the contact
    /// (`bclr #6,(a3)`) and calls `Player_ForceSurfJump`; any other costs
    /// `touch_damage` (both branches of its `$30` test write 1).
    fn touch(&self, cx: &mut Ctx, e: &Enemy, area: Hitbox, y: i32) {
        if !cx.collide_player(area) {
            return;
        }
        let protected_state = cx.player_state.is_some_and(|state| {
            crate::movement::dying(state) || state >= crate::movement::state::NINJUTSU
        });
        if y as u16 >= cx.player_at.1 as u16 && !protected_state {
            cx.force_surf_jump();
        } else {
            cx.contact_player(self.touch_damage[usize::from(e.routine >= 0x30)]);
        }
    }
}

impl Behavior for Boss4 {
    fn label(&self, _kind: u16) -> String {
        format!("Boss4 ${HANDLER:05X}")
    }
    fn integrates(&self) -> bool {
        false
    }
    /// The update performs the hull and part contacts explicitly, including
    /// the forced jump from above. Disable the generic enemy contact path.
    fn touches_player(&self) -> bool {
        false
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        let weak = matches!(enemy.phase, PART_C | ARM);
        match enemy.routine {
            // Not collidable ($2E1EA, $2ED22 set obj_flags bit 13).
            0 | 0x2c => HitResponse::Pass,
            r if r >= 0x84 => HitResponse::Pass,
            // $2DB10: the hull turns weapons away until the arm is gone,
            // and during routine $30.
            _ if weak && enemy.vars[OPEN] == 0 => HitResponse::Deflect,
            _ if enemy.vars[COOL] > 0 => HitResponse::Ignore,
            0x30 if !weak => HitResponse::Deflect,
            _ => HitResponse::Accept,
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        // $2D9E8: the H-interrupt, rewritten before the routine runs.
        cx.palette_op(PaletteOp::HInt((e.routine != 0x88).then_some(self.reload)));
        if e.routine == 0 {
            e.health = SENTINEL;
            e.tile = Some(0);
        }
        // A surviving hit keeps the routine; the lethal one enters $84.
        e.hurt_routine = e.routine;
        e.death_routine = 0x84;
        // Health before the hit: the counter + 1 in the final stage.
        let expected = if e.phase == HULL {
            (e.step_x + 1).max(1) as u16
        } else {
            SENTINEL
        };
        match cx.process_hit(e) {
            Some(Reaction::Hurt) => {
                let damage = i32::from(expected.saturating_sub(e.health));
                self.damage(cx, e, damage);
                e.health = if e.phase == HULL {
                    (e.step_x + 1).max(1) as u16
                } else {
                    SENTINEL
                };
            }
            Some(Reaction::Death) => {
                // $2DB00: routine $84, the $1A drift of 4.
                e.vars[DRIFT] = 4;
                e.vars[T] = 0;
                Self::set_waves(e, &[(0, 0, 0), (0, 0, 0)]);
                e.home_x = e.x;
                e.launch_y = e.y;
                Self::clear_missiles(cx);
            }
            _ => {}
        }
        e.counter = e.counter.wrapping_add(1);
        // $2DA3E: the hull's own `Object_CollidePlayer`, where the last
        // update left it (plane A: its flags hide it, which the test
        // ignores).
        if touchable(e) {
            let hull = self.hulls[usize::from(e.routine >= 0x30)].placed(e.x, e.y, false, false);
            self.touch(cx, e, hull, e.y);
        }
        self.steer_missiles(cx);
        self.routine(cx, e);
        if e.live() || e.dying() {
            self.place(cx, e);
            self.update_spread_charge(cx, e);
            cx.plane_op(self.scroll(cx, e));
            // List 4 after the boss: the bow waves follow its new place.
            self.sweep_bow_waves(cx, e);
            // The parts' list-1 updates follow in the same frame.
            self.run_parts(cx, e);
            self.target(cx, e);
            e.offscreen = cx.outside_view(e.x, e.y);
        } else {
            cx.effects.retain(|p| {
                !matches!(
                    p.kind,
                    EffectKind::BossPart | EffectKind::Boss4Launcher | EffectKind::Boss4Charge
                )
            });
        }
        if e.state == EnemyState::Hurt {
            e.state = EnemyState::Waiting;
        }
    }
}

mod effects;
mod parts;
