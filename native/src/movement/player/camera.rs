//! `Func_0225A2`, the follow camera the player update moves.
use super::{Player, UNIT};
use crate::input::{Button, Input};

impl Player {
    /// `Func_0225A2`, run by the stepper after every player update below the
    /// ninjutsu states. The camera moves toward the player at a limited
    /// rate instead of locking onto it; `$1C66` applies the part limits
    /// afterwards (natively `Game::apply_camera_model`). Positions are
    /// relative to the camera's top-left corner (the original adds `$80`).
    /// `accel` is the Y acceleration the original integrates after this
    /// routine: it reads `y + vy` before it (a grounded player holds vy
    /// `-$3400`), one pixel above a resting native Y. While sinking, the
    /// native camera follows the displayed feet so each pixel of descent
    /// moves Joe and the camera together.
    pub(super) fn follow_camera(&mut self, input: Input, accel: i32) {
        use crate::movement::state::{AIR_STAB, DASH_KICK, NINJUTSU};
        // Player_ReadInput ($3BBC..$3C4A): updates since the current press.
        // A press is new when `(old ^ new) & new` is nonzero on the -1/0/+1
        // words, so a reversal from -1 to +1 (Left to Right, Up to Down)
        // keeps counting while +1 to -1 restarts.
        let new_press = |old: i32, new: i32| (old as u16 ^ new as u16) & new as u16 != 0;
        let direction_x = self.direction;
        if new_press(self.previous_direction_x, direction_x) {
            self.x_held = 0;
        }
        self.previous_direction_x = direction_x;
        if direction_x != 0 {
            self.x_held = self.x_held.saturating_add(1);
        }
        let direction_y = i32::from(input.held(Button::Down)) - i32::from(input.held(Button::Up));
        if new_press(self.direction_y, direction_y) {
            self.y_held = 0;
        }
        self.direction_y = direction_y;
        if direction_y != 0 {
            self.y_held = self.y_held.saturating_add(1);
        }
        let state = self.animator.state;
        if state >= NINJUTSU {
            return;
        }
        let x = self.position().0;
        // Material 12 leaves the ROM's pre-integration vy at zero. Keep
        // its native camera aligned with the integrated sprite instead:
        // subtracting gravity makes it lag one tick at each pixel crossing
        // and Joe bobs by a pixel throughout the slow descent.
        let lead = if self.grounded && self.floor.slows() {
            0
        } else {
            accel
        };
        let y = (self.y - lead).div_euclid(UNIT);
        // Horizontal ($225A2..$226B2): beyond 108..180 catch up at 8 px per
        // update; otherwise ease at 4 toward a dead zone or a look-ahead.
        let rx = x - self.camera_x;
        let dx = if rx < 108 {
            (rx - 108).max(-8)
        } else if rx > 180 {
            (rx - 180).min(8)
        } else {
            let (mut low, mut high) = (112, 176);
            let mut ahead = false;
            if self.scroll_flags & 0x8000 != 0 {
                (low, high) = (144, 144); // $2260A
            } else if matches!(state, DASH_KICK | AIR_STAB) {
                self.look_right = !self.facing_left; // $22632
                ahead = true;
            } else if self.direction != 0 {
                // $2264A: a direction held beyond 16 updates turns the
                // look-ahead; a shorter one keeps it when it agrees.
                if self.x_held > 16 {
                    self.look_right = self.direction > 0;
                    ahead = true;
                } else {
                    ahead = (self.direction > 0) == self.look_right;
                }
            }
            if ahead {
                let target = if self.look_right { 112 } else { 176 };
                (low, high) = (target, target);
            }
            if rx < low {
                (rx - low).max(-4)
            } else if rx > high {
                (rx - high).min(4)
            } else {
                0
            }
        };
        self.camera_x += dx;
        // Vertical ($226B4..$22778): beyond 88..216 follow at once; grounded
        // or hanging, settle at 2 px per update on the part's target (Up or
        // Down alone held 48 updates looks up or down); airborne, a dead zone.
        let ry = y - self.camera_y;
        let dy = if ry < 88 {
            ry - 88
        } else if ry > 216 {
            ry - 216
        } else {
            let (mut low, mut high) = (96, 208);
            if self.hanging || self.grounded {
                let target = crate::stage::RuntimeFlags(self.scroll_flags).camera_anchor_y();
                (low, high) = (target, target);
                if direction_y != 0 {
                    let others = [
                        Button::Left,
                        Button::Right,
                        Button::Attack,
                        Button::Jump,
                        Button::Ninjutsu,
                        Button::Start,
                    ];
                    if others.iter().any(|&button| input.held(button)) {
                        self.y_held = 0; // $22732
                    } else if self.y_held >= 48 {
                        let look = if direction_y < 0 { 208 } else { 96 };
                        (low, high) = (look, look);
                    }
                }
            }
            if ry < low {
                (ry - low).max(-2)
            } else if ry > high {
                (ry - high).min(2)
            } else {
                0
            }
        };
        self.camera_y += dy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_dead_zone_is_112_through_176() {
        for (screen_x, expected_delta) in [(111, -1), (112, 0), (176, 0), (177, 1)] {
            let mut player = Player::new(screen_x, 96);
            player.camera_x = 0;
            player.camera_y = 0;
            player.follow_camera(Input::default(), 0);
            assert_eq!(player.camera_x, expected_delta, "screen x {screen_x}");
        }
    }

    #[test]
    fn pan_uses_four_pixel_lead_and_eight_pixel_hard_band_rates() {
        let mut outside = Player::new(300, 96);
        outside.camera_x = 0;
        outside.camera_y = 0;
        outside.follow_camera(Input::default(), 0);
        assert_eq!(outside.camera_x, 8);

        let mut outside_left = Player::new(0, 96);
        outside_left.camera_x = 200;
        outside_left.camera_y = 0;
        outside_left.follow_camera(Input::default(), 0);
        assert_eq!(outside_left.camera_x, 192);

        let mut walking = Player::new(144, 96);
        walking.camera_x = 0;
        walking.camera_y = 0;
        walking.direction = 1;
        let right = Input {
            held: Button::Right.mask(),
            pressed: 0,
        };
        for held_ticks in 1..=16 {
            walking.follow_camera(right, 0);
            assert_eq!(walking.camera_x, 0, "held tick {held_ticks}");
        }
        walking.follow_camera(right, 0);
        assert_eq!(walking.camera_x, 4);
    }

    #[test]
    fn dash_kick_and_air_stab_lead_toward_the_facing_side() {
        use crate::movement::state::{AIR_STAB, DASH_KICK};

        for state in [DASH_KICK, AIR_STAB] {
            for (facing_left, expected_delta) in [(false, 4), (true, -4)] {
                let mut player = Player::new(144, 96);
                player.camera_x = 0;
                player.camera_y = 0;
                player.animator.state = state;
                player.facing_left = facing_left;
                player.follow_camera(Input::default(), 0);
                assert_eq!(
                    player.camera_x, expected_delta,
                    "state {state}, facing_left {facing_left}"
                );
            }
        }
    }

    #[test]
    fn look_up_and_down_start_on_the_48th_held_tick() {
        let mut look_up = Player::new(160, 160);
        look_up.grounded = true;
        look_up.camera_x = 0;
        look_up.camera_y = 64; // screen Y 96, the stage's ordinary anchor.
        let up = Input {
            held: Button::Up.mask(),
            pressed: 0,
        };
        for held_ticks in 1..48 {
            look_up.follow_camera(up, 0);
            assert_eq!(look_up.camera_y, 64, "Up held tick {held_ticks}");
        }
        look_up.follow_camera(up, 0);
        assert_eq!(look_up.camera_y, 62);

        let mut look_down = Player::new(160, 272);
        look_down.grounded = true;
        look_down.camera_x = 0;
        look_down.camera_y = 64; // screen Y 208, selected by flags $7000.
        look_down.scroll_flags = 0x7000;
        let down = Input {
            held: Button::Down.mask(),
            pressed: 0,
        };
        for held_ticks in 1..48 {
            look_down.follow_camera(down, 0);
            assert_eq!(look_down.camera_y, 64, "Down held tick {held_ticks}");
        }
        look_down.follow_camera(down, 0);
        assert_eq!(look_down.camera_y, 66);
    }
}
