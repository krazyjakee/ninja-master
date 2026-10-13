//! Type `$10`, `Enemy10` (`$1383C`): the 5-1 mid-round boss.
use super::{ENTRANCE_SIGNAL, SIGNAL};
use crate::{
    combat::{Blast, Effect, EffectKind, Enemy, Motion, PaletteOp, Projectile, Spawn},
    enemies::{Behavior, Ctx, HitResponse, set_whole, shot_damage, stop},
    game::Terrain,
    rom::{Result, byte, sequence, signed, word, word_i, words},
    sound_events::Table,
};

/// `$2A` countdown, `$2C` hit flash (-1 idle), `$2E` health at the last
/// pattern change, `$30` shots left, `$4C` passes left, native fade steps,
/// hittable window (`Func_013C38`), and the entrance signal.
const TIMER: usize = 0;
const FLASH: usize = 1;
const MARK: usize = 2;
const SHOTS: usize = 3;
const PASSES: usize = 4;
const FADE: usize = 5;
const WINDOW: usize = 6;

/// `$10`: a commando that enters at the right edge once the camera locks,
/// then alternates two patterns. Dash: stands 61 ticks (`$183`) and
/// accelerates ($3000) off the screen with an afterimage, reappears at the
/// edge it left, turned round, three passes. Shots: slides in at 6 px/tick
/// braking by $E00, turns, charges 33 ticks (`$187`) and fires three
/// accelerating energy shots toward the player (list-4 `$25`, 7 px/tick
/// plus $7FFF, damage 2), 22 ticks apart; three volleys. Taking 4 damage
/// since the last change cuts a pattern short. 31 health; a hit flashes
/// it for 7 ticks, during which it cannot be hit; it can only be hit
/// within 192 px of the screen centre. Death: nine bursts 8 ticks apart,
/// the large blast, then the stage-clear music and the next substage.
pub(crate) struct Commando {
    entry: (i32, i32),
    entry_pose: u16,
    stand: u16,
    fire: u16,
    rest: u16,
    stand_ticks: i32,
    slide_wait: i32,
    charge_ticks: i32,
    shots: i32,
    glow_ticks: i32,
    rest_ticks: i32,
    passes: i32,
    big_damage: i32,
    /// `Func_013C64`: (vx, ax) of the braking slide and the accelerating dash.
    slide: (i32, i32),
    dash: (i32, i32),
    wrap: (i32, i32),
    window: (u16, i32, i32),
    hurt_routine: u16,
    death_routine: u16,
    hurt_flash: i32,
    /// `$13762` selects a six-colour row from `$13782` by health counter.
    health_rows: [u8; 32],
    health_palettes: [[u16; 6]; 7],
    flash_color: u16,
    afterimage_delay: u16,
    dash_sound: u8,
    shot: EnergyShot,
    death_period: u16,
    death_bursts: i32,
    burst_sound: u8,
    blast: Blast,
    vanish_sound: u8,
    vanish_ticks: i32,
    clear_ticks: i32,
    clear_music: u8,
}
/// List-4 `$25` (`$1DC4A`): the charged shot.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EnergyShot {
    /// `Data_0137D8`: offsets for the player on the right, then left.
    offsets: [(i32, i32); 2],
    ax: i32,
    vx: i32,
    frames: Vec<u16>,
    damage: u8,
    sound: u8,
}
impl Commando {
    /// Native palette fade on entry: `$6E8` steps every fourth tick.
    pub(crate) const FADE_STEPS: i32 = 8;
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut health_rows = [0; 32];
        for (i, row) in health_rows.iter_mut().enumerate() {
            *row = byte(rom, 0x13762 + i)?;
        }
        let mut health_palettes = [[0; 6]; 7];
        for (i, palette) in health_palettes.iter_mut().enumerate() {
            *palette = words(rom, 0x13782 + i * 12)?;
        }
        Ok(Self {
            health_rows,
            health_palettes,
            flash_color: word(rom, 0x13850)?,
            entry: (word_i(rom, 0x13be8)?, word_i(rom, 0x13bf0)?),
            entry_pose: word(rom, 0x137d6)?,
            stand: word(rom, 0x13912)?,
            fire: word(rom, 0x139ea)?,
            rest: word(rom, 0x13bce)?,
            stand_ticks: word_i(rom, 0x13918)?,
            slide_wait: word_i(rom, 0x13982)?,
            charge_ticks: word_i(rom, 0x139d6)?,
            shots: word_i(rom, 0x139dc)?,
            glow_ticks: word_i(rom, 0x13a22)?,
            rest_ticks: word_i(rom, 0x13a3a)?,
            passes: word_i(rom, 0x139d0)?,
            big_damage: word_i(rom, 0x13a76)?,
            slide: (signed(rom, 0x13c6e)?, signed(rom, 0x13c72)?),
            dash: (signed(rom, 0x13c78)?, signed(rom, 0x13c7c)?),
            wrap: (signed(rom, 0x13a8c)?, signed(rom, 0x13a98)?),
            window: (
                word(rom, 0x13c44)?,
                word_i(rom, 0x13c4e)?,
                word_i(rom, 0x13c5a)?,
            ),
            hurt_routine: word(rom, 0x13882)?,
            death_routine: word(rom, 0x13888)?,
            hurt_flash: word_i(rom, 0x13b5c)?,
            afterimage_delay: word(rom, 0x13e84)?,
            dash_sound: byte(rom, 0x1394b)?,
            shot: EnergyShot {
                offsets: [
                    (signed(rom, 0x137d8)?, signed(rom, 0x137da)?),
                    (signed(rom, 0x137dc)?, signed(rom, 0x137de)?),
                ],
                ax: signed(rom, 0x137e0)?,
                vx: signed(rom, 0x137e2)?,
                frames: sequence(rom, 0x1dc40)?,
                damage: shot_damage(rom, 0x25)?,
                sound: byte(rom, 0x13a15)?,
            },
            death_period: word(rom, 0x13ad0)?,
            death_bursts: word_i(rom, 0x13aba)?,
            burst_sound: byte(rom, 0x13ad5)?,
            // $13AE8: moveq #5,d7 / moveq #1,d4 into $36DB6 (d6 as left by
            // the bursts; native uses the finale blast's sound).
            blast: Blast {
                y_shift: u32::from(byte(rom, 0x13af3)?),
                x_shift: u32::from(byte(rom, 0x13ae9)?),
                sound: Table::Effect.command_at(rom, 0x36d97)?,
            },
            vanish_sound: byte(rom, 0x13b19)?,
            vanish_ticks: word_i(rom, 0x13b10)?,
            clear_ticks: word_i(rom, 0x13b2e)?,
            clear_music: Table::Music.command_at(rom, 0x13b37)?,
        })
    }
    /// `Func_013C64`: the dash from a standstill (`obj_flags` bit 0 set)
    /// or the braking slide, toward the facing (unflipped: left).
    fn go(&self, e: &mut Enemy, dash: bool) {
        let (mut vx, mut ax) = if dash { self.dash } else { self.slide };
        if !e.facing_left {
            vx = -vx;
            ax = -ax;
        }
        e.motion.vx = set_whole(e.motion.vx, vx);
        e.motion.ax = ax;
    }
    /// `Func_013A6A`: 4 or more damage since the last check.
    fn hurt_enough(&self, e: &mut Enemy) -> bool {
        let health = i32::from(e.health);
        if e.vars[MARK] - health >= self.big_damage {
            e.vars[MARK] = health;
            true
        } else {
            false
        }
    }
    /// `Func_013E54`: a list-9 afterimage replaying this motion 5 ticks
    /// later, blinking, until it stops or leaves the screen.
    fn afterimage(&self, cx: &mut Ctx, e: &Enemy) {
        let base = cx.base(0x10);
        let mut ghost = Effect::new(EffectKind::Pieces, e.x, e.y, e.mapping);
        ghost.base = base;
        ghost.flip = e.facing_left;
        ghost.blink = true;
        ghost.delay = self.afterimage_delay + 1;
        ghost.motion = Motion {
            vx: e.motion.vx,
            ax: e.motion.ax,
            ..Motion::default()
        };
        // $13E1C: a braking dash's image is freed when its motion stops.
        ghost.counter = if (e.motion.vx < 0) != (e.motion.ax < 0) && e.motion.ax != 0 {
            (e.motion.vx / e.motion.ax).abs()
        } else {
            240
        };
        cx.effect(ghost);
    }
    /// `Func_01DC24`: list-4 `$25` from the muzzle toward the player.
    fn fire(&self, cx: &mut Ctx, e: &Enemy) {
        let left = cx.player_left(e.x);
        let (dx, dy) = self.shot.offsets[usize::from(left)];
        let sign = if left { -1 } else { 1 };
        let mapping = self.shot.frames.first().copied().unwrap_or(0);
        let mut shot = Projectile::new(e.x + dx, e.y + dy, 0, true, mapping, self.shot.damage)
            .with_motion(Motion {
                vx: sign * (self.shot.vx << 16),
                ax: sign * self.shot.ax,
                ..Motion::default()
            })
            // `$1DC4A` never loads a box (the record's last one stays):
            // native keeps the frame drawn.
            .box_after();
        shot.owner = Some(cx.current);
        cx.shoot(shot);
        cx.effect_sound(self.shot.sound);
    }
    /// `$1DCE6`: the shot's frames advance every second tick, then hold.
    fn animate_shots(&self, cx: &mut Ctx) {
        let me = cx.current;
        let frames = &self.shot.frames;
        for shot in cx.projectiles.iter_mut() {
            if shot.owner == Some(me) && frames.contains(&shot.mapping) {
                let step = usize::from(shot.age / 2).min(frames.len().saturating_sub(1));
                shot.mapping = frames[step];
            }
        }
    }
}
impl Behavior for Commando {
    fn label(&self, _kind: u16) -> String {
        "Enemy10 $1383C".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        if (0x3c..=0x44).contains(&e.routine) {
            cx.player_event(crate::movement::PlayerEvent::Freeze);
        }
        self.animate_shots(cx);
        self.step(cx, e);
        self.update_palette(cx, e);
        // Func_013C38: hits only in the fight, near the screen centre.
        let (min_routine, centre, reach) = self.window;
        e.vars[WINDOW] =
            i32::from(e.routine >= min_routine && ((cx.camera.0 + centre) - e.x).abs() < reach);
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        let fighting = enemy.routine >= self.window.0 && enemy.routine < self.hurt_routine;
        if fighting && enemy.vars[FLASH] < 0 && enemy.vars[WINDOW] != 0 {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
}
impl Commando {
    /// `$13B68`, after the routine: six white updates, then restore colours
    /// 2..7 from the remaining-health row. Colour 8 stays white. The firing
    /// glow ($2C) pauses both the palette helper and its countdown.
    fn update_palette(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.routine == 0x2c || e.vars[FLASH] < 0 || e.dying() {
            return;
        }
        if e.vars[FLASH] > 0 {
            for index in 2..=8 {
                cx.palette_op(PaletteOp::Color(1, index, self.flash_color));
            }
        } else {
            let counter = usize::from(e.health.saturating_sub(1)).min(31);
            let row = usize::from(self.health_rows[counter]);
            for (i, &color) in self.health_palettes[row].iter().enumerate() {
                cx.palette_op(PaletteOp::Color(1, 2 + i as u8, color));
            }
        }
        e.vars[FLASH] -= 1;
    }
    /// One dispatch through the `$137F4` table.
    fn step(&self, cx: &mut Ctx, e: &mut Enemy) {
        loop {
            match e.routine {
                0 => {
                    // $13864: invisible at 0,0 until the camera locks.
                    stop(e);
                    e.x = 0;
                    e.y = 0;
                    e.mapping = 0;
                    e.hurt_routine = self.hurt_routine;
                    e.death_routine = self.death_routine;
                    e.vars[FLASH] = -1;
                    e.vars[MARK] = i32::from(e.health);
                    e.routine = 4;
                }
                4 => {
                    // $138A2 waits for Camera_Mode >= 8, not the layout edge.
                    if !cx.camera_locked {
                        return;
                    }
                    e.x = cx.camera.0 + self.entry.0;
                    e.y = self.entry.1;
                    e.mapping = self.entry_pose;
                    e.vars[SIGNAL] = ENTRANCE_SIGNAL;
                    e.vars[FADE] = 0;
                    e.routine = 8;
                    return;
                }
                8 => {
                    // $138CE: the palette fade, one step every fourth tick.
                    if e.timer & 3 != 0 {
                        return;
                    }
                    e.vars[FADE] += 1;
                    if e.vars[FADE] < Self::FADE_STEPS {
                        return;
                    }
                    e.vars[SIGNAL] = 0;
                    e.vars[PASSES] = self.passes;
                    e.routine = 0x10;
                    return;
                }
                0x0c => {
                    // $138FE
                    e.vars[PASSES] = self.passes;
                    e.routine = 0x10;
                    return;
                }
                0x10 => {
                    // $1390A: stand.
                    e.mapping = self.stand;
                    e.vars[TIMER] = self.stand_ticks;
                    e.routine = 0x14;
                }
                0x14 => {
                    // $13920: then dash from a standstill.
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    self.go(e, true);
                    e.routine = 0x18;
                    e.vars[TIMER] = 3;
                    self.afterimage(cx, e);
                    cx.effect_sound(self.dash_sound);
                    return;
                }
                0x18 => {
                    // $13952: off screen, reappear at the edge it left,
                    // turned round.
                    if !e.offscreen {
                        return;
                    }
                    e.x = cx.camera.0
                        + if e.facing_left {
                            self.wrap.1
                        } else {
                            self.wrap.0
                        };
                    e.facing_left = !e.facing_left;
                    stop(e);
                    e.vars[PASSES] -= 1;
                    if e.vars[PASSES] >= 0 && !self.hurt_enough(e) {
                        e.routine = 0x10;
                        continue;
                    }
                    e.vars[TIMER] = self.slide_wait;
                    e.routine = 0x1c;
                }
                0x1c => {
                    // $1398A: the braking slide.
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    self.go(e, false);
                    e.vars[TIMER] = 3;
                    self.afterimage(cx, e);
                    e.routine = 0x20;
                    return;
                }
                0x20 => {
                    // $139AE: stopped (velocity turned to the braking sign).
                    if (e.motion.vx < 0) != (e.motion.ax < 0) {
                        return;
                    }
                    e.facing_left = !e.facing_left;
                    stop(e);
                    e.vars[PASSES] = self.passes;
                    e.vars[TIMER] = self.charge_ticks;
                    e.vars[SHOTS] = self.shots;
                    e.routine = 0x24;
                    return;
                }
                0x24 => {
                    // $139E8: charge.
                    e.mapping = self.fire;
                    if e.timer & 3 == 0 {
                        // Data_0137D8 -> Func_01DBE6 -> Func_031A7A. The
                        // offset selector is the inverse of obj_tile bit 3.
                        let offset = if e.facing_left {
                            self.shot.offsets[0]
                        } else {
                            self.shot.offsets[1]
                        };
                        cx.charge_sparkle((e.x + offset.0, e.y + offset.1), 0);
                    }
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    e.vars[TIMER] = self.charge_ticks;
                    e.routine = 0x28;
                }
                0x28 => {
                    // $13A08: fire, with the muzzle glow.
                    self.fire(cx, e);
                    e.routine = 0x2c;
                    e.vars[TIMER] = self.glow_ticks;
                    e.mapping = self.fire;
                    return;
                }
                0x2c => {
                    // $13A2C
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    e.routine = 0x30;
                    e.vars[TIMER] = self.rest_ticks;
                    e.mapping = self.rest;
                    return;
                }
                0x30 => {
                    // $13A44: next shot, next volley, or the dashes.
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    e.vars[SHOTS] -= 1;
                    if e.vars[SHOTS] >= 0 {
                        e.vars[TIMER] = self.charge_ticks;
                        e.routine = 0x28;
                        continue;
                    }
                    e.vars[PASSES] -= 1;
                    if e.vars[PASSES] >= 0 && !self.hurt_enough(e) {
                        e.vars[TIMER] = self.charge_ticks;
                        e.vars[SHOTS] = self.shots;
                        e.routine = 0x24;
                        return;
                    }
                    e.vars[PASSES] = self.passes;
                    e.routine = 0x10;
                }
                r if r == self.hurt_routine => {
                    // $13B5A: flash, resume.
                    e.vars[FLASH] = self.hurt_flash;
                    // `$13B68` flashes only CRAM colours 2..8.
                    e.flash = 0;
                    e.routine = e.saved_routine;
                    return;
                }
                r if r == self.death_routine => {
                    // $13AA2: slowed to a quarter (score central).
                    e.motion.ax = 0;
                    e.motion.vx >>= 2;
                    e.vars[TIMER] = self.death_bursts;
                    e.vars[SIGNAL] = 0;
                    e.routine = 0x3c;
                    return;
                }
                0x3c => {
                    // $13AC4: a burst every eighth tick, nine in all.
                    if e.timer & self.death_period != 0 {
                        return;
                    }
                    cx.effect_sound(self.burst_sound);
                    cx.fire_puffs(e.x, e.y);
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    let blast = self.blast;
                    cx.explode(e.x, e.y, blast);
                    e.motion.stop();
                    e.mapping = 0;
                    e.vars[TIMER] = self.vanish_ticks;
                    e.routine = 0x40;
                    cx.effect_sound(self.vanish_sound);
                    return;
                }
                0x40 => {
                    // $13B20: the stage-clear music after 61 ticks.
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    e.vars[TIMER] = self.clear_ticks;
                    e.routine = 0x44;
                    cx.play(self.clear_music);
                    return;
                }
                _ => {
                    // $13B3E: Game_Requests bit 1 after 181 more ticks.
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    cx.next_substage();
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
}
