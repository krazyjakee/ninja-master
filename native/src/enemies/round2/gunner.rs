//! Type `$2C`, `Enemy2C` (`$16358`): the stationary gunner.
use super::Gun;
use crate::{
    combat::{Enemy, Spawn},
    enemies::{Behavior, Ctx, Offsets, Wreckage, set_whole, shot_damage},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `$2A` timer, `$2C` shots left in the burst / death countdown.
const WAIT: usize = 0;
const SHOTS: usize = 1;

/// `$2C`: a stationary gunner facing its spawn flag bit 0. After 121 ticks,
/// with the player within 160 px vertically, it fires three shots 5 ticks
/// apart (`$1C1`/`$1C0`); losing ground drops it. Death: thrown back
/// 1.5 px/tick with three wreckage pieces, then the debris burst.
pub(crate) struct Gunner {
    idle: u16,
    fire: u16,
    fall: u16,
    wait_ticks: i32,
    burst: i32,
    range: i32,
    step_ticks: i32,
    death: u16,
    death_vy: i32,
    death_gravity: i32,
    death_vx: i32,
    death_ticks: i32,
    drop_gravity: i32,
    gun: Gun,
    gun_offsets: [(i32, i32); 2],
    wreckage: Wreckage,
}
impl Gunner {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            idle: word(rom, 0x16340)?,
            fire: word(rom, 0x1633e)?,
            fall: word(rom, 0x16356)?,
            wait_ticks: word_i(rom, 0x163dc)?,
            burst: word_i(rom, 0x163e2)?,
            range: word_i(rom, 0x16404)?,
            step_ticks: word_i(rom, 0x16422)?,
            death: word(rom, 0x16392)?,
            death_vy: long(rom, 0x16476)?,
            death_gravity: word_i(rom, 0x1647e)?,
            death_vx: long(rom, 0x16484)?,
            death_ticks: word_i(rom, 0x16496)?,
            drop_gravity: word_i(rom, 0x164ce)?,
            gun: Gun {
                // Projectile ID $24 ($1DB7A) and the type-$1C casing.
                offsets: Offsets {
                    pairs: [(0, 0), (0, 0)],
                    extra: [0, 0],
                },
                casing: Offsets::read(rom, 0x36cfc)?,
                casing_kind: 1,
                speed: long(rom, 0x1dbae)?,
                mapping: word(rom, 0x1db78)?,
                damage: shot_damage(rom, 0x24)?,
                sound: byte(rom, 0x16527)?,
            },
            // Data_01DB70: word offsets, unflipped then flipped.
            gun_offsets: [
                (signed(rom, 0x1db70)?, signed(rom, 0x1db72)?),
                (signed(rom, 0x1db74)?, signed(rom, 0x1db76)?),
            ],
            // Func_01625E with $2A = 4.
            wreckage: Wreckage::load(rom, 1)?,
        })
    }
    /// `.loc_0163D2`: idle pose, 121 ticks, a three-shot burst next.
    fn idle(&self, e: &mut Enemy) {
        e.mapping = self.idle;
        e.vars[WAIT] = self.wait_ticks;
        e.vars[SHOTS] = self.burst;
        e.routine = 4;
    }
    /// `Func_0164BC`: no ground at the feet drops the gunner ($1C).
    fn support(&self, cx: &mut Ctx, e: &mut Enemy, terrain: &dyn Terrain) -> bool {
        if terrain.solid_at(e.x, e.y) {
            return false;
        }
        e.motion.gravity = self.drop_gravity;
        e.motion.vy = set_whole(e.motion.vy, -2);
        e.mapping = self.fall;
        e.routine = 0x1c;
        self.wreckage.throw(cx, e, 0x2c);
        true
    }
    fn shoot(&self, cx: &mut Ctx, e: &Enemy) {
        let (dx, dy) = self.gun_offsets[usize::from(e.facing_left)];
        let gun = Gun {
            offsets: Offsets {
                pairs: [(dx, dy), (dx, dy)],
                extra: [0, 0],
            },
            ..self.gun
        };
        gun.fire(cx, e, false);
    }
}
impl Behavior for Gunner {
    fn label(&self, _kind: u16) -> String {
        "Enemy2C $16358".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        0
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $1638A: faces spawn flag bit 0; the hurt routine stays 0.
                    e.timer = 0;
                    e.motion.stop();
                    e.death_routine = self.death;
                    e.facing_left = spawn.flags & 1 != 0;
                    self.idle(e);
                }
                4 => {
                    // $163EC: wait; the player must be within 160 px vertically.
                    if self.support(cx, e, terrain) {
                        return;
                    }
                    e.vars[WAIT] -= 1;
                    if e.vars[WAIT] >= 0 {
                        return;
                    }
                    if cx.player_dy(e.y) >= self.range {
                        self.idle(e);
                        continue;
                    }
                    e.routine = 8;
                }
                8 => {
                    // $1640C: fire.
                    if self.support(cx, e, terrain) {
                        return;
                    }
                    self.shoot(cx, e);
                    e.mapping = self.idle;
                    e.vars[WAIT] = self.step_ticks;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $1642A: recoil pose; burst over returns to idle.
                    if self.support(cx, e, terrain) {
                        return;
                    }
                    e.vars[WAIT] -= 1;
                    if e.vars[WAIT] >= 0 {
                        return;
                    }
                    e.mapping = self.fire;
                    e.vars[SHOTS] -= 1;
                    if e.vars[SHOTS] < 0 {
                        self.idle(e);
                        continue;
                    }
                    e.vars[WAIT] = self.step_ticks;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $16450
                    if self.support(cx, e, terrain) {
                        return;
                    }
                    e.vars[WAIT] -= 1;
                    if e.vars[WAIT] >= 0 {
                        return;
                    }
                    e.routine = 8;
                    return;
                }
                0x14 => {
                    // $16466: thrown back with the wreckage (score central).
                    e.mapping = self.fall;
                    e.motion.vy = self.death_vy;
                    e.motion.gravity = self.death_gravity;
                    e.motion.vx = if cx.player_left(e.x) {
                        self.death_vx
                    } else {
                        -self.death_vx
                    };
                    e.vars[SHOTS] = self.death_ticks;
                    e.routine = 0x18;
                    self.wreckage.throw(cx, e, 0x2c);
                }
                0x18 => {
                    // $164AA: 41 ticks, then the $36EEE debris burst.
                    e.vars[SHOTS] -= 1;
                    if e.vars[SHOTS] >= 0 {
                        return;
                    }
                    cx.burst(e.x, e.y);
                    Ctx::free(e);
                    return;
                }
                _ => {
                    // $164F4: dropped; freed off screen.
                    if e.offscreen {
                        Ctx::free(e);
                    }
                    return;
                }
            }
        }
    }
}
