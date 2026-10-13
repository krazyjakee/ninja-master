//! Round 5 enemy types (issue #51), ported from their handlers:
//!
//! | Type | Handler | Where | Native model |
//! | --- | --- | --- | --- |
//! | `$28` | `Enemy28` `$160C2` | 5-1 (parts 1, 3) | [`Grenadier`]: crouching soldier lobbing grenades that burst into flames |
//! | `$10` | `Enemy10` `$1383C` | 5-1 (part 3) | [`Commando`]: the mid-round boss; dashes across the screen and fires charged shots |
//!
//! Each routine is the byte offset into the handler's `bra.w` table (or its
//! `jsr (pc,d0.w)` table for `$10`); the comments cite the routine's address.
//! Immediates and tables are read from the ROM at load (each type's
//! `tables_match_rom` test pins them).
//!
//! Native simplifications, all visual or cross-object:
//! - `$10` fades the palettes in on entry (`$6E8`, 4-tick steps) and darkens
//!   palette line 1 as its health drops (`Data_013762`/`Data_013782`); the
//!   native entry waits [`Commando::FADE_STEPS`] fade steps and the hit
//!   flash uses the shared white flash. Its dust trail (`$13C94`, list-1
//!   `$23`) is not drawn; the list-9 afterimage (`$13E54` -> `$13D66`) is a
//!   blinking [`EffectKind::Pieces`] copy of the pose that keeps the dash's
//!   motion.
//! - `$10` freezes the player (`plr_event` 6) through its death sequence;
//!   `Ctx` has no player handle, so the native player keeps control.
//! - `Sys_Flags` bit 3, which `$10` sets while it enters so that every `$28`
//!   frees itself, is passed through the `$10` record's scratch
//!   ([`ENTRANCE_SIGNAL`] in `vars[SIGNAL]`), read from `Ctx::others`.
//! - The `$28` grenade (list-4 `$23`, `$1DACE`) is a native projectile; so
//!   are its three landing flames (`$1D0EA`, list-4 `$03` at `$1D144`),
//!   which sway to both sides of the landing point in turn
//!   ([`crate::combat::Sway`]).
//!
//! [`EffectKind::Pieces`]: crate::combat::EffectKind::Pieces
use super::{Roster, VARS};
use crate::rom::Result;
use std::sync::Arc;

mod commando;
mod grenadier;
use commando::Commando;
use grenadier::Grenadier;

/// Type offsets (`$12E9A` byte offsets) with a handler in this module.
pub(crate) const KINDS: &[u16] = &[0x10, 0x28];

pub(crate) fn register(rom: &[u8], roster: &mut Roster) -> Result<()> {
    roster.add(0x10, Arc::new(Commando::load(rom)?));
    roster.add(0x28, Arc::new(Grenadier::load(rom)?));
    Ok(())
}

/// Native value of `vars[SIGNAL]` while a `$10` holds `Sys_Flags` bit 3
/// (its entry), which frees every `$28` (`$160C2`'s first test).
pub(crate) const ENTRANCE_SIGNAL: i32 = 0x5359_0003;
/// Scratch slot of [`ENTRANCE_SIGNAL`].
const SIGNAL: usize = VARS - 1;
