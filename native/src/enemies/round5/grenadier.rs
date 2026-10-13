//! Type `$28`, `Enemy28` (`$160C2`), with its grenade (list-4 `$23`, `$1DACE`).
use super::{ENTRANCE_SIGNAL, SIGNAL};
use crate::{
    combat::{Enemy, Motion, Projectile, Spawn, Sway},
    enemies::{Behavior, Ctx, HitResponse, Wreckage, shot_damage},
    game::Terrain,
    rom::{Result, byte, long, sequence, signed, word, word_i},
};

/// `$2A`: countdown; `$2C`: death countdown; the tracked grenade.
const T: usize = 0;
const DEATH_T: usize = 1;
const GRENADE: usize = 2;
const GRENADE_X: usize = 3;
const GRENADE_Y: usize = 4;

/// List-4 `$23` (`$1DACE`): the lobbed grenade.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Grenade {
    /// `Data_0160B0`: (dx, dy, 16.16 speed), unflipped then flipped.
    throws: [(i32, i32, i32); 2],
    mapping: u16,
    vy: i32,
    gravity: i32,
    damage: u8,
    /// Effect IDs: thrown (visible only, `$4B2E`) and 10 ticks later.
    throw_sound: u8,
    fuse_sound: u8,
    /// `$1D0EA`: flame angles, frames, gravity, Y offset and damage.
    flame_angles: [u16; 3],
    flame_frames: Vec<u16>,
    flame_gravity: i32,
    flame_drop: i32,
    flame_damage: u8,
}
impl Grenade {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let throw = |p: usize| -> Result<(i32, i32, i32)> {
            Ok((signed(rom, p)?, signed(rom, p + 2)?, long(rom, p + 4)?))
        };
        Ok(Self {
            throws: [throw(0x160b0)?, throw(0x160b8)?],
            mapping: word(rom, 0x1daf2)?,
            vy: long(rom, 0x1daf8)?,
            gravity: word_i(rom, 0x1db00)?,
            damage: shot_damage(rom, 0x23)?,
            throw_sound: byte(rom, 0x1db11)?,
            fuse_sound: byte(rom, 0x1db2b)?,
            flame_angles: [
                word(rom, 0x1d0e2)?,
                word(rom, 0x1d0e4)?,
                word(rom, 0x1d0e6)?,
            ],
            flame_frames: sequence(rom, 0x1d0d4)?,
            flame_gravity: word_i(rom, 0x1d170)?,
            flame_drop: word_i(rom, 0x1d10e)?,
            flame_damage: shot_damage(rom, 0x03)?,
        })
    }
    /// `Func_01DA7E`: thrown from the hand toward the facing at 2 px/tick
    /// plus 1/256 of the distance to the player (`obj_dx & $1FF`).
    fn throw(&self, cx: &mut Ctx, e: &Enemy) {
        let (dx, dy, speed) = self.throws[usize::from(e.facing_left)];
        let distance = cx.player_dx(e.x) & 0x1ff;
        let vx = speed + (distance << 8);
        // `$1DACE` sets `$363` after its first `Object_LoadHitbox`.
        let mut shot = Projectile::new(e.x + dx, e.y + dy, 0, true, self.mapping, self.damage)
            .with_motion(Motion {
                vx: if e.facing_left { -vx } else { vx },
                vy: self.vy,
                gravity: self.gravity,
                ..Motion::default()
            })
            .unset_frame();
        shot.owner = Some(cx.current);
        if cx.shoot(shot) {
            cx.visible_sound(e.x, e.y, self.throw_sound);
        }
    }
    /// Follow the object's grenade; when it has gone, a landing (terrain
    /// under its last position) bursts into flames (`$1DB36` -> `$1D0EA`).
    fn track(&self, cx: &mut Ctx, e: &mut Enemy, terrain: &dyn Terrain) {
        let me = cx.current;
        let found = cx
            .projectiles
            .iter()
            .find(|shot| shot.owner == Some(me) && shot.mapping == self.mapping && shot.alive)
            .map(|shot| (shot.x, shot.y, shot.age));
        match found {
            Some((x, y, age)) => {
                if e.vars[GRENADE] == 0 {
                    e.vars[GRENADE] = 1;
                }
                e.vars[GRENADE_X] = x;
                e.vars[GRENADE_Y] = y;
                // $1DB22: routine 8's nine-tick countdown ends at age 10.
                if age == 10 {
                    cx.effect_sound(self.fuse_sound);
                }
            }
            None if e.vars[GRENADE] != 0 => {
                e.vars[GRENADE] = 0;
                let (x, y) = (e.vars[GRENADE_X], e.vars[GRENADE_Y]);
                if terrain.solid_at(x, y + 16) || terrain.solid_at(x, y + 8) {
                    self.flames(cx, x, (y & !15) + self.flame_drop);
                }
            }
            None => {}
        }
    }
    /// `$1D0EA`: three list-4 `$03` flames rising from the landing point,
    /// allocated for `Data_01D0E2`'s angles from the third to the first,
    /// each swaying from side to side (`$1D144`, [`Sway`]).
    fn flames(&self, cx: &mut Ctx, x: i32, y: i32) {
        for &angle in self.flame_angles.iter().rev() {
            let (cos, sin) = cx.shared.sine.polar(angle);
            // `Func_01D0EA` leaves `obj_frame` 0; `$1D144` loads the box
            // before it sets a frame.
            let mut flame = Projectile::new(x, y, 0, true, 0, self.flame_damage)
                .with_motion(Motion {
                    vx: cos << 1,
                    vy: sin << 2,
                    gravity: self.flame_gravity,
                    ..Motion::default()
                })
                .unset_frame();
            flame.passes_terrain = true;
            flame.sway = Some(Sway {
                base: x,
                offset: 0,
                frames: self.flame_frames.clone(),
            });
            if !cx.shoot(flame) {
                return;
            }
        }
    }
}

/// `$28`: a grenadier kneeling behind cover, facing spawn flag bit 0. On
/// each 64-tick boundary it winds up (`$1BB`) for 65 ticks, lobs a grenade
/// (`$1BC`, 2 px/tick plus 1/256 of the player's distance, -7 px/tick up)
/// and rests 33 ticks: one throw every 128 ticks. The grenade bursts into
/// rising flames where it lands. One hit kills: thrown back in `$1BF` with
/// four wreckage pieces, then the debris burst. Inactive (not even
/// hittable) while off screen; freed while a `$10` enters.
pub(crate) struct Grenadier {
    wind: u16,
    throw_pose: u16,
    dead: u16,
    period: u16,
    wind_ticks: i32,
    rest_ticks: i32,
    after_ticks: i32,
    death: u16,
    death_vy: i32,
    death_gravity: i32,
    death_vx: i32,
    death_ticks: i32,
    grenade: Grenade,
    wreckage: Wreckage,
}
impl Grenadier {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            wind: word(rom, 0x160a6)?,
            throw_pose: word(rom, 0x160a8)?,
            dead: word(rom, 0x160c0)?,
            period: word(rom, 0x16158)?,
            wind_ticks: word_i(rom, 0x16168)?,
            rest_ticks: word_i(rom, 0x16194)?,
            after_ticks: word_i(rom, 0x161b4)?,
            death: word(rom, 0x16108)?,
            death_vy: long(rom, 0x161ca)?,
            death_gravity: word_i(rom, 0x161d2)?,
            death_vx: long(rom, 0x161d8)?,
            death_ticks: word_i(rom, 0x161ec)?,
            grenade: Grenade::load(rom)?,
            wreckage: Wreckage::load(rom, 0)?,
        })
    }
    fn entrance(cx: &Ctx) -> bool {
        cx.others
            .iter()
            .any(|other| other.live && other.vars[SIGNAL] == ENTRANCE_SIGNAL)
    }
}
impl Behavior for Grenadier {
    fn label(&self, _kind: u16) -> String {
        "Enemy28 $160C2".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        // $160C2: Sys_Flags bit 3 (a $10 entering) frees it at once.
        if Self::entrance(cx) {
            Ctx::free(e);
            return;
        }
        self.grenade.track(cx, e, terrain);
        // Off screen it does nothing at all (no timer, no hit test).
        if e.offscreen && e.routine != 0 && !e.dying() {
            return;
        }
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $16100: Func_01DD9C (also clears the timer); flip from
                    // spawn flag bit 0. The hurt routine stays 0.
                    e.motion.stop();
                    e.timer = 0;
                    e.death_routine = self.death;
                    e.facing_left = spawn.flags & 1 != 0;
                    e.mapping = self.throw_pose;
                    e.routine = 4;
                }
                4 => {
                    // $1614C: wind up on each 64-tick boundary.
                    if e.timer & self.period != 0 {
                        return;
                    }
                    e.mapping = self.wind;
                    e.vars[T] = self.wind_ticks;
                    e.routine = 8;
                }
                8 => {
                    // $16170
                    e.vars[T] -= 1;
                    if e.vars[T] >= 0 {
                        return;
                    }
                    self.grenade.throw(cx, e);
                    e.mapping = self.throw_pose;
                    e.vars[T] = self.rest_ticks;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $1619C
                    e.vars[T] -= 1;
                    if e.vars[T] >= 0 {
                        return;
                    }
                    e.mapping = self.wind;
                    e.vars[T] = self.after_ticks;
                    e.routine = 4;
                    return;
                }
                0x10 => {
                    // $161C0: thrown back 1.5 px/tick away from the player
                    // (score central), with the wreckage.
                    e.mapping = self.dead;
                    e.motion.vy = self.death_vy;
                    e.motion.gravity = self.death_gravity;
                    e.motion.vx = if cx.player_left(e.x) {
                        self.death_vx
                    } else {
                        -self.death_vx
                    };
                    e.vars[DEATH_T] = self.death_ticks;
                    e.routine = 0x14;
                    self.wreckage.throw(cx, e, 0x28);
                }
                _ => {
                    // $16206: 41 ticks, then the $36EEE debris burst.
                    e.vars[DEATH_T] -= 1;
                    if e.vars[DEATH_T] >= 0 {
                        return;
                    }
                    cx.burst(e.x, e.y);
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        if enemy.routine == 0 || enemy.offscreen {
            HitResponse::Pass
        } else {
            HitResponse::Accept
        }
    }
}
