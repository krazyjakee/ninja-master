//! `Func_037244`: a body part that shoves the player out of itself.
//!
//! A part calls it with its own box (`Object_LoadHitbox`). When the box
//! overlaps the player's (`Func_0373E6`, which tests neither the player's
//! collision flags nor the priority layer), it frees the list-1 pusher
//! (type `$27`) it finds and allocates a new one, so every overlapping
//! frame re-arms it. The pusher (`Func_0372C4`) runs in the list-1 pass of
//! the same frame: it picks a side once, then writes `plr_push` =
//! ±`$34`/16 px each update while `$34` counts down from `$30`, and
//! `Player_Physics` adds that to the X velocity on the next player update.
//! Genesis Plus GX, 3-3 against the Hydra: `plr_push` `$FFFD1000` on every
//! overlapping frame, then `$FFFD2000`.. as the pusher decays.
//!
//! Callers: the 1-1 Samurai Leader (`$1BB48..$1C6EE`) and the Hydra's face,
//! neck and chin (`loc_02C57C`, `loc_02C5E2`, `.loc_02C61A`).
//!
//! Not modelled: routine `$C` (`.loc_037356`). In a boss arena
//! (`Stage_InhibitFlags` bit 0) a player the pusher finds at screen X below
//! 17 or from 305 is clamped to camera X + 16..=304 (`Func_037370`) and
//! given a `$40`-damage contact hit. The arena's `Func_00342E` clamp keeps
//! him within + 32..=288 every frame, so the pusher never sees him there.
use super::Hitbox;
use crate::movement::NATIVE_UNIT;

/// `$372F0`: the countdown each new pusher starts from.
const TICKS: i32 = 0x30;

/// The live list-1 `$27` pusher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shove {
    /// The part box's centre X (its `$20` plus the copied `$10` offset).
    centre_x: i32,
    /// `$34`.
    ticks: i32,
    /// Routine 4 (right) or 8 (left); `None` before the first update.
    right: Option<bool>,
}
impl Shove {
    /// `Func_037244` after `Func_0373E6` found the overlap.
    pub(crate) fn new(part: Hitbox) -> Self {
        Self {
            centre_x: part.x,
            ticks: TICKS,
            right: None,
        }
    }
    /// One `Func_0372C4` update with the player's X: the `plr_push` it
    /// writes, in native units per tick, or `None` once it frees itself.
    pub(crate) fn tick(&mut self, player_x: i32) -> Option<i32> {
        // .loc_0372E0: right when the centre is left of the player's X
        // (`bcs`), then straight into the routine chosen.
        let right = *self.right.get_or_insert_with(|| {
            self.ticks = TICKS;
            self.centre_x < player_x
        });
        // loc_0373A2 / .loc_0373B8: free at zero, else (`$34` << 16) >> 4.
        self.ticks -= 1;
        if self.ticks == 0 {
            return None;
        }
        let push = self.ticks * NATIVE_UNIT / 16;
        Some(if right { push } else { -push })
    }
}
