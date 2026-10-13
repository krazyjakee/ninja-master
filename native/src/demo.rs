//! Synthetic engine exercise. None of these physics constants, tiles, or
//! character behaviors claim to reproduce the original game.
use crate::input::{Button, Input};
use crate::render::{Flip, Frame, Palette, Tile};
use crate::{HEIGHT, WIDTH};

const UNIT: i32 = 256;
const FLOOR: i32 = 192;
const WORLD_WIDTH: i32 = 1280;
const PLATFORMS: [(i32, i32, i32); 3] = [(144, 154, 64), (288, 128, 80), (472, 154, 64)];
const PALETTE: Palette = [
    [0, 0, 0],
    [85, 151, 120],
    [49, 86, 78],
    [30, 51, 58],
    [178, 204, 149],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
    [0, 0, 0],
];
const GROUND_TILE: Tile = Tile([
    0x11, 0x14, 0x11, 0x14, 0x22, 0x12, 0x22, 0x12, 0x33, 0x23, 0x33, 0x23, 0x32, 0x33, 0x32, 0x33,
    0x33, 0x33, 0x33, 0x33, 0x33, 0x32, 0x33, 0x32, 0x33, 0x33, 0x33, 0x33, 0x23, 0x33, 0x23, 0x33,
]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demo {
    pub ticks: u64,
    pub x: i32,
    pub y: i32,
    velocity_y: i32,
    grounded: bool,
    facing_left: bool,
    attack_ticks: u8,
}

impl Default for Demo {
    fn default() -> Self {
        Self {
            ticks: 0,
            x: 40 * UNIT,
            y: FLOOR * UNIT,
            velocity_y: 0,
            grounded: true,
            facing_left: false,
            attack_ticks: 0,
        }
    }
}

impl Demo {
    /// Returns whether a demo sound should play this tick.
    pub fn tick(&mut self, input: Input) -> bool {
        if input.pressed(Button::Start) {
            *self = Self::default();
            return false;
        }
        self.ticks += 1;
        let horizontal = i32::from(input.held(Button::Right)) - i32::from(input.held(Button::Left));
        if horizontal != 0 {
            self.facing_left = horizontal < 0;
        }
        self.x = (self.x + horizontal * 2 * UNIT).clamp(8 * UNIT, (WORLD_WIDTH - 8) * UNIT);
        let jump = input.pressed(Button::Jump) && self.grounded;
        if jump {
            self.velocity_y = -6 * UNIT;
        }
        let old_y = self.y;
        self.velocity_y = (self.velocity_y + 64).min(8 * UNIT);
        self.y += self.velocity_y;
        self.grounded = false;
        // Demo one-way platforms. Original collision will be reconstructed as
        // game logic, not replaced with these generic physics.
        let x = self.x / UNIT;
        let mut landing = FLOOR * UNIT;
        for (left, top, width) in PLATFORMS {
            let surface = top * UNIT;
            if x + 6 > left && x - 6 < left + width && old_y <= surface && self.y >= surface {
                landing = landing.min(surface);
            }
        }
        if self.velocity_y >= 0 && self.y >= landing {
            self.y = landing;
            self.velocity_y = 0;
            self.grounded = true;
        }
        self.attack_ticks = self.attack_ticks.saturating_sub(1);
        if input.pressed(Button::Attack) {
            self.attack_ticks = 10;
        }
        jump || input.pressed(Button::Attack)
    }

    pub fn draw(&self, frame: &mut Frame) {
        let camera = (self.x / UNIT - WIDTH as i32 / 2).clamp(0, WORLD_WIDTH - WIDTH as i32);
        frame.clear([15, 24, 42]);
        frame.rect(238 - camera / 10, 30, 24, 24, [201, 211, 184]);
        // Two independently scrolling layers exercise camera/parallax behavior.
        for layer in [3, 2] {
            for column in -1..12 {
                let x = column * 64 - (camera / layer) % 64;
                let top = if layer == 3 { 62 } else { 94 };
                let color = if layer == 3 {
                    [24, 42, 58]
                } else {
                    [32, 61, 68]
                };
                frame.rect(x, top, 16, FLOOR - top, color);
                frame.rect(x - 15, top + 16, 48, 8, color);
            }
        }
        for world_x in (0..WORLD_WIDTH).step_by(8) {
            if world_x - camera < -8 || world_x - camera >= WIDTH as i32 {
                continue;
            }
            for y in (FLOOR..HEIGHT as i32).step_by(8) {
                frame.tile(&GROUND_TILE, &PALETTE, world_x - camera, y, Flip::default());
            }
        }
        for (left, top, width) in PLATFORMS {
            for x in (left..left + width).step_by(8) {
                frame.tile(&GROUND_TILE, &PALETTE, x - camera, top, Flip::default());
            }
        }
        let x = self.x / UNIT - camera;
        let feet = self.y / UNIT;
        frame.rect(x - 5, feet - 24, 10, 9, [231, 222, 190]);
        frame.rect(x - 6, feet - 21, 12, 3, [191, 70, 85]);
        frame.rect(x - 6, feet - 15, 12, 9, [223, 229, 209]);
        frame.rect(x - 5, feet - 6, 4, 6, [159, 182, 178]);
        frame.rect(x + 1, feet - 6, 4, 6, [159, 182, 178]);
        if self.attack_ticks > 0 {
            let blade_x = if self.facing_left { x - 25 } else { x + 6 };
            frame.rect(blade_x, feet - 15, 19, 2, [248, 239, 185]);
        }
        // Status indicators: grounded, attack, progress through the test level.
        frame.rect(
            8,
            8,
            6,
            6,
            if self.grounded {
                [85, 151, 120]
            } else {
                [191, 70, 85]
            },
        );
        frame.rect(18, 8, i32::from(self.attack_ticks), 6, [248, 239, 185]);
        frame.rect(8, 20, 80, 2, [32, 61, 68]);
        frame.rect(8, 20, self.x / UNIT * 80 / WORLD_WIDTH, 2, [178, 204, 149]);
    }
}

/// Repeatable demo input for headless checks and desktop smoke runs.
pub fn scripted_input(tick: u64) -> Input {
    Input {
        held: if tick % 480 < 240 {
            Button::Right.mask()
        } else {
            Button::Left.mask()
        },
        pressed: if tick % 90 == 10 {
            Button::Jump.mask()
        } else if tick % 90 == 50 {
            Button::Attack.mask()
        } else {
            0
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::press;

    #[test]
    fn jump_lands_and_holding_does_not_auto_jump() {
        let mut demo = Demo::default();
        demo.tick(press(Button::Jump));
        assert!(demo.y < FLOOR * UNIT);
        for _ in 0..120 {
            demo.tick(Input {
                held: Button::Jump.mask(),
                pressed: 0,
            });
        }
        assert_eq!(demo.y, FLOOR * UNIT);
        assert!(demo.grounded);
    }

    #[test]
    fn walking_off_platform_falls_to_floor() {
        let mut demo = Demo {
            x: 160 * UNIT,
            y: 154 * UNIT,
            ..Demo::default()
        };
        for _ in 0..100 {
            demo.tick(Input {
                held: Button::Right.mask(),
                pressed: 0,
            });
        }
        assert_eq!(demo.y, FLOOR * UNIT);
        assert!(demo.grounded);
    }
}
