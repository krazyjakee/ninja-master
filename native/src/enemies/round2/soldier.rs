//! Type `$04`, `Enemy04` (`$12FD8`): the rifle soldier.
use super::Gun;
use crate::{
    combat::{Enemy, EnemyState, Spawn},
    enemies::{Behavior, Ctx, Offsets, set_whole, shot_damage},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `$2A`: walk frame index; `$2C`: timer; `obj_flags` bit 1: crouching.
const WALK: usize = 0;
const TIMER: usize = 1;
const CROUCH: usize = 2;

/// `$04`: walks toward the player at 2 px/tick for 61 ticks (turning at
/// ledges and walls), stands and shoots once (`$12C`/`$12D`), or crouches
/// (`$12E`) and shoots when a player attack comes in above it; falls off
/// ledges; dies in the shared arc and explosion.
pub(crate) struct Soldier {
    walk_first: u16,
    walk_frames: i32,
    stand: u16,
    crouch: u16,
    fall: u16,
    walk_ticks: i32,
    stand_ticks: i32,
    crouch_ticks: i32,
    aim_ticks: i32,
    rest_ticks: i32,
    crouch_rest_ticks: i32,
    speed: i32,
    drop_gravity: i32,
    drop_vy: i32,
    reach: i32,
    rise: i32,
    death: u16,
    hurt: u16,
    gun: Gun,
    full_sound: u8,
}
impl Soldier {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            walk_first: word(rom, 0x131d2)?,
            walk_frames: word_i(rom, 0x131cc)?,
            stand: word(rom, 0x1307a)?,
            crouch: word(rom, 0x1308e)?,
            // Data_012F9C word 0, used by $13150 and $13196.
            fall: word(rom, 0x12f9c)?,
            walk_ticks: word_i(rom, 0x1302e)?,
            stand_ticks: word_i(rom, 0x13080)?,
            crouch_ticks: word_i(rom, 0x1309a)?,
            aim_ticks: word_i(rom, 0x130bc)?,
            rest_ticks: word_i(rom, 0x130da)?,
            crouch_rest_ticks: word_i(rom, 0x13114)?,
            speed: signed(rom, 0x1301c)? << 16,
            drop_gravity: word_i(rom, 0x13180)?,
            drop_vy: signed(rom, 0x13186)?,
            reach: word_i(rom, 0x13240)?,
            rise: word_i(rom, 0x13244)?,
            death: word(rom, 0x13000)?,
            hurt: word(rom, 0x13006)?,
            gun: Gun {
                // Data_013254: projectile ID 0 ($1CD64) and its type-2 casing.
                offsets: Offsets::read(rom, 0x13254)?,
                casing: Offsets::read(rom, 0x1325a)?,
                casing_kind: 0,
                speed: long(rom, 0x1cd86)?,
                mapping: word(rom, 0x1cd62)?,
                damage: shot_damage(rom, 0)?,
                sound: byte(rom, 0x1327f)?,
            },
            full_sound: byte(rom, 0x13287)?,
        })
    }
    /// `Func_01300A`: face the player and walk (routine 4 follows at once).
    fn walk(&self, cx: &Ctx, e: &mut Enemy) {
        e.facing_left = cx.player_left(e.x);
        let sign = if e.facing_left { -1 } else { 1 };
        e.motion.vx = set_whole(0, sign * (self.speed >> 16));
        e.vars[TIMER] = self.walk_ticks;
        e.routine = 4;
    }
    /// `Func_01316A`: without ground at the feet + 8 hop down reversed at
    /// half speed and fall ($2C).
    fn ledge(&self, e: &mut Enemy, terrain: &dyn Terrain) -> bool {
        if terrain.solid_at(e.x, e.y + 8) {
            return false;
        }
        e.motion.gravity = self.drop_gravity;
        e.motion.vy = set_whole(e.motion.vy, self.drop_vy);
        e.motion.vx = -(e.motion.vx >> 1);
        e.mapping = self.fall;
        e.routine = 0x2c;
        true
    }
    /// `Func_0131BC`: every fourth tick the next of the six walk poses.
    fn animate(&self, e: &mut Enemy) {
        if e.timer & 3 != 0 {
            return;
        }
        if e.vars[WALK] < 0 || e.vars[WALK] >= self.walk_frames {
            e.vars[WALK] = 0;
        }
        e.mapping = self.walk_first + e.vars[WALK] as u16;
        e.vars[WALK] += 1;
    }
    /// `Func_0131DE`: turn at a ledge (no ground 16 px ahead at the feet) or
    /// a wall (solid 16 px ahead, 16 px up).
    fn turn(e: &mut Enemy, terrain: &dyn Terrain) {
        let ahead = if e.facing_left { e.x - 16 } else { e.x + 16 };
        if !terrain.solid_at(ahead, e.y) || terrain.solid_at(ahead, e.y - 16) {
            e.facing_left = !e.facing_left;
            e.motion.vx = -e.motion.vx;
        }
    }
    fn shoot(&self, cx: &mut Ctx, e: &Enemy, crouched: bool) {
        if !self.gun.fire(cx, e, crouched) {
            // $13286: the list-4 allocation failed.
            cx.visible_sound(e.x, e.y, self.full_sound);
        }
    }
}
impl Behavior for Soldier {
    fn label(&self, _kind: u16) -> String {
        "Enemy04 $12FD8".into()
    }
    fn activate(&self, _cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.state = EnemyState::Waiting;
        // Object_SpawnFromRecord copies the flags with bit 15 set into $2A.
        e.vars[WALK] = i32::from((spawn.flags | 0x8000) as i16);
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $12FEA: Func_01DD9C, then Func_01300A.
                    e.timer = 0;
                    e.motion.stop();
                    e.death_routine = self.death;
                    e.hurt_routine = self.hurt;
                    self.walk(cx, e);
                }
                4 => {
                    // $13036
                    if self.ledge(e, terrain) {
                        return;
                    }
                    self.animate(e);
                    Self::turn(e, terrain);
                    e.routine = 8;
                    return;
                }
                8 => {
                    // $13052: walk; stand to shoot, or crouch under an attack.
                    if self.ledge(e, terrain) {
                        return;
                    }
                    self.animate(e);
                    Self::turn(e, terrain);
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] < 0 {
                        e.mapping = self.stand;
                        e.vars[TIMER] = self.stand_ticks;
                        e.routine = 0x0c;
                        e.motion.vx = 0;
                    } else if cx.attack_near(e.x, e.y, 0, self.reach, self.rise) {
                        e.mapping = self.crouch;
                        e.vars[CROUCH] = 1;
                        e.vars[TIMER] = self.crouch_ticks;
                        e.routine = 0x18;
                        e.motion.vx = 0;
                    }
                    return;
                }
                0x0c => {
                    // $130AC: aim, then fire with the next pose.
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    self.shoot(cx, e, false);
                    e.vars[TIMER] = self.aim_ticks;
                    e.mapping += 1;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $130C8
                    if self.ledge(e, terrain) {
                        return;
                    }
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    e.vars[TIMER] = self.rest_ticks;
                    e.mapping -= 1;
                    e.routine = 0x14;
                }
                0x14 | 0x1c => {
                    // $130E6 / $1311C: rest, then walk again ($13130).
                    if self.ledge(e, terrain) {
                        return;
                    }
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    e.vars[CROUCH] = 0;
                    self.walk(cx, e);
                }
                0x18 => {
                    // $130FA: crouched, then fire low.
                    if self.ledge(e, terrain) {
                        return;
                    }
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    self.shoot(cx, e, true);
                    e.vars[TIMER] = self.crouch_rest_ticks;
                    e.routine = 0x1c;
                }
                0x20 => {
                    // $13142: hurt (only a zero-damage hit) resumes.
                    e.routine = e.saved_routine;
                    return;
                }
                0x24 => {
                    // $1314A: score (central), Func_012E14 arc.
                    cx.death_arc(e, self.fall);
                    e.routine = 0x28;
                    return;
                }
                0x28 => {
                    // $1315C
                    if cx.landed_blast(e, terrain) {
                        Ctx::free(e);
                    }
                    return;
                }
                _ => {
                    // $131AA: fell off a ledge; freed off screen.
                    if e.offscreen {
                        Ctx::free(e);
                    }
                    return;
                }
            }
        }
    }
}
