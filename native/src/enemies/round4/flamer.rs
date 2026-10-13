//! Type `$30`, `Enemy30` (`$16558`): the flame-breathing flyer.
use super::count_down;
use crate::{
    combat::{Enemy, Motion, Projectile, Spawn},
    enemies::{Behavior, Ctx, shot_damage, stop},
    game::Terrain,
    rom::{Result, byte, long, sequence, signed, word, word_i},
};

/// `$2A`/`$2C` orbit angles, `$2E` timer, `$30` flame wave angle.
const F_ORBIT_Y: usize = 0;
const F_ORBIT_X: usize = 1;
const F_TIMER: usize = 2;
const F_WAVE: usize = 3;

/// `$30`: hovers in a slow ellipse (`Func_016664`) facing the player;
/// within 160 px it charges ($1CA, 61 ticks), then breathes a flame
/// (list-4 `$22`, damage 3) every fourth tick for 65 ticks ($1CB), each
/// flame drifting at -3.5 / +4.5 px/tick on a wave and animating through
/// `Data_01D9B0` for 24 ticks. Dies in the ordinary explosion.
pub(crate) struct Flamer {
    poses: [u16; 3],
    ticks: [i32; 3],
    range: i32,
    orbit: (u16, u16),
    wave_step: u16,
    /// `Data_016536`: unflipped and flipped (dx, dy, vx).
    flames: [(i32, i32, i32); 2],
    flame_frames: Vec<u16>,
    flame_damage: u8,
    sound: u8,
}
impl Flamer {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let flame = |p: usize| -> Result<(i32, i32, i32)> {
            Ok((signed(rom, p)?, signed(rom, p + 2)?, long(rom, p + 4)?))
        };
        Ok(Self {
            poses: [
                word(rom, 0x16546)?,
                word(rom, 0x165ea)?,
                word(rom, 0x16614)?,
            ],
            ticks: [
                word_i(rom, 0x165a4)?,
                word_i(rom, 0x165e4)?,
                word_i(rom, 0x1660e)?,
            ],
            range: word_i(rom, 0x165de)?,
            orbit: (word(rom, 0x16666)?, 4),
            wave_step: word(rom, 0x1d96e)?,
            flames: [flame(0x16536)?, flame(0x1653e)?],
            flame_frames: sequence(rom, 0x1d9b0)?,
            flame_damage: shot_damage(rom, 0x22)?,
            sound: byte(rom, 0x1661d)?,
        })
    }
    /// `Func_016664`.
    fn orbit(&self, cx: &Ctx, e: &mut Enemy) {
        e.vars[F_ORBIT_Y] = (e.vars[F_ORBIT_Y] + i32::from(self.orbit.0)) & 0xffff;
        e.vars[F_ORBIT_X] = (e.vars[F_ORBIT_X] + i32::from(self.orbit.1)) & 0xffff;
        e.motion.vy = cx.shared.sine.polar(e.vars[F_ORBIT_Y] as u16).0 >> 1;
        e.motion.vx = cx.shared.sine.polar(e.vars[F_ORBIT_X] as u16).0;
    }
    /// `Func_01D96C`: one list-4 `$22` flame.
    fn breathe(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.vars[F_WAVE] = (e.vars[F_WAVE] + i32::from(self.wave_step)) & 0xffff;
        let (dx, dy, vx) = self.flames[usize::from(e.facing_left)];
        let vy = cx.shared.sine.polar(e.vars[F_WAVE] as u16).0 >> 1;
        let mut flame = Projectile::new(
            e.x + dx,
            e.y + dy,
            0,
            true,
            self.flame_frames[0],
            self.flame_damage,
        )
        .with_motion(Motion {
            vx,
            vy,
            ..Motion::default()
        })
        // `Func_01D72E` leaves `obj_frame` 0; `$1D9E2` loads first.
        .unset_frame();
        flame.owner = Some(cx.current);
        flame.life = self.flame_frames.len() as u16;
        cx.shoot(flame);
    }
    /// `Func_01D9E2`: this object's flames show frame `age`.
    fn animate(&self, cx: &mut Ctx) {
        let me = cx.current;
        for shot in cx.projectiles.iter_mut() {
            if shot.owner == Some(me)
                && let Some(&mapping) = self.flame_frames.get(usize::from(shot.age))
            {
                shot.mapping = mapping;
            }
        }
    }
}
impl Behavior for Flamer {
    fn label(&self, _kind: u16) -> String {
        "Enemy30 $16558".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        self.animate(cx);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $16582
                    stop(e);
                    e.hurt_routine = 0x10;
                    e.death_routine = 0x14;
                    e.vars[F_TIMER] = self.ticks[0];
                    e.routine = 4;
                }
                4 => {
                    // $165AE: face the player (flipped while it is right).
                    e.facing_left = !cx.player_left(e.x);
                    e.mapping = self.poses[0];
                    self.orbit(cx, e);
                    if count_down(&mut e.vars[F_TIMER]) {
                        return;
                    }
                    e.vars[F_TIMER] = self.ticks[0];
                    if cx.player_dx(e.x) >= self.range {
                        return;
                    }
                    e.vars[F_TIMER] = self.ticks[1];
                    e.mapping = self.poses[1];
                    e.routine = 8;
                    return;
                }
                8 => {
                    // $165F4: charging.
                    self.orbit(cx, e);
                    if e.timer & 3 == 0 {
                        // Data_01652E -> Func_01DBF2 -> Func_031A7A.
                        cx.charge_sparkle((e.x, e.y - 56), 0);
                    }
                    if count_down(&mut e.vars[F_TIMER]) {
                        return;
                    }
                    e.vars[F_TIMER] = self.ticks[2];
                    e.mapping = self.poses[2];
                    e.routine = 0x0c;
                    cx.visible_sound(e.x, e.y, self.sound);
                    return;
                }
                0x0c => {
                    // $16624
                    self.orbit(cx, e);
                    if e.timer & 3 != 0 {
                        return;
                    }
                    self.breathe(cx, e);
                    if count_down(&mut e.vars[F_TIMER]) {
                        return;
                    }
                    e.vars[F_TIMER] = self.ticks[0];
                    e.routine = 4;
                    return;
                }
                0x10 => {
                    // $1665C
                    e.routine = e.saved_routine;
                    return;
                }
                _ => {
                    // $1664A
                    let blast = cx.shared.blasts[0];
                    cx.explode(e.x, e.y, blast);
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
}
