//! List-9 type `$3C` (`Func_014592`): the `$18` end guard's parts. Its
//! entry (routine 0) allocates two jet-skis (`Func_0144E8`, `Data_0144D0`)
//! that wait, dash right and brake, wait again and speed off right,
//! throwing up list-9 wake splashes (`Func_014630`) while moving. The drop
//! onto the water (routine `$10`) allocates the glide's flame
//! (`Func_01452C`, routine `$10` of the part), which follows the guard until
//! the glide ends. None collides with anything: list 9 is drawn behind the
//! other objects and no collision pass reads it.
//!
//! Native: each part is an [`EffectKind::GuardPart`] that the guard's update
//! drives after its own handler, where the ROM runs list 9 after list 5: a
//! part gets its first update in the frame that allocates it and sees the
//! guard's position before the guard's motion step.
use super::splash;
use crate::{
    combat::{Effect, EffectKind, Enemy},
    enemies::Ctx,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `Data_0144D0` record of a jet-ski.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ski {
    /// `+0`/`+2`: the poses (+ `$183`) shown on even and odd ticks
    /// (`Func_01465C`); `+0` also while it brakes.
    poses: [u16; 2],
    /// `+4`: ticks before the dash; `+6`: ticks it waits after braking.
    waits: (i32, i32),
    /// `+8`: the spray's offset below it (`Func_014630`).
    spray_dy: i32,
    /// `+$A`: its offset below the guard at the entry.
    dy: i32,
}
impl Ski {
    fn load(rom: &[u8], at: usize) -> Result<Self> {
        Ok(Self {
            poses: [word(rom, at)?, word(rom, at + 2)?],
            waits: (word_i(rom, at + 4)?, word_i(rom, at + 6)?),
            spray_dy: signed(rom, at + 8)?,
            dy: signed(rom, at + 10)?,
        })
    }
}

/// `+$04` of the parts.
const WAIT: u8 = 0;
const DASH: u8 = 4;
const PAUSE: u8 = 8;
const LEAVE: u8 = 0x0c;
const FLAME: u8 = 0x10;

/// `+$28` of each part's recycled record before its first update:
/// `Object_InitRecord` leaves it. Genesis Plus GX, 4-1 idle from the r4-1
/// state: 0 for the first jet-ski, -1 (a spent splash's) for the second
/// and the flame. The phase of the poses, the sprays and the flame's
/// sway follows it.
const RECYCLED_TICKS: [u16; 3] = [0, 0xffff, 0xffff];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Escort {
    skis: [Ski; 2],
    /// `addi.w #$183,d0`: the first of the guard's poses.
    pose_base: u16,
    /// `$145A6`: the dash, `$F700` in `+$14` and 4.0 px/tick.
    dash: (i32, i32),
    /// `$14600`: the acceleration away.
    leave_accel: i32,
    /// `Func_004A4E` effect `$22` at the dash, the departure and the
    /// flame's allocation.
    sound: u8,
    /// `Func_014630`: a list-9 splash with `+$28` = `spray_life` on
    /// ticks with no bit of `spray_mask` set.
    spray_mask: u16,
    spray_life: i32,
    /// `Func_01452C`: the flame's pose and its offset from the guard
    /// (`Data_013ECE`), shifted by `sway` on even ticks (`$14692`).
    flame: u16,
    flame_offset: (i32, i32),
    sway: i32,
}
impl Escort {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            // moveq #0,d6 / moveq #$C,d6 before each jsr Func_0144E8.
            skis: [Ski::load(rom, 0x144d0)?, Ski::load(rom, 0x144dc)?],
            pose_base: word(rom, 0x1451e)?,
            dash: (signed(rom, 0x145aa)?, long(rom, 0x145b0)?),
            leave_accel: signed(rom, 0x14602)?,
            sound: byte(rom, 0x145bb)?,
            spray_mask: word(rom, 0x14636)?,
            spray_life: i32::from(byte(rom, 0x1463b)?),
            flame: word(rom, 0x14568)?,
            flame_offset: (signed(rom, 0x13ece)?, signed(rom, 0x13ed0)?),
            sway: signed(rom, 0x14692)?,
        })
    }
    /// Every jet-ski pose, unflipped as [`Self::part`] draws them.
    pub(super) fn ski_poses(&self) -> [u16; 4] {
        let [a, b] = self.skis;
        [a.poses[0], a.poses[1], b.poses[0], b.poses[1]].map(|pose| self.pose_base + pose)
    }
    /// Routine 0 (`$13FE6`): both jet-skis at the guard, `Func_01DDB2`'s
    /// tile word 0.
    pub(super) fn enter(&self, cx: &mut Ctx, e: &Enemy) {
        for (index, ski) in self.skis.iter().enumerate() {
            let mut part = Self::part(e.x, e.y + ski.dy, self.pose_base + ski.poses[0]);
            part.counter = ski.waits.0;
            part.phase = WAIT;
            part.period = index as u16;
            part.angle = RECYCLED_TICKS[index];
            cx.effect(part);
        }
    }
    /// Routine `$10` (`$140BE`): the glide's flame.
    pub(super) fn glide(&self, cx: &mut Ctx, e: &Enemy) {
        let (dx, dy) = self.flame_offset;
        let mut part = Self::part(e.x + dx, e.y + dy, self.flame);
        part.phase = FLAME;
        part.angle = RECYCLED_TICKS[2];
        cx.effect(part);
        cx.effect_sound(self.sound);
    }
    /// `$140EE`: `move.w #1,$30(a1)` ends the flame at its next update.
    pub(super) fn land(cx: &mut Ctx) {
        for part in cx.effects.iter_mut() {
            if part.kind == EffectKind::GuardPart && part.phase == FLAME {
                part.counter = 1;
            }
        }
    }
    fn part(x: i32, y: i32, mapping: u16) -> Effect {
        let mut part = Effect::new(EffectKind::GuardPart, x, y, mapping);
        part.back = true;
        part
    }
    /// One update of every part, after the guard's handler: the part's
    /// handler, then `$19D4`'s motion.
    pub(super) fn drive(&self, cx: &mut Ctx, e: &Enemy) {
        for index in 0..cx.effects.len() {
            if cx.effects[index].kind != EffectKind::GuardPart || !cx.effects[index].alive {
                continue;
            }
            let mut part = cx.effects[index].clone();
            self.update(cx, e, &mut part);
            cx.effects[index] = part;
        }
        cx.effects
            .retain(|part| part.alive || part.kind != EffectKind::GuardPart);
    }
    fn update(&self, cx: &mut Ctx, e: &Enemy, part: &mut Effect) {
        // Func_03670C: addq.w #1,$28(a0).
        part.angle = part.angle.wrapping_add(1);
        let ski = self.skis[usize::from(part.period & 1)];
        match part.phase {
            WAIT => {
                // $1459C
                part.counter -= 1;
                if part.counter < 0 {
                    part.motion.ax = self.dash.0;
                    part.motion.vx = self.dash.1;
                    part.phase = DASH;
                    cx.effect_sound(self.sound);
                }
            }
            DASH => {
                // $145C2: brakes once the speed's whole part has the
                // acceleration's sign (`eor.w d0,d1` / `bmi`).
                self.spray(cx, part, ski);
                self.pose(part, ski);
                if ((part.motion.vx >> 16) as i16 ^ part.motion.ax as i16) >= 0 {
                    part.motion.ax = i32::from(part.motion.ax as i16 >> 1);
                    part.counter = ski.waits.1;
                    part.mapping = self.pose_base + ski.poses[0];
                    part.phase = PAUSE;
                    self.pause(cx, part);
                }
            }
            PAUSE => self.pause(cx, part),
            LEAVE => {
                // $14612: freed once off screen (`obj_flags` bit 7 of
                // the last update), beyond a widened view's edges too.
                self.spray(cx, part, ski);
                self.pose(part, ski);
                if cx.outside_view(part.x, part.y) {
                    part.alive = false;
                    return;
                }
            }
            _ => {
                // loc_014674: the flame follows the guard, `sway` px
                // further on even ticks, until the landing's flag.
                let (dx, dy) = self.flame_offset;
                part.x = e.x + dx + if part.angle & 1 == 0 { self.sway } else { 0 };
                part.y = e.y + dy;
                if part.counter != 0 {
                    part.alive = false;
                }
                return;
            }
        }
        part.motion.step(&mut part.x, &mut part.y);
    }
    /// `$145FA`: wait, then accelerate away to the right.
    fn pause(&self, cx: &mut Ctx, part: &mut Effect) {
        part.counter -= 1;
        if part.counter < 0 {
            part.motion.ax = self.leave_accel;
            part.phase = LEAVE;
            cx.effect_sound(self.sound);
        }
    }
    /// `Func_01465C`: the pose of this tick's parity.
    fn pose(&self, part: &mut Effect, ski: Ski) {
        part.mapping = self.pose_base + ski.poses[usize::from(part.angle & 1)];
    }
    /// `Func_014630`: `Func_0176C6`'s splash at the jet-ski, `+8` below.
    fn spray(&self, cx: &mut Ctx, part: &Effect, ski: Ski) {
        if part.angle & self.spray_mask == 0 {
            splash(cx, (part.x, part.y + ski.spray_dy), self.spray_life, true);
        }
    }
}
