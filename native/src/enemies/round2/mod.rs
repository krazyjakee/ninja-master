//! Round 2 enemy types (issue #51), ported from their handlers:
//!
//! | Type | Handler | Where | Native model |
//! | --- | --- | --- | --- |
//! | `$04` | `Enemy04` `$12FD8` | 2-2, 5-1, 5-2 | [`Soldier`]: walks, stops to shoot or crouches under a shot |
//! | `$08` | `Enemy08` `$132BC` | 2-1 | [`Spawner`]: invisible rider spawner and its list-9 carrier |
//! | `$0C` | `Enemy0C` `$13466` | 2-1 (from `$08`) | [`Rider`]: horseman passes, jumps and throws aimed stars |
//! | `$1C` | `Enemy1C` `$15308` | 2-1 | [`Guard`]: the end guard, riding in on a kite, leaping with spears and diving with a kick |
//! | `$94` | `Enemy94` `$15B60` | 2-1 (from `$1C`) | [`Spear`]: falls point first, sticks and slides with the ground |
//! | `$2C` | `Enemy2C` `$16358` | 2-2, 5-1, 5-2 | [`Gunner`]: stationary three-shot bursts |
//! | `$B0` | `EnemyB0` `$19284` | 2-2 | [`WallGun`]: slides out of the wall and fires |
//! | `$E0` | `EnemyE0` `$1A54C` | 2-2, 5-1 | [`Kusarigama`]: chain throws, close slash, deflects shurikens |
//! | `$40` | `Enemy40` `$3616A` | 2-1, 4-1 | [`PickupScheduler`]: drops list-7 items as the camera advances |
//!
//! Each routine is the byte offset into the handler's `bra.w` table; the
//! comments cite the routine's address. Immediates and tables are read from
//! the ROM at load (each type's `tables_match_rom` test pins them). Simplifications:
//! list-1 dust/puffs of the wreckage are not drawn; hits spark natively at
//! the body. The list-9 kite, its `$16E` streaks and the guard's afterimage
//! and hurt images are drawn in the list-9 pass (`SpriteList::Back`).
use super::{Ctx, Offsets, Roster};
use crate::{
    combat::{Enemy, Projectile},
    rom::Result,
};
use std::sync::Arc;

mod guard;
mod gunner;
mod kusarigama;
mod pickups;
mod riders;
mod soldier;
mod wall_gun;
use guard::{Guard, Spear};
use gunner::Gunner;
use kusarigama::Kusarigama;
use pickups::PickupScheduler;
use riders::{Rider, Spawner};
use soldier::Soldier;
use wall_gun::WallGun;

pub(crate) const KINDS: &[u16] = &[0x04, 0x08, 0x0c, 0x1c, 0x2c, 0x40, 0x94, 0xb0, 0xe0];

pub(crate) fn register(rom: &[u8], roster: &mut Roster) -> Result<()> {
    roster.add(0x04, Arc::new(Soldier::load(rom)?));
    roster.add(0x08, Arc::new(Spawner::load(rom)?));
    roster.add(0x0c, Arc::new(Rider::load(rom)?));
    roster.add(0x1c, Arc::new(Guard::load(rom)?));
    roster.add(0x94, Arc::new(Spear::load(rom)?));
    roster.add(0x2c, Arc::new(Gunner::load(rom)?));
    roster.add(0x40, Arc::new(PickupScheduler::load(rom)?));
    roster.add(0xb0, Arc::new(WallGun::load(rom)?));
    roster.add(0xe0, Arc::new(Kusarigama::load(rom)?));
    Ok(())
}

/// A list-4 shot placed by `Func_01D480` (or a word offset table), with its
/// casing: offsets, 16.16 speed, mapping, damage (`$1FBF6 + ID`) and sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Gun {
    offsets: Offsets,
    casing: Offsets,
    casing_kind: usize,
    speed: i32,
    mapping: u16,
    damage: u8,
    sound: u8,
}
impl Gun {
    /// Fire from `enemy`; `posture` is `obj_flags` bit 1 (crouching).
    /// Returns false when list 4 is full.
    fn fire(&self, cx: &mut Ctx, enemy: &Enemy, posture: bool) -> bool {
        let (x, y) = self
            .offsets
            .place(enemy.x, enemy.y, enemy.facing_left, posture);
        let speed = self.speed >> 16;
        // `$13260` leaves `obj_frame` 0; list-4 `$00` (`$1CD64`) and `$24`
        // (`$1DB7A`) set the frame after their first `Object_LoadHitbox`.
        let shot = Projectile::new(
            x,
            y,
            if enemy.facing_left { -speed } else { speed },
            true,
            self.mapping,
            self.damage,
        )
        .unset_frame();
        if !cx.shoot(shot) {
            return false;
        }
        let (x, y) = self
            .casing
            .place(enemy.x, enemy.y, enemy.facing_left, posture);
        cx.casing(x, y, enemy.facing_left, self.casing_kind);
        cx.visible_sound(enemy.x, enemy.y, self.sound);
        true
    }
}
