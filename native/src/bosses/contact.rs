//! Contact helpers shared by boss handlers.
use crate::{
    combat::{Hitbox, Motion, Projectile, mapping_hitbox},
    enemies::Ctx,
};

/// Player body box (mapping 1) at the player's feet.
pub(super) fn player_box(cx: &Ctx) -> Hitbox {
    mapping_hitbox(cx.mappings, 1, cx.player.0, cx.player.1, false)
}

/// Native offset of an unguardable [`strike`] below the feet, and of a
/// guardable one above them (see `Combat::check_player_damage`'s guard window).
const UNGUARDED_Y: i32 = 40;
const GUARDED_Y: i32 = 10;

/// `move.b #n,obj_health(a3)` after `Object_CollidePlayer`: `n` damage to the
/// player this tick. `Ctx` has no player-damage entry, so this queues a
/// one-tick list-4 record overlapping the player's body box (mapping 1,
/// removed before drawing). `guard_from` = X of a weapon the guard may
/// block (`Func_01E898` `$2A` = 1); `None` sets the player's collided bit
/// like the handlers, which `Player_CheckDamage` never guards.
pub(super) fn strike(cx: &mut Ctx, damage: u8, guard_from: Option<i32>) {
    let (px, py) = cx.player;
    let (x, y) = match guard_from {
        Some(from) => (px + if from < px { -1 } else { 1 }, py - GUARDED_Y),
        None => (px, py + UNGUARDED_Y),
    };
    let mut shot = Projectile::new(x, y, 0, true, 1, damage).with_motion(Motion {
        vx: 1,
        ..Motion::default()
    });
    shot.life = 1;
    shot.passes_terrain = true;
    shot.body_contact = guard_from.is_none();
    // Not a list-4 record of the original: bypass `Ctx::shoot`'s limit.
    cx.push_projectile_unchecked(shot);
}
