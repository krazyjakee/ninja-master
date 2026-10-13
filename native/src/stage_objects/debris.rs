//! List 1 and the device particles: fragments, smoke and sparkles that
//! only move and animate.
use super::StageObjects;
use crate::combat::Motion;

/// `$35E6C`, `$29826`: four list-1 type-22 fragments, indices 3..0.
pub(super) fn push_debris(debris: &mut Vec<Debris>, x: i32, y: i32) {
    for index in (0..4).rev() {
        debris.push(Debris {
            index,
            x,
            y,
            motion: Motion::default(),
            mapping: 0,
            routine: 0,
            counter: 0,
            alive: true,
        });
    }
}

/// List-1 type 22 (`$36C6E`): a box fragment, then a sparkle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Debris {
    index: usize,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) motion: Motion,
    pub(super) mapping: u16,
    routine: u8,
    counter: i32,
    alive: bool,
}

/// Fragments shed by horseback obstacles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DeviceParticle {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) mapping: u16,
    pub(super) motion: Motion,
    pub(super) timer: i32,
}

/// Barrel smoke: list-1 type 5 (`$3696A`), hidden on alternate updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BarrelSmoke {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) mapping: u16,
    age: usize,
}
impl BarrelSmoke {
    pub(super) fn new(x: i32, y: i32) -> Self {
        Self {
            x,
            y,
            mapping: 0,
            age: 0,
        }
    }
    pub(super) fn visible(&self) -> bool {
        self.age.is_multiple_of(2)
    }
}

/// Barrel bursts: list-4 type `$2F` (`$29B84`). Each repeats its ten-count
/// animation at the spawn position one, two or four times (`$299EC`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BarrelBurst {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) mapping: u16,
    pub(super) hitbox_mapping: u16,
    pub(super) motion: Motion,
    origin: (i32, i32),
    velocity: (i32, i32),
    repeats: i32,
    period: i32,
    counter: i32,
}
impl BarrelBurst {
    pub(super) fn new(x: i32, y: i32, [dx, vx, vy, repeats, period]: [i32; 5]) -> Self {
        Self {
            x: x + dx,
            y,
            mapping: 0,
            hitbox_mapping: 0,
            motion: Motion::default(),
            origin: (x + dx, y),
            velocity: (vx, vy),
            repeats,
            period,
            counter: 0,
        }
    }
    fn restart(&mut self) {
        self.mapping = 0x39c;
        (self.x, self.y) = self.origin;
        // $29BB4..$29BCC write only the integer position/velocity words.
        self.motion.vx = (self.velocity.0 << 16) | (self.motion.vx & 0xffff);
        self.motion.vy = (self.velocity.1 << 16) | (self.motion.vy & 0xffff);
        // The integration helper sign-extends this acceleration word.
        self.motion.gravity = -0x8000;
        self.counter = self.period;
    }
    fn update(&mut self) -> bool {
        // $29B84 loads the hitbox before changing the animation.
        self.hitbox_mapping = self.mapping;
        if self.counter == 0 {
            self.restart();
        }
        self.counter -= 1;
        if self.counter == 0 {
            self.repeats -= 1;
            if self.repeats == 0 {
                return false;
            }
            self.restart();
            self.counter -= 1;
        }
        if self.counter % 2 == 0 {
            self.mapping += 1;
        }
        true
    }
}

impl StageObjects {
    // List 4 runs before the barrel controller; new bursts first update
    // on the following tick. List 1 smoke updates on its allocation tick.
    pub(super) fn update_barrel_bursts(&mut self) {
        self.barrel_bursts.retain_mut(BarrelBurst::update);
    }

    pub(super) fn update_barrel_smoke(&mut self) {
        self.barrel_smoke.retain_mut(|smoke| {
            let Some(&mapping) = self.tables.barrel_smoke.get(smoke.age) else {
                return false;
            };
            smoke.mapping = mapping;
            smoke.age += 1;
            true
        });
    }

    /// `$36C6E`: the fragments hold their piece for nine ticks, then sparkle.
    pub(super) fn update_debris(&mut self) {
        let tables = &self.tables;
        for debris in &mut self.debris {
            loop {
                match debris.routine {
                    0 => {
                        debris.motion = Motion::default();
                        debris.mapping = tables.debris_mappings[debris.index];
                        let (cos, sin) = tables.sine.polar(tables.debris_angles[debris.index]);
                        debris.motion.vx = cos << 2;
                        debris.motion.vy = sin << 2;
                        debris.motion.gravity = tables.debris_gravity;
                        debris.counter = tables.debris_hold;
                        debris.routine = 1;
                        break;
                    }
                    1 => {
                        debris.counter -= 1;
                        if debris.counter >= 0 {
                            break;
                        }
                        debris.counter = 0;
                        debris.routine = 2;
                    }
                    _ => {
                        match tables.debris_sequence.get(debris.counter as usize) {
                            Some(&mapping) => debris.mapping = mapping,
                            None => debris.alive = false,
                        }
                        debris.counter += 1;
                        break;
                    }
                }
            }
        }
        self.debris.retain(|debris| debris.alive);
    }

    pub(super) fn update_device_debris(&mut self) {
        for debris in &mut self.device_debris {
            debris.timer -= 1;
        }
        self.device_debris.retain(|debris| debris.timer > 0);
    }
}
