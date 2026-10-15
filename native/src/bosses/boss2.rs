//! Round 2 flying machine: attacks, hit reactions and defeat sequence.
//!
//! The shield clears weapon collision before reflection, so a passing sword
//! makes no hit sound. Health is eleven damage points (ROM counter starts at ten).
use super::{
    HealthPalette,
    contact::{player_box, strike},
    sine,
};
use crate::{
    combat::{
        Effect, EffectKind, Enemy, EnemyState, Hitbox, Motion, PaletteOp, PlaneOp, Projectile,
        Reaction, ShotDown, Signal, Spawn, advance, pose_hitbox,
    },
    enemies::{Behavior, Ctx, HitResponse, Roster, set_whole},
    game::Terrain,
    movement::PlayerEvent,
    rom::{Result, byte, signed, signed_byte, word, word_i},
    sound_events::Table,
};
use std::sync::Arc;

/// `Boss_Jumps` target of this round.
pub(crate) const HANDLER: usize = 0x2b104;

pub(crate) fn fight(rom: &[u8]) -> Result<Arc<dyn Behavior>> {
    Ok(Arc::new(Boss2::load(rom)?))
}

/// List-5 types this boss spawns and owns (registered by [`register_parts`]).
/// `Boss2`'s helpers are list-1, list-4 (`$33`) and list-9 objects.
pub(crate) const PART_KINDS: &[u16] = &[];

pub(crate) fn register_parts(_rom: &[u8], _roster: &mut Roster) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------

/// `obj_routine` indices of the `$2B25C` table.
const SETUP: u16 = 0; // $2B290
const HURT: u16 = 1; // $2B30C
const DEATH: u16 = 2; // $2B38C
const SPIRAL: u16 = 3; // $2B4BC
const HOVER: u16 = 4; // $2B51C
const CHARGE: u16 = 5; // $2B670
const BEAM: u16 = 6; // $2B704
const FLASH_IN: u16 = 7; // $2B76A
const SWEEP: u16 = 8; // $2B892
const BURST: u16 = 9; // $2B960
const FLASH_OUT: u16 = 10; // $2BAB0
const WARP: u16 = 11; // $2BBA0
const CLEAR: u16 = 12; // $2B4A4

/// `Enemy::timer` = `$34` (immunity), `Enemy::counter` = `$3A` (frames),
/// `Enemy::home_x` = `$42`, `Enemy::launch_y` = the `$44` long (16.16),
/// `Enemy::step_x` = the `$36`/`$38` target (X << 16 | Y); `vars`:
const ANIM: usize = 0; // $28: animation counter (high byte = frame / sub-phase)
const FLAGS: usize = 1; // $2A, plus native bits
const P2C: usize = 2; // $2C
const P2E: usize = 3; // $2E
const P30: usize = 4; // $30
const BOB: usize = 5; // $3C angle | $3E step << 16
const AMP: usize = 6; // $40: bob amplitude

/// `$2B` bit 0: the animation reached its end marker.
const ANIM_END: i32 = 1 << 0;
/// `$2B` bit 1: `Func_02BD3C` leaves the palette alone (the rising flash).
const HOLD_PALETTE: i32 = 1 << 1;
/// `$2B` bit 7: the reflecting field and the flicker poses.
const SHIELD: i32 = 1 << 7;

/// One `Data_02BE92` animation: `$28` advance per frame and frame bytes
/// (mapping = `$256` + byte).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Animation {
    speed: i32,
    frames: Vec<u8>,
}

/// `Boss2` (`$2B104`).
pub(crate) struct Boss2 {
    animations: Vec<Animation>,
    /// `Data_02BE8A`: the field's flicker frames by `$3A & 3`.
    flicker: [u16; 4],
    base: u16,
    health: i32,
    /// `Data_02AFA4`: palette line 1 by the health word `$32` (0..=10),
    /// copied by `Func_02BF46` into CRAM (`$FFF020`) and the fade target
    /// (`$FF171A`); it reddens as the health falls. Entry 0, shown at the
    /// death, is the stage's own line 1.
    health_palette: HealthPalette,
    center: (i32, i32),
    radius: i32,
    spiral_end: i32,
    hover_ticks: i32,
    hover_step: i32,
    hover_speed: i32,
    hover_bounds: (i32, i32),
    hover_band: (i32, i32),
    bob_amplitude: i32,
    field: (i32, i32),
    hurt_immunity: u16,
    hit_immunity: u16,
    charge_y: i32,
    beam_timing: i32,
    beam_shots: i32,
    beam_drift: i32,
    beam: Projectile,
    beam_y: i32,
    sweep_offset: i32,
    sweep_height: i32,
    shot: Projectile,
    shot_speed: (i32, i32),
    shot_offset: (i32, i32),
    shot_floor: i32,
    burst_y: i32,
    burst_ticks: i32,
    rise_y: i32,
    flash_ticks: i32,
    orb: Projectile,
    orb_vx: [i32; 4],
    orb_rest: i32,
    /// `$25FCE`: a resting orb bobs ±16 px on `$2C`, `$10` a tick; its tile
    /// spins through three frames of four patterns, one per `$100` / `$10`
    /// ticks (`$25F74`).
    orb_bob_step: u16,
    orb_spin: (u16, u16),
    burst: Vec<(i32, i32)>,
    warp_y: i32,
    warp_side: i32,
    warp_center: i32,
    attacks: [(u16, i32, i32); 3],
    death_fall: (i32, i32),
    arena: (i32, i32, i32),
    contact: [u8; 2],
    list4: usize,
    sounds: Sounds,
    pieces: Pieces,
    wave: Wave,
    cycle: Cycle,
    /// `move.l #$0EEE0EEE,d0`: the flashes' CRAM colour.
    white: u16,
    /// `cmpi.w #$24,$28(a0)`: the flashes fade while the animation is below.
    fade_end: i32,
}

/// The list-1 type `$2B` (handler `$26178`) and list-9 pieces the handler
/// allocates. Life counts are the `$28` countdowns: a piece is drawn on
/// `life - 1` frames.
struct Pieces {
    /// `Func_02BEE2`: six fragments (routine `$20`, `$26406`), mapping
    /// `$25A + (r & 1)`, velocity from the random word, gravity, life.
    debris: (usize, u16, i32, i32),
    /// Then eight flares (routine `$18`, `$2634E`) after `(r & $F) + 1`
    /// ticks: first mapping, frame count to `$264` and `Data_0263C6`'s
    /// velocity and acceleration by index.
    flares: (usize, u16, u8, [Motion; 8]),
    delay_mask: u16,
    /// `$2637C`: `move.w #7,$28(a0)`, ticks per flare frame.
    flare_period: u16,
    /// `Func_02BCDA`: the warp streak (routine `$14`), life, speed and the
    /// arriving pair's offset (`Func_02BC3A`).
    streak: (u16, i32, i32, i32),
    /// `Func_02BF68`: the list-9 afterimage's tile, the hover's mapping and
    /// life (`$2B58A`) and the orb attack's (`$2BFAA`).
    trail_tile: u16,
    hover_trail: (u16, i32),
    burst_trail: (u16, i32),
    /// `$2B6C0`: the charge's circling spark (routine 8, `$262B6`): tile,
    /// Y offset, life and first mapping (the radius is life + 8).
    orbit: (u16, i32, i32, u16),
    /// `$2B942`: the gun's shell casing (routine `$30`, `$2650E`): mapping,
    /// Y offset, X speed, Y speed and gravity; it lands on Y `$1F0`.
    casing: (u16, i32, i32, i32, i32),
    /// `move.w #$8000,d1` (`$2651A`): the casing's tile.
    casing_tile: u16,
}

/// `Func_02B832`: lines, phase step and amplitude growth and cap.
struct Wave {
    phase_step: i32,
    growth: i32,
    cap: i32,
}

/// `Func_02BD3C`: line 3 colours 5..11 (`Data_02BDAA` rows picked by
/// `Data_02BDF0`) and 12..13 (`Data_02BE00`), every fourth tick.
struct Cycle {
    rows: [[u16; 7]; 5],
    order: [u8; 16],
    pairs: [[u16; 2]; 8],
}
/// Commands of the `$5B56` effects (and the `$5986` fade) the handler requests.
struct Sounds {
    entry: u8,
    hurt: [u8; 2],
    death_music: u8,
    fall: u8,
    boom: u8,
    beam: u8,
    laser: u8,
    flash: u8,
    gun: u8,
    drop: u8,
    charge: u8,
    burst: u8,
}

/// A still list-4 record: `Combat::tick` sweeps a record only while it
/// moves, so every resting shot carries a 1/65536 px drift.
fn still() -> Motion {
    Motion {
        vx: 1,
        ..Motion::default()
    }
}

impl Boss2 {
    fn center_y(e: &Enemy) -> i32 {
        e.launch_y >> 16
    }
    fn set_center_y(e: &mut Enemy, y: i32) {
        e.launch_y = set_whole(e.launch_y, y);
    }
    fn target(e: &Enemy) -> (i32, i32) {
        (e.step_x >> 16, i32::from(e.step_x as i16))
    }
    fn bob_angle(e: &Enemy) -> i32 {
        e.vars[BOB] & 0xffff
    }
    fn set_bob(e: &mut Enemy, angle: i32, step: i32) {
        e.vars[BOB] = (angle & 0xffff) | (step << 16);
    }
    /// `sign(d) | 1` (`ext.l; swap; ori.w #1`).
    fn sign(d: i32) -> i32 {
        if d < 0 { -1 } else { 1 }
    }

    /// `Func_02B624`: hover, drifting away from the player.
    fn hover(&self, cx: &Ctx, e: &mut Enemy) {
        e.routine = HOVER;
        e.vars[ANIM] = 0;
        e.vars[P2E] = -Self::sign(cx.player.0 - e.x) * 4;
        e.vars[P2C] = self.hover_ticks;
        Self::set_bob(e, 0, self.hover_step);
        e.vars[AMP] = self.bob_amplitude;
        e.home_x = e.x;
        e.launch_y = (e.y << 16) | i32::from(e.motion.fy);
        e.timer = 2;
    }
    /// `Func_02B53A`: stop and warp for a random attack (`$2C` 0, 2 or 4).
    fn choose(cx: &mut Ctx, e: &mut Enemy) {
        e.motion.stop();
        e.vars[ANIM] = 0;
        e.routine = WARP;
        let mut pick = i32::from(cx.random() & 6);
        while pick == 6 {
            pick = i32::from(cx.random() & 6);
        }
        e.vars[P2C] = pick;
    }

    /// `Object_AllocL4` for this boss. Its set-up raises the list-4 limit
    /// (`Object_ListParams+2` = `$30`, 49 records) above the framework's
    /// eight, so the four orbs can each burst into eight shots.
    fn fire(&self, cx: &mut Ctx, template: &Projectile, x: i32, y: i32, motion: Motion) {
        if cx
            .projectiles
            .iter()
            .filter(|shot| shot.enemy && shot.alive)
            .count()
            >= self.list4
        {
            return;
        }
        let mut shot = template.clone();
        shot.x = x;
        shot.y = y;
        shot.motion = motion;
        shot.owner = Some(cx.current);
        cx.push_projectile_unchecked(shot);
    }
    /// The orbs and shots after the boss (`$25E06`): shots end on the floor
    /// (`$25E7A`), orbs rest at Y `$1B0` and burst into eight shots while the
    /// boss hovers (`$25FEE`).
    fn manage_shots(&self, cx: &mut Ctx, e: &Enemy) {
        let me = Some(cx.current);
        let mut bursts = Vec::new();
        let mut dust = Vec::new();
        for shot in cx.projectiles.iter_mut() {
            if shot.owner != me || !shot.alive {
                continue;
            }
            if shot.mapping == self.shot.mapping && shot.y >= self.shot_floor {
                shot.alive = false;
                dust.push((shot.x, shot.y));
            } else if shot.mapping == self.orb.mapping {
                let (frames, ticks) = self.orb_spin;
                shot.base = (shot.age / ticks % frames) * 4;
                match shot.rest {
                    // $25FB4: stopped where it crossed the line, no bounce.
                    None if shot.y >= self.orb_rest => {
                        shot.motion = still();
                        shot.rest = Some(self.orb_rest);
                    }
                    None => {}
                    Some(centre) => {
                        let (_, sin) = cx.shared.sine.polar(shot.angle);
                        shot.y = centre + ((sin << 4) >> 16);
                        shot.angle = shot.angle.wrapping_add(self.orb_bob_step);
                        if e.routine == HOVER {
                            shot.alive = false;
                            bursts.push((shot.x, shot.y));
                        }
                    }
                }
            }
        }
        for (x, y) in dust {
            cx.dust(x, y);
        }
        for (x, y) in bursts {
            if let Some(down) = self.orb.shot_down {
                let puff = down.puff((x, y), *cx.random);
                cx.effect(puff);
            }
            for &(vx, vy) in &self.burst {
                self.fire(
                    cx,
                    &self.shot,
                    x,
                    y,
                    Motion {
                        vx,
                        vy,
                        ..Motion::default()
                    },
                );
            }
            cx.play(self.sounds.burst);
        }
    }
    /// `Func_02BE20`: advance the routine's animation and set the pose.
    fn animate(&self, e: &mut Enemy) {
        let Some(animation) = self.animations.get(usize::from(e.routine)) else {
            return;
        };
        e.vars[FLAGS] &= !ANIM_END;
        let index = ((e.vars[ANIM] >> 8) & 0xff) as usize;
        let frame = match animation.frames.get(index) {
            Some(&frame) => frame,
            None => {
                e.vars[ANIM] = 0;
                animation.frames.first().copied().unwrap_or(0)
            }
        };
        e.vars[ANIM] = (e.vars[ANIM] + animation.speed) & 0xffff;
        if animation
            .frames
            .get(((e.vars[ANIM] >> 8) & 0xff) as usize)
            .is_none()
        {
            e.vars[FLAGS] |= ANIM_END;
        }
        e.mapping = if e.vars[FLAGS] & SHIELD != 0 {
            self.flicker[usize::from(e.counter & 3)]
        } else {
            self.base + u16::from(frame)
        };
    }
    /// `Func_02BD12`: Y = `$44` + sine(`$3C`) * `$40`; `$3C` += `$3E`.
    fn bob(cx: &Ctx, e: &mut Enemy) {
        let angle = Self::bob_angle(e);
        e.y = Self::center_y(e) + ((sine(cx, angle) * e.vars[AMP]) >> 16);
        let step = e.vars[BOB] >> 16;
        Self::set_bob(e, angle + step, step);
    }

    /// One routine; true when the bob (`Func_02BD12`) follows.
    fn dispatch(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        match e.routine {
            SETUP => {
                e.health = (self.health + 1) as u16;
                e.hurt_routine = HURT;
                e.death_routine = DEATH;
                e.state = EnemyState::Emerging;
                e.motion = Motion::default();
                // move.w d0,obj_tile(a0): tile base 0 (Boss_Spawn gave $2000).
                e.tile = Some(0);
                e.facing_left = false;
                e.home_x = cx.camera.0 + self.center.0;
                Self::set_center_y(e, cx.camera.1 + self.center.1);
                e.vars[P2E] = self.radius;
                e.vars[P30] = 0;
                cx.play(self.sounds.entry);
                e.routine = SPIRAL;
                e.vars[ANIM] = 0;
                false
            }
            HURT => {
                if e.vars[P2C] == 0 {
                    // $2B314: $FF16FA back into CRAM (ends a flash).
                    self.reload_palette(cx, e);
                    e.vars[P2C] = 1;
                    return false;
                }
                let sound = self.sounds.hurt[usize::from(e.health % 3 != 1)];
                cx.play(sound);
                self.burst_pieces(cx, e);
                Self::explosion(cx, e);
                // $2B356: VDP register $0B = 0 ends a wave; the player keeps
                // the turn of a flash that no second flash undoes.
                cx.plane_op(PlaneOp::FullScroll);
                if e.saved_routine == HOVER {
                    Self::choose(cx, e);
                } else {
                    self.hover(cx, e);
                }
                e.timer = self.hurt_immunity;
                false
            }
            DEATH => {
                self.death(cx, e);
                false
            }
            SPIRAL => {
                e.timer = 1;
                let angle = (i32::from(e.counter & 3) << 8) + e.vars[P30];
                let (cos, sin) = cx.shared.sine.polar(angle as u16);
                let radius = e.vars[P2E];
                e.x = e.home_x + ((((cos >> 2) as i16 as i32) * radius) >> 16);
                e.y = Self::center_y(e) + ((((sin >> 2) as i16 as i32) * radius) >> 16);
                e.vars[P2E] = (radius - 2).max(0);
                e.vars[P30] += 8;
                if e.vars[P30] >= self.spiral_end {
                    self.hover(cx, e);
                }
                false
            }
            HOVER => {
                if cx.projectiles.iter().any(|shot| shot.enemy && shot.alive) {
                    return true;
                }
                e.vars[AMP] = 0;
                if e.vars[P2C] == 0 {
                    Self::choose(cx, e);
                } else {
                    self.drift(cx, e);
                }
                false
            }
            CHARGE => {
                if e.counter & 3 == 0 {
                    self.orbit(cx, e);
                }
                e.vars[FLAGS] |= SHIELD;
                if e.vars[FLAGS] & ANIM_END != 0 {
                    e.vars[P2C] = Self::sign(cx.player.0 - e.x) * 2;
                    e.vars[P2E] = self.beam_timing;
                    e.vars[P30] = self.beam_shots;
                    e.routine = BEAM;
                    e.vars[ANIM] = 0;
                }
                let dy = self.charge_y - e.y;
                if dy != 0 {
                    e.launch_y += Self::sign(dy) << 16;
                }
                true
            }
            BEAM => {
                e.vars[FLAGS] |= SHIELD;
                let (count, drift) = ((e.vars[P2E] >> 8) & 0xff, e.vars[P2E] & 0xff);
                if drift != 0 {
                    e.vars[P2E] = (count << 8) | (drift - 1);
                    e.x += e.vars[P2C];
                    return true;
                }
                let count = count - 1;
                e.vars[P2E] = count << 8;
                if count == 0 {
                    e.vars[P2E] = self.beam_drift;
                    e.vars[P30] -= 1;
                    if e.vars[P30] == 0 {
                        self.hover(cx, e);
                    }
                } else if count == 4 {
                    let y = e.y + self.beam_y;
                    self.fire(cx, &self.beam, e.x, y, still());
                    cx.play(self.sounds.beam);
                    cx.play(self.sounds.laser);
                }
                true
            }
            FLASH_IN | FLASH_OUT => {
                let out = e.routine == FLASH_OUT;
                if out && cx.projectiles.iter().any(|shot| shot.enemy && shot.alive) {
                    e.vars[ANIM] = 0;
                    self.wave(cx, e);
                    return true;
                }
                let anim = e.vars[ANIM];
                if anim < 4 {
                    if out {
                        self.wave(cx, e);
                    }
                    return true;
                }
                if anim == 4 {
                    // $2B774 / $2BAC0: the white flash turns the player.
                    cx.signal(Signal::BlockNinjutsu);
                    self.white_flash(cx);
                    cx.signal(Signal::TurnPlayer);
                    if out {
                        self.wave(cx, e);
                    } else {
                        // $2B78E: every line at line 0's scroll, register
                        // $0B = 3; YAJIMA POWER.
                        e.vars[P2E] = 0;
                        e.vars[P30] = 0;
                        cx.plane_op(PlaneOp::Wave {
                            amplitude: 0,
                            phase: 0,
                        });
                        cx.play(self.sounds.flash);
                    }
                    return true;
                }
                if anim < self.fade_end {
                    cx.signal(Signal::BlockNinjutsu);
                    self.fade_palette(cx, e);
                }
                e.vars[FLAGS] |= SHIELD;
                if out {
                    if e.vars[FLAGS] & ANIM_END != 0 {
                        // $2BB1E: the lines flattened, register $0B = 0.
                        cx.plane_op(PlaneOp::FullScroll);
                        self.hover(cx, e);
                    } else {
                        self.calm(cx, e);
                    }
                    return true;
                }
                if e.vars[FLAGS] & ANIM_END != 0 {
                    // ±32, three times: ±96 from the player.
                    e.vars[P2C] = Self::sign(cx.player.0 - e.x) * self.sweep_offset * 3;
                    e.routine = SWEEP;
                    e.vars[ANIM] = 0;
                }
                self.wave(cx, e);
                true
            }
            SWEEP => {
                self.wave(cx, e);
                if e.vars[FLAGS] & ANIM_END != 0 {
                    e.routine = FLASH_OUT;
                    e.vars[ANIM] = 0;
                    return true;
                }
                let anim = e.vars[ANIM];
                if (anim >> 8) & 1 == 0 {
                    if anim & 0xff == 0 {
                        e.vars[P2C] = -e.vars[P2C];
                    }
                    // Ease 1/16 toward the player's X less $2C.
                    let here = (i64::from(e.x) << 16) + i64::from(e.motion.fx);
                    let goal = i64::from(cx.player.0 - e.vars[P2C]) << 16;
                    advance(&mut e.x, &mut e.motion.fx, ((goal - here) >> 4) as i32);
                    let dy = cx.player.1 - Self::center_y(e) - self.sweep_height;
                    if dy != 0 {
                        e.launch_y += Self::sign(dy) << 16;
                    }
                } else if anim & 0x1f == 0 {
                    cx.play(self.sounds.gun);
                    let right = e.vars[P2C] >= 0;
                    let (vx, vy) = self.shot_speed;
                    let (dx, dy) = self.shot_offset;
                    let x = e.x + if right { dx } else { -dx };
                    let motion = Motion {
                        vx: if right { vx } else { -vx },
                        vy,
                        ..Motion::default()
                    };
                    self.fire(cx, &self.shot, x, e.y + dy, motion);
                    self.casing(cx, e);
                }
                true
            }
            BURST => {
                self.burst(cx, e);
                true
            }
            WARP => {
                self.warp(cx, e);
                false
            }
            CLEAR => {
                cx.player_event(PlayerEvent::Freeze);
                cx.next_substage();
                Ctx::free(e);
                false
            }
            _ => true,
        }
    }

    /// `$2B564`: drift across the upper screen, re-targeting every 33
    /// frames and leaving an afterimage every fourth.
    fn drift(&self, cx: &mut Ctx, e: &mut Enemy) {
        let count = e.vars[P2C] - 1;
        e.vars[P2C] = count;
        if (count as u8 as i8) < 0 {
            e.vars[P2C] = (count & !0xff) | 0x20;
            e.step_x = (e.home_x << 16) | (Self::center_y(e) & 0xffff);
        }
        if e.counter & 3 == 0 {
            self.trail(cx, e, self.pieces.hover_trail);
        }
        e.home_x += e.vars[P2E];
        let angle = Self::bob_angle(e);
        e.launch_y += (sine(cx, angle) << 4) & !0xffff;
        let step = e.vars[BOB] >> 16;
        Self::set_bob(e, angle + 0x30, step);
        let (tx, ty) = Self::target(e);
        e.motion.vx = ((tx - e.x) << 16) >> 4;
        e.motion.vy = ((ty - e.y) << 16) >> 4;
        let screen = e.home_x - (cx.camera.0 - 0x80);
        if screen < self.hover_bounds.0 {
            e.vars[P2E] = self.hover_speed;
        }
        if screen >= self.hover_bounds.1 {
            e.vars[P2E] = -self.hover_speed;
        }
        if e.y < self.hover_band.0 {
            e.launch_y += 1 << 16;
        }
        if e.y >= self.hover_band.1 {
            e.launch_y -= 1 << 16;
        }
    }

    /// `$2B960`: follow, drop, throw the orbs, rise behind a flash.
    fn burst(&self, cx: &mut Ctx, e: &mut Enemy) {
        let low = e.vars[ANIM] & 0xff;
        let mut phase = (e.vars[ANIM] >> 8) & 0xff;
        if phase == 0 {
            e.vars[FLAGS] |= SHIELD;
            e.vars[ANIM] = low;
            e.timer = 2;
            let here = (i64::from(e.x) << 16) + i64::from(e.motion.fx);
            let goal = i64::from(cx.player.0) << 16;
            advance(&mut e.x, &mut e.motion.fx, ((goal - here) >> 4) as i32);
            if low & 0xf == 0 {
                self.trail(cx, e, self.pieces.burst_trail);
            }
            return;
        }
        if phase == 1 {
            // move.w #$100,$28: sub-phases 1..4 hold the whole word.
            e.vars[ANIM] = 0x100;
            let y = (e.y << 16) + i32::from(e.motion.fy);
            e.launch_y += ((self.burst_y << 16) - y) >> 4;
            if Self::center_y(e) < self.burst_y {
                return;
            }
            e.vars[P2C] = self.burst_ticks;
            cx.play(self.sounds.drop);
            phase = 2;
        }
        if phase == 2 {
            e.vars[ANIM] = 0x200;
            if e.vars[P2C] == self.burst_ticks {
                // d7 = 3..0 selects Data_025F20[d7]; `$2C` = d7 ror 7.
                for (d7, &vx) in self.orb_vx.iter().enumerate().rev() {
                    let motion = Motion {
                        vx,
                        ..self.orb.motion
                    };
                    let mut orb = self.orb.clone();
                    orb.angle = (d7 as u16).rotate_right(7);
                    self.fire(cx, &orb, e.x, e.y, motion);
                }
            }
            e.vars[P2C] -= 1;
            if e.vars[P2C] != 0 {
                return;
            }
            phase = 3;
        }
        if phase == 3 {
            e.vars[ANIM] = 0x300;
            e.launch_y -= 2 << 16;
            if Self::center_y(e) >= self.rise_y {
                return;
            }
            // $2BA42: line 0's colours 0..10 black, lines 1..3 white.
            cx.signal(Signal::BlockNinjutsu);
            cx.palette_op(PaletteOp::FillFirst(0, 11, 0));
            for line in 1..4 {
                cx.fill_palette(line, self.white);
            }
            e.vars[P2C] = self.flash_ticks;
        }
        e.vars[ANIM] = 0x400;
        e.vars[P2C] -= 1;
        if e.vars[P2C] == 0 {
            self.hover(cx, e);
            return;
        }
        e.vars[FLAGS] |= HOLD_PALETTE;
        cx.signal(Signal::BlockNinjutsu);
        self.fade_palette(cx, e);
    }

    /// `$2BBA0`: hidden, reappear, and set up the chosen attack (`$2BC66`).
    fn warp(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.state = EnemyState::Emerging;
        e.timer = 2;
        let low = e.vars[ANIM] & 0xff;
        let (_, _, speed, offset) = self.pieces.streak;
        match (e.vars[ANIM] >> 8) & 0xff {
            0 => {
                // Two streaks fly out of the machine.
                self.streak(cx, e.x, e.y, speed);
                self.streak(cx, e.x, e.y, -speed);
                e.vars[ANIM] = 0x100 | low;
            }
            1 => e.vars[ANIM] = 0x100 | low,
            2 => {
                e.y = self.warp_y;
                e.x = match e.vars[P2C] {
                    0 => {
                        let left = cx.random() & 0x8000 != 0;
                        cx.camera.0
                            + if left {
                                self.warp_side & 0xff
                            } else {
                                self.warp_side
                            }
                    }
                    2 => cx.player.0,
                    _ => cx.camera.0 + self.warp_center,
                };
                // Func_02BC3A: two more close in on the new place.
                self.streak(cx, e.x + offset, e.y, -speed);
                self.streak(cx, e.x - offset, e.y, speed);
                e.vars[ANIM] = 0x300 | low;
            }
            3 => e.vars[ANIM] = 0x300 | low,
            _ => {
                e.vars[ANIM] = 0;
                e.home_x = e.x;
                e.launch_y = (e.y << 16) | i32::from(e.motion.fy);
                let (routine, step, amplitude) =
                    self.attacks[(e.vars[P2C] / 2).clamp(0, 2) as usize];
                e.routine = routine;
                Self::set_bob(e, 0, step);
                e.vars[AMP] = amplitude;
                if routine == CHARGE {
                    cx.play(self.sounds.charge);
                }
            }
        }
    }

    /// `$2B38C`: bounce and burn until the animation ends, then routine 12.
    fn death(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.player_event(PlayerEvent::Freeze);
        cx.plane_op(PlaneOp::FullScroll);
        e.timer = 2;
        if e.vars[FLAGS] & ANIM_END != 0 {
            e.routine = CLEAR;
            return;
        }
        if e.vars[ANIM] & 0xff == 4 {
            cx.play(self.sounds.fall);
            e.motion.vx = Self::sign(e.x - e.vars[P2C]) << 16;
            e.motion.gravity = self.death_fall.0;
            e.motion.vy = self.death_fall.1 << 16;
        }
        let (left, right, floor) = self.arena;
        if e.x < left || e.x >= right {
            e.x = if e.x < left { left } else { right };
            e.motion.vx = -e.motion.vx;
        }
        if e.y >= floor {
            e.y = floor;
            let mut vy = e.motion.vy;
            if (vy + 0x8000) >> 16 == 0 {
                vy = 0;
                e.motion.ax = 0;
                e.motion.gravity = 0;
            }
            e.motion.vy = -(vy >> 1);
            e.motion.vx >>= 1;
        }
        if e.motion.vy != 0 {
            if e.counter & 3 == 0 {
                Self::explosion(cx, e);
            }
            if e.counter & 0xf == 0 {
                cx.play(self.sounds.boom);
                self.burst_pieces(cx, e);
            }
        }
    }
}

impl Behavior for Boss2 {
    fn label(&self, _kind: u16) -> String {
        format!("Boss2 ${HANDLER:05X}")
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.base
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.counter = e.counter.wrapping_add(1);
        if e.state == EnemyState::Emerging {
            e.state = EnemyState::Waiting; // andi.w #$B7DF: shown again
        }
        match cx.process_hit(e) {
            Some(Reaction::Hurt) => {
                // $2B1E8: Func_02BF46 with `$32` before the damage is
                // subtracted, so each hit shows the previous hit's health.
                let before = e.health_before_hit() - 1;
                self.health_palette.show(cx, usize::from(before));
                // Hurt for two frames, 4 frames of immunity.
                e.vars[ANIM] = 0;
                e.vars[P2C] = 0;
                e.timer = self.hit_immunity;
            }
            Some(Reaction::Death) => {
                // $2B218: `$32` cleared, Func_02BF46 (the wreck's line),
                // then fall away from the blow.
                self.health_palette.show(cx, 0);
                e.vars[ANIM] = 0;
                e.vars[P2C] = cx.player.0;
                e.motion.stop();
                cx.play(self.sounds.death_music);
            }
            _ => {}
        }
        // $2B1AA: contact, 2 inside the field's states, else 1 (queued after
        // the routine so the hover's list-4 test does not see it). Its
        // `Object_CollidePlayer` (`$2B1B0`) is `Object_CollideList`: an
        // empty box is a point.
        let shield = e.vars[FLAGS] & SHIELD != 0;
        let touching = !matches!(e.routine, DEATH | CLEAR)
            && pose_hitbox(cx.mappings, e).collides(player_box(cx));
        e.timer = e.timer.saturating_sub(1);
        e.vars[FLAGS] &= !(SHIELD | HOLD_PALETTE);
        let bob = self.dispatch(cx, e);
        if e.state == EnemyState::Defeated && !e.dying {
            return; // freed at the request
        }
        if bob {
            Self::bob(cx, e);
        }
        if e.vars[FLAGS] & HOLD_PALETTE == 0 {
            self.cycle_palette(cx, e);
        }
        self.animate(e);
        // The charge's sparks read their parent's place on every update.
        let (_, dy, ..) = self.pieces.orbit;
        for spark in cx.effects.iter_mut() {
            if spark.kind == EffectKind::Orbit {
                spark.origin = (e.x, e.y + dy);
            }
        }
        // $2B126: the field is the pose's hitbox centre with $60 x $30.
        let bounds = cx
            .mappings
            .get(&e.mapping)
            .map_or(Hitbox::NONE, |mapping| mapping.bounds);
        e.body = if e.vars[FLAGS] & SHIELD != 0 {
            Some(Hitbox {
                half_width: self.field.0,
                half_height: self.field.1,
                ..bounds
            })
        } else {
            None
        };
        self.manage_shots(cx, e);
        if touching {
            strike(cx, self.contact[usize::from(shield)], None);
        }
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        if matches!(e.routine, SETUP | DEATH | CLEAR) {
            HitResponse::Pass
        } else if e.vars[FLAGS] & SHIELD != 0 {
            HitResponse::Reflect
        } else if e.timer > 0 {
            HitResponse::Pass
        } else {
            HitResponse::Accept
        }
    }
}

mod load;
mod presentation;
