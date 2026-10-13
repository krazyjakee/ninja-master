//! Boss4's sprite parts: list-1 type-`$2B` records at routine `$68`
//! (`Func_02FCEC`), allocated in a chain through `+$3C` by `Func_02F022`
//! (and freed by `Func_02EFFC`), and run after the boss in the same frame
//! (`Func_0366A6` walks lists 1 and 9 after `Boss_Update`).
//!
//! Each part keeps `+$36`/`+$38`, its offset from the boss, and `+$28`
//! selects its role in the `$2FCFE` table; `+$2A` is the role's own
//! sub-state. Natively a part is an [`EffectKind::BossPart`] in the effect
//! list, so it keeps its list-1 (or list-9) link order among the other
//! effects: `phase` is `+$28`, `angle` `+$2A`, `counter` `+$2C` (health,
//! the rise timer or the recoil's home), `period` `+$2E` (hit immunity),
//! `origin` (`+$36`, `+$38`) and `flags` `obj_flags` (`$800` hidden, `$8`
//! destroyed). The reach registers `+$30..$34` of the one part that reaches
//! at a time are boss vars ([`P30`], [`P32`], [`P34`]).
//!
//! The boss record's X and Y longs hold `+$22`/`+$26` (its home position)
//! in their low words, so what reads them as longs (`Func_030190`, the
//! returning reach, `Func_02FEBC`'s debris) carries them as fractions
//! ([`parent`]).
use super::{Boss4, P30, P32, P34};
use crate::{
    combat::{
        Effect, EffectKind, Enemy, Hitbox, Motion, PartTarget, PlaneOp, WeaponContact, ambient,
    },
    enemies::Ctx,
    rom::{Result, pointer, signed, word},
};

/// `+$28` roles (`$2FCFE`).
pub(super) const FIXED: u8 = 0x00;
pub(super) const RECOIL: u8 = 0x04;
pub(super) const RAM: u8 = 0x08;
pub(super) const GUN: u8 = 0x0c;
pub(super) const ARM: u8 = 0x10;
pub(super) const SEGMENT: u8 = 0x14;
pub(super) const RISE: u8 = 0x18;
pub(super) const FIRE: u8 = 0x1c;

/// `obj_flags`: hidden (`$800`) and destroyed (`$8`).
pub(super) const HIDDEN: u16 = 0x800;
pub(super) const DESTROYED: u16 = 0x8;
/// The tile word's palette bits (`andi.w #$9FFF`) and the hit flash line
/// (`ori.w #$4000`: line 2 added to the pieces' line 1 is line 3, which
/// the flying machine fills with white).
const PALETTE: u16 = 0x6000;
const FLASH: u16 = 0x4000;

/// The chain parts of each machine that the routines name.
pub(super) const CANNON: usize = 0;
pub(super) const RAM_PART: usize = 1;
pub(super) const GLOW: usize = 3;
pub(super) const ARM_PART: usize = 0;
pub(super) const GUN_PART: usize = 1;
/// The flying machine's hull fire (`$2E242`: the ninth part burns from
/// the start) and the first of the four `Func_02F2A2` burns.
const LAST_FIRE: usize = 8;
const FIRST_BURN: usize = 4;

/// One `Func_02F022` entry: tile word (`& $F800`), mapping (`$28C` + the
/// low bits), role and offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Spec {
    pub(super) tile: u16,
    pub(super) mapping: u16,
    pub(super) role: u8,
    pub(super) offset: (i32, i32),
}

/// `Object_LoadHitbox` at the top of `Func_02FCEC`: the box of the part's
/// mapping, its offsets mirrored by the part's tile flips.
fn part_box(cx: &Ctx, p: &Effect) -> Option<Hitbox> {
    let vertical = p.base & 0x1000 != 0;
    cx.mappings
        .get(&p.mapping)
        .map(|m| m.bounds.placed(p.x, p.y, p.flip, vertical))
}

/// A `Func_02EFBE` table: target offsets of the first parts of the chain.
pub(super) type Pose = Vec<(i32, i32)>;

/// `Func_02EFBE`'s tables for the phase-2 attacks (`d7` = 1: the arm, then
/// the gun), each also with the gun destroyed where the routine checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Poses {
    /// `Data_02E52A`: the intro's final offsets of parts 1..4.
    pub(super) intro: Pose,
    /// `Data_02E5F6` (`$34`, `$38`).
    pub(super) aim: Pose,
    /// `Data_02E71E`/`Data_02E728` (`$38`, `$3C`, `$5C`).
    pub(super) rest: [Pose; 2],
    /// `Data_02E82E`/`Data_02E838` (`$44`).
    pub(super) low: [Pose; 2],
    /// `$2EA54`, `$2EAB6`, `$2EAF8`: the arm alone (`$64`, `$68`, `$6C`).
    pub(super) arm: [Pose; 3],
}

/// `Func_02FEBC`'s eight pieces: mapping, `+$28` life, gravity, X speed
/// `(r >> 5) + x` px plus half the water's drift, Y speed `-((r >> 6) + y)`
/// px; and the item `Func_02F406` schedules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Wreck {
    pub(super) mapping: u16,
    pub(super) life: i32,
    pub(super) gravity: i32,
    pub(super) speed: (i32, i32),
    pub(super) item: (u8, i32, i32),
    /// `Func_0361D2`: `move.w #$FFFB,$18(a6)`.
    pub(super) item_speed: i32,
}

/// The chain tables, poses and constants read from the ROM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Parts {
    /// `Data_02DC9C` (gunboat) and `Data_02E376` (flying machine).
    pub(super) chains: [Vec<Spec>; 2],
    /// `$2FE6A`: `move.w #$D,$2C(a0)` (the arm's is `$2E4E2`).
    pub(super) health: i32,
    pub(super) poses: Poses,
    /// The intro rises: target below the boss (`$2E402`, `$2E470`) and
    /// the mapping each leaves (`$2E40A`, `$2E478`).
    pub(super) rises: [(i32, u16); 2],
    /// `move.w #$60`: ticks of each half of a reach (`$3003E`).
    pub(super) reach_ticks: i32,
    /// The arm's reach target from its own position (`$30048`, `$30050`).
    pub(super) arm_reach: (i32, i32),
    /// `moveq #$F,d7`: segments, their mapping and the rise's mapping.
    pub(super) segments: usize,
    pub(super) segment_mapping: u16,
    pub(super) rise_mapping: u16,
    /// Where the segments end (`Func_030188`, `$30104`) and where each
    /// reach returns (`$3032C`, `$30132`), from the boss.
    pub(super) rise_anchor: i32,
    pub(super) arm_anchor: (i32, i32),
    pub(super) arm_home: i32,
    /// `$3023E`: a rising part splashes while its Y is in `splash_rows`,
    /// on `splash_y`, for `splash_life`.
    pub(super) splash_rows: (i32, i32),
    pub(super) splash_y: i32,
    pub(super) splash_life: i32,
    /// `$2FE8C`/`$2FE94`: a destroyed part's X speed and gravity; it is
    /// gone below `water` (`$2FEAC`).
    pub(super) fall: (i32, i32),
    pub(super) water: i32,
    pub(super) wreck: Wreck,
    /// `$30386`: the burning part's mapping and tiles (`$303AE`).
    pub(super) fire_mapping: u16,
    pub(super) fire_tiles: [u16; 8],
    /// `Func_02F2A2`: below each health the next part burns, with a
    /// plane-A byte map; and the sound.
    pub(super) burns: [(i32, u32); 4],
    pub(super) burn_sound: u8,
    /// `$2DD8C`: the cannon's recoil and sub-state.
    pub(super) recoil: i32,
    /// `$2E656`: the gun's recoil per shell.
    pub(super) gun_recoil: (i32, i32),
    /// `Func_02FFBA`: the arm's box half sizes while it collides, and the
    /// damage its touch writes (`move.b #2,7(a3)`).
    pub(super) arm_box: (i32, i32),
    pub(super) arm_damage: u8,
    /// The gunboat's ram: its X offset while the steam runs (`$2DF92`)
    /// and when it is stowed again (`$2E122`).
    pub(super) ram: (i32, i32),
    /// The bow glow: mappings at `+$28` 8 and 4 (`$2E038`, `$2E04C`), its
    /// Y offset while shown and after (`$2E03E`, `$2E0A8`).
    pub(super) glow: [u16; 2],
    pub(super) glow_y: (i32, i32),
    /// `$2EA88`: the arm's reach sound; `$3009E`: the spark it leaves.
    pub(super) reach_sound: u8,
    pub(super) spark: (i32, i32),
}

fn pose(rom: &[u8], at: usize, count: usize) -> Result<Pose> {
    (0..count)
        .map(|i| Ok((signed(rom, at + i * 4)?, signed(rom, at + 2 + i * 4)?)))
        .collect()
}
/// A table with its `d7` count word first.
fn counted(rom: &[u8], at: usize) -> Result<Pose> {
    pose(rom, at + 2, usize::from(word(rom, at)?) + 1)
}
fn chain(rom: &[u8], at: usize, base: u16) -> Result<Vec<Spec>> {
    let count = usize::from(word(rom, at)?) + 1;
    (0..count)
        .map(|i| {
            let p = at + 2 + i * 8;
            let tile = word(rom, p)?;
            Ok(Spec {
                tile: tile & 0xf800,
                mapping: base + (tile & 0x7ff),
                role: word(rom, p + 2)? as u8,
                offset: (signed(rom, p + 4)?, signed(rom, p + 6)?),
            })
        })
        .collect()
}
/// The word operand of the instruction at `at`.
fn imm(rom: &[u8], at: usize) -> Result<i32> {
    signed(rom, at + 2)
}
/// The long operand's whole pixels (`move.l #$FFE40000,d0`).
fn long_px(rom: &[u8], at: usize) -> Result<i32> {
    Ok(pointer(rom, at + 2)? as u32 as i32 >> 16)
}
/// `addq`/`subq` data (bits 9..11, 0 = 8).
fn quick(rom: &[u8], at: usize) -> Result<i32> {
    Ok(match (word(rom, at)? >> 9) & 7 {
        0 => 8,
        n => i32::from(n),
    })
}
/// `moveq #n`'s byte.
fn moveq(rom: &[u8], at: usize) -> Result<i32> {
    let op = word(rom, at)?;
    if op & 0xf100 != 0x7000 {
        return Err(format!("${op:04x} at ${at:05x} is not moveq"));
    }
    Ok(i32::from(op as u8 as i8))
}

impl Parts {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        // Func_02F022: `addi.w #$28C,d1`; `Func_02FCEC`'s dispatch.
        if word(rom, 0x2f04a)? != 0x0641 || word(rom, 0x2fcfa)? != 0x4efb {
            return Err("Func_02F022 is not where Boss4 builds its parts".into());
        }
        let base = word(rom, 0x2f04c)?;
        // `lea Data(pc),a1` eight bytes before each `cmpi.w #n,d0`.
        let burn = |cmpi: usize| -> Result<(i32, u32)> {
            let lea = cmpi - 8;
            if word(rom, lea)? != 0x43fa || word(rom, cmpi)? != 0x0c40 {
                return Err(format!("no Func_02F2A2 burn at ${cmpi:05x}"));
            }
            let map = (lea as i64 + 2 + i64::from(signed(rom, lea + 2)?)) as u32;
            Ok((imm(rom, cmpi)?, map))
        };
        let mut fire_tiles = [0; 8];
        for (i, tile) in fire_tiles.iter_mut().enumerate() {
            *tile = word(rom, 0x303ae + i * 2)?;
        }
        let size = word(rom, 0x2ffca)?;
        Ok(Self {
            chains: [chain(rom, 0x2dc9c, base)?, chain(rom, 0x2e376, base)?],
            health: imm(rom, 0x2fe6a)?,
            poses: Poses {
                intro: pose(rom, 0x2e52a, 4)?,
                aim: counted(rom, 0x2e5f6)?,
                rest: [counted(rom, 0x2e71e)?, counted(rom, 0x2e728)?],
                low: [counted(rom, 0x2e82e)?, counted(rom, 0x2e838)?],
                arm: [
                    counted(rom, 0x2ea54)?,
                    counted(rom, 0x2eab6)?,
                    counted(rom, 0x2eaf8)?,
                ],
            },
            rises: [
                (imm(rom, 0x2e402)?, word(rom, 0x2e40c)?),
                (imm(rom, 0x2e470)?, word(rom, 0x2e47a)?),
            ],
            reach_ticks: imm(rom, 0x3003e)?,
            arm_reach: (-imm(rom, 0x30048)?, moveq(rom, 0x30050)?),
            segments: moveq(rom, 0x3005c)? as usize + 1,
            segment_mapping: word(rom, 0x3007c)?,
            rise_mapping: word(rom, 0x30280)?,
            rise_anchor: long_px(rom, 0x30188)?,
            arm_anchor: (long_px(rom, 0x30104)?, long_px(rom, 0x3010a)?),
            arm_home: long_px(rom, 0x30132)?,
            splash_rows: (imm(rom, 0x3023e)?, imm(rom, 0x30246)?),
            splash_y: imm(rom, 0x30270)?,
            splash_life: imm(rom, 0x30276)?,
            fall: (pointer(rom, 0x2fe8e)? as u32 as i32, imm(rom, 0x2fe94)?),
            water: imm(rom, 0x2feac)?,
            wreck: Wreck {
                mapping: word(rom, 0x2ff3e)?,
                life: imm(rom, 0x2ff8c)?,
                gravity: pointer(rom, 0x2ff86)? as u32 as i32,
                speed: (quick(rom, 0x2ff5e)?, quick(rom, 0x2ff7c)?),
                item: (
                    moveq(rom, 0x2f406)? as u8,
                    imm(rom, 0x2f408)?,
                    imm(rom, 0x2f40c)?,
                ),
                item_speed: signed(rom, 0x36208)? << 16,
            },
            fire_mapping: word(rom, 0x303a6)?,
            fire_tiles,
            burns: [
                burn(0x2f2c8)?,
                burn(0x2f2d6)?,
                burn(0x2f2e4)?,
                burn(0x2f2f2)?,
            ],
            burn_sound: moveq(rom, 0x2f30c)? as u8,
            recoil: quick(rom, 0x2dd8c)?,
            gun_recoil: (imm(rom, 0x2e656)?, -imm(rom, 0x2e65c)?),
            arm_box: (i32::from(size >> 8), i32::from(size & 0xff)),
            arm_damage: (word(rom, 0x2ffda)? == 0x177c)
                .then(|| word(rom, 0x2ffdc))
                .ok_or("Func_02FFBA writes no contact damage")?? as u8,
            ram: (moveq(rom, 0x2df92)?, moveq(rom, 0x2e122)?),
            glow: [word(rom, 0x2e03a)?, word(rom, 0x2e04e)?],
            glow_y: (imm(rom, 0x2e03e)?, imm(rom, 0x2e0a8)?),
            reach_sound: moveq(rom, 0x2ea88)? as u8,
            spark: (moveq(rom, 0x300ae)?, moveq(rom, 0x300b8)?),
        })
    }
}

/// The boss record's X and Y longs: the words `+$22`/`+$26` sit in their
/// low halves.
pub(super) fn parent(e: &Enemy) -> (i32, i32) {
    (
        (e.x << 16) | (e.home_x & 0xffff),
        (e.y << 16) | (e.launch_y & 0xffff),
    )
}
fn long(p: &Effect) -> (i32, i32) {
    (
        (p.x << 16) | i32::from(p.motion.fx),
        (p.y << 16) | i32::from(p.motion.fy),
    )
}
fn set_long(p: &mut Effect, (x, y): (i32, i32)) {
    p.x = x >> 16;
    p.motion.fx = x as u16;
    p.y = y >> 16;
    p.motion.fy = y as u16;
}
/// `+$36`/`+$38` added to the boss's X and Y words (`loc_02FDC4`).
fn follow(p: &mut Effect, e: &Enemy) {
    p.x = e.x + p.origin.0;
    p.y = e.y + p.origin.1;
}
/// The `ext.l`/`swap`/`ori.w #1` step: one toward `target`.
pub(super) fn step_toward(value: &mut i32, target: i32) -> bool {
    if *value == target {
        return false;
    }
    *value += (target - *value).signum();
    true
}

/// Effect-list indices of the chain parts, in `+$3C` order.
pub(super) fn chain_of(effects: &[Effect]) -> Vec<usize> {
    effects
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == EffectKind::BossPart && e.phase != SEGMENT && e.alive)
        .map(|(i, _)| i)
        .collect()
}
/// Chain part `n`, if the chain has it.
pub(super) fn part(effects: &mut [Effect], n: usize) -> Option<&mut Effect> {
    let index = *chain_of(effects).get(n)?;
    effects.get_mut(index)
}
pub(super) fn part_ref(effects: &[Effect], n: usize) -> Option<&Effect> {
    chain_of(effects).get(n).map(|&i| &effects[i])
}
/// `btst #3,1(a1)`: chain part `n` is destroyed.
pub(super) fn destroyed(effects: &[Effect], n: usize) -> bool {
    part_ref(effects, n).is_some_and(|p| p.flags & DESTROYED != 0)
}

impl Boss4 {
    /// `Func_02EFFC` then `Func_02F022`: free the old chain and build
    /// machine `machine`'s parts, in list 1 behind the effects already
    /// there.
    pub(super) fn build_parts(&self, cx: &mut Ctx, e: &Enemy, machine: usize) {
        cx.effects.retain(|p| p.kind != EffectKind::BossPart);
        for spec in &self.parts.chains[machine] {
            let mut p = Effect::new(
                EffectKind::BossPart,
                e.x + spec.offset.0,
                e.y + spec.offset.1,
                spec.mapping,
            );
            set_tile(&mut p, spec.tile);
            p.phase = spec.role;
            p.origin = spec.offset;
            cx.effect(p);
        }
    }
    /// `Func_02EFBE`: the first `pose.len()` parts one pixel toward their
    /// offsets on each axis; true while any moved.
    pub(super) fn approach(cx: &mut Ctx, pose: &[(i32, i32)]) -> bool {
        let chain = chain_of(cx.effects);
        let mut moving = false;
        for (&index, &(x, y)) in chain.iter().zip(pose) {
            let p = &mut cx.effects[index];
            moving |= step_toward(&mut p.origin.0, x);
            moving |= step_toward(&mut p.origin.1, y);
        }
        moving
    }

    /// The parts' updates after the boss's (list 1, then the list-9
    /// segments, which only collide), then `Object_Integrate`.
    pub(super) fn run_parts(&self, cx: &mut Ctx, e: &mut Enemy) {
        for index in chain_of(cx.effects) {
            self.run_part(cx, e, index);
        }
        for p in cx.effects.iter_mut() {
            if p.kind == EffectKind::BossPart && p.motion != Motion::default() {
                p.motion.step(&mut p.x, &mut p.y);
            }
        }
        cx.effects
            .retain(|p| p.kind != EffectKind::BossPart || p.alive);
        if super::touchable(e) {
            for p in cx.effects.iter().filter(|p| {
                p.kind == EffectKind::BossPart && p.alive && p.flags & (HIDDEN | DESTROYED) == 0
            }) {
                let target = match p.phase {
                    FIXED | RECOIL | RAM => {
                        if matches!(e.phase, super::GUNBOAT | super::HULL) {
                            PartTarget::Forward(cx.current)
                        } else {
                            PartTarget::Deflect
                        }
                    }
                    SEGMENT | RISE => PartTarget::Deflect,
                    _ => continue,
                };
                if let Some(area) = part_box(cx, p) {
                    cx.weapon_contacts.push(WeaponContact {
                        area,
                        target,
                        high_priority: e.high_priority,
                    });
                }
            }
        }
    }
    fn run_part(&self, cx: &mut Ctx, e: &mut Enemy, index: usize) {
        // Func_02FD1E, where the fixed parts, the recoiling cannon, the ram
        // (hidden or not) and the gun's first sub-states start: the part's
        // box where it was left, then `Func_02F1EA`.
        let p = &cx.effects[index];
        let touches = match p.phase {
            FIXED | RECOIL | RAM => true,
            GUN => p.angle <= 4,
            _ => false,
        };
        if touches
            && super::touchable(e)
            && let Some(area) = part_box(cx, p)
        {
            let y = p.y;
            self.touch(cx, e, area, y);
        }
        match cx.effects[index].phase {
            RECOIL => {
                // loc_02FDDE: placed first, then the recoil returns.
                let p = &mut cx.effects[index];
                follow(p, e);
                match p.angle {
                    0 => {
                        p.counter = p.origin.0;
                        p.angle = 4;
                    }
                    8 => {
                        p.origin.0 -= 1;
                        if p.origin.0 == p.counter {
                            p.angle = 4;
                        }
                    }
                    _ => {}
                }
            }
            GUN => self.run_gun(cx, e, index),
            ARM => self.run_arm(cx, e, index),
            RISE => self.run_rise(cx, e, index),
            FIRE => {
                // $30376 / $30386: a burning part picks random flame tiles.
                if cx.effects[index].angle & 4 != 0 {
                    let r = cx.random();
                    let tiles = &self.parts.fire_tiles;
                    let tile = 0xa000 | (r & 0x1800) | tiles[usize::from(r & 0xe) >> 1];
                    let p = &mut cx.effects[index];
                    set_tile(p, tile);
                    p.mapping = self.parts.fire_mapping;
                }
                follow(&mut cx.effects[index], e);
            }
            // Func_02FE16.
            FIXED | RAM => follow(&mut cx.effects[index], e),
            // SEGMENT: collision only.
            _ => {}
        }
    }
    /// `.loc_02FE4C`: the flying machine's gun (part 2 after the intro).
    fn run_gun(&self, cx: &mut Ctx, e: &mut Enemy, index: usize) {
        let p = &mut cx.effects[index];
        if p.angle == 0 {
            // $2FE6A
            p.counter = self.parts.health;
            p.period = 0;
            p.angle = 4;
        }
        match p.angle {
            4 if p.counter < 0 => {
                // $2FE82: the wreck (list 9) drifts down into the sea.
                self.wreck(cx, e, index, true);
                let p = &mut cx.effects[index];
                p.angle = 8;
                self.sink(p);
            }
            4 => {
                countdown(p);
                follow(p, e);
            }
            8 => self.sink(p),
            _ => follow(p, e),
        }
    }
    /// `$2FE9E`: a destroyed part falls until it is under water, then is
    /// hidden and flagged destroyed (`ori.w #$808`).
    fn sink(&self, p: &mut Effect) {
        p.base &= !PALETTE;
        if p.y >= self.parts.water {
            p.flags |= HIDDEN | DESTROYED;
            p.angle += 4;
        }
    }
    /// `Func_02FF94`: the arm (part 1 after the intro) and its reach.
    fn run_arm(&self, cx: &mut Ctx, e: &mut Enemy, index: usize) {
        let gun_down = destroyed(cx.effects, GUN_PART);
        let (sub, health) = (cx.effects[index].angle, cx.effects[index].counter);
        if sub < 0x10 && health < 0 {
            // $30014: the reach ends and the wreck (list 1) falls.
            Self::free_segments(cx);
            self.wreck(cx, e, index, false);
            cx.effects[index].angle = 0x10;
            return;
        }
        if sub < 0x10
            && super::touchable(e)
            && let Some(mut area) = part_box(cx, &cx.effects[index])
        {
            // Func_02FFBA: `Object_CollidePlayer` through the `$1810` box
            // (the loaded offsets kept), with priority bit 15 cleared for
            // the test; the touch costs 2 and never forces a jump.
            (area.half_width, area.half_height) = self.parts.arm_box;
            if cx.collide_player(area) {
                cx.contact_player(self.parts.arm_damage);
            }
        }
        if sub < 0x10 && gun_down {
            // loc_02FD56 through Func_02FFBA.
            countdown(&mut cx.effects[index]);
        }
        match sub {
            0 => follow(&mut cx.effects[index], e),
            4 => {
                // $3003E
                let p = &cx.effects[index];
                let (tile, at) = (tile_of(p), (p.x, p.y));
                e.vars[P30] = self.parts.reach_ticks;
                e.vars[P32] = at.0 + self.parts.arm_reach.0;
                e.vars[P34] = at.1 + self.parts.arm_reach.1;
                self.spawn_segments(cx, tile);
                // A type-5 spark (list 9) over the shoulder.
                let mut spark = Effect::new(
                    EffectKind::WallSpark,
                    at.0 + self.parts.spark.0,
                    at.1 + self.parts.spark.1,
                    cx.shared.dust_seq[0],
                );
                set_tile(&mut spark, tile);
                spark.blink = true;
                spark.back = true;
                cx.effect(spark);
                cx.effects[index].angle = 8;
                self.arm_out(cx, e, index);
            }
            8 => self.arm_out(cx, e, index),
            0x0c => self.arm_back(cx, e, index),
            0x10 => self.sink(&mut cx.effects[index]),
            _ => follow(&mut cx.effects[index], e),
        }
    }
    /// `$300D8`: the arm reaches toward `+$32`/`+$34` for `+$30` ticks.
    fn arm_out(&self, cx: &mut Ctx, e: &mut Enemy, index: usize) {
        let p = &mut cx.effects[index];
        let (x, y) = long(p);
        let x = x + (((e.vars[P32] << 16) - x) >> 4);
        let y = y + (((e.vars[P34] << 16) - y) >> 4);
        set_long(p, (x, y));
        self.place_segments(cx, e, index, self.parts.arm_anchor);
        e.vars[P30] -= 1;
        if e.vars[P30] != 0 {
            return;
        }
        e.vars[P30] = self.parts.reach_ticks;
        cx.effects[index].angle = 0x0c;
        self.arm_back(cx, e, index);
    }
    /// `$30126`: and back toward the boss.
    fn arm_back(&self, cx: &mut Ctx, e: &mut Enemy, index: usize) {
        let (bx, by) = parent(e);
        let p = &mut cx.effects[index];
        let (x, y) = long(p);
        let x = x + ((bx + (self.parts.arm_home << 16) - x) >> 5);
        let y = y + ((by - y) >> 5);
        set_long(p, (x, y));
        self.place_segments(cx, e, index, self.parts.arm_anchor);
        e.vars[P30] -= 1;
        if e.vars[P30] == 0 {
            let p = &mut cx.effects[index];
            p.origin = (p.x - e.x, p.y - e.y);
            p.angle = 0;
            Self::free_segments(cx);
        }
    }
    /// `loc_030212`: a part pulled up out of the sea in the intro.
    fn run_rise(&self, cx: &mut Ctx, e: &mut Enemy, index: usize) {
        let p = &mut cx.effects[index];
        match p.angle {
            0 => follow(p, e),
            4 => {
                // $3027E
                p.mapping = self.parts.rise_mapping;
                p.motion = Motion::default();
                p.counter = self.parts.reach_ticks;
                let tile = tile_of(p);
                self.spawn_segments(cx, tile);
                cx.effects[index].angle = 8;
                self.rise_down(cx, e, index);
            }
            8 => self.rise_down(cx, e, index),
            _ => self.rise_up(cx, e, index),
        }
    }
    /// `$302E0`: down to `+$32`/`+$34`, then the mapping `+$30`.
    fn rise_down(&self, cx: &mut Ctx, e: &mut Enemy, index: usize) {
        self.waterline(cx, index);
        let p = &mut cx.effects[index];
        let (x, y) = long(p);
        let x = x + (((e.vars[P32] << 16) - x) >> 4);
        let y = y + (((e.vars[P34] << 16) - y) >> 4);
        set_long(p, (x, y));
        self.place_segments(cx, e, index, (self.parts.rise_anchor, 0));
        let p = &mut cx.effects[index];
        p.counter -= 1;
        if p.counter != 0 {
            return;
        }
        p.mapping = e.vars[P30] as u16;
        p.counter = self.parts.reach_ticks;
        p.angle = 0x0c;
        self.rise_up(cx, e, index);
    }
    /// `$30324`: back up beside the boss, where it keeps its offset.
    fn rise_up(&self, cx: &mut Ctx, e: &mut Enemy, index: usize) {
        self.waterline(cx, index);
        let (bx, by) = parent(e);
        let p = &mut cx.effects[index];
        let (x, y) = long(p);
        let x = x + ((bx + (self.parts.rise_anchor << 16) - x) >> 4);
        let y = y + ((by - y) >> 4);
        set_long(p, (x, y));
        self.place_segments(cx, e, index, (self.parts.rise_anchor, 0));
        let p = &mut cx.effects[index];
        p.counter -= 1;
        if p.counter == 0 {
            p.origin = (p.x - e.x, p.y - e.y);
            p.angle = 0;
            Self::free_segments(cx);
        }
    }
    /// `$3023E`: crossing the waterline, a rising part leaves a list-9 wake
    /// splash on every update.
    fn waterline(&self, cx: &mut Ctx, index: usize) {
        let (x, y) = (cx.effects[index].x, cx.effects[index].y);
        let rows = self.parts.splash_rows;
        if !(rows.0..rows.1).contains(&(y as u16 as i32)) {
            return;
        }
        let water = -crate::special::WATER_FLOW << 16;
        let at = (x + ((y & 3) << 4), self.parts.splash_y);
        let wake = ambient::wake(&cx.shared.ambient, at, self.parts.splash_life, water, true);
        cx.effect(wake);
    }
    /// `$3005E`/`$3029C`: sixteen list-9 segments with the part's tile.
    fn spawn_segments(&self, cx: &mut Ctx, tile: u16) {
        for _ in 0..self.parts.segments {
            let mut s = Effect::new(EffectKind::BossPart, 0, 0, self.parts.segment_mapping);
            set_tile(&mut s, tile);
            s.phase = SEGMENT;
            s.back = true;
            cx.effect(s);
        }
    }
    /// `Func_030190`: the segments evenly from the part to the boss plus
    /// `anchor`, in 16ths of the distance, the first beside the part.
    fn place_segments(&self, cx: &mut Ctx, e: &Enemy, index: usize, anchor: (i32, i32)) {
        let (bx, by) = parent(e);
        let (x, y) = long(&cx.effects[index]);
        let dx = (bx + (anchor.0 << 16) - x) >> 4;
        let dy = (by + (anchor.1 << 16) - y) >> 4;
        let (mut sx, mut sy) = (x, y);
        for s in cx
            .effects
            .iter_mut()
            .filter(|s| s.kind == EffectKind::BossPart && s.phase == SEGMENT)
        {
            sx = sx.wrapping_add(dx);
            sy = sy.wrapping_add(dy);
            set_long(s, (sx, sy));
        }
    }
    /// `Func_0301C2`.
    pub(super) fn free_segments(cx: &mut Ctx) {
        for s in cx.effects.iter_mut() {
            if s.kind == EffectKind::BossPart && s.phase == SEGMENT {
                s.alive = false;
            }
        }
    }
    /// `Func_02FEBC` and the fall: the item, effect 1, a type-`$1D` blast
    /// at the part, eight pieces thrown from the boss (list 9 for the gun,
    /// list 1 for the arm) and the part's own drift and gravity.
    fn wreck(&self, cx: &mut Ctx, e: &Enemy, index: usize, back: bool) {
        let w = self.parts.wreck;
        cx.signal(crate::combat::Signal::Drop(crate::stage_objects::Drop {
            kind: w.item.0,
            x: w.item.1,
            y: w.item.2,
            vx: w.item_speed,
            high_priority: false,
        }));
        let (x, y) = (cx.effects[index].x, cx.effects[index].y);
        let blast = Self::blast(cx, 1);
        cx.explode(x, y, blast);
        let water = -crate::special::WATER_FLOW << 16;
        let tile = if back { 0 } else { 0x8000 };
        for _ in 0..8 {
            let r = cx.random();
            let vx = ((i32::from(r as u8 as i8) >> 5) + w.speed.0) << 16;
            let vy = -((i32::from(r >> 8) >> 6) + w.speed.1);
            let mut piece = Effect::new(EffectKind::Pieces, 0, 0, w.mapping);
            set_long(&mut piece, parent(e));
            set_tile(&mut piece, tile);
            piece.back = back;
            piece.motion.vx = vx.wrapping_add(water.wrapping_neg() >> 1);
            piece.motion.vy = vy << 16;
            piece.motion.gravity = w.gravity;
            countdown_life(&mut piece, w.life);
            cx.effect(piece);
        }
        let p = &mut cx.effects[index];
        p.motion.vx = self.parts.fall.0;
        p.motion.gravity = self.parts.fall.1;
    }

    /// A weapon's damage on the weak part (`loc_02FD56`): line 3 is filled
    /// with white again and the part shows it for its immunity. A part
    /// below zero is wrecked on its own update.
    pub(super) fn hit_part(cx: &mut Ctx, n: usize, damage: i32) {
        cx.fill_palette(3, 0x0eee);
        if let Some(p) = part(cx.effects, n) {
            p.base = (p.base & !PALETTE) | FLASH;
            p.period = 4;
            p.counter -= damage;
            if p.counter < 0 {
                p.base &= !PALETTE;
            }
        }
    }
    /// The weak part's collision box (relative to the boss) while it
    /// takes hits, and its immunity.
    pub(super) fn weak_part(&self, cx: &Ctx, e: &Enemy, n: usize) -> Option<(Hitbox, u16)> {
        let p = part_ref(cx.effects, n)?;
        let open = match p.phase {
            GUN => p.angle == 4,
            ARM => p.angle < 0x10 && destroyed(cx.effects, GUN_PART),
            _ => false,
        };
        if !open || p.counter < 0 {
            return None;
        }
        let mapping = cx.mappings.get(&p.mapping)?;
        // Object_LoadHitbox: the offsets follow the tile's flips.
        let mut area = mapping
            .bounds
            .placed(p.x - e.x, p.y - e.y, p.flip, p.base & 0x1000 != 0);
        if p.phase == ARM {
            (area.half_width, area.half_height) = self.parts.arm_box;
        }
        Some((area, p.period))
    }

    /// `Func_02F2A2`: a hull hit that leaves the flying machine's health
    /// under a threshold sets the next part burning, draws the damage on
    /// the hull and blasts a type-`$1D` there.
    pub(super) fn burn(&self, cx: &mut Ctx, e: &Enemy) {
        let health = e.step_x & 0xff;
        let Some(k) = self
            .parts
            .burns
            .iter()
            .position(|&(below, _)| health < below)
        else {
            return;
        };
        let (_, map) = self.parts.burns[k];
        let Some(p) = part(cx.effects, FIRST_BURN + k) else {
            return;
        };
        if p.angle & 4 != 0 {
            return;
        }
        p.angle |= 4;
        let at = (e.x + p.origin.0, e.y + p.origin.1);
        cx.plane_op(PlaneOp::ByteMap(map));
        let blast = Self::blast(cx, self.parts.burn_sound);
        cx.explode(at.0, at.1, blast);
    }
    /// `$2E242`: the ninth part burns from the start.
    pub(super) fn light_last(cx: &mut Ctx) {
        if let Some(p) = part(cx.effects, LAST_FIRE) {
            p.angle = 4;
        }
    }
}

/// The tile word (`+8`) of a part: base, the H flip kept apart.
fn tile_of(p: &Effect) -> u16 {
    p.base | if p.flip { 0x800 } else { 0 }
}
pub(super) fn set_tile(p: &mut Effect, tile: u16) {
    p.flip = tile & 0x800 != 0;
    p.base = tile & !0x800;
    p.high_priority = tile & 0x8000 != 0;
}
/// `loc_02FD9C`: the immunity runs out and the part's palette returns.
fn countdown(p: &mut Effect) {
    if p.period > 0 {
        p.period -= 1;
        if p.period == 0 {
            p.base &= !PALETTE;
        }
    }
}
/// A `+$28` countdown record (`$2B` routine `$14`) allocated during the
/// boss update: its list-1 update and `Object_Integrate` run in the same
/// frame (the effect pass), so it shows `life - 1` frames, the first
/// already moved.
pub(super) fn countdown_life(piece: &mut Effect, life: i32) {
    piece.counter = life - 1;
}
