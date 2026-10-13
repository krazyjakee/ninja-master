//! Type `$B0`, `EnemyB0` (`$19284`): the gun that slides out of the wall.
use crate::{
    combat::{Effect, EffectKind, Enemy, Motion, Projectile, Reaction, Spawn, advance},
    enemies::{Behavior, Ctx, HitResponse, Offsets, frame_at, frame_list, shot_damage},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `$30` slide distance, `$34` countdown, `$28` frame-list counter,
/// `$2D` stun countdown.
const SLIDE: usize = 0;
const COUNTER: usize = 1;
const FRAME: usize = 2;
const STUN: usize = 3;

/// `$B0`: a gun hidden 96 px inside the wall (spawn flag bit 1). While the
/// player is within 64 px vertically it slides out 64 ticks at 1.5 px/tick,
/// aims 30 ticks (`$1F8`) and fires a 4.25 px/tick shot with a muzzle flash
/// and casing, recoils and waits 30; otherwise it retracts. Freed off
/// screen; one hit kills (debris burst).
pub(crate) struct WallGun {
    poses: [u16; 5],
    out: Vec<(u8, u16)>,
    back: Vec<(u8, u16)>,
    recoil: Vec<(u8, u16)>,
    hide: i32,
    wait: i32,
    reach: i32,
    step: i32,
    aim_ticks: i32,
    level: i32,
    window: i32,
    muzzle: i32,
    rise: i32,
    angle: u16,
    speed_shift: u32,
    mapping: u16,
    damage: u8,
    casing: Offsets,
    sound: u8,
}
impl WallGun {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let pose = |i: usize| word(rom, 0x192be + i * 2);
        Ok(Self {
            poses: [pose(0)?, pose(1)?, pose(2)?, pose(3)?, pose(4)?],
            out: frame_list(rom, 0x19404, 0x192be)?,
            back: frame_list(rom, 0x1945a, 0x192be)?,
            recoil: frame_list(rom, 0x19484, 0x192be)?,
            hide: word_i(rom, 0x19318)?,
            wait: word_i(rom, 0x19386)?,
            reach: word_i(rom, 0x193ce)?,
            step: long(rom, 0x193d8)?,
            aim_ticks: word_i(rom, 0x194e4)?,
            // Func_01932C: (y - player y + $40) below $80, unsigned.
            level: word_i(rom, 0x19336)?,
            window: word_i(rom, 0x1933a)?,
            muzzle: signed(rom, 0x19584)?,
            rise: signed(rom, 0x195a6)?,
            angle: word(rom, 0x19580)?,
            speed_shift: 2,
            // $30418 (list-4 ID $2A, handler $303EC).
            mapping: word(rom, 0x3041a)?,
            damage: shot_damage(rom, 0x2a)?,
            casing: Offsets::read(rom, 0x19602)?,
            sound: byte(rom, 0x1955b)?,
        })
    }
    /// `.loc_01937A`: wait 30 ticks.
    fn wait(&self, e: &mut Enemy) {
        e.routine = 4;
        e.vars[FRAME] = 0;
        e.vars[COUNTER] = self.wait;
    }
    /// `.loc_0194DC`: aim for 30 ticks.
    fn aim(&self, e: &mut Enemy) {
        e.routine = 0x1c;
        e.vars[COUNTER] = self.aim_ticks;
        e.mapping = self.poses[3];
    }
    /// `Func_01BA56`: the next frame of `list`; false past its end.
    fn play(e: &mut Enemy, list: &[(u8, u16)]) -> bool {
        match frame_at(list, e.vars[FRAME] as u16) {
            Some((_, mapping, _)) => {
                e.mapping = mapping;
                e.vars[FRAME] = (e.vars[FRAME] + 1) & 0xff;
                true
            }
            None => false,
        }
    }
    /// Slide step: 1.5 px/tick out of the wall (toward the facing).
    fn out_step(&self, spawn: &Spawn) -> i32 {
        if spawn.flags & 1 != 0 {
            self.step
        } else {
            -self.step
        }
    }
    /// `loc_019534`: projectile ID $2A ($303EC) at 72 px ahead, 12 up, with
    /// the list-1 muzzle flash and casing, and effect 9.
    fn fire(&self, cx: &mut Ctx, e: &Enemy, spawn: &Spawn) {
        let screen = (e.x - cx.camera.0 + 0x80 - 0x60) & 0xffff;
        if screen >= 0x180 {
            return;
        }
        let right = spawn.flags & 1 != 0;
        let (dx, angle) = if right {
            (-self.muzzle, 0)
        } else {
            (self.muzzle, self.angle)
        };
        let (vx, vy) = cx.shared.sine.polar(angle);
        let (x, y) = (e.x + dx, e.y + self.rise);
        let mut shot =
            Projectile::new(x, y, 0, true, self.mapping, self.damage).with_motion(Motion {
                vx: vx << self.speed_shift,
                vy: vy << self.speed_shift,
                ..Motion::default()
            });
        shot.passes_terrain = true;
        if !cx.shoot(shot) {
            return;
        }
        let first = cx.shared.muzzle.first().map_or(0, |frame| frame.1);
        cx.effect(Effect::new(EffectKind::Flash, x, y, first));
        let (x, y) = self.casing.place(e.x, e.y, e.facing_left, false);
        cx.casing(x, y, e.facing_left, 0);
        cx.effect_sound(self.sound);
    }
}
impl Behavior for WallGun {
    fn label(&self, _kind: u16) -> String {
        "EnemyB0 $19284".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, _terrain: &dyn Terrain) {
        // $19284: a gun in the wall is freed off screen (no score).
        if spawn.flags & 2 != 0 && e.offscreen && e.routine != 0 {
            Ctx::free(e);
            return;
        }
        // Func_0194FE's damage outcome, recorded on the previous tick.
        match cx.process_hit(e) {
            Some(Reaction::Death) => return,
            Some(Reaction::Hurt) => {
                e.vars[STUN] = 30;
                return;
            }
            _ => {}
        }
        match e.routine {
            0 => {
                // $19352
                e.motion.stop();
                e.offscreen = false;
                e.facing_left = spawn.flags & 1 == 0;
                if spawn.flags & 2 != 0 {
                    e.x += if spawn.flags & 1 != 0 {
                        -self.hide
                    } else {
                        self.hide
                    };
                }
                e.vars[SLIDE] = 0;
                e.mapping = self.poses[1];
                e.death_routine = 0x18;
                e.hurt_routine = 0x14;
                self.wait(e);
            }
            4 => {
                // $1938C: after 30 ticks, slide out while the player is level.
                e.vars[COUNTER] -= 1;
                if e.vars[COUNTER] != 0 {
                    return;
                }
                let level = ((e.y - cx.player.1 + self.level) & 0xffff) < self.window;
                if spawn.flags & 2 != 0 {
                    if level {
                        e.routine = 8;
                    } else {
                        e.routine = 0x0c;
                        e.vars[FRAME] = 0;
                    }
                } else if level {
                    self.aim(e);
                } else {
                    self.wait(e);
                }
            }
            8 => {
                // $193C8
                if e.vars[SLIDE] == self.reach {
                    self.aim(e);
                    return;
                }
                advance(&mut e.x, &mut e.motion.fx, self.out_step(spawn));
                e.vars[SLIDE] += 1;
                if !Self::play(e, &self.out) {
                    e.vars[FRAME] = 0;
                }
            }
            0x0c => {
                // $19420: retract.
                if e.vars[SLIDE] == 0 {
                    self.wait(e);
                    return;
                }
                advance(&mut e.x, &mut e.motion.fx, -self.out_step(spawn));
                e.vars[SLIDE] -= 1;
                if !Self::play(e, &self.back) {
                    e.vars[FRAME] = 0;
                }
            }
            0x10 => {
                // $1946E: recoil.
                if !Self::play(e, &self.recoil) {
                    self.wait(e);
                }
            }
            0x14 => {
                // $194AA: stunned (a zero-damage hit), not hittable.
                e.vars[STUN] -= 1;
                if e.vars[STUN] <= 0 {
                    e.routine = e.saved_routine;
                }
            }
            0x18 => {
                // $194CA: hit effect (sparked centrally), debris, free.
                cx.burst(e.x, e.y);
                Ctx::free(e);
            }
            _ => {
                // $194EE: aim, then fire and recoil.
                e.vars[COUNTER] -= 1;
                if e.vars[COUNTER] != 0 {
                    return;
                }
                e.routine = 0x10;
                e.vars[FRAME] = 0;
                self.fire(cx, e, spawn);
            }
        }
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        // Func_0194FE runs at the top of routines 4, 8, $C, $10 and $1C.
        if matches!(enemy.routine, 4 | 8 | 0x0c | 0x10 | 0x1c) {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
}
