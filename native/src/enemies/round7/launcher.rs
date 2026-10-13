//! Type `$EC`, `EnemyEC` (`$1B28A`), and its missile (projectile `$36`, `$1B460`).
use super::{Shot, bursts};
use crate::{
    combat::{Enemy, Motion, Projectile, ShotDown, Spawn},
    enemies::{Behavior, Ctx, HitResponse, set_whole, shot_damage},
    game::Terrain,
    rom::{Result, byte, signed, word, word_i},
};

/// `$2A`: countdown; `$2C`: rest; `$2E`: rest X; `$30`: shots left; hurt
/// countdown.
const EC_COUNT: usize = 0;
const EC_REST: usize = 1;
const EC_HOME: usize = 2;
const EC_SHOTS: usize = 3;
const EC_GUARD: usize = 4;

/// `$EC`: a launcher drawn from stage tiles (`$E000`). Runs only while its
/// X is within camera X + 1..`$1C0`. Waits `flags & $FFF` ticks, charges 61
/// (CHARGE `$34`, converging sparkles every 4 ticks), aims 31, then fires
/// three missiles (`$1A1`, -4 px/tick accelerating by -0.3125, damage 3,
/// effect `$1B`) 32 ticks apart, each kicking it 16 px back to slide home at
/// 1 px/tick. A hit makes it invulnerable for 17 ticks; six points kill it:
/// it is thrown up and back, bursting every 4 ticks, and explodes (`$36D90`)
/// on terrain.
pub(crate) struct Launcher {
    window: i32,
    tile: u16,
    frame: u16,
    charge_ticks: i32,
    charge_sound: u8,
    aim_ticks: i32,
    shots: i32,
    kick_ticks: i32,
    kick: i32,
    slide_ticks: i32,
    guard_ticks: i32,
    death_vx: i32,
    death_vy: i32,
    death_gravity: i32,
    death_ticks: i32,
    burst_rise: i32,
    burst_sound: u8,
    missile: Shot,
    missile_vx: i32,
    missile_ax: i32,
}
impl Launcher {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            window: word_i(rom, 0x1b296)?,
            tile: word(rom, 0x1b2b8)?,
            frame: word(rom, 0x1b2be)?,
            charge_ticks: word_i(rom, 0x1b2ea)?,
            charge_sound: byte(rom, 0x1b2f3)?,
            aim_ticks: word_i(rom, 0x1b30c)?,
            shots: word_i(rom, 0x1b312)?,
            kick_ticks: word_i(rom, 0x1b328)?,
            kick: word_i(rom, 0x1b334)?,
            slide_ticks: word_i(rom, 0x1b344)?,
            guard_ticks: word_i(rom, 0x1b388)?,
            death_vx: signed(rom, 0x1b3b8)?,
            death_vy: signed(rom, 0x1b3be)?,
            death_gravity: word_i(rom, 0x1b3c4)?,
            death_ticks: word_i(rom, 0x1b3ca)?,
            burst_rise: signed(rom, 0x1b3ea)?,
            burst_sound: byte(rom, 0x1b3f7)?,
            missile: Shot {
                mapping: word(rom, 0x1b44e)?,
                damage: shot_damage(rom, usize::from(byte(rom, 0x1b41f)?))?,
                sound: byte(rom, 0x1b459)?,
            },
            missile_vx: signed(rom, 0x1b442)?,
            missile_ax: signed(rom, 0x1b448)?,
        })
    }
    /// `Func_01B41E`: projectile `$36` leftward, accelerating.
    fn fire(&self, cx: &mut Ctx, e: &Enemy) {
        let mut missile =
            Projectile::new(e.x, e.y, 0, true, self.missile.mapping, self.missile.damage)
                .with_art(self.tile, true)
                .with_motion(Motion {
                    vx: self.missile_vx << 16,
                    ax: self.missile_ax,
                    ..Motion::default()
                });
        let blast = cx.shared.blasts[1]; // $36D90
        missile.contact_blast = Some(blast);
        missile.shot_down = Some(ShotDown {
            blast: Some(blast),
            first: 0,
            frames: 0,
            period: 0,
            sound: None,
            smoke: None,
        });
        if cx.shoot(missile) {
            cx.effect_sound(self.missile.sound);
        }
    }
}
impl Behavior for Launcher {
    fn label(&self, _kind: u16) -> String {
        "EnemyEC $1B28A".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.frame
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        // Func_0128E4 while alive and not guarding ($36 = -1 in $18/$1C).
        if (4..0x18).contains(&e.routine) {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        let left = cx.camera.0;
        if !(e.x > left && e.x <= left + self.window) {
            return;
        }
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $1B2B0: $2A = spawn flags | $8000, masked to 12 bits.
                    e.motion.stop();
                    e.timer = 0;
                    e.tile = Some(self.tile);
                    e.mapping = self.frame;
                    e.death_routine = 0x20;
                    e.hurt_routine = 0x18;
                    let rest = i32::from((spawn.flags | 0x8000) & 0xfff);
                    e.vars[EC_REST] = rest;
                    e.vars[EC_COUNT] = rest;
                    e.routine = 4;
                }
                4 => {
                    // $1B2E2
                    e.vars[EC_COUNT] -= 1;
                    if e.vars[EC_COUNT] >= 0 {
                        return;
                    }
                    e.vars[EC_COUNT] = self.charge_ticks;
                    e.routine = 8;
                    cx.effect_sound(self.charge_sound);
                    return;
                }
                8 => {
                    // $1B2FC: charge (`Func_01DBE6`).
                    if e.timer & 3 == 0 {
                        cx.charge_sparkle((e.x, e.y), 0);
                    }
                    e.vars[EC_COUNT] -= 1;
                    if e.vars[EC_COUNT] >= 0 {
                        return;
                    }
                    e.vars[EC_COUNT] = self.aim_ticks;
                    e.vars[EC_SHOTS] = self.shots;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $1B31A: fire and kick back.
                    e.vars[EC_COUNT] -= 1;
                    if e.vars[EC_COUNT] >= 0 {
                        return;
                    }
                    self.fire(cx, e);
                    e.vars[EC_COUNT] = self.kick_ticks;
                    e.vars[EC_HOME] = e.x;
                    e.x += self.kick;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $1B33C
                    e.vars[EC_COUNT] -= 1;
                    if e.vars[EC_COUNT] >= 0 {
                        return;
                    }
                    e.vars[EC_COUNT] = self.slide_ticks;
                    e.routine = 0x14;
                }
                0x14 => {
                    // $1B34C: slide home at 1 px/tick.
                    e.x = (e.x - 1).max(e.vars[EC_HOME]);
                    e.vars[EC_COUNT] -= 1;
                    if e.vars[EC_COUNT] >= 0 {
                        return;
                    }
                    e.routine = 0x0c;
                    e.vars[EC_COUNT] = -1;
                    e.vars[EC_SHOTS] -= 1;
                    if e.vars[EC_SHOTS] >= 0 {
                        continue;
                    }
                    e.vars[EC_COUNT] = e.vars[EC_REST];
                    e.routine = 4;
                    return;
                }
                0x18 => {
                    // $1B386: invulnerable 17 ticks.
                    e.vars[EC_GUARD] = self.guard_ticks;
                    e.routine = 0x1c;
                }
                0x1c => {
                    // $1B39C
                    e.vars[EC_GUARD] -= 1;
                    if e.vars[EC_GUARD] >= 0 {
                        return;
                    }
                    e.routine = e.saved_routine;
                    return;
                }
                0x20 => {
                    // $1B3B0: score (central), thrown up and back.
                    e.motion.vx = set_whole(e.motion.vx, self.death_vx);
                    e.motion.vy = set_whole(e.motion.vy, self.death_vy);
                    e.motion.gravity = self.death_gravity;
                    e.vars[EC_COUNT] = self.death_ticks;
                    e.routine = 0x24;
                }
                0x24 => {
                    // $1B3D2
                    e.vars[EC_COUNT] -= 1;
                    if e.vars[EC_COUNT] >= 0 {
                        return;
                    }
                    e.routine = 0x28;
                }
                _ => {
                    // $1B3DC: bursts until terrain; native removal off screen.
                    if e.timer & 3 == 0 {
                        bursts(cx, e, (0, self.burst_rise));
                        cx.effect_sound(self.burst_sound);
                    }
                    if terrain.solid_at(e.x, e.y) {
                        let blast = cx.shared.blasts[1];
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
