//! The hydra's part objects: the tongue (list-4 `$3C`..`$68`), the jaw part
//! (`$4C`..`$58`) and its laser, run by the boss record on its [`State`], and
//! the list-5 `$78` puppets that show them.
use super::{
    HANDLER, Hydra, JAW_OPEN, LASER_HOLE, LASER_HOLE_CELL, LASER_LINE, LASER_ROWS, LASER_SHIFT,
    Record, State, WARNING, level,
};
use crate::{
    combat::{Enemy, Hitbox, Motion, PaletteOp, Plane, PlaneOp, Projectile, Signal, Spawn},
    enemies::{Behavior, Ctx, HitResponse},
    game::Terrain,
    rom::{Result, word},
};

// Tongue and jaw routines (the ROM part routines).
pub(super) const T_IDLE: u8 = 0x3c;
const T_SWEEP: u8 = 0x40;
const T_RISE: u8 = 0x44;
const T_RETURN: u8 = 0x48;
const T_DEATH_RISE: u8 = 0x64;
const T_DEATH_SINK: u8 = 0x68;
const J_LASER: u8 = 0x28;
const J_OPEN: u8 = 0x38;
pub(super) const J_IDLE: u8 = 0x4c;
const J_DROP: u8 = 0x50;
const J_INHALE: u8 = 0x54;
const J_RAISE: u8 = 0x58;
/// `$2CFA2`: the laser's `$2C`, ticks without the player test.
const LASER_QUIET: u8 = 2;
/// `.loc_02C952`: `addi.w #$FFD0` to the jaw's X before the pull test.
const INHALE_REACH: i32 = 0x30;

impl Hydra {
    /// Tongue (list-4 `$3C`..`$68`).
    pub(super) fn tongue(&self, cx: &mut Ctx, s: &mut State, dying: bool) {
        let cam_y = cx.camera.1 << 16;
        let home = cam_y + (self.tongue.y << 16);
        match s.t_rt {
            T_IDLE => {
                if dying {
                    s.t_x = (cx.camera.0 + self.tongue_x) << 16;
                    s.t_y = home;
                    s.t_rt = T_DEATH_RISE;
                } else if s.tongue_request {
                    s.t_x = (cx.camera.0 + self.tongue_x) << 16;
                    s.t_y = home;
                    s.t_drool = self.drools[level(cx)] as i16;
                    s.t_rt = T_RISE;
                    cx.effect_sound(0);
                }
            }
            T_RISE | T_DEATH_RISE => {
                let mut drool = i32::from(s.t_drool);
                self.drool(cx, &mut drool, s.t_x >> 16, self.tongue_drool_dx);
                s.t_drool = drool as i16;
                if s.t_rt == T_RISE && s.t_drool >= 0 {
                    return;
                }
                // Func_02C84A: approach camera Y + $E8.
                let target = cam_y + self.tongue_rise;
                let speed = ((s.t_y - target) >> 4).max(0x10000);
                s.t_y -= speed;
                if target >= s.t_y {
                    s.t_rt = if s.t_rt == T_RISE {
                        T_SWEEP
                    } else {
                        T_DEATH_SINK
                    };
                    s.t_count = self.tongue_sweep;
                }
            }
            T_SWEEP => {
                let mut drool = i32::from(s.t_drool);
                self.drool(cx, &mut drool, s.t_x >> 16, self.tongue_drool_dx);
                s.t_drool = drool as i16;
                s.t_x -= (s.t_x - 0x40) >> 5;
                s.t_count = s.t_count.saturating_sub(1);
                if s.t_count == 0 {
                    s.t_rt = T_RETURN;
                    cx.effect_sound(0);
                }
            }
            T_RETURN => {
                let mut drool = i32::from(s.t_drool);
                self.drool(cx, &mut drool, s.t_x >> 16, self.tongue_drool_dx);
                s.t_drool = drool as i16;
                s.t_x += self.tongue_step << 16;
                let speed = ((home - s.t_y) >> 5).max(0x10000);
                s.t_y += speed;
                if home < s.t_y || cx.outside_view(s.t_x >> 16, s.t_y >> 16) {
                    s.t_rt = T_IDLE;
                    s.t_y = home;
                    s.tongue_request = false;
                }
            }
            _ => {
                let mut drool = i32::from(s.t_drool);
                self.drool(cx, &mut drool, s.t_x >> 16, self.tongue_drool_dx);
                s.t_drool = drool as i16;
                s.t_y += 2 << 16;
            }
        }
    }
    /// Jaw part (`$4C`..`$58`) and the laser.
    pub(super) fn jaw(&self, cx: &mut Ctx, s: &mut State, dying: bool, health: u8) {
        // `$2C8AE` falls through `$2C8CA`, and `$2C8E8` falls through
        // `$2C904`: both entry updates run in the transition frame.
        if s.j_rt == J_IDLE && !dying && s.inhale_request {
            s.j_rt = J_DROP;
        }
        if s.j_rt == J_DROP && s.j_drop == self.drop_max {
            s.j_rt = J_INHALE;
            s.j_timer = self.inhales[level(cx)];
            cx.plane_op(PlaneOp::WordMap(JAW_OPEN));
        }
        match s.j_rt {
            J_IDLE => {
                if dying {
                    s.j_rt = J_OPEN;
                } else if s.inhale_request {
                    s.j_rt = J_DROP;
                }
            }
            J_DROP => {
                s.j_drop += 1;
            }
            J_INHALE => {
                // Func_02C968: in the last $40 ticks, line 1 flashes the
                // warning colour on even ticks; otherwise the health line.
                if s.j_timer < 0x40 && s.j_timer & 1 == 0 {
                    cx.fill_palette(1, WARNING);
                } else {
                    self.health_palette(cx, health);
                }
                s.j_timer -= 1;
                if s.j_timer <= 0 {
                    // loc_02C988: health line, shut jaw, fire, recoil.
                    self.health_palette(cx, health);
                    self.shut_jaw(cx, health);
                    s.j_rt = J_LASER;
                    s.j_recoil = self.recoil;
                    self.fire_laser(cx, s);
                    return;
                }
                if s.j_timer == self.inhale_sound_at {
                    cx.effect_sound(self.inhale_sound);
                }
                let (mx, my) =
                    jaw_position(cx, (s.x, s.y, s.phase), s.j_drop, (self.jaw.x, self.jaw.y));
                if s.j_timer & 3 == 0 {
                    // Func_031A7A: particles drawn into the mouth. Its tile
                    // word ($8000) is high priority, over the head's cells.
                    let (px, py) = (mx - 0x30, my + 0x18);
                    let first = cx.effects.len();
                    cx.charge_sparkle((px, py), 2);
                    if let Some(sparkle) = cx.effects.get_mut(first) {
                        sparkle.high_priority = sparkle.base & 0x8000 != 0;
                    }
                }
                // .loc_02C952: the inhale draws Joe in at 1 px/tick while he
                // is at or left of the jaw's X - $30 (an unsigned compare).
                if (cx.player.0 as u16) <= ((mx - INHALE_REACH) as u16) {
                    cx.signal(Signal::PushPlayer(1));
                }
            }
            J_LASER => {
                if s.j_recoil > 0 {
                    s.j_recoil -= 1;
                    s.x += 8 << 16;
                }
                if !s.laser {
                    // Func_02D2CC: black backdrop, the corridor rows back.
                    cx.palette_op(PaletteOp::Color(0, 0, 0));
                    cx.plane_op(PlaneOp::Restore {
                        vram: LASER_ROWS,
                        width: 64,
                        height: 4,
                    });
                    s.j_rt = J_RAISE;
                }
            }
            J_RAISE => {
                if s.j_drop == 0 {
                    // loc_02C89C
                    s.j_rt = J_IDLE;
                    s.inhale_request = false;
                } else {
                    s.j_drop -= 1;
                }
            }
            _ => {
                // $38: dying, the jaw hangs open.
                if s.j_drop < self.drop_max {
                    s.j_drop += 1;
                }
            }
        }
    }
    /// `Func_02CF5C` + `$2CF96`: LASER L and the beam across the floor.
    fn fire_laser(&self, cx: &mut Ctx, s: &mut State) {
        cx.effect_sound(self.laser_sound);
        let r = self.laser;
        let mut shot = Projectile::new(r.x, r.y, 0, true, r.frame, r.damage);
        shot.life = u16::from(self.laser_ticks);
        shot.passes_terrain = true;
        shot.motion = Motion {
            fx: 1,
            ..Motion::default()
        };
        if !cx.shoot(shot) {
            // $2E = -1: no laser, straight to the raise ($58).
            s.laser = false;
            s.j_rt = J_RAISE;
            return;
        }
        s.laser = true;
        s.laser_ticks = self.laser_ticks;
        s.laser_step = self.laser_start as i8;
        // Func_02D23A: the band's hole in plane A and a white corridor line;
        // the laser object clears the backdrop and opens its band.
        cx.plane_op(PlaneOp::Fill {
            vram: LASER_HOLE,
            width: 32,
            height: 4,
            cell: LASER_HOLE_CELL,
        });
        cx.palette_op(PaletteOp::Load(2, self.laser_white));
        cx.palette_op(PaletteOp::Color(0, 0, 0));
        self.laser_band(cx, self.laser_start);
    }
    /// `loc_02D1C2` for the laser's `$28` = `step`: plane A lines 144..175
    /// back to 0, then `Data_02D006[step]` of them, centred, shifted onto
    /// the hole so the backdrop shows.
    fn laser_band(&self, cx: &mut Ctx, step: u8) {
        cx.plane_op(PlaneOp::HScroll {
            plane: Plane::A,
            first: LASER_LINE,
            count: 32,
            value: 0,
        });
        let lines = self.laser_widths[usize::from(step & 31)].min(32);
        if lines != 0 {
            cx.plane_op(PlaneOp::HScroll {
                plane: Plane::A,
                first: LASER_LINE + ((32 - lines) & !1) / 2,
                count: lines,
                value: LASER_SHIFT,
            });
        }
    }
    /// `Func_02D1FC`: a random backdrop colour, the corridor line one step
    /// back from white.
    fn laser_backdrop(&self, cx: &mut Ctx) {
        let mut roll = || cx.random() as i16;
        let color = (roll() >> 4).wrapping_add(roll() >> 4).wrapping_add(roll()) as u16 & 0xfff;
        cx.palette_op(PaletteOp::Color(0, 0, color));
        cx.fade_palette(2, self.corridor);
    }
    /// `$2CFB8`: the beam thins after 16 ticks.
    pub(super) fn laser_tick(&self, cx: &mut Ctx, s: &mut State) {
        if !s.laser {
            return;
        }
        self.laser_backdrop(cx);
        if s.laser_ticks > 0 {
            // $2C = 2: the first two ticks skip the player test and run
            // Func_02D1FC again (the projectile stands in for the test).
            if s.laser_ticks + LASER_QUIET > self.laser_ticks {
                self.laser_backdrop(cx);
            }
            s.laser_ticks -= 1;
            return;
        }
        if s.laser_step < 0 {
            s.laser = false;
            return;
        }
        self.laser_band(cx, s.laser_step as u8);
        s.laser_step -= 1;
    }
}

/// `loc_02CA08`: the jaw sprite under the head at the boss state (X, Y,
/// phase) and drop, swaying with the wobble.
fn jaw_position(
    cx: &Ctx,
    (x, y, phase): (i32, i32, u16),
    drop: u8,
    (dx, dy): (i32, i32),
) -> (i32, i32) {
    let (cos, sin) = cx.shared.sine.polar(phase >> 1);
    let px = x + (dx << 16) - (cos << 2);
    let py = y + (0x110 << 16) + (dy << 16) + sin + (i32::from(drop) << 16);
    (px >> 16, py >> 16)
}

/// Puppet scratch: role (the ROM part routine it stands for).
pub(super) const ROLE: usize = 0;
/// `$2C` face (`Data_02C522`), `$30` neck, `$4C` jaw, `$3C` tongue.
pub(super) const ROLE_FACE: i32 = 1;
pub(super) const ROLE_NECK: i32 = 2;
pub(super) const ROLE_JAW: i32 = 3;
pub(super) const ROLE_TONGUE: i32 = 4;
/// A list-4 spit ball (`Data_02CA50`, routine `$5C`, `loc_02CB2E`).
pub(super) const ROLE_SPIT: i32 = 5;
/// Puppet scratch of a spit ball: its `$28` countdown.
pub(super) const SPIT_TICKS: usize = 1;
/// `Func_02CAA2`: `move.w #$20,$28(a6)`; `obj_tile` bit 15 is ORed in
/// while the decremented count is not negative (32 updates).
pub(super) const SPIT_HIGH: i32 = 0x20;

/// A `$78` part placed from the boss snapshot each tick, or a spit ball.
pub(crate) struct Puppet {
    face: Record,
    neck: Record,
    jaw: Record,
    tongue: Record,
    spit: Record,
    tongue_frame: u16,
    boxes: [Hitbox; 3],
}
impl Puppet {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let face = Record::load(rom, 0x2c522)?;
        let neck = Record::load(rom, 0x2c530)?;
        let tongue_frame = word(rom, 0x2c71a)?;
        let bounds =
            |id: u16| -> Result<Hitbox> { Ok(crate::combat::Mapping::load(rom, id)?.bounds) };
        Ok(Self {
            face,
            neck,
            jaw: Record::load(rom, 0x2c878)?,
            tongue: Record::load(rom, 0x2c63c)?,
            spit: Record::load(rom, 0x2ca50)?,
            tongue_frame,
            boxes: [
                bounds(face.frame)?,
                bounds(neck.frame)?,
                bounds(tongue_frame)?,
            ],
        })
    }
}
impl Puppet {
    /// `loc_02CB2E` after `Object_Integrate`: high priority for the first
    /// 32 updates, freed off screen or once a player weapon collided
    /// (`Enemy::struck`, last pass's `Object_CollideL3`). Joe's contact is
    /// the list-4 test, a one-tick shot of the ball's box and damage; it
    /// does not free the ball (`bclr #6` precedes the weapon test).
    fn spit_ball(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.motion.step(&mut e.x, &mut e.y);
        e.vars[SPIT_TICKS] -= 1;
        // `ori.w #$8000,8(a0)`, then `andi.w #$7FFF` once the count is
        // negative; `enemies::run` reads the layer back from the tile.
        e.high_priority = e.vars[SPIT_TICKS] >= 0;
        if let Some(tile) = &mut e.tile {
            *tile = (*tile & 0x7fff) | if e.high_priority { 0x8000 } else { 0 };
        }
        if e.struck || cx.outside_view(e.x, e.y) {
            Ctx::free(e);
            return;
        }
        let r = self.spit;
        super::hurtbox(cx, (e.x, e.y), r.frame, r.damage, false, false, false);
        if let Some(shot) = cx.projectiles.last_mut() {
            shot.high_priority = e.high_priority;
        }
    }
}
impl Behavior for Puppet {
    fn label(&self, _kind: u16) -> String {
        format!("Boss3_Hydra ${HANDLER:05X} part")
    }
    fn integrates(&self) -> bool {
        false
    }
    /// The boss record's [`super::hurtbox`]es carry the contact damage.
    fn touches_player(&self) -> bool {
        false
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        match e.vars[ROLE] {
            // Object_CollideL3 (Func_037244 is the body shove): weapons glance off.
            ROLE_FACE | ROLE_NECK => HitResponse::Deflect,
            // A list-4 object colliding with list 3: the weapon is spent.
            ROLE_TONGUE if e.body.is_some() => HitResponse::Ignore,
            // loc_02CB2E: Object_CollideL3 against the player's weapons.
            ROLE_SPIT => HitResponse::Collides,
            _ => HitResponse::Pass,
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        if e.vars[ROLE] == ROLE_SPIT {
            self.spit_ball(cx, e);
            return;
        }
        let Some(boss) = cx.parent(e).filter(|boss| boss.live) else {
            Ctx::free(e);
            return;
        };
        cx.process_hit(e);
        let (x, y) = (boss.vars[0] >> 16, (boss.vars[1] >> 16) + 0x110);
        match e.vars[ROLE] {
            ROLE_FACE => {
                (e.x, e.y) = (x + self.face.x, y + self.face.y);
                e.body = Some(self.boxes[0]);
                e.mapping = 0;
            }
            ROLE_NECK => {
                (e.x, e.y) = (x + self.neck.x, y + self.neck.y);
                e.body = Some(self.boxes[1]);
                e.mapping = 0;
            }
            ROLE_JAW => {
                let phase = boss.vars[2] as u16;
                let drop = State::drop_of(&boss.vars);
                (e.x, e.y) = jaw_position(
                    cx,
                    (boss.vars[0], boss.vars[1], phase),
                    drop,
                    (self.jaw.x, self.jaw.y),
                );
                e.mapping = self.jaw.frame;
                e.tile = Some(self.jaw.tile);
                e.body = None;
            }
            _ => {
                let (rt, tx, ty) = State::tongue_of(&boss.vars);
                if rt == T_IDLE {
                    e.mapping = 0;
                    e.body = None;
                } else {
                    (e.x, e.y) = (tx, ty);
                    e.mapping = self.tongue_frame;
                    e.tile = Some(self.tongue.tile & !0x800);
                    e.facing_left = self.tongue.tile & 0x800 != 0;
                    e.body = Some(self.boxes[2]);
                }
            }
        }
    }
}
