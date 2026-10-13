//! Type `$68`, `Enemy68` (`$1703C`): the flame vent.
use super::{count_down, pairs, round_sources};
use crate::{
    combat::{Effect, EffectKind, Enemy, EnemyState, Motion, Projectile, Spawn},
    enemies::{Behavior, Ctx, HitResponse, metatile_list, shot_damage, spawn_priority, stop},
    game::Terrain,
    rom::{Result, byte, pointer, signed, word},
};
use std::collections::BTreeSet;

/// `$2C` countdown, `$2E` off ticks, `$30` on ticks, `$32` emission step.
const V_TIMER: usize = 0;
const V_OFF: usize = 1;
const V_ON: usize = 2;
const V_STEP: usize = 3;
const V_FLAGS: usize = 4;
/// Marks the decorative (list-1) flames in `Effect::angle`.
const FLAME_TAG: u16 = 0x68;

/// `$68`: a vent in one of four directions (flags & 3) that jets flames for
/// ((flags >> 8 & $F) + 1) seconds and rests ((flags >> 4 & $F) + 1)
/// seconds. Every fourth tick of a jet it emits a flame at 4 px/tick
/// (angle +-8/1024), braked by a random $1000..$17FF; every eighth flame is
/// a damaging list-4 `$0D`, the others list-1 decoration. Flames animate
/// through `Data_01715E` ($362 .. $35F .. $362). Idle while off screen;
/// one hit destroys it.
pub(crate) struct Vent {
    /// `Data_016F70`: angle, dx, dy per direction.
    jets: [(u16, i32, i32); 4],
    /// `Data_016FD8` (round 4) and `Data_017018`: mapping and attribute.
    looks: [[(u16, u16); 4]; 2],
    frames: Vec<(u16, i32)>,
    /// `Data_017180`: which of the eight emission steps is list 4.
    damaging: [bool; 8],
    damage: u8,
    round4: BTreeSet<usize>,
    /// `Func_017124`: the damaged-vent metatiles by spawn flags & `$F`,
    /// from `Data_016F88` (round 4) and `Data_016FE8`.
    wrecks: [Vec<Vec<(u8, i32, i32)>>; 2],
}
impl Vent {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut jets = [(0, 0, 0); 4];
        for (i, jet) in jets.iter_mut().enumerate() {
            let p = 0x16f70 + i * 6;
            *jet = (word(rom, p)?, signed(rom, p + 2)?, signed(rom, p + 4)?);
        }
        let mut looks = [[(0, 0); 4]; 2];
        for (table, p) in looks.iter_mut().zip([0x16fd8, 0x17018]) {
            for (i, look) in table.iter_mut().enumerate() {
                *look = (word(rom, p + i * 4)?, word(rom, p + i * 4 + 2)?);
            }
        }
        // Sixteen index bytes, then one terminated list per index.
        let wrecks = |table: usize| -> Result<Vec<Vec<(u8, i32, i32)>>> {
            (0..16)
                .map(|i| metatile_list(rom, table + 0x10 + usize::from(byte(rom, table + i)?) * 8))
                .collect()
        };
        let mut damaging = [false; 8];
        for (i, flag) in damaging.iter_mut().enumerate() {
            // Object_AllocL4 ($16B0) or Object_AllocL1 ($16F0).
            *flag = pointer(rom, 0x17180 + i * 6 + 2)? == 0x16b0;
        }
        Ok(Self {
            jets,
            looks,
            frames: pairs(rom, 0x1715e)?,
            damaging,
            damage: shot_damage(rom, 0x0d)?,
            round4: round_sources(rom, 4, 0x68)?,
            wrecks: [
                wrecks(pointer(rom, 0x17126)?)?,
                wrecks(pointer(rom, 0x17138)?)?,
            ],
        })
    }
    /// `Func_017278`'s `Data_01715E` walk: one tick to read each entry,
    /// then `count` four-tick steps.
    fn frame(&self, age: u16) -> Option<u16> {
        let mut left = i32::from(age);
        for &(mapping, count) in &self.frames {
            let span = count * 4 + 1;
            if left < span {
                return Some(mapping);
            }
            left -= span;
        }
        None
    }
    fn life(&self) -> u16 {
        self.frames
            .iter()
            .map(|&(_, count)| count * 4 + 1)
            .sum::<i32>() as u16
    }
    /// `Func_0171E6` and `Func_017308`.
    fn emit(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.vars[V_STEP] += 1;
        let step = (e.vars[V_STEP] & 7) as usize;
        let direction = (e.vars[V_FLAGS] & 3) as usize;
        let (angle, dx, dy) = self.jets[direction];
        let spread = i32::from(cx.random() & 0xf) - 8;
        let (cos, sin) = cx.shared.sine.polar((i32::from(angle) + spread) as u16);
        let brake = (i32::from(cx.random()) & 0x7ff) + 0x1000;
        let (ax, ay) = match direction {
            0 => (0, -brake),
            1 => (brake, 0),
            2 => (0, brake),
            _ => (-brake, 0),
        };
        let motion = Motion {
            vx: cos << 2,
            vy: sin << 2,
            ax,
            gravity: ay,
            ..Motion::default()
        };
        let (x, y) = (e.x + dx, e.y + dy);
        let first = self.frames[0].0;
        if self.damaging[step] {
            let mut flame = Projectile::new(x, y, 0, true, first, self.damage).with_motion(motion);
            flame.passes_terrain = true;
            flame.owner = Some(cx.current);
            flame.life = self.life();
            cx.shoot(flame);
        } else {
            let mut flame = Effect::new(EffectKind::Pieces, x, y, first);
            // $17252..$1725A: list-1 decorations inherit the emitter's bit 15
            // just like its damaging list-4 flames. A low puff over a
            // raised box would otherwise hide both behind the high planes.
            flame.base = e.tile.unwrap_or(0) & 0x8000;
            flame.high_priority = flame.base != 0;
            flame.motion = motion;
            flame.counter = i32::from(self.life());
            flame.angle = FLAME_TAG;
            cx.effect(flame);
        }
    }
    /// The flames' own animation (idempotent over every vent's flames).
    fn animate(&self, cx: &mut Ctx) {
        let me = cx.current;
        for shot in cx.projectiles.iter_mut() {
            if shot.owner == Some(me)
                && let Some(mapping) = self.frame(shot.age)
            {
                shot.mapping = mapping;
            }
        }
        for flame in cx.effects.iter_mut() {
            if flame.kind == EffectKind::Pieces
                && flame.angle == FLAME_TAG
                && let Some(mapping) = self.frame(flame.age)
            {
                flame.mapping = mapping;
            }
        }
    }
}
impl Behavior for Vent {
    fn label(&self, _kind: u16) -> String {
        "Enemy68 $1703C".into()
    }
    fn activate(&self, _cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.state = EnemyState::Waiting;
        e.vars[V_FLAGS] = i32::from(spawn.flags | 0x8000);
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        if enemy.routine == 0 || enemy.offscreen {
            HitResponse::Pass
        } else {
            HitResponse::Accept
        }
    }
    fn integrates(&self) -> bool {
        // Stationary; Object_UpdateVisibility still runs natively below.
        false
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, _terrain: &dyn Terrain) {
        e.offscreen = cx.outside_view(e.x, e.y);
        if e.offscreen {
            return;
        }
        e.timer = e.timer.wrapping_add(1);
        self.animate(cx);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $17056
                    stop(e);
                    e.death_routine = 0x0c;
                    e.hurt_routine = 0x10;
                    let flags = e.vars[V_FLAGS];
                    e.vars[V_OFF] = (((flags & 0xf0) >> 4) + 1) * 60;
                    e.vars[V_ON] = (((flags >> 8) & 0xf) + 1) * 60;
                    e.vars[V_TIMER] = e.vars[V_ON];
                    let table = usize::from(!self.round4.contains(&spawn.source));
                    let (mapping, attr) = self.looks[table][(flags & 3) as usize];
                    e.mapping = mapping;
                    e.facing_left = attr & 0x800 != 0;
                    // $17068: `$2A` (spawn flags) bit 14 lifted into
                    // `obj_tile` bit 15, then `$170CE` ORs the attribute.
                    let priority = spawn_priority(flags as u16);
                    e.tile = Some((attr & !0x0800) | priority);
                    e.routine = 4;
                }
                4 => {
                    // $170D2: jetting.
                    if e.timer & 3 == 0 {
                        self.emit(cx, e);
                    }
                    if count_down(&mut e.vars[V_TIMER]) {
                        return;
                    }
                    e.vars[V_TIMER] = e.vars[V_OFF];
                    e.routine = 8;
                    return;
                }
                8 => {
                    // $170EA: resting.
                    if count_down(&mut e.vars[V_TIMER]) {
                        return;
                    }
                    e.vars[V_TIMER] = e.vars[V_ON];
                    e.routine = 4;
                    return;
                }
                0x0c => {
                    // $170FE: Func_017124 writes the damaged vent into the
                    // layout and plane A, then Func_036FF2's small blast.
                    let table = usize::from(!self.round4.contains(&spawn.source));
                    let wreck = &self.wrecks[table][(e.vars[V_FLAGS] & 0xf) as usize];
                    cx.write_metatiles(e.x, e.y, wreck);
                    cx.small_blast(e.x, e.y);
                    Ctx::free(e);
                    return;
                }
                _ => {
                    // $1711C
                    e.routine = e.saved_routine;
                    return;
                }
            }
        }
    }
}
