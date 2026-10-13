//! Type `$64`, `Enemy64` (`$16D5E`): the perching flier.
use crate::{
    combat::{Enemy, Spawn},
    enemies::{Behavior, Ctx, aim},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `$2E`/`$30`: perch; `$2A`/`$2B` bytes: FLUTTER sounds left and their
/// 8-tick timer; `$2C`: re-aim countdown; `$2A` while dying: fall ticks.
const HOME_X: usize = 0;
const HOME_Y: usize = 1;
const FLAPS: usize = 2;
const FLAP_TIMER: usize = 3;
const STEER: usize = 4;
const FALL: usize = 5;

/// `$64`: perches until the player is within 128 px of the perch, then
/// flutters at a point 56 px above the player (acceleration re-aimed every
/// 16 ticks, speed capped at 2.25 px/tick per axis), flipping its sprite
/// every tick; more than 256 px away it flies home and perches again. It
/// dies falling and bursts into three puffs.
pub(crate) struct Flier {
    wing_first: u16,
    perch: u16,
    fall: u16,
    death: u16,
    steer_ticks: i32,
    wake: i32,
    rewake: i32,
    leave: i32,
    flaps: i32,
    flap_start: i32,
    flap_ticks: i32,
    flutter_sound: u8,
    chase_limit: i32,
    home_limit: i32,
    near: i32,
    arrive: i32,
    aim_offset: (i32, i32),
    fall_gravity: i32,
    fall_ticks: i32,
}
impl Flier {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let flaps = word(rom, 0x16db4)?;
        Ok(Self {
            wing_first: word(rom, 0x16f06)?,
            perch: word(rom, 0x16d9e)?,
            // Data_016D44
            fall: word(rom, 0x16d44)?,
            death: word(rom, 0x16d78)?,
            steer_ticks: word_i(rom, 0x16d8c)?,
            wake: word_i(rom, 0x16dae)?,
            rewake: word_i(rom, 0x16e1a)?,
            leave: word_i(rom, 0x16e08)?,
            flaps: i32::from((flaps >> 8) as u8 as i8),
            flap_start: i32::from(flaps as u8 as i8),
            flap_ticks: i32::from(byte(rom, 0x16dcd)?),
            flutter_sound: byte(rom, 0x16dd1)?,
            chase_limit: long(rom, 0x16de4)?,
            home_limit: long(rom, 0x16e4a)?,
            near: word_i(rom, 0x16e50)?,
            arrive: word_i(rom, 0x16e62)?,
            aim_offset: (signed(rom, 0x16f1c)?, signed(rom, 0x16f24)?),
            fall_gravity: word_i(rom, 0x16e8e)?,
            fall_ticks: word_i(rom, 0x16e94)?,
        })
    }
    /// `Func_016EEE`: flip every tick, wing pose every other tick.
    fn wings(&self, e: &mut Enemy) {
        e.facing_left = !e.facing_left;
        e.mapping = self.wing_first + ((e.timer >> 1) & 3);
    }
    /// `Func_016F3A`: cap each velocity axis at `limit`.
    fn clamp(e: &mut Enemy, limit: i32) {
        e.motion.vx = e.motion.vx.clamp(-limit, limit);
        e.motion.vy = e.motion.vy.clamp(-limit, limit);
    }
    /// `Func_016F26`: accelerate at 1/16 px/tick² toward `target`.
    fn steer(e: &mut Enemy, target: (i32, i32)) {
        let (ax, ay) = aim((e.x, e.y), target);
        e.motion.ax = ax >> 4;
        e.motion.gravity = ay >> 4;
    }
    /// `$16DA6`: wake when the player comes within 128 px of the perch.
    fn perched(&self, cx: &Ctx, e: &mut Enemy) {
        if cx.player_dx(e.vars[HOME_X]) < self.wake {
            e.vars[FLAPS] = self.flaps;
            e.vars[FLAP_TIMER] = self.flap_start;
            e.routine = 8;
        }
    }
}
impl Behavior for Flier {
    fn label(&self, _kind: u16) -> String {
        "Enemy64 $16D5E".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        match e.routine {
            0 => {
                // $16D70
                e.timer = 0;
                e.motion.stop();
                e.death_routine = self.death;
                e.vars[STEER] = self.steer_ticks;
                e.vars[HOME_X] = e.x;
                e.vars[HOME_Y] = e.y;
                e.mapping = self.perch;
                e.routine = 4;
                self.perched(cx, e);
            }
            4 => self.perched(cx, e),
            8 => {
                // $16DBE: two FLUTTER sounds eight ticks apart, then chase.
                if e.vars[FLAPS] >= 0 {
                    e.vars[FLAP_TIMER] -= 1;
                    if e.vars[FLAP_TIMER] < 0 {
                        e.vars[FLAP_TIMER] = self.flap_ticks;
                        cx.effect_sound(self.flutter_sound);
                        e.vars[FLAPS] -= 1;
                    }
                }
                self.wings(e);
                Self::clamp(e, self.chase_limit);
                e.vars[STEER] -= 1;
                if e.vars[STEER] < 0 {
                    e.vars[STEER] = self.steer_ticks;
                    let target = (
                        cx.player.0 + self.aim_offset.0,
                        cx.player.1 + self.aim_offset.1,
                    );
                    Self::steer(e, target);
                }
                if cx.player_dx(e.vars[HOME_X]) >= self.leave {
                    e.routine = 0x0c;
                }
            }
            0x0c => {
                // $16E12: fly home at 1.125 (0.5625 within 16) px/tick.
                if cx.player_dx(e.vars[HOME_X]) < self.rewake {
                    e.routine = 8;
                    return;
                }
                self.wings(e);
                Self::steer(e, (e.vars[HOME_X], e.vars[HOME_Y]));
                let dist = (e.vars[HOME_X] - e.x).abs() + (e.vars[HOME_Y] - e.y).abs();
                let limit = if dist < self.near {
                    self.home_limit >> 1
                } else {
                    self.home_limit
                };
                Self::clamp(e, limit);
                if dist < self.arrive {
                    e.mapping = self.perch;
                    e.motion.stop();
                    e.routine = 4;
                }
            }
            0x10 => {
                // $16E82: score (central); fall with the old sideways drift.
                e.motion.vx = 0;
                e.motion.vy = 0;
                e.motion.gravity = self.fall_gravity;
                e.vars[FALL] = self.fall_ticks;
                e.mapping = self.fall;
                e.routine = 0x14;
            }
            _ => {
                // $16EAA
                e.vars[FALL] -= 1;
                if e.vars[FALL] < 0 {
                    cx.small_blast(e.x, e.y);
                    Ctx::free(e);
                }
            }
        }
    }
}
