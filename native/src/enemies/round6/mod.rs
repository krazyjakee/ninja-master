//! Round 6 enemy types (issue #51), ported from their handlers:
//!
//! | Type | Handler | Where | Native model |
//! | --- | --- | --- | --- |
//! | `$8C` | `Enemy8C` `$17E02` | 1-2 (parts 1, 2), 6-2 (parts 1, 7) | [`Spike`]: a spike that thrusts up out of the floor on a timer |
//! | `$90` | `Enemy90` `$17F28` | no spawn record or spawner | [`Spike`]: the same, triggered by the player passing within 56 px |
//! | `$9C` | `Enemy9C` `$185B8` | 6-2 (parts 1, 4) | [`Runner`]: a ninja running in from a screen edge to slash or leap at the player |
//! | `$C8` | `EnemyC8` `$19B0A` | 6-2 (part 7) | [`DartTrap`]: an invisible launcher firing darts along the wall |
//! | `$F8` | `EnemyF8` `$34BEE` | 6-1 (part 1) | [`Karura`]: the mid-round boss; a swirl of feathers that forms a bird-man and swoops |
//!
//! `$8C` is also used by round 1 (1-2); it lives here by assignment, next
//! to its twin `$90` (identical code apart from the trigger). `$F8`'s
//! handler lives next to the boss code (`$34BEE`).
//!
//! Each routine is the byte offset into the handler's `bra.w` table; the
//! comments cite the routine's address. Immediates and tables are read from
//! the ROM at load (each type's `tables_match_rom` test pins them).
//!
//! Native simplifications:
//! - `$8C`/`$90` call `Func_012880`, so weapons pass through and only their
//!   bodies collide with the player. Spawn flag bit 0 (a vertically flipped,
//!   hanging spike) is ported, but no spawn record sets it.
//! - The `$C8` dart (list-4 `$2D`, `$19B80`) waits visibly at the launcher,
//!   then flies; on wall or player contact it rebounds at quarter speed and
//!   drops harmlessly for sixteen updates (`$19C2E`).
//! - `$9C` skips its terrain probes in 2-2 (`Round_Index` 1, `Substage_Index`
//!   1), where it is never spawned; the native port always probes. Its
//!   `obj_flags` bit 2 landing test (a hit or the body touching the player)
//!   is read from the player's body box.
use super::{Ctx, Roster};
use crate::{
    combat::{Enemy, enemy_hitbox, mapping_hitbox},
    rom::Result,
};
use std::sync::Arc;

mod dart_trap;
mod karura;
mod karura_fx;
mod runner;
mod spike;
use dart_trap::DartTrap;
use karura::Karura;
pub(super) use karura_fx::Rules as KaruraEffects;
use runner::Runner;
use spike::Spike;

/// Type offsets (`$12E9A` byte offsets) with a handler in this module.
pub(crate) const KINDS: &[u16] = &[0x8c, 0x90, 0x9c, 0xc8, 0xf8];

pub(crate) fn register(rom: &[u8], roster: &mut Roster) -> Result<()> {
    roster.add(0x8c, Arc::new(Spike::load(rom, 0x17e02)?));
    roster.add(0x90, Arc::new(Spike::load(rom, 0x17f28)?));
    roster.add(0x9c, Arc::new(Runner::load(rom)?));
    roster.add(0xc8, Arc::new(DartTrap::load(rom)?));
    roster.add(0xf8, Arc::new(Karura::load(rom)?));
    Ok(())
}

/// The player's body box (mapping 1) touches the object: the
/// `Object_CollidePlayer` of `Object_ProcessHit` (`$1285C`), which sets
/// `obj_flags` bit 2. `Object_CollideList`'s strict test, where an empty
/// box is a point.
fn touches_player(cx: &Ctx, e: &Enemy) -> bool {
    let body = mapping_hitbox(cx.mappings, 1, cx.player.0, cx.player.1, false);
    enemy_hitbox(cx.mappings, e).collides(body)
}
