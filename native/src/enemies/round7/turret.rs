//! Type `$DC`, `EnemyDC` (`$1A146`), with its list-9 barrel (`$1A2E4`).
use super::{Shot, animate, runs, shot};
use crate::{
    combat::{Effect, EffectKind, Enemy, Motion, Spawn},
    enemies::{Behavior, Ctx, HitResponse, shot_damage},
    game::Terrain,
    rom::{Result, byte, long, sequence, signed, signed_byte, word, word_i},
};

/// `$2E`: countdown; `$2A`: barrel direction 0..23; `$34`: barrel recoil
/// (x, y); the flying barrel's spin counter.
const COUNT: usize = 0;
const DIR: usize = 1;
const RECOIL_X: usize = 2;
const RECOIL_Y: usize = 3;
const SPIN: usize = 4;
/// Native routine: the record carries the barrel after the turret died.
const BARREL: u16 = 0x20;

/// `$DC`: a wall turret. Only runs on screen and above camera Y + `$D8`.
/// Within 192 px on both axes it blinks (`$221`/`$222`) and turns its
/// barrel (list-9 `$1A2E4`, frames `$209 + direction`) toward the player's
/// chest for 61 ticks, holds 21, fires projectile `$15` (`$393`, 3 px/tick
/// along the barrel, from its tip) with a recoil of the barrel and a casing,
/// and starts over. Killed, it explodes and the barrel flies off spinning
/// (-3.5 px/tick up, away from the player) trailing smoke until it hits
/// terrain or leaves the screen.
pub(crate) struct Turret {
    blink: u16,
    idle: u16,
    barrel: u16,
    band: i32,
    range: i32,
    notice: i32,
    aim_ticks: i32,
    recoil_ticks: i32,
    chest: i32,
    directions: Vec<u8>,
    angles: Vec<u16>,
    shot: Shot,
    launch_vy: i32,
    launch_gravity: i32,
    spin: i32,
    trail: Vec<(u16, u16)>,
    trail_gravity: i32,
}
impl Turret {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut directions = Vec::with_capacity(32);
        for i in 0..32 {
            directions.push(byte(rom, 0x1a0c6 + i)?);
        }
        let mut angles = Vec::with_capacity(24);
        for i in 0..24 {
            angles.push(word(rom, 0x1a0e6 + i * 2)?);
        }
        Ok(Self {
            blink: word(rom, 0x1a0c2)?,
            idle: word(rom, 0x1a0c4)?,
            barrel: word(rom, 0x1a2d6)?,
            band: word_i(rom, 0x1a154)?,
            range: word_i(rom, 0x1a1ba)?,
            notice: word_i(rom, 0x1a1ce)?,
            aim_ticks: word_i(rom, 0x1a1f0)?,
            recoil_ticks: word_i(rom, 0x1a20e)?,
            chest: signed(rom, 0x1a268)?,
            directions,
            angles,
            shot: Shot {
                mapping: word(rom, 0x1a446)?,
                damage: shot_damage(rom, usize::from(byte(rom, 0x1a3d5)?))?,
                sound: byte(rom, 0x1a451)?,
            },
            launch_vy: long(rom, 0x1a34a)?,
            launch_gravity: word_i(rom, 0x1a352)?,
            spin: signed_byte(rom, 0x1a3a9)?,
            // Data_036878: list-1 type 5 ($3696A), one frame per tick, blinking.
            trail: runs(&sequence(rom, 0x36878)?, 1),
            trail_gravity: signed(rom, 0x1a39c)?,
        })
    }
    /// `Func_01A256`: barrel direction toward the player's chest (Y - 32).
    fn aim(&self, cx: &Ctx, e: &mut Enemy) {
        let angle = cx
            .shared
            .angle_to((e.x, e.y), (cx.player.0, cx.player.1 + self.chest));
        let dir = self
            .directions
            .get(usize::from(angle >> 3))
            .copied()
            .unwrap_or(0);
        e.vars[DIR] = i32::from(dir);
    }
    /// `Func_01A3D4`: projectile `$15` from 12 px along the barrel at
    /// 3 px/tick; the barrel recoils 2 px.
    fn fire(&self, cx: &mut Ctx, e: &mut Enemy) {
        let angle = self.angles.get(e.vars[DIR] as usize).copied().unwrap_or(0);
        let (c, s) = cx.shared.sine.polar(angle);
        let (vx, vy) = (c * 3, s * 3);
        let x = e.x + ((vx << 2) >> 16);
        let y = e.y + ((vy << 2) >> 16);
        let motion = Motion {
            vx,
            vy,
            ..Motion::default()
        };
        let mut bullet = shot(x, y, self.shot, motion);
        bullet.owner = Some(cx.current);
        if !cx.shoot(bullet) {
            return;
        }
        e.vars[RECOIL_X] = (-(c << 1)) >> 16;
        e.vars[RECOIL_Y] = (-(s << 1)) >> 16;
        cx.visible_sound(e.x, e.y, self.shot.sound);
    }
    fn barrel_at(&self, e: &Enemy) -> (i32, i32) {
        (e.x + e.vars[RECOIL_X], e.y + e.vars[RECOIL_Y])
    }
    /// `$1A2EE`: the barrel follows its turret, frame `$209 + direction`.
    fn draw_barrel(&self, cx: &mut Ctx, e: &Enemy) {
        let (x, y) = self.barrel_at(e);
        // $1A2A2: list 9, retaining the turret's tile and priority.
        let mapping = self.barrel + e.vars[DIR] as u16;
        let mut barrel = Effect::new(EffectKind::Pieces, x, y, mapping);
        barrel.base = cx.tile_base(0xdc);
        barrel.high_priority = e.high_priority;
        barrel.back = true;
        barrel.counter = 1;
        cx.effect(barrel);
    }
    /// `$1A366`: the loose barrel spins (`$2C` 23..0) and smokes every other
    /// tick; it explodes on terrain and is freed off screen.
    fn fly(&self, cx: &mut Ctx, e: &mut Enemy, terrain: &dyn Terrain) {
        if e.offscreen {
            Ctx::free(e);
            return;
        }
        if e.timer & 1 == 0 {
            let angle = cx.random() & 0x3ff;
            let (c, s) = cx.shared.sine.polar(angle);
            let motion = Motion {
                vx: c << 1,
                vy: s << 1,
                gravity: self.trail_gravity,
                ..Motion::default()
            };
            let first = cx.effects.len();
            let tile = if e.high_priority { 0x8000 } else { 0 };
            // $1A372 -> $3690C -> $12DD8: the trail shares the barrel's layer.
            animate(cx, (e.x, e.y), motion, tile, 0, &self.trail, true);
            for puff in &mut cx.effects[first..] {
                puff.high_priority = e.high_priority;
            }
        }
        let spin = if e.vars[SPIN] - 1 < 0 {
            self.spin
        } else {
            e.vars[SPIN] - 1
        };
        e.vars[SPIN] = spin;
        e.mapping = self.barrel + spin as u16;
        if terrain.object_solid_at(e.x, e.y, e.high_priority) {
            cx.small_blast(e.x, e.y);
            Ctx::free(e);
        }
    }
}
impl Behavior for Turret {
    fn label(&self, _kind: u16) -> String {
        "EnemyDC $1A146".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.idle
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        // Func_0128B2 while alive (routines 4..$10).
        if e.routine == 0 || e.routine >= 0x14 {
            HitResponse::Pass
        } else {
            HitResponse::Accept
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        if e.routine >= BARREL {
            self.fly(cx, e, terrain);
            return;
        }
        // $1A146: nothing runs off screen or below camera Y + $D8.
        let frozen = e.offscreen || e.y > cx.camera.1 + self.band;
        if !frozen {
            cx.process_hit(e);
            loop {
                match e.routine {
                    0 => {
                        // $1A170
                        e.hurt_routine = 0x1c;
                        e.death_routine = 0x14;
                        e.motion.stop();
                        e.timer = 0;
                        e.vars[RECOIL_X] = 0;
                        e.vars[RECOIL_Y] = 0;
                        self.aim(cx, e);
                        e.routine = 4;
                    }
                    4 => {
                        // $1A1A8: notice the player within 192 px.
                        e.mapping = self.idle;
                        let dx = cx.player_dx(e.x);
                        let dy = cx.player_dy(e.y);
                        if dx < self.range && dy < self.range {
                            e.vars[COUNT] = self.notice;
                            e.routine = 8;
                        }
                        break;
                    }
                    8 => {
                        // $1A1D8: blink and aim.
                        e.mapping = self.blink + ((e.timer >> 3) & 1);
                        self.aim(cx, e);
                        e.vars[COUNT] -= 1;
                        if e.vars[COUNT] >= 0 {
                            break;
                        }
                        e.vars[COUNT] = self.aim_ticks;
                        e.routine = 0x0c;
                        e.mapping = self.blink;
                        break;
                    }
                    0x0c => {
                        // $1A200
                        e.vars[COUNT] -= 1;
                        if e.vars[COUNT] >= 0 {
                            break;
                        }
                        self.fire(cx, e);
                        e.vars[COUNT] = self.recoil_ticks;
                        e.routine = 0x10;
                    }
                    0x10 => {
                        // $1A216: the casing (`Func_01A46C`; its side is
                        // obj_dx bit 1), then idle.
                        e.vars[COUNT] -= 1;
                        if e.vars[COUNT] >= 0 {
                            break;
                        }
                        let side = (cx.player_dx(e.x) >> 1) & 1 == 1;
                        let first = cx.effects.len();
                        cx.casing(e.x, e.y, side, 0);
                        // $1A46C allocates the unraised casing in list 9.
                        for casing in &mut cx.effects[first..] {
                            casing.back = true;
                        }
                        e.vars[RECOIL_X] = 0;
                        e.vars[RECOIL_Y] = 0;
                        e.routine = 4;
                        break;
                    }
                    0x14 => {
                        // $1A230/$1A23C: score (central), blast; the barrel
                        // ($1A33A) launches away from the player.
                        cx.small_blast(e.x, e.y);
                        let (x, y) = self.barrel_at(e);
                        e.x = x;
                        e.y = y;
                        e.motion = Motion {
                            vx: (e.x - cx.player.0) << 8,
                            vy: self.launch_vy,
                            gravity: self.launch_gravity,
                            ..Motion::default()
                        };
                        e.vars[SPIN] = 0;
                        // The body is freed; its original list-9 barrel survives.
                        e.back = true;
                        e.routine = BARREL;
                        self.fly(cx, e, terrain);
                        return;
                    }
                    _ => {
                        // $1A24E: hurt resumes.
                        e.routine = e.saved_routine;
                        break;
                    }
                }
            }
        }
        if e.routine < 0x14 && e.live() {
            self.draw_barrel(cx, e);
        }
    }
}
