//! `PlayerState_Death` (`$204E4`), the handler of every death state (#58).
//!
//! `Player_Die` (`$21A94`) enters state 41 on foot, 54 on the horse or 62 on
//! the surfboard at cursor 0; the pit enters 43 (`$1FD84`), a lethal slam 46
//! at the thrown cursor (`$20418`) and Mijin without a spare life 42 or 30
//! (`$20746`). States 42 and 54 run `PlayerState_DeathHidden` (`$204E0`),
//! which only hides the sprite first. On every update the handler:
//!
//! - sets `Sys_Flags2` bits 5-6, so `$4A4E` effects and `$4A66` voices are
//!   dropped; the world keeps running;
//! - sets `Game_Flags` bit 1, which makes `Game_CheckPause` ignore Start;
//! - sets `plr_flags` bit 3 (no damage entry and no pit test, `$1FD74`),
//!   `plr_ctrl` bits 9-11 (no jump, attack or ninjutsu) and `plr_hurt` 1;
//! - clears the X velocity and the Ikazuchi shield (`plr_shield`).
//!
//! Then it acts on the animation cursor, before the stepper advances it:
//!
//! | Cursor | Action |
//! | --- | --- |
//! | `$0020` | Music fade `$FC` (`$20594`) |
//! | `$0800` | WABI (music 11) on the last life, else SABI (12), `$20552` |
//! | slot `$2B` | `Game_Flags` bit 0, the lost life `Mode_Game` state `$1C` waits for; the flags above are cleared and `$FC` is queued again (`$20564`) |
//! | slot 5 and later | With a spare life, A, B or C does the same at once (`$2059A`) |
//! | slots 0-4, except pit state 43 | `$800` iterations of two `muls.w` (`$20540`) |
//!
//! The multiply loop is a deliberate slowdown. Genesis Plus GX shows each of
//! those updates take three frames. The native game skips `SLOW_LAG` ticks
//! after such an update instead of burning the time.
//!
//! This module holds the handler's own cursor. Every death sequence has
//! speed 32, so it stays equal to the player animator's cursor on foot, and
//! it also drives the mounted deaths, whose frames `crate::special` shows.
use crate::{movement::state, sound_events::SoundEvent};

/// Cursor increment of every death sequence (`$235F6` speed 32).
pub const SPEED: u16 = 0x20;
/// `$20518`: the music fade.
pub const FADE_CURSOR: u16 = 0x0020;
/// `$20520`: WABI or SABI.
pub const JINGLE_CURSOR: u16 = 0x0800;
/// `$20528`: the life is lost.
pub const LIFE_LOST_SLOT: u16 = 0x2b;
/// `$20530`: from this slot on, no slowdown and the A/B/C skip.
pub const SKIP_SLOT: u16 = 5;
/// Frames a slowed update takes beyond its own (Genesis Plus GX, 1-1 hit
/// death: 38 of the 39 slowed updates took three frames, one took four).
pub const SLOW_LAG: u8 = 2;

/// What one handler call asks of the game.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Update {
    pub sounds: Vec<SoundEvent>,
    /// The multiply loop ran: the game waits `SLOW_LAG` ticks.
    pub slow: bool,
    /// `Game_Flags` bit 0: `Mode_Game` leaves play for state `$20`.
    pub life_lost: bool,
}

/// A running `PlayerState_Death`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Death {
    /// The death state (`crate::movement::state`), 30, 41-43, 46, 54 or 62.
    pub state: usize,
    /// `plr_cursor`, 8.8: the high byte is the sequence slot.
    pub cursor: u16,
}
impl Death {
    pub fn new(state: usize, cursor: u16) -> Self {
        Self { state, cursor }
    }
    /// One player update: the handler at the current cursor, then the
    /// stepper. `lives` is `Lives` (spare lives, 0 on the last one) and
    /// `skip` is A, B or C newly pressed (`act_pressed & $70`).
    pub fn update(&mut self, lives: u8, skip: bool) -> Update {
        let update = self.handle(lives, skip);
        self.cursor = self.cursor.wrapping_add(SPEED);
        update
    }
    fn handle(&self, lives: u8, skip: bool) -> Update {
        let slot = self.cursor >> 8;
        let lost = || Update {
            sounds: vec![SoundEvent::DeathFade],
            slow: false,
            life_lost: true,
        };
        if self.cursor == FADE_CURSOR {
            Update {
                sounds: vec![SoundEvent::DeathFade],
                ..Update::default()
            }
        } else if self.cursor == JINGLE_CURSOR {
            Update {
                sounds: vec![if lives == 0 {
                    SoundEvent::GameOver
                } else {
                    SoundEvent::DeathJingle
                }],
                ..Update::default()
            }
        } else if slot == LIFE_LOST_SLOT {
            lost()
        } else if slot >= SKIP_SLOT {
            if lives != 0 && skip {
                lost()
            } else {
                Update::default()
            }
        } else if self.state == state::PIT_DEATH {
            Update::default()
        } else {
            Update {
                slow: true,
                ..Update::default()
            }
        }
    }
    /// The multiply loop runs in this state's next update.
    pub fn slows(&self) -> bool {
        self.cursor >> 8 < SKIP_SLOT && self.cursor != FADE_CURSOR && self.state != state::PIT_DEATH
    }
    /// Display frames spent beyond a slowed update. The surf death's
    /// retained water/effect work makes the multiply loop average 3.4
    /// frames in the reference capture. Spread its twelve extra frames
    /// over the 32 updates from cursor $100 through $4E0. This models the
    /// measured cadence without emulating the CPU's variable MULS cycles.
    pub fn slow_lag(&self) -> u8 {
        if self.state == state::SURF_DEATH && (0x100..0x500).contains(&self.cursor) {
            let step = (self.cursor - 0x100) / SPEED;
            SLOW_LAG + (((step + 1) * 3 / 8) - (step * 3 / 8)) as u8
        } else {
            SLOW_LAG
        }
    }
}
