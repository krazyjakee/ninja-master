//! `plr_event` (+$28): the codes enemies, bosses and stage objects write
//! for `Player_Update` to consume once.
use super::{Player, PlayerHooks, UNIT};
use crate::{
    combat::PlayerHit,
    input::{Button, Input},
    movement::{
        control::JumpControl,
        state::{IDLE, THROWN, THROWN_DEATH, WALK},
    },
};

/// Values written to `plr_event` and consumed once by `Player_Update`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PlayerEvent {
    ThrownLeft = 1,
    ThrownRight = 2,
    WalkRight = 3,
    WalkLeft = 4,
    Idle = 5,
    Freeze = 6,
    Ninjutsu = 7,
    Attack = 8,
    Jump = 9,
    GrabDamage = 0x0a,
    Stun = 0x0b,
}
impl PlayerEvent {
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::ThrownLeft,
            2 => Self::ThrownRight,
            3 => Self::WalkRight,
            4 => Self::WalkLeft,
            5 => Self::Idle,
            6 => Self::Freeze,
            7 => Self::Ninjutsu,
            8 => Self::Attack,
            9 => Self::Jump,
            0x0a => Self::GrabDamage,
            0x0b => Self::Stun,
            _ => return None,
        })
    }
    pub fn code(self) -> u8 {
        self as u8
    }
    /// The forced-input table at `$1FCCE` for codes 3-9 (code 6 has its own
    /// path): the input it substitutes, the state it selects and the last
    /// cursor slot that state keeps.
    pub(super) fn forced(self) -> Option<(Input, usize, u8)> {
        let button = |button: Button, held| Input {
            held: if held { button.mask() } else { 0 },
            pressed: button.mask(),
        };
        let walk = |button: Button| Input {
            held: button.mask(),
            pressed: 0,
        };
        Some(match self {
            Self::WalkRight => (walk(Button::Right), WALK, 6),
            Self::WalkLeft => (walk(Button::Left), WALK, 6),
            Self::Idle => (Input::default(), IDLE, 2),
            // Code 7 is A newly pressed, code 8 B newly pressed, and code 9
            // C held and newly pressed in the default control layout.
            Self::Ninjutsu => (button(Button::Ninjutsu, false), IDLE, 2),
            Self::Attack => (button(Button::Attack, false), IDLE, 2),
            Self::Jump => (button(Button::Jump, true), IDLE, 2),
            _ => return None,
        })
    }
}

impl Player {
    /// `Player_ApplyHit` with `plr_event` 1/2: `PlayerState_Thrown` at
    /// cursor 0, facing the throw, with the hit's damage saved for the slam.
    pub(super) fn enter_thrown(&mut self, hit: PlayerHit, left: bool) {
        self.animator.force(THROWN, 0);
        self.animator.cursor = 0;
        self.thrown_damage = hit.damage;
        self.facing_left = left;
        self.thrown_rightward = left;
        self.hanging = false;
        self.jump_control = JumpControl::default();
        self.run_control.stop();
    }
    /// `PlayerState_Thrown` (`$203A8`) at `cursor`: the lift, the turn and
    /// the slam, which subtracts the saved damage. Returns true when the
    /// slam was lethal and entered state 46 (`$20418`).
    pub(super) fn thrown_step(&mut self, cursor: u16, hooks: &mut impl PlayerHooks) -> bool {
        match cursor {
            0x100 => self.y -= 40 * UNIT,                  // `$203E0`
            0x300 => self.facing_left = !self.facing_left, // `$203E8`
            0x600 => {
                self.velocity_y = -2 * UNIT;
                if hooks.damage(self.thrown_damage) {
                    self.animator.die(THROWN_DEATH, cursor);
                    self.velocity_y = 0;
                    self.halt();
                    return true;
                }
            }
            _ => {}
        }
        false
    }
    /// The thrown state's X velocity at `cursor`, in the direction the
    /// throw began.
    pub(super) fn thrown_velocity(&self, cursor: u16) -> i32 {
        const SPEEDS: [i32; 52] = [
            0, 0, 0, 0, 8, 7, 6, 5, 4, 4, 4, 4, 8, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 2,
            2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let speed = SPEEDS.get(usize::from(cursor >> 6)).copied().unwrap_or(0);
        (if self.thrown_rightward { speed } else { -speed }) * UNIT
    }
}
