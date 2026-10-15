//! Round 4 gunboat and flying machine: surf combat and chained parts.
//!
//! Hull art scrolls on plane A; sprite parts and projectiles keep their own lists.
//! Health uses a sentinel until the final hull stage so only its lethal hit scores.
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

mod attacks;
mod phases;
