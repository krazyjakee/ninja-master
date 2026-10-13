//! Type `$50`, `Enemy50` (`$1C6EE`): the swooping flyer.
use super::{Shot, animate, runs, shot};
use crate::{
    combat::{Enemy, Motion, Spawn},
    enemies::{Behavior, Ctx, HitResponse, set_whole, shot_damage},
    game::Terrain,
    rom::{Result, byte, long, signed, signed_byte, word, word_i},
};

/// `$28`: swoop mode (0 level, 4 left, 8 right), then the shot/hover
/// countdown; `$2A`: mode started, then the death sway angle; `$2C`: sway
/// centre; `$2E`: smoke countdown.
const MODE: usize = 0;
const STARTED: usize = 1;
const HOME: usize = 2;
const SMOKE: usize = 3;

/// `$50`: waits above the top of the screen until the player is within 80
/// px horizontally, drops at 4 px/tick, brakes 64 px above the player and
/// climbs back, then swoops: level (1 px/tick, rising) when the player is
/// farther away than below, otherwise a -4 px/tick hop toward the player's
/// side. Once the player is at least as far below as away it passes at
/// 1.125 px/tick, firing one diagonal shot (`$393`, 3 px/tick down), keeps
/// 80..112 px above the player, hovers 8 ticks and swoops again. Any weapon
/// contact kills it: it falls at 1 px/tick swaying ±16 px and trailing
/// smoke, and explodes 32 ticks after reaching the ground.
pub(crate) struct Flyer {
    frame: u16,
    dead: u16,
    above: i32,
    reach: i32,
    drop: i32,
    lead: i32,
    brake: i32,
    climb: i32,
    climb_ay: i32,
    wander: i32,
    pass: i32,
    below: i32,
    swoop_ax: [i32; 2],
    swoop_vy: [i32; 2],
    swoop_gravity: [i32; 2],
    clamp: [u16; 2],
    hover: i32,
    band: (i32, i32),
    speed: (i32, i32),
    shot: Shot,
    fall: i32,
    sway: Vec<i32>,
    sway_start: i32,
    smoke_ticks: i32,
    land_smoke_ticks: i32,
    land_ticks: i32,
    /// Death hitbox written at `$1CAB6`: centre (x, y), half width, height.
    dead_box: (i32, i32, i32, i32),
    smoke: Vec<(u16, u16)>,
}
impl Flyer {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut sway = Vec::with_capacity(65);
        for i in 0..65 {
            // Data_034A00: (cos, sin) 4.12 pairs over half a turn.
            sway.push(i32::from(word(rom, 0x34a00 + i * 4)? as i16));
        }
        Ok(Self {
            frame: word(rom, 0x1c772)?,
            dead: word(rom, 0x1ca9e)?,
            above: word_i(rom, 0x1c780)?,
            reach: word_i(rom, 0x1c78c)?,
            drop: signed(rom, 0x1c7a4)?,
            lead: word_i(rom, 0x1c7b4)?,
            brake: signed(rom, 0x1c7be)?,
            climb: signed(rom, 0x1c7d8)?,
            climb_ay: signed(rom, 0x1c828)?,
            wander: long(rom, 0x1c842)?,
            pass: long(rom, 0x1c898)?,
            below: word_i(rom, 0x1c868)?,
            swoop_ax: [signed(rom, 0x1c8be)?, signed(rom, 0x1c934)?],
            swoop_vy: [long(rom, 0x1c8c4)?, long(rom, 0x1c93a)?],
            swoop_gravity: [signed(rom, 0x1c8cc)?, signed(rom, 0x1c942)?],
            clamp: [word(rom, 0x1c8d2)?, word(rom, 0x1c948)?],
            hover: word_i(rom, 0x1c880)?,
            band: (
                word_i(rom, 0x1c868)? + word_i(rom, 0x1c9c2)?,
                word_i(rom, 0x1c868)? + word_i(rom, 0x1c9da)?,
            ),
            speed: (signed(rom, 0x1ca3e)?, signed(rom, 0x1ca42)?),
            shot: Shot {
                mapping: word(rom, 0x1ca6e)?,
                damage: shot_damage(rom, usize::from(byte(rom, 0x1ca27)?))?,
                sound: byte(rom, 0x1ca79)?,
            },
            fall: long(rom, 0x1cab0)?,
            sway,
            sway_start: word_i(rom, 0x1cada)?,
            smoke_ticks: word_i(rom, 0x1cad0)?,
            land_smoke_ticks: word_i(rom, 0x1cb74)?,
            land_ticks: word_i(rom, 0x1cb7e)?,
            dead_box: (
                signed_byte(rom, 0x1cac5)?,
                signed_byte(rom, 0x1cacb)?,
                i32::from(byte(rom, 0x1cab9)?),
                i32::from(byte(rom, 0x1cabf)?),
            ),
            // Data_037B8C: list-1 type 9 ($37BA4), one frame per tick.
            // Data_037B8C has no terminator: $37BFE stops after 12 frames.
            smoke: runs(
                &(0..12)
                    .map(|i| word(rom, 0x37b8c + i * 2))
                    .collect::<Result<Vec<_>>>()?,
                1,
            ),
        })
    }
    /// |player X - X| and how far the player is below X, Y + 40 (`$1CBB2`).
    fn gaps(&self, cx: &Ctx, e: &Enemy) -> (i32, i32) {
        let across = cx.player_dx(e.x);
        let below = (cx.player.1 - self.below - e.y).max(0);
        (across, below)
    }
    /// `Func_01CBB2`: the next swoop mode.
    fn choose(&self, cx: &Ctx, e: &mut Enemy) {
        e.vars[STARTED] = 0;
        let (across, below) = self.gaps(cx, e);
        e.vars[MODE] = if below < across {
            0
        } else if cx.player.0 - e.x < 0 {
            4
        } else {
            8
        };
    }
    /// End of a swoop: pass at 1.125 px/tick and fire (`$18`).
    fn pass(&self, e: &mut Enemy, left: bool) {
        e.motion.vx = if left { -self.pass } else { self.pass };
        e.motion.vy = 0;
        e.motion.ax = 0;
        e.motion.gravity = 0;
        e.vars[MODE] = self.hover;
        e.routine = 0x18;
    }
    /// `Func_01CA26`: projectile 0 (routine 4, `$1CDE4`) at the object,
    /// 3 px/tick toward the player's side and 3 down.
    fn fire(&self, cx: &mut Ctx, e: &Enemy) {
        let vx = if cx.player_left(e.x) {
            -self.speed.0
        } else {
            self.speed.0
        };
        let motion = Motion {
            vx: vx << 16,
            vy: self.speed.1 << 16,
            ..Motion::default()
        };
        if cx.shoot(shot(e.x, e.y, self.shot, motion)) {
            cx.visible_sound(e.x, e.y, self.shot.sound);
        }
    }
    /// `Func_01CBF8`: a list-1 type-9 smoke puff somewhere in the hitbox.
    fn smoke(&self, cx: &mut Ctx, e: &Enemy) {
        let (hx, hy, hw, hh) = self.dead_box;
        let x = e.x + hx + ((2 * hw * i32::from(cx.random() & 0xff)) >> 8) - hw;
        let y = e.y + hy + ((2 * hh * i32::from(cx.random() & 0xff)) >> 8) - hh;
        animate(cx, (x, y), Motion::default(), 0, 0, &self.smoke, false);
    }
}
impl Behavior for Flyer {
    fn label(&self, _kind: u16) -> String {
        "Enemy50 $1C6EE".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.frame
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        // $1C6F2: Object_CollideL3 while obj_routine < $20; any contact is
        // lethal (health counter 0).
        if e.routine == 0 || e.routine >= 0x20 {
            HitResponse::Pass
        } else {
            HitResponse::Accept
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $1C756
                    e.motion.stop();
                    e.death_routine = 0x20;
                    e.mapping = self.frame;
                    e.routine = 4;
                }
                4 => {
                    // $1C77A: above the screen until the player is near.
                    e.y = cx.camera.1 - self.above;
                    let px = cx.player.0;
                    if e.x <= px + self.reach && e.x > px - self.reach {
                        e.motion.vy = set_whole(e.motion.vy, self.drop);
                        e.routine = 8;
                    }
                    return;
                }
                8 => {
                    // $1C7AE: brake once 64 px above the player.
                    if e.y > cx.player.1 - self.lead {
                        e.motion.gravity = self.brake;
                        e.routine = 0x0c;
                    }
                    return;
                }
                0x0c => {
                    // $1C7C8: climb at 1 px/tick.
                    if e.motion.vy < 0 {
                        e.motion.vy = self.climb << 16;
                        e.motion.gravity = 0;
                        e.routine = 0x10;
                    }
                    return;
                }
                0x10 => {
                    // $1C7E2
                    if e.y <= cx.player.1 - self.lead {
                        e.motion.vy = 0;
                        self.choose(cx, e);
                        e.routine = 0x14;
                    }
                    return;
                }
                0x14 => {
                    // $1C800: dispatch on $28.
                    let (across, below) = self.gaps(cx, e);
                    match e.vars[MODE] {
                        0 => {
                            // $1C814: level, the whole Y velocity pinned at -1.
                            if e.vars[STARTED] == 0 {
                                e.vars[STARTED] = 1;
                                e.motion.vy = -1;
                                e.motion.gravity = self.climb_ay;
                                e.motion.vx = if cx.player.0 - e.x >= 0 {
                                    self.wander
                                } else {
                                    -self.wander
                                };
                            }
                            if ((e.motion.vy >> 16) as u16) < 0xffff {
                                e.motion.vy = set_whole(e.motion.vy, -1);
                            }
                            if below >= across {
                                let left = cx.player.0 - e.x < 0;
                                self.pass(e, left);
                            }
                        }
                        4 => {
                            // $1C8AA: hop left, X velocity not below -2.
                            if e.vars[STARTED] == 0 {
                                e.vars[STARTED] = 1;
                                e.motion.vx = -1;
                                e.motion.ax = self.swoop_ax[0];
                                e.motion.vy = self.swoop_vy[0];
                                e.motion.gravity = self.swoop_gravity[0];
                            }
                            if ((e.motion.vx >> 16) as u16) < self.clamp[0] {
                                e.motion.vx = set_whole(e.motion.vx, self.clamp[0] as i16 as i32);
                            }
                            if below <= across {
                                self.pass(e, false);
                            }
                        }
                        _ => {
                            // $1C924: hop right, whole X velocity at most 2.
                            if e.vars[STARTED] == 0 {
                                e.vars[STARTED] = 1;
                                e.motion.vx = 0;
                                e.motion.ax = self.swoop_ax[1];
                                e.motion.vy = self.swoop_vy[1];
                                e.motion.gravity = self.swoop_gravity[1];
                            }
                            if ((e.motion.vx >> 16) as u16) >= self.clamp[1] {
                                e.motion.vx = set_whole(e.motion.vx, i32::from(self.clamp[1]));
                            }
                            if below <= across {
                                self.pass(e, true);
                            }
                        }
                    }
                    return;
                }
                0x18 => {
                    // $1C996: shoot once, flap, keep 80..112 px above.
                    if e.vars[MODE] == self.hover {
                        self.fire(cx, e);
                    }
                    e.mapping = self.frame + (e.vars[MODE] & 1) as u16;
                    let py = cx.player.1;
                    if e.y > py - self.band.0 {
                        e.y -= 1;
                    } else if e.y <= py - self.band.1 {
                        e.y += 1;
                    }
                    if e.vars[MODE] != 0 {
                        e.vars[MODE] -= 1;
                        return;
                    }
                    if e.y > py - self.band.0 || e.y <= py - self.band.1 {
                        return;
                    }
                    e.motion.vx = 0;
                    e.vars[MODE] = self.hover;
                    e.routine = 0x1c;
                    return;
                }
                0x1c => {
                    // $1CA82: hover, then swoop again.
                    e.mapping = self.frame;
                    e.vars[MODE] -= 1;
                    if e.vars[MODE] == 0 {
                        self.choose(cx, e);
                        e.routine = 0x14;
                    }
                    return;
                }
                0x20 => {
                    // $1CA9C: score (central), small blast, fall.
                    e.mapping = self.dead;
                    e.motion.vx = 0;
                    e.motion.ax = 0;
                    e.motion.gravity = 0;
                    e.motion.vy = self.fall;
                    e.vars[SMOKE] = self.smoke_ticks;
                    e.vars[MODE] = 0;
                    e.vars[STARTED] = self.sway_start;
                    e.vars[HOME] = e.x;
                    e.routine = 0x24;
                    cx.small_blast(e.x, e.y);
                    return;
                }
                0x24 => {
                    // $1CAF4: sway down until off screen or on the ground.
                    if e.offscreen {
                        cx.small_blast(e.x, e.y);
                        Ctx::free(e);
                        return;
                    }
                    if e.vars[MODE] != 0 {
                        e.vars[STARTED] += 2;
                        if e.vars[STARTED] > 64 {
                            e.vars[STARTED] = (-e.vars[STARTED]) & 63;
                            e.vars[MODE] = 0;
                        }
                    } else {
                        e.vars[STARTED] -= 2;
                        if e.vars[STARTED] < 0 {
                            e.vars[STARTED] = -e.vars[STARTED];
                            e.vars[MODE] += 1;
                        }
                    }
                    let cos = self
                        .sway
                        .get(e.vars[STARTED] as usize)
                        .copied()
                        .unwrap_or(0);
                    e.x = e.vars[HOME] + ((cos * 16) >> 12);
                    e.vars[SMOKE] -= 1;
                    if e.vars[SMOKE] == 0 {
                        e.vars[SMOKE] = self.smoke_ticks;
                        self.smoke(cx, e);
                    }
                    if terrain.solid_at(e.x, e.y) {
                        e.vars[SMOKE] = self.land_smoke_ticks;
                        e.motion.vy = 0;
                        e.vars[MODE] = self.land_ticks;
                        e.routine = 0x28;
                    }
                    return;
                }
                0x28 => {
                    // $1CB88: smoke on the ground.
                    e.vars[SMOKE] -= 1;
                    if e.vars[SMOKE] == 0 {
                        e.vars[SMOKE] = self.land_smoke_ticks;
                        self.smoke(cx, e);
                    }
                    e.vars[MODE] -= 1;
                    if e.vars[MODE] == 0 {
                        e.routine = 0x2c;
                    }
                    return;
                }
                _ => {
                    // $1CBA6
                    cx.small_blast(e.x, e.y);
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
}
