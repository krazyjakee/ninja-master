//! Type `$98`, `Enemy98` (`$18086`): the surging blob spitter.
use crate::{
    combat::{Effect, EffectKind, Enemy, Motion, Projectile, Spawn},
    enemies::{Behavior, Ctx, HitResponse, shot_damage},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `$2A`: surfacing frame; `$2C`: hold ticks; `$2E`: wait; `$32`: surges
/// left; `$34`: surges before turning; `obj_flags` bit 0: moving right.
const ANIM: usize = 0;
const HOLD: usize = 1;
const WAIT: usize = 2;
const SURGES: usize = 3;
const TURNS: usize = 4;
const RIGHT: usize = 5;

/// `Func_018352`: the list-4 blob (projectile 9) lobbed from the mouth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Blob {
    /// `Data_01834A`: unflipped / flipped.
    offsets: [(i32, i32); 2],
    speed: i32,
    vy: i32,
    gravity: i32,
    mapping: u16,
    damage: u8,
}
impl Blob {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            offsets: [
                (signed(rom, 0x1834a)?, signed(rom, 0x1834c)?),
                (signed(rom, 0x1834e)?, signed(rom, 0x18350)?),
            ],
            speed: long(rom, 0x18390)?,
            vy: long(rom, 0x183a0)?,
            gravity: word_i(rom, 0x183a8)?,
            // Data_018348; `Func_0183C4` alternates it with the next every 8 ticks.
            mapping: word(rom, 0x18348)?,
            damage: shot_damage(rom, usize::from(byte(rom, 0x18353)?))?,
        })
    }
    fn fire(&self, cx: &mut Ctx, e: &Enemy) {
        let (dx, dy) = self.offsets[usize::from(e.facing_left)];
        let base = cx.base(0x98);
        let mut shot = Projectile::new(e.x + dx, e.y + dy, 0, true, self.mapping, self.damage)
            .with_art(base, false)
            .with_motion(Motion {
                vx: if e.facing_left {
                    self.speed
                } else {
                    -self.speed
                },
                vy: self.vy,
                gravity: self.gravity,
                ..Motion::default()
            });
        let (a, b) = ((self.mapping, false), (self.mapping + 1, false));
        shot.frames = Some([a, a, b, b]);
        cx.shoot(shot);
    }
}

/// `Func_018280`: two list-1 `$20` bubbles near the object, rising at twice
/// the unit speed within 45 degrees of vertical. The ROM steps
/// `Data_01829E` every four ticks; natively they keep its first mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bubbles {
    count: usize,
    x_mask: u16,
    x_bias: i32,
    y_mask: u16,
    angle_mask: u16,
    angle_base: u16,
    gravity: i32,
    base: u16,
    mapping: u16,
    life: i32,
}
impl Bubbles {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            count: usize::from(byte(rom, 0x18281)?) + 1,
            x_mask: word(rom, 0x182c2)?,
            x_bias: word_i(rom, 0x182c6)?,
            y_mask: word(rom, 0x182d4)?,
            angle_mask: word(rom, 0x182e4)?,
            angle_base: word(rom, 0x182e8)?,
            gravity: word_i(rom, 0x18300)?,
            base: word(rom, 0x18306)?,
            mapping: word(rom, 0x1829e)?,
            // Four Data_01829E frames, one per four ticks.
            life: 16,
        })
    }
    fn spawn(&self, cx: &mut Ctx, x: i32, y: i32) {
        for _ in 0..self.count {
            let dx = i32::from(cx.random() & self.x_mask) - self.x_bias;
            let dy = -i32::from(cx.random() & self.y_mask);
            let angle = (cx.random() & self.angle_mask) + self.angle_base;
            let (cos, sin) = cx.shared.sine.polar(angle);
            let mut bubble = Effect::new(EffectKind::Pieces, x + dx, y + dy, self.mapping);
            bubble.base = self.base;
            bubble.counter = self.life;
            bubble.motion = Motion {
                vx: cos << 1,
                vy: sin << 1,
                gravity: self.gravity,
                ..Motion::default()
            };
            cx.effect(bubble);
        }
    }
}

/// `$98`: waits until the player is within 256 px, then surges along the
/// floor in 2.25 px/tick bursts that decelerate to rest (four bursts per
/// direction), stops after 1..4 bursts, surfaces (`Data_018068`, bubbles),
/// holds 60 ticks, rears (`$195`), lobs a blob (`$196`) and sinks again.
/// It can be hit only while surfaced (`$14`..`$1C`).
pub(crate) struct Spitter {
    /// `Data_018068`: (mapping, word) — low byte: hold ticks, high byte: bubbles.
    frames: Vec<(u16, u16)>,
    /// `Data_018064`, `Data_018066`.
    rise: [u16; 2],
    hurt: u16,
    death: u16,
    turns: i32,
    wake: i32,
    surge_speed: i32,
    surge_accel: i32,
    surge_mask: u16,
    sound: u8,
    hold_wait: i32,
    rise_wait: i32,
    blob: Blob,
    bubbles: Bubbles,
}
impl Spitter {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut frames = Vec::new();
        for i in 0..16 {
            let mapping = word(rom, 0x18068 + i * 4)?;
            if mapping & 0x8000 != 0 {
                break;
            }
            frames.push((mapping, word(rom, 0x1806a + i * 4)?));
        }
        if frames.is_empty() {
            return Err("empty Enemy98 frame list".into());
        }
        Ok(Self {
            frames,
            rise: [word(rom, 0x18064)?, word(rom, 0x18066)?],
            hurt: word(rom, 0x180ea)?,
            death: word(rom, 0x180f0)?,
            turns: word_i(rom, 0x180f6)?,
            wake: word_i(rom, 0x18116)?,
            surge_speed: long(rom, 0x1812a)?,
            surge_accel: signed(rom, 0x18130)?,
            surge_mask: word(rom, 0x18180)?,
            sound: byte(rom, 0x18199)?,
            hold_wait: word_i(rom, 0x181dc)?,
            rise_wait: word_i(rom, 0x1820e)?,
            blob: Blob::load(rom)?,
            bubbles: Bubbles::load(rom)?,
        })
    }
    /// `$1815E`: the next surge; every fourth reverses the direction.
    fn next_surge(&self, e: &mut Enemy) {
        e.routine = 8;
        e.vars[TURNS] -= 1;
        if e.vars[TURNS] < 0 {
            e.vars[TURNS] = self.turns;
            e.vars[RIGHT] ^= 1;
        }
    }
}
impl Behavior for Spitter {
    fn label(&self, _kind: u16) -> String {
        "Enemy98 $18086".into()
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        // Object_ProcessHit is called only while surfaced.
        if matches!(e.routine, 0x14 | 0x18 | 0x1c) {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        // $18086: flipped while the player is to the right.
        e.facing_left = !cx.player_left(e.x);
        e.timer = e.timer.wrapping_add(1);
        if matches!(e.routine, 0x14 | 0x18 | 0x1c) && cx.process_hit(e).is_some() {
            return;
        }
        loop {
            match e.routine {
                0 => {
                    // $180D4
                    e.timer = 0;
                    e.motion.stop();
                    e.hurt_routine = self.hurt;
                    e.death_routine = self.death;
                    e.vars[TURNS] = self.turns;
                    e.vars[SURGES] = 0;
                    e.mapping = self.frames[0].0;
                    e.routine = 4;
                }
                4 => {
                    // $1810C
                    if cx.player_dx(e.x) < self.wake {
                        e.routine = 8;
                    }
                    return;
                }
                8 => {
                    // $18120: a surge against its own deceleration.
                    e.mapping = self.frames[0].0;
                    let right = e.vars[RIGHT] != 0;
                    e.motion.vx = if right {
                        self.surge_speed
                    } else {
                        -self.surge_speed
                    };
                    e.motion.ax = if right {
                        self.surge_accel
                    } else {
                        -self.surge_accel
                    };
                    e.routine = 0x0c;
                    return;
                }
                0x0c => {
                    // $1814C: the surge ends when the velocity turns.
                    if ((e.motion.ax as i16) ^ ((e.motion.vx >> 16) as i16)) < 0 {
                        return;
                    }
                    e.vars[SURGES] -= 1;
                    if e.vars[SURGES] >= 0 {
                        self.next_surge(e);
                        return;
                    }
                    // $18178: stop and surface.
                    e.vars[SURGES] = i32::from(cx.random() & self.surge_mask);
                    e.timer = 0;
                    e.motion.stop();
                    e.vars[ANIM] = 0;
                    e.routine = 0x10;
                    cx.visible_sound(e.x, e.y, self.sound);
                    return;
                }
                0x10 => {
                    // $181A0: one frame every eight ticks.
                    if e.timer & 7 != 0 {
                        return;
                    }
                    let index = e.vars[ANIM].max(0) as usize;
                    e.vars[ANIM] += 1;
                    let Some(&(mapping, extra)) = self.frames.get(index) else {
                        self.next_surge(e);
                        return;
                    };
                    e.mapping = mapping;
                    if extra & 0xff != 0 {
                        e.vars[HOLD] = i32::from(extra & 0xff);
                        e.vars[WAIT] = self.hold_wait;
                        e.routine = 0x14;
                    } else if extra & 0xff00 != 0 {
                        self.bubbles.spawn(cx, e.x, e.y);
                    }
                    return;
                }
                0x14 => {
                    // $181EE: surfaced, then rear up.
                    e.vars[WAIT] -= 1;
                    if e.vars[WAIT] >= 0 {
                        return;
                    }
                    e.mapping = self.rise[0];
                    e.vars[WAIT] = self.rise_wait;
                    e.routine = 0x18;
                }
                0x18 => {
                    // $18216
                    e.vars[WAIT] -= 1;
                    if e.vars[WAIT] >= 0 {
                        return;
                    }
                    e.mapping = self.rise[1];
                    self.blob.fire(cx, e);
                    e.routine = 0x1c;
                }
                0x1c => {
                    // $18238: hold the pose, then sink (the rest of the frames).
                    e.vars[HOLD] -= 1;
                    if e.vars[HOLD] >= 0 {
                        return;
                    }
                    e.routine = 0x10;
                    cx.visible_sound(e.x, e.y, self.sound);
                    return;
                }
                0x20 => {
                    // $18260: score (central), Func_036DAE, free.
                    let blast = cx.shared.blasts[0];
                    cx.explode(e.x, e.y, blast);
                    Ctx::free(e);
                    return;
                }
                _ => {
                    // $18272: hurt resumes.
                    e.vars[HOLD] = 0;
                    e.routine = e.saved_routine;
                    return;
                }
            }
        }
    }
}
