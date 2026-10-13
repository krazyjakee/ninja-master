//! Jump startup/hold and the run double tap.
use super::{DASH_SPEED, GRAVITY, JUMP_HOLD_TICKS, TAKEOFF_DELAY_TICKS, WALK_SPEED};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JumpFrame {
    pub requested: bool,
    pub preparing: bool,
    pub launch: bool,
    pub gravity: i32,
}

/// Ordinary jump startup and hold policy, excluding collision-special
/// states. Call once after support detection, before velocity integration.
/// The state-16 handler launches at cursor `$100` (`$20FB0`) without testing
/// the floor, so a preparation that leaves the ground still jumps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JumpControl {
    pending: Option<u8>,
    hold: u8,
}
impl JumpControl {
    pub fn tick(
        &mut self,
        pressed: bool,
        held: bool,
        grounded: bool,
        velocity_y: i32,
    ) -> JumpFrame {
        let requested = pressed && self.pending.is_none();
        if requested {
            self.pending = Some(TAKEOFF_DELAY_TICKS);
        }
        if let Some(remaining) = self.pending {
            if remaining != 0 {
                self.pending = Some(remaining - 1);
                return JumpFrame {
                    requested,
                    preparing: true,
                    ..JumpFrame::default()
                };
            }
            self.pending = None;
            self.hold = JUMP_HOLD_TICKS;
            return JumpFrame {
                launch: true,
                gravity: GRAVITY,
                ..JumpFrame::default()
            };
        }
        if grounded {
            self.hold = 0;
            return JumpFrame::default();
        }
        let mut gravity = GRAVITY;
        if velocity_y < 0 {
            if held && self.hold != 0 {
                self.hold -= 1;
                gravity = 0;
            } else if !held {
                self.hold = 0;
            }
        }
        JumpFrame {
            gravity,
            ..JumpFrame::default()
        }
    }
    /// A request is still waiting for its launch tick.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    /// The preparation left state 16 (an air attack replaced it): no launch.
    pub fn cancel_pending(&mut self) {
        self.pending = None;
    }
    /// $213CA: the wall jump installs the same eleven-tick hold allowance;
    /// climbing out of a hang (`$2357A`) installs seven.
    pub fn start_hold(&mut self, ticks: u8) {
        self.pending = None;
        self.hold = ticks;
    }
    /// States other than 16 and 25 never run the hold code ($21020).
    pub fn cancel_hold(&mut self) {
        self.hold = 0;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HorizontalFrame {
    pub velocity: i32,
    pub running: bool,
}

/// $3C00..$3C4A direction ages and $1FEC4..$1FF4C ordinary double tap.
/// Both preceding hold and release durations must be strictly below eight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunControl {
    direction: i32,
    held_ticks: u8,
    released_ticks: u8,
    running: bool,
    pending: bool,
}
impl Default for RunControl {
    fn default() -> Self {
        Self {
            direction: 0,
            held_ticks: 0,
            released_ticks: 255,
            running: false,
            pending: false,
        }
    }
}
impl RunControl {
    /// Supply facing from BEFORE processing this direction, as the ROM gate
    /// compares the new edge against the previous facing flag.
    pub fn tick(&mut self, direction: i32, facing_left: bool, grounded: bool) -> HorizontalFrame {
        let direction = direction.signum();
        let previous = self.direction;
        let changed = direction != previous;
        let previous_hold = self.held_ticks;
        if changed {
            if previous != 0 {
                self.released_ticks = 0;
            }
            if direction != 0 {
                self.held_ticks = 0;
            }
        }
        if direction == 0 {
            self.released_ticks = self.released_ticks.saturating_add(1);
        } else {
            self.held_ticks = self.held_ticks.saturating_add(1);
        }
        self.direction = direction;
        if self.running || self.pending {
            if direction == 0 || changed {
                self.running = false;
                self.pending = false;
                return HorizontalFrame::default();
            }
            self.running = true;
            self.pending = false;
        } else if grounded
            && direction != 0
            && changed
            && previous_hold < 8
            && self.released_ticks < 8
            && (direction < 0) == facing_left
        {
            self.pending = true;
            // $1FEE6 installs state 28 on the accepted edge, but its three-pixel
            // speed starts on the next tick. Report the state for animation.
            return HorizontalFrame {
                velocity: 0,
                running: true,
            };
        }
        HorizontalFrame {
            velocity: direction * if self.running { DASH_SPEED } else { WALK_SPEED },
            running: self.running,
        }
    }
    /// Leave the run without changing the direction ages (dash kick, hurt).
    pub fn stop(&mut self) {
        self.running = false;
        self.pending = false;
    }
    /// +$2A bit 14 without a double tap: state 25 sets it in the air
    /// (`$21314`) and the wall attacks keep it, so state 16 after them
    /// moves at run speed while the direction holds.
    pub fn latch(&mut self) {
        self.running = true;
        self.pending = false;
    }
    /// `Func_0223F6` (`$22428..$22436`): a wall stopped the motion. The run
    /// ends and `act_x_held` becomes `$FF`, so the next press cannot be the
    /// second tap of a run.
    pub fn blocked(&mut self) {
        self.stop();
        self.held_ticks = u8::MAX;
    }
}

/// Launch selection at $20FCE..$21006. Keep the flag unnamed until its writer
/// semantics are recovered; calling it a power-up would exceed the evidence.
pub const fn launch_velocity_16(flag_29_bit1: bool, material: u8) -> i32 {
    match (flag_29_bit1, material == 12) {
        (true, false) => -0x60000,
        (false, true) => -0x3b000,
        _ => -0x4b000,
    }
}
