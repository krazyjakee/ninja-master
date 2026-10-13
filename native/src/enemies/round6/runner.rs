//! Type `$9C`, `Enemy9C` (`$185B8`): the running ninja.
use super::touches_player;
use crate::{
    combat::{Enemy, Projectile, Spawn},
    enemies::{Behavior, Ctx, shot_damage},
    game::Terrain,
    rom::{Result, long, signed, word, word_i},
};

/// `$2A` run frame, `$2C` countdown (off level), `$2E` step countdown,
/// `$30` script step, `$48` behaviour, `$32` run velocity.
const FRAME: usize = 0;
const OFF_LEVEL: usize = 1;
const STEP_T: usize = 2;
const STEP: usize = 3;
const MODE: usize = 4;
const RUN_VX: usize = 5;

/// `$9C`: a ninja entering from a screen edge (spawn flag bit 0 picks the
/// edge, bit 1 a random one), landing on the floor and running at
/// 2.16 px/tick toward the player (`$13D`-`$142`). Level with the player
/// (within 10 px), one in four leaps at it (-6 px/tick, within 160 px),
/// the others slash (`$145`-`$148`, a four-tick list-4 cut, within 40 px)
/// and run away again. Off level it stands looking (`$143`/`$144`). It hops
/// over gaps and walls. One hit kills (`$149`, then the explosion).
pub(crate) struct Runner {
    run: Vec<(u16, u16)>,
    slash: Vec<(u16, i32, bool)>,
    look: Vec<(u16, i32)>,
    jump: u16,
    land_hit: u16,
    cut: u16,
    dead: u16,
    edges: [i32; 3],
    run_speed: i32,
    jump_chance: (u16, u16),
    slash_reach: i32,
    leap_reach: i32,
    level: i32,
    leap_vy: i32,
    hop_vy: i32,
    climb_vy: i32,
    jump_gravity: i32,
    jump_vx: i32,
    windup: i32,
    after: i32,
    look_ticks: i32,
    retry: i32,
    fall_ticks: i32,
    land_ticks: i32,
    cut_life: u16,
    cut_damage: u8,
    death: u16,
    death_vy: i32,
    death_gravity: i32,
    death_vx: i32,
}
impl Runner {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut run = Vec::new();
        for i in 0..6 {
            run.push((word(rom, 0x1851a + i * 4)?, word(rom, 0x1851c + i * 4)?));
        }
        let mut slash = Vec::new();
        for i in 0..8 {
            let p = 0x18532 + i * 6;
            let mapping = word(rom, p)?;
            if mapping & 0x8000 != 0 {
                break;
            }
            slash.push((mapping, word_i(rom, p + 2)?, word(rom, p + 4)? != 0));
        }
        let mut look = Vec::new();
        for i in 0..8 {
            let p = 0x1854c + i * 4;
            let mapping = word(rom, p)?;
            if mapping & 0x8000 != 0 {
                break;
            }
            look.push((mapping, word_i(rom, p + 2)?));
        }
        // Func_018AC2: 2.0 plus -$3FFF >> 1 + -$3FFF >> 3 in magnitude.
        let fraction = -i32::from(word(rom, 0x18ac8)?);
        Ok(Self {
            run,
            slash,
            look,
            jump: word(rom, 0x1855e)?,
            land_hit: word(rom, 0x18564)?,
            cut: word(rom, 0x18566)?,
            dead: word(rom, 0x1856e)?,
            edges: [
                signed(rom, 0x18568)?,
                signed(rom, 0x1856a)?,
                signed(rom, 0x1856c)?,
            ],
            run_speed: -long(rom, 0x18640)? - ((fraction >> 1) + (fraction >> 3)),
            jump_chance: (word(rom, 0x18654)?, word(rom, 0x18658)?),
            slash_reach: word_i(rom, 0x1870c)?,
            leap_reach: word_i(rom, 0x18734)?,
            level: word_i(rom, 0x13298)?,
            leap_vy: signed(rom, 0x18742)? << 16,
            hop_vy: long(rom, 0x18752)?,
            climb_vy: long(rom, 0x1875e)?,
            jump_gravity: word_i(rom, 0x18766)?,
            jump_vx: long(rom, 0x1877a)?,
            windup: word_i(rom, 0x1879a)?,
            after: word_i(rom, 0x18800)?,
            look_ticks: word_i(rom, 0x18870)?,
            retry: word_i(rom, 0x186d6)?,
            fall_ticks: word_i(rom, 0x18902)?,
            land_ticks: word_i(rom, 0x1896a)?,
            cut_life: word(rom, 0x18a68)?,
            cut_damage: shot_damage(rom, 0x2b)?,
            death: word(rom, 0x185e0)?,
            death_vy: long(rom, 0x189c8)?,
            death_gravity: word_i(rom, 0x189d0)?,
            death_vx: long(rom, 0x189de)?,
        })
    }
    /// `Func_01328E`: level with the player (within 10 px vertically).
    fn level(&self, cx: &Ctx, e: &Enemy) -> bool {
        cx.player_dy(e.y) < self.level
    }
    /// `Func_012CF4` (flip when `d0` is negative) and `Func_018AC2`
    /// (2.16 px/tick, negative when `toward` is left).
    fn head(&self, e: &mut Enemy, flip: bool, left: bool) {
        e.facing_left = flip;
        e.vars[RUN_VX] = if left {
            -self.run_speed
        } else {
            self.run_speed
        };
    }
    /// `Func_018854`: run again from the first frame.
    fn run_again(e: &mut Enemy) {
        e.motion.vx = e.vars[RUN_VX];
        e.vars[FRAME] = 0;
        e.routine = 4;
    }
    /// `Func_018A82`: the next run frame every fourth tick.
    fn animate(&self, e: &mut Enemy) {
        if e.timer & 3 != 0 {
            return;
        }
        let (mapping, _) = self.run[e.vars[FRAME] as usize % self.run.len()];
        e.mapping = mapping;
        e.vars[FRAME] = (e.vars[FRAME] + 1) % self.run.len() as i32;
    }
    /// `Func_012D1A`: solid at (x +- dx, y + dy) and open at (x +- dx, y - 8),
    /// the offset negated while flipped.
    fn probe(e: &Enemy, terrain: &dyn Terrain, dx: i32, dy: i32) -> bool {
        let x = if e.facing_left { e.x - dx } else { e.x + dx };
        terrain.solid_at(x, e.y + dy) && !terrain.solid_at(x, e.y - 8)
    }
    /// `.loc_018764`: jump with `vy`, 3 px/tick along the run.
    fn leap(&self, e: &mut Enemy, vy: i32) {
        e.motion.vy = vy;
        e.motion.gravity = self.jump_gravity;
        e.mapping = self.jump;
        e.routine = 0x2c;
        e.motion.vx = if e.vars[RUN_VX] < 0 {
            -self.jump_vx
        } else {
            self.jump_vx
        };
        e.vars[MODE] = 2;
    }
    /// `Func_018A32`: list-4 `$2B`, the cut ($14A) at the body for 4 ticks.
    fn cut(&self, cx: &mut Ctx, e: &Enemy) {
        let base = cx.base(0x9c);
        let mut cut =
            Projectile::new(e.x, e.y, 0, true, self.cut, self.cut_damage).with_art(base, false);
        // A hair of leftward motion mirrors the cut with the flipped body;
        // the fraction marks the still cut as colliding (moving).
        cut.motion.fx = 1;
        if e.facing_left {
            cut.motion.vx = -1;
        }
        cut.passes_terrain = true;
        cut.life = self.cut_life + 1;
        cx.shoot(cut);
    }
    /// `.loc_01891E`/`.loc_018970`: landing on the floor.
    fn landed(&self, e: &mut Enemy, terrain: &dyn Terrain, mapping: u16) -> bool {
        if !terrain.solid_at(e.x, e.y) {
            return false;
        }
        e.y &= !7;
        e.motion.stop();
        e.mapping = mapping;
        e.routine = 0x3c;
        e.vars[STEP_T] = self.land_ticks;
        true
    }
}
impl Behavior for Runner {
    fn label(&self, _kind: u16) -> String {
        "Enemy9C $185B8".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $185CA: enter at a screen edge, drop to the floor.
                    e.motion.stop();
                    e.timer = 0;
                    e.death_routine = self.death;
                    let mut edge = usize::from(spawn.flags & 1);
                    if spawn.flags & 2 != 0 {
                        edge += usize::from(cx.random() & 1);
                    }
                    e.x = cx.camera.0 + self.edges[edge];
                    // Func_012D92 (d7 = 1): 8-pixel steps down to the floor.
                    e.y &= !7;
                    for _ in 0..64 {
                        if terrain.solid_at(e.x, e.y) {
                            break;
                        }
                        e.y += 8;
                    }
                    // Faces and runs toward the player.
                    let left = cx.player_left(e.x);
                    e.vars[OFF_LEVEL] = Self::distance_word(cx, e);
                    self.head(e, !left, left);
                    let roll = cx.random() & self.jump_chance.0;
                    e.vars[MODE] = i32::from(roll < self.jump_chance.1);
                    e.motion.vx = e.vars[RUN_VX];
                    e.vars[FRAME] = 0;
                    e.routine = 4;
                }
                4 => {
                    // $1867A: run; hop gaps and walls.
                    self.animate(e);
                    if !Self::probe(e, terrain, 16, 1) {
                        self.leap(e, self.hop_vy);
                        return;
                    }
                    if Self::probe(e, terrain, 16, -16) {
                        self.leap(e, self.climb_vy);
                        return;
                    }
                    let level = self.level(cx, e);
                    if !level {
                        e.vars[OFF_LEVEL] -= 1;
                        if e.vars[OFF_LEVEL] < 0 {
                            e.vars[OFF_LEVEL] = self.retry;
                            e.routine = 0x1c;
                            return;
                        }
                    }
                    let distance = cx.player_dx(e.x);
                    match e.vars[MODE] {
                        0 if level && distance < self.slash_reach => {
                            e.routine = 8;
                            e.vars[MODE] = 2;
                        }
                        1 if level && distance < self.leap_reach => {
                            let fraction = i32::from(cx.random());
                            self.leap(e, self.leap_vy | fraction);
                        }
                        _ => {}
                    }
                    return;
                }
                8 => {
                    // $18792: stop and wind up.
                    e.motion.vx = 0;
                    e.vars[STEP_T] = self.windup;
                    e.mapping = self.slash[0].0;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $187AA
                    e.vars[STEP_T] -= 1;
                    if e.vars[STEP_T] >= 0 {
                        return;
                    }
                    e.vars[STEP] = 0;
                    e.routine = 0x10;
                    if self.slash_step(cx, e) {
                        continue;
                    }
                    return;
                }
                0x10 => {
                    // $187F0: the slash script, the cut on its last pose.
                    e.vars[STEP_T] -= 1;
                    if e.vars[STEP_T] >= 0 {
                        return;
                    }
                    e.vars[STEP] += 1;
                    if self.slash_step(cx, e) {
                        continue;
                    }
                    return;
                }
                0x14 => {
                    // $18808
                    e.vars[STEP_T] -= 1;
                    if e.vars[STEP_T] >= 0 {
                        return;
                    }
                    e.mapping = self.slash[0].0;
                    e.vars[STEP_T] = self.after;
                    e.routine = 0x18;
                }
                0x18 => {
                    // $18820: turn away from the player and run off.
                    e.vars[STEP_T] -= 1;
                    if e.vars[STEP_T] >= 0 {
                        return;
                    }
                    let left = cx.player_left(e.x);
                    e.vars[OFF_LEVEL] = Self::distance_word(cx, e);
                    self.head(e, left, !left);
                    Self::run_again(e);
                    return;
                }
                0x1c => {
                    // $18868: stand looking.
                    e.motion.vx = 0;
                    e.vars[STEP_T] = self.look_ticks;
                    e.mapping = self.look[0].0;
                    e.routine = 0x20;
                }
                0x20 => {
                    // $18880
                    e.vars[STEP_T] -= 1;
                    if e.vars[STEP_T] >= 0 {
                        return;
                    }
                    e.vars[STEP] = 0;
                    e.routine = 0x24;
                    match self.look.first() {
                        Some(&(mapping, ticks)) => {
                            e.mapping = mapping;
                            e.vars[STEP_T] = ticks;
                        }
                        None => {
                            e.motion.vx = e.vars[RUN_VX];
                            e.routine = 4;
                            return;
                        }
                    }
                }
                0x24 => {
                    // $188B2: look around until level again.
                    if self.level(cx, e) {
                        e.motion.vx = e.vars[RUN_VX];
                        e.routine = 4;
                        return;
                    }
                    e.vars[STEP_T] -= 1;
                    if e.vars[STEP_T] >= 0 {
                        return;
                    }
                    e.vars[STEP] += 1;
                    match self.look.get(e.vars[STEP] as usize) {
                        Some(&(mapping, ticks)) => {
                            e.mapping = mapping;
                            e.vars[STEP_T] = ticks;
                            continue;
                        }
                        None => {
                            e.motion.vx = e.vars[RUN_VX];
                            e.routine = 4;
                            return;
                        }
                    }
                }
                0x28 => {
                    // $188D8: run off, freed off screen.
                    self.animate(e);
                    if e.offscreen {
                        Ctx::free(e);
                    }
                    return;
                }
                0x2c => {
                    // $188F8: rising; falling, the next pose.
                    if e.motion.vy < 0 {
                        return;
                    }
                    e.vars[STEP_T] = self.fall_ticks;
                    e.mapping += 1;
                    e.routine = 0x30;
                }
                0x30 => {
                    // $1890E
                    e.vars[STEP_T] -= 1;
                    if e.vars[STEP_T] >= 0 {
                        return;
                    }
                    e.mapping += 1;
                    e.routine = 0x34;
                }
                0x34 => {
                    // $1891E: obj_flags bit 2 (touched the player) lands
                    // in the other pose.
                    if touches_player(cx, e) {
                        e.routine = 0x38;
                    }
                    self.landed(e, terrain, self.jump);
                    return;
                }
                0x38 => {
                    // $18970
                    self.landed(e, terrain, self.land_hit);
                    return;
                }
                0x3c => {
                    // $189AC: then run again.
                    e.vars[STEP_T] -= 1;
                    if e.vars[STEP_T] >= 0 {
                        return;
                    }
                    Self::run_again(e);
                    return;
                }
                r if r == self.death => {
                    // $189C6: thrown back 1.5 px/tick (score central).
                    e.motion.vy = self.death_vy;
                    e.motion.gravity = self.death_gravity;
                    e.mapping = self.dead;
                    e.motion.vx = if cx.player_left(e.x) {
                        self.death_vx
                    } else {
                        -self.death_vx
                    };
                    e.routine = self.death + 4;
                    return;
                }
                _ => {
                    // $18A14: the explosion on landing (native: or off screen).
                    if terrain.solid_at(e.x, e.y) {
                        let blast = cx.shared.blasts[0];
                        cx.explode(e.x, e.y, blast);
                        Ctx::free(e);
                    } else if e.offscreen {
                        Ctx::free(e);
                    }
                    return;
                }
            }
        }
    }
}
impl Runner {
    /// `obj_dx` as the ROM stores it (magnitude, bit 15 when the player is
    /// left), used as a countdown: negative at once for a player on the left.
    fn distance_word(cx: &Ctx, e: &Enemy) -> i32 {
        let dx = cx.player.0 - e.x;
        let magnitude = dx.abs() & 0x7fff;
        if dx < 0 {
            i32::from((magnitude | 0x8000) as u16 as i16)
        } else {
            magnitude
        }
    }
    /// `.loc_0187BC`: apply script step `vars[STEP]`; false when a step
    /// holds (return), true when the script ended and routine $14 follows.
    fn slash_step(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        match self.slash.get(e.vars[STEP] as usize).copied() {
            Some((mapping, ticks, cut)) => {
                e.mapping = mapping;
                e.vars[STEP_T] = ticks;
                if cut {
                    self.cut(cx, e);
                }
                false
            }
            None => {
                e.vars[STEP_T] = self.after;
                e.routine = 0x14;
                true
            }
        }
    }
}
