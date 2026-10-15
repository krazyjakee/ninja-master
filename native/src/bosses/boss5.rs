//! Round 5 mech: script controller, body movement, core and defeat sequence.
//!
//! Parent commands retain serials so repeated head and hand commands are distinct.
//! The core stays at the mech position; its hands remain standing platforms.

use super::{aim_angle, fade_lines, half_turn, slopes, unit_vector};
use crate::{
    combat::{
        Effect, EffectKind, Enemy, MechShot, Motion, PaletteOp, PlaneOp, Projectile, Reaction,
        Signal, Spawn,
    },
    enemies::{Behavior, Ctx, HitResponse, Roster},
    game::Terrain,
    movement::PlayerEvent,
    rom::{Result, byte, signed, word, words},
    sound_events::Table,
};
use std::sync::Arc;

/// `Boss_Jumps` target of this round.
pub(crate) const HANDLER: usize = 0x30654;
/// List-5 type of the parts (`Enemy7C` `$30FAC`).
pub(crate) const PART: u16 = 0x7c;

pub(crate) fn fight(rom: &[u8]) -> Result<Arc<dyn Behavior>> {
    Ok(Arc::new(Boss5::load(rom)?))
}

/// List-5 types this boss spawns and owns (registered by [`register_parts`]).
pub(crate) const PART_KINDS: &[u16] = &[PART];

pub(crate) fn register_parts(rom: &[u8], roster: &mut Roster) -> Result<()> {
    roster.add(PART, Arc::new(MechPart::load(rom)?));
    Ok(())
}

// ---------------------------------------------------------------------------
// Boss5 (the list-8 record, natively also part C).

/// Record scratch: mech `$40`/`$44`, hop state, script id and step,
/// routine timer, flash, and the two part commands.
const BODY_X: usize = 0;
const BODY_Y: usize = 1;
const HOP: usize = 2;
const REST: usize = 3;
const SCRIPT: usize = 4;
const T: usize = 5;
/// `serial << 4 | command` for part A (1 roar, 2 flame, 3 destroyed).
pub(crate) const HEAD_CMD: usize = 6;
/// `serial << 4 | 1` for part B (the orbs).
pub(crate) const HANDS_CMD: usize = 7;
/// Native: `Func_030D78`'s reddenings so far (`$FF3A26` = `$1C` less `$C`
/// each, at least 0).
const REDDENED: usize = 8;
/// `$3A`: the flash fading back, deflecting (natively a countdown).
const FADE: usize = 9;
const SWEEP_FLIP: usize = 10;
const CYCLE: usize = 11;
const JOLT: usize = 12;
const HEAD_ROAR: i32 = 1;
const HEAD_FLAME: i32 = 2;
const HEAD_DOWN: i32 = 3;

/// Native routine: the lethal hit's death routine.
const DEATH: u16 = 0x74;
/// Native hurt routine (resumes the saved routine).
const HURT: u16 = 0x90;

// Durations walked from the ROM.
/// `$10` eyes / `$18` body fades (observed 60 each at 4-tick steps).
const FADE_TICKS: i32 = 60;
/// `$20`: part A's roar (`$14`, 60 ticks).
const ROAR_TICKS: i32 = 60;
/// Part A's sweep: $100 -> $40 -> $1C0 -> $100 in steps of 4.
const SWEEP_STEP: i32 = 4;
/// `$4C`: part B wait 98 then 128 ticks after the orbs.
const HANDS_TICKS: i32 = 98 + 128;
/// `$5C`: the head destroyed, 60 ticks.
const HEAD_DOWN_TICKS: i32 = 60;
/// The flash fade that makes the core deflect after a hit (`$30DF4`).
const FLASH_TICKS: i32 = 64;
/// `$30D8A`: the hit flash over line 1 colours 0..11.
const FLASH_COLOR: u16 = 0x0fff;
/// `$30DE0` `addq.w #2`: a reddening's red step.
const RED_STEP: u16 = 2;
/// `$30ADA` hop: 25 frames, rest 60 (`$30`).
const REST_TICKS: i32 = 60;
/// Hop height: `$2C` from $20000 losing $2000 per frame.
const HOP_SPEED: i32 = 0x20000;
const HOP_DRAG: i32 = 0x2000;
/// Mech ground `$44` = $D0 - height; world Y = `$44 + $110`.
const GROUND: i32 = 0xd0;
const WORLD_Y: i32 = 0x110;
/// Core / head above the mech origin, hands controller, hands.
const CORE_UP: i32 = 0x60;
const HANDS_UP: i32 = 0x28;
const HAND_UP: i32 = 0x30;
/// Back away: until 128 px, within `$40` in [$40, $FE].
const KEEP_AWAY: i32 = 0x80;
const RANGE: (i32, i32) = (0x40, 0xfe);
/// Health counter threshold of the head (below 10).
const HEAD_LIMIT: u16 = 10;

/// `Boss5`: the script machine, the hop movement, the core, and the death.
pub(crate) struct Boss5 {
    /// `$3073E` script pointers resolved into routine lists (`$FF` ends).
    scripts: Vec<Vec<u8>>,
    wait: [u16; 4],
    attacks: [u8; 4],
    core: u16,
    small_core: u16,
    counter: u16,
    start: (i32, i32),
    rock_frames: [u16; 2],
    rock_damage: u8,
    /// Part C's record `obj_tile` (`Data_030F66`), worn by the native core.
    core_tile: u16,
    /// The rock record's `obj_tile` (`Data_0318AC`).
    rock_tile: u16,
    roar: u8,
    stomp: u8,
    claw: u8,
    boom: u8,
    burst_offsets: [(i32, i32); 3],
    death_palette: [[u16; 16]; 3],
    /// `$FF3A06`: line 1 as the intro fades it in (`Data_042062 + $760`),
    /// the target the hit flash fades back to.
    body_palette: [u16; 16],
    arena_palette: [u16; 16],
    jolt: [u8; 20],
    /// `$30852`: the first reddening threshold (`$FF3A26`), and `$30DBC`
    /// its step down.
    red_threshold: u16,
    red_threshold_step: u16,
    /// `$30DD2` `moveq #$B`: colours flashed, reddened and faded back.
    red_colors: usize,
}

/// `$3109C..$31116`: the reversal falls through and moves back four in
/// the same update; return-to-centre stops at $FC or $100 before moving.
fn sweep_pose(tick: i32, flipped: bool) -> Option<(i32, bool)> {
    if !(0..=190).contains(&tick) {
        return None;
    }
    let direction = if flipped { 1 } else { -1 };
    let first = if flipped { 0x1c0 } else { 0x40 };
    let second = if flipped { 0x40 } else { 0x1c0 };
    let (mut angle, mut leg) = (0x100, 0);
    for _ in 0..=tick {
        if leg == 0 {
            angle += direction * SWEEP_STEP;
            if angle == first {
                leg = 1;
                angle -= direction * SWEEP_STEP;
            }
        } else if leg == 1 {
            angle -= direction * SWEEP_STEP;
            if angle == second {
                leg = 2;
                angle += direction * SWEEP_STEP;
            }
        } else {
            if matches!(angle, 0xfc | 0x100) {
                return None;
            }
            angle += if angle > 0x100 {
                -SWEEP_STEP
            } else {
                SWEEP_STEP
            };
        }
    }
    Some((angle, leg < 2))
}

/// `$31962`: smoke's ROM mapping/attribute sequence, with the allocating
/// routine's velocity and initial animation counter.
fn smoke(cx: &mut Ctx, at: (i32, i32), velocity: (i32, i32), gravity: i32, counter: i32) {
    let mut effect = Effect::new(EffectKind::MechParticle, at.0, at.1, 0);
    effect.base = 0x8000;
    effect.high_priority = true;
    effect.counter = counter;
    effect.motion = Motion {
        vx: velocity.0,
        vy: velocity.1,
        gravity,
        ..Motion::default()
    };
    cx.effect(effect);
}

/// `$31910`: scatter in the upper half-circle at twice the ROM sine speed.
fn particles(cx: &mut Ctx, at: (i32, i32), count: usize) {
    for _ in 0..count {
        let angle = (cx.random() & 0x1ff) + 0x200;
        let (vx, vy) = cx.shared.sine.polar(angle);
        smoke(cx, at, (vx * 2, vy * 2), 0x200, 0);
    }
}

impl Boss5 {
    fn load(rom: &[u8]) -> Result<Self> {
        let mut scripts = Vec::new();
        for i in 0..7 {
            let start = crate::rom::pointer(rom, 0x3073e + i * 4)?;
            let mut steps = Vec::new();
            for p in start..start + 16 {
                let routine = byte(rom, p)?;
                if routine == 0xff {
                    break;
                }
                steps.push(routine);
            }
            scripts.push(steps);
        }
        let offset =
            |p: usize| -> Result<(i32, i32)> { Ok((signed(rom, p)?, signed(rom, p + 2)?)) };
        // `$30F00..$30F12` loads one bank, then writes it three times.
        let death_palette = [words(rom, 0x30f36)?; 3];
        Ok(Self {
            scripts,
            // `$30C88..$30C96`: Game_Options bits 2-3 select the delay.
            wait: words(rom, 0x30cb0)?,
            attacks: [
                byte(rom, 0x30e58)?,
                byte(rom, 0x30e59)?,
                byte(rom, 0x30e5a)?,
                byte(rom, 0x30e5b)?,
            ],
            core: 0x2c3,
            small_core: 0x2c4,
            // Part C's record ($30F66): health byte 40.
            counter: u16::from(byte(rom, 0x30f66 + 7)?),
            start: (0xc8, 0xd8),
            rock_frames: [word(rom, 0x3059e)?, word(rom, 0x305a0)?],
            rock_damage: byte(rom, 0x1fbf6 + 0x28)?,
            core_tile: word(rom, 0x30f66 + 4)?,
            rock_tile: word(rom, 0x318ac + 4)?,
            roar: Table::Effect.command(rom, 0x2b)?,
            stomp: Table::Voice.command(rom, 0x03)?,
            claw: Table::Voice.command(rom, 0x0f)?,
            boom: Table::Effect.command(rom, 0x33)?,
            burst_offsets: [offset(0x31a38)?, offset(0x31a3c)?, offset(0x31a40)?],
            death_palette,
            body_palette: words(rom, 0x427c2)?,
            arena_palette: words(rom, 0x428c2)?,
            jolt: rom[0x32f2..0x3306].try_into().unwrap(),
            red_threshold: word(rom, 0x30854)?,
            red_threshold_step: word(rom, 0x30dbe)?,
            red_colors: usize::from(byte(rom, 0x30dd3)?) + 1,
        })
    }
    /// `$FF3A26` after `reddened` reddenings.
    fn threshold(&self, reddened: i32) -> u16 {
        let step = u32::from(self.red_threshold_step) * reddened as u32;
        u32::from(self.red_threshold).saturating_sub(step) as u16
    }
    /// `$FF3A06` after `reddened` reddenings: red + 2 each on the flashed
    /// colours, at most `$F`.
    pub(crate) fn body_target(&self, reddened: i32) -> [u16; 16] {
        let mut line = self.body_palette;
        for color in &mut line[..self.red_colors] {
            let red = (*color & 0xf) + RED_STEP * reddened as u16;
            *color = (*color & 0xff0) | red.min(0xf);
        }
        line
    }
    /// `Func_030D78` on a hit that neither destroys the head nor kills:
    /// line 1 colours 0..11 white and fading back (deflecting), and once
    /// per threshold the target one step redder (#92).
    fn flash(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.palette_op(PaletteOp::FillFirst(1, self.red_colors as u8, FLASH_COLOR));
        e.vars[FADE] = FLASH_TICKS;
        // byte 7: the core counter before this hit's damage.
        let before = e.health_before_hit().saturating_sub(1);
        if before < self.threshold(e.vars[REDDENED]) {
            e.vars[REDDENED] += 1;
        }
    }
    /// `Func_030DF4`: every fourth update of the flash, one `$6E8` step of
    /// line 1 toward `$FF3A06` (16 steps reach it from white). The fade
    /// covers colours 0..11, preserving the live eye and colour cycle.
    fn fade_back(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[FADE] > 0 {
            e.vars[FADE] -= 1;
            if e.vars[FADE] & 3 == 0 {
                cx.palette_op(PaletteOp::FadeFirst(
                    1,
                    12,
                    self.body_target(e.vars[REDDENED]),
                ));
            }
        }
    }
    /// The mech origin (`$40 + 4`, `$44 + $110`) and the core above it.
    fn place(e: &mut Enemy) {
        let x = e.vars[BODY_X] + if e.facing_left { -4 } else { 4 };
        e.x = x;
        e.y = e.vars[BODY_Y] + WORLD_Y - CORE_UP;
    }
    fn draw_body(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.plane_op(PlaneOp::MechBody {
            x: e.vars[BODY_X],
            y: e.vars[BODY_Y],
            flip: e.facing_left,
        });
        if e.vars[SCRIPT] >> 8 != 0 {
            e.vars[CYCLE] += 1;
            if e.vars[CYCLE] & 7 == 0 {
                // ROM rotates left, while PaletteOp::Rotate rotates right.
                cx.palette_op(PaletteOp::Rotate(1, 13, 3));
                cx.palette_op(PaletteOp::Rotate(1, 13, 3));
                let mut pulse = (e.age >> 2) as u16;
                if pulse & 0x10 != 0 {
                    pulse = !pulse;
                }
                cx.palette_op(PaletteOp::Color(1, 12, pulse & 15));
            }
        }
    }
    fn script(&self, e: &Enemy) -> (usize, usize) {
        (
            (e.vars[SCRIPT] >> 8) as usize,
            (e.vars[SCRIPT] & 0xff) as usize,
        )
    }
    /// `$3075E`: the routine of the current step; `$FF` ends (`$3078A`):
    /// S0 goes on with S4, SC with S10, the others loop.
    fn enter(&self, e: &mut Enemy) {
        for _ in 0..8 {
            let (script, step) = self.script(e);
            match self.scripts.get(script).and_then(|s| s.get(step)) {
                Some(&routine) => {
                    e.routine = u16::from(routine);
                    e.vars[T] = 0;
                    return;
                }
                None => {
                    let next = match script {
                        0 => 1,
                        3 => 4,
                        other => other,
                    };
                    e.vars[SCRIPT] = (next as i32) << 8;
                }
            }
        }
        // An empty script (S14, never used) stands.
        e.routine = 0x4c;
        e.vars[T] = 0;
    }
    fn next(&self, e: &mut Enemy) {
        e.vars[SCRIPT] += 1;
        self.enter(e);
    }
    fn command(e: &mut Enemy, slot: usize, command: i32) {
        e.vars[slot] = ((e.vars[slot] >> 4) + 1) << 4 | command;
    }
    /// `$30ADA`: one frame of the hop, or of the rest; true on a hop frame.
    fn hop(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        if e.vars[REST] > 0 {
            e.vars[REST] -= 1;
            return false;
        }
        // HOP: frames into the hop; the height adds the high word of `$2C`
        // after each $2000 loss (1..8, eight at 8, 7..0).
        let frame = e.vars[HOP];
        let height: i32 = (1..=frame + 1)
            .map(|f| (HOP_SPEED - HOP_DRAG * f) >> 16)
            .sum();
        e.vars[BODY_Y] = GROUND - height.max(0);
        e.vars[HOP] += 1;
        if height < 0 {
            e.vars[BODY_Y] = GROUND;
            e.vars[HOP] = 0;
            e.vars[REST] = REST_TICKS;
            cx.play(self.stomp);
            e.vars[JOLT] = 19;
            if (e.vars[SCRIPT] >> 8) == 1 {
                self.rocks(cx);
            }
        }
        true
    }
    /// `$31848`: two rocks from the top of the screen.
    fn rocks(&self, cx: &mut Ctx) {
        for _ in 0..2 {
            let mut r = i32::from(cx.random() & 0x1ff);
            while r >= 0x140 {
                r = i32::from(cx.random() & 0x1ff);
            }
            let frame = self.rock_frames[usize::from((cx.random() >> 1) & 1)];
            let wait = (cx.random() & 0x3f) + 1;
            let mut rock = Projectile::new(
                cx.camera.0 + r,
                cx.camera.1,
                0,
                true,
                frame,
                self.rock_damage,
            )
            .with_motion(Motion {
                fx: 1,
                ..Motion::default()
            })
            .box_after();
            rock.base = 0x2000;
            rock.owner = Some(cx.current);
            rock.mech = Some(MechShot::Rock {
                delay: wait - 1,
                bounced: false,
            });
            if wait == 1 {
                rock.motion.gravity = 0x1000;
            }
            rock.life = u16::MAX;
            rock.passes_terrain = true;
            cx.shoot_tiled_capped(rock, self.rock_tile, 16);
        }
    }
    /// The rocks start falling ($1000) after their wait.
    fn drop_rocks(&self, cx: &mut Ctx, terrain: &dyn Terrain) {
        let mut impacts = 0;
        for rock in cx
            .projectiles
            .iter_mut()
            .filter(|p| p.owner == Some(cx.current) && p.alive)
        {
            let Some(MechShot::Rock { delay, bounced }) = &mut rock.mech else {
                continue;
            };
            if *delay > 0 {
                *delay -= 1;
            }
            if *delay == 0 {
                rock.motion.gravity = 0x1000;
                if !*bounced && terrain.solid_at(rock.x, rock.y) {
                    // move.w #$FFFF,$1C: retain the fractional velocity word.
                    rock.motion.vy = (rock.motion.vy & 0xffff) | -0x10000;
                    rock.no_collision = true;
                    *bounced = true;
                    impacts += 1;
                }
            }
        }
        for _ in 0..impacts {
            cx.effect_sound(0x25);
        }
    }
    /// `$2C` walk toward the player's 64-px cell; true when level.
    fn walk(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        let cell = |x: i32| (x + 0x100) >> 6;
        let hopping = self.hop(cx, e);
        let (mine, theirs) = (cell(e.x), cell(cx.player.0));
        if hopping && mine != theirs {
            let right = theirs > mine;
            e.vars[BODY_X] += if right { 1 } else { -1 };
            e.facing_left = right;
        }
        !hopping && mine == theirs
    }
    /// `$40` back away; true when far enough or out of range.
    fn back_away(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        let dx = cx.player.0 - e.x;
        let far = dx.abs() >= KEEP_AWAY || !(RANGE.0..=RANGE.1).contains(&e.vars[BODY_X]);
        if self.hop(cx, e) {
            e.facing_left = dx > 0;
            if !far {
                e.vars[BODY_X] += if dx > 0 { -1 } else { 1 };
            }
            return false;
        }
        far
    }
    fn counter(e: &Enemy) -> u16 {
        e.health.saturating_sub(1)
    }
}

impl Behavior for Boss5 {
    fn label(&self, _kind: u16) -> String {
        format!("Boss5 ${HANDLER:05X} (+ core, part C)")
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        // $30654 `ori.w #$8000,8(a3)`: every frame the player joins the
        // high-priority layer of the parts, their shots and the hands.
        cx.signal(crate::combat::Signal::RaisePlayerPriority);
        cx.signal(Signal::PlayerMaxX(0x1b0));
        // Setup at $307F2 writes $80 to $FF014A, overriding the part's
        // $C0 camera limit. The arena's right edge is camera X $80.
        cx.signal(Signal::CameraMaxX(0x80));
        if e.vars[JOLT] > 0 {
            e.vars[JOLT] -= 1;
        }
        let jolt = e.vars[JOLT] as usize;
        cx.signal(Signal::VScrollOffset(if jolt > 0 && jolt & 1 == 0 {
            i32::from(self.jolt[jolt])
        } else {
            0
        }));
        self.fade_back(cx, e);
        if let Some(reaction) = cx.process_hit(e) {
            match reaction {
                Reaction::Hurt => {
                    // The hands at once when list 4 is empty; below 10 in
                    // S4 the head is destroyed instead of Func_030D78.
                    e.routine = e.saved_routine;
                    if (e.vars[SCRIPT] >> 8) == 1 && Self::counter(e) < HEAD_LIMIT {
                        e.vars[FADE] = 0;
                        e.vars[SCRIPT] = 3 << 8;
                        self.enter(e);
                        Self::command(e, HEAD_CMD, HEAD_DOWN);
                    } else {
                        self.flash(cx, e);
                        if !cx.projectiles.iter().any(|p| p.enemy) {
                            Self::command(e, HANDS_CMD, 1);
                        }
                    }
                }
                Reaction::Death => {
                    e.vars[SCRIPT] = 6 << 8;
                    e.routine = DEATH;
                    // `$30E60`: an entry frame, then 30 + 180 countdowns.
                    e.vars[T] = -1;
                }
                Reaction::Deflect => {}
            }
        }
        self.drop_rocks(cx, terrain);
        // $30CCE: standing on either hand interrupts the current action,
        // provided the head is idle and list 4 is empty.
        if e.vars[SCRIPT] >> 8 == 1
            && cx.mech_hand_contact
            && !cx.projectiles.iter().any(|p| p.enemy && p.alive)
            && !matches!(e.routine, 0x30 | 0x34 | 0x38)
        {
            e.routine = if cx.random() & 1 == 0 { 0x44 } else { 0x30 };
            e.vars[T] = 0;
        }
        let t = e.vars[T];
        e.vars[T] += 1;
        match e.routine {
            0 | 4 => {
                cx.player_event(PlayerEvent::Idle);
                // $307F2: parts A (head) and B (hands), script 0.
                e.health = self.counter + 1;
                e.hurt_routine = HURT;
                e.death_routine = DEATH;
                // Part C's $A000: palette line 1 on the high layer.
                e.tile = Some(self.core_tile);
                e.body = None;
                e.vars[BODY_X] = self.start.0;
                e.vars[BODY_Y] = self.start.1;
                e.vars[HOP] = 0;
                e.vars[REST] = 0;
                e.vars[HEAD_CMD] = 0;
                e.vars[HANDS_CMD] = 0;
                e.vars[REDDENED] = 0;
                e.vars[FADE] = 0;
                Self::place(e);
                let (x, y) = (e.x, e.y);
                if let Some(head) = cx.spawn(PART, x, y) {
                    head.routine = 0;
                }
                if let Some(hands) = cx.spawn(PART, x, y + CORE_UP - HANDS_UP) {
                    hands.routine = 0x30;
                }
                e.mapping = self.core;
                e.vars[SCRIPT] = 0;
                self.enter(e);
            }
            0x4c => {
                cx.player_event(PlayerEvent::Freeze);
                e.routine = 0x50;
                e.vars[T] = 0;
            }
            0x50 => {
                cx.player_event(PlayerEvent::Freeze);
                if t + 1 >= i32::from(self.wait[usize::from(cx.level & 3)]) {
                    self.next(e);
                }
            }
            0x08 => {
                // Stomp: voice WALK (and a jolt).
                cx.play(self.stomp);
                e.vars[JOLT] = 19;
                cx.player_event(PlayerEvent::Freeze);
                e.routine = 0x0c;
            }
            0x0c => {
                cx.player_event(PlayerEvent::Freeze);
                self.next(e);
            }
            0x10 | 0x18 => {
                cx.player_event(PlayerEvent::Freeze);
                e.routine += 4;
                e.vars[T] = 0;
            }
            0x14 | 0x1c => {
                cx.player_event(PlayerEvent::Freeze);
                // Eyes, then the body, fade in.
                if (t + 1) & 3 == 0 {
                    if e.routine == 0x14 {
                        cx.palette_op(PaletteOp::FadeColor(1, 12, self.body_palette[12]));
                    } else {
                        cx.fade_palette(1, self.body_palette);
                    }
                }
                if t + 1 >= FADE_TICKS {
                    self.next(e);
                }
            }
            0x20 | 0x24 | 0x30 | 0x34 | 0x38 => {
                // Roar (and in $30 the flame sweep after it).
                if t == 0 {
                    e.vars[SWEEP_FLIP] = i32::from(e.facing_left);
                    cx.play(self.roar);
                    Self::command(
                        e,
                        HEAD_CMD,
                        if e.routine >= 0x30 {
                            HEAD_FLAME
                        } else {
                            HEAD_ROAR
                        },
                    );
                }
                // $100 -> $40 -> $1C0 -> $100.
                let sweep = if e.vars[SWEEP_FLIP] == 0 { 190 } else { 189 };
                let length = if e.routine >= 0x30 {
                    ROAR_TICKS + sweep
                } else {
                    ROAR_TICKS
                };
                if e.routine < 0x30 {
                    cx.player_event(PlayerEvent::Freeze);
                    if t > 0 && t & 3 == 0 {
                        cx.fade_palette(3, self.arena_palette);
                    }
                } else if t >= ROAR_TICKS - 1
                    && let Some((angle, _)) =
                        sweep_pose(t - (ROAR_TICKS - 1), e.vars[SWEEP_FLIP] != 0)
                {
                    e.facing_left = angle < 0x100;
                }
                if t >= length {
                    self.next(e);
                }
            }
            0x3c | 0x64 => {
                e.vars[HOP] = 0;
                e.routine += 4;
                let dx = cx.player.0 - e.x;
                e.facing_left = dx > 0;
                if dx.abs() >= KEEP_AWAY || !(RANGE.0..=RANGE.1).contains(&e.vars[BODY_X]) {
                    self.next(e);
                }
            }
            0x40 | 0x68 => {
                // Back away.
                if self.back_away(cx, e) {
                    self.next(e);
                }
            }
            0x28 => {
                e.vars[HOP] = 0;
                e.vars[REST] = 0;
                e.routine = 0x2c;
                if (e.x + 0x100) >> 6 == (cx.player.0 + 0x100) >> 6 {
                    self.next(e);
                }
            }
            0x2c => {
                // Walk toward.
                if self.walk(cx, e) {
                    self.next(e);
                }
            }
            0x84 => e.routine = 0x88,
            0x88 => {
                // $30E4C: the next attack.
                let pick = self.attacks[usize::from(cx.random() & 3)];
                e.routine = u16::from(pick);
                e.vars[T] = 0;
            }
            0x44 | 0x48 | 0x6c | 0x70 => {
                // The hands: part B's orbs; face the player.
                if t == 0 {
                    Self::command(e, HANDS_CMD, 1);
                }
                e.facing_left = cx.player.0 > e.x;
                if t >= HANDS_TICKS {
                    self.next(e);
                }
            }
            0x5c | 0x60 => {
                // The head destroyed: 60 ticks of bursts, then S10.
                if t > 0 && t < HEAD_DOWN_TICKS && t % 4 == 0 {
                    particles(cx, (e.x, e.y), 1);
                }
                if t >= HEAD_DOWN_TICKS {
                    self.next(e);
                }
            }
            DEATH => {
                // $30E60/$30E88: entry, then sink 1/8 px per tick. The
                // independent 128-tick explosion clock runs across the
                // 30-tick hold; the 180-tick palette clock follows it.
                if t < 0 {
                    e.timer = 30;
                    e.counter = 180;
                    e.motion.stop();
                    self.draw_body(cx, e);
                    return;
                }
                cx.player_event(PlayerEvent::Freeze);
                // Sink the origin before placing every part and the plane.
                e.motion.vy = 0;
                let (fraction, carry) = e.motion.fy.overflowing_add(0x2000);
                e.vars[BODY_Y] += i32::from(carry);
                e.motion.fy = fraction;
                Self::place(e);
                self.draw_body(cx, e);
                let left = 127 - t;
                if left >= 0 && left & 7 == 0 {
                    for (dx, dy) in self.burst_offsets {
                        smoke(cx, (e.x + dx, e.y + CORE_UP + dy), (0, -0x20000), 0, 0);
                    }
                    let dx = i32::from(cx.random() & 0x7f) - 0x40;
                    let dy = -i32::from(cx.random() & 0x7f);
                    particles(cx, (e.x + dx, e.y + CORE_UP + dy), 4);
                    e.vars[JOLT] = 19;
                }
                if matches!(left, 96 | 64 | 32) {
                    cx.play(self.claw);
                }
                if left == 0 {
                    cx.play(self.boom);
                }
                if e.timer > 0 {
                    e.timer -= 1;
                    if e.timer > 0 {
                        return;
                    }
                }
                e.counter = e.counter.saturating_sub(1);
                if e.counter & 7 == 0 {
                    fade_lines(cx, 1, &self.death_palette);
                }
                if e.counter == 0 {
                    cx.signal(Signal::MechHands(None));
                    cx.next_substage();
                    Ctx::free(e);
                }
                return;
            }
            _ => self.next(e),
        }
        let small = (e.vars[SCRIPT] >> 8) == 4;
        e.mapping = if small { self.small_core } else { self.core };
        Self::place(e);
        self.draw_body(cx, e);
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        // $31000 (part C): by script id.
        match enemy.vars[SCRIPT] >> 8 {
            1 | 4 if enemy.vars[FADE] > 0 => HitResponse::Deflect,
            1 | 4 => HitResponse::Accept,
            0 | 2 | 5 => HitResponse::Deflect,
            _ => HitResponse::Pass,
        }
    }
    fn touches_player(&self) -> bool {
        // The mech's core is tested against list 3 weapons by `$313E8` /
        // `$31418`; neither it nor the plane-B body tests player contact.
        false
    }
    fn integrates(&self) -> bool {
        true
    }
}

mod parts;
pub(crate) use parts::MechPart;
