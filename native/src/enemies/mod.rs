//! Per-type enemy handlers (issues #41, #51).
//!
//! `Objects_RunEnemyList` (`$126C2`) calls `Object_HandlerTable[obj_type]`
//! (`$12E9A`) once per frame for every list-5 record. Natively each type is a
//! [`Behavior`] registered in a [`Roster`] under its type offset; the combat
//! loop calls [`run`] for every [`Enemy`], which handles spawn activation and
//! the shared object-list integration, then the type's own update.
//!
//! File layout, so types can be ported in parallel without touching the same
//! file: a type lives in the module of the first round whose spawn lists use
//! it (`forest` for round 1, then `round2` .. `round7`, one submodule per
//! type or family); bosses and their list-5 parts live under
//! [`crate::bosses`]. Each round module exposes `KINDS` and `register`;
//! adding a type means adding it to its module's `KINDS` and registering a
//! [`Behavior`] there. Nothing else changes. [`Ctx`] (`ctx`) is the handler
//! API and [`Shared`] (`shared`) the ROM tables every type reads.
//!
//! Porting pattern for a ROM-style type (see `round2` for examples):
//! - [`Enemy::routine`] is `obj_routine`, the byte offset into the handler's
//!   `bra.w` slot table (`EnemyTT_Routines`); dispatch with a `match`.
//! - `obj_frame` writes set [`Enemy::mapping`] from the handler's pose tables,
//!   read from the ROM at load time with the table address cited.
//! - Velocities and accelerations go into [`Enemy::motion`] (16.16); the
//!   framework integrates them after the update like `Object_Integrate`
//!   (`$19D4`) and sets [`Enemy::offscreen`] like `Object_UpdateVisibility`
//!   (`$17A4`), unless [`Behavior::integrates`] is false.
//! - `Object_ProcessHit` is [`Ctx::process_hit`] at the top of the update;
//!   [`Behavior::hit`] says when weapons collide at all. The death update
//!   credits the pending score like `Func_0129C0`.
//! - Children: [`Ctx::spawn`] (`Object_AllocL5`), [`Ctx::shoot`]
//!   (`Object_AllocL4` projectiles), the effect helpers ([`Ctx::explode`],
//!   [`Ctx::burst`], [`Ctx::spark`]); sounds through [`Ctx::effect_sound`]
//!   (`$4A4E`) and [`Ctx::visible_sound`] (`$4B2E`).
//! - Freeing the object (`Func_0129B8`) is [`Ctx::free`].
//!
//! Types without a registered behavior are "unported": they stand still in
//! the first-`obj_frame` placeholder pose and `Combat::draw` outlines them in
//! magenta, so a guessed sprite is never mistaken for a recovered one.
use crate::{
    combat::{Enemy, EnemyState, Hitbox, Mapping, Motion, Reaction, Spawn},
    game::Terrain,
    rom::Result,
};
use std::{collections::BTreeMap, sync::Arc};

mod ctx;
mod shared;

pub(crate) use ctx::{Ctx, aim, explode, next_random, small_blast};
pub(crate) use shared::{
    Offsets, Shared, Wreckage, frame_at, frame_list, metatile_list, set_fraction, set_whole,
    shot_damage, stop,
};

pub(crate) mod forest;
pub(crate) mod round2;
pub(crate) mod round3;
pub(crate) mod round4;
pub(crate) mod round5;
pub(crate) mod round6;
pub(crate) mod round7;

pub(crate) const LIST5_CAPACITY: usize = 12;
/// List 4 (enemy projectiles) holds 8 records (dbf 7).
pub(crate) const LIST4_CAPACITY: usize = 8;
/// Whether list 4, limited to `cap` records, has a free enemy-shot record.
fn list4_has_space(projectiles: &[crate::combat::Projectile], cap: usize) -> bool {
    projectiles.iter().filter(|shot| shot.enemy).count() < cap
}
/// Handler scratch words kept per object (`obj_var*`, `$2A..$4E`).
pub(crate) const VARS: usize = 20;

/// Whether `kind` (a `$12E9A` byte offset) has a native handler.
pub fn ported(kind: u16) -> bool {
    [
        forest::KINDS,
        round2::KINDS,
        round3::KINDS,
        round4::KINDS,
        round5::KINDS,
        round6::KINDS,
        round7::KINDS,
    ]
    .iter()
    .any(|kinds| kinds.contains(&kind))
        || crate::bosses::part_kinds().any(|part| part == kind)
}

/// How a weapon contact is treated (`Object_ProcessHit` `$127C0` and the
/// handler's own gates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitResponse {
    /// The handler does not call `Object_ProcessHit` now: weapons pass
    /// through and the body does not touch the player.
    Pass,
    /// The weapon collides and is spent, but no damage is taken (for
    /// example during a knockback arc).
    Ignore,
    /// `$368EC`: the weapon is deflected with its sound and spark.
    Deflect,
    /// Damage is subtracted from the health counter.
    Accept,
    /// `Func_0128B2`: weapons damage the object, but its body does not
    /// collide with the player.
    WeaponsOnly,
    /// `Func_012880`: the body collides with the player, while weapons pass
    /// through without being spent.
    BodyOnly,
    /// `Func_0128E4`: ordinary damage and body contact, but no lethal spark.
    NoDeathSpark,
    /// Boss2's field (`$2B126`): it clears the weapon's collision bit, so
    /// the weapon is neither spent nor heard and nothing is damaged. A
    /// shuriken not yet turned is sent into `Func_024BE0` ([`Turn`]); the
    /// sword passes on to the next object.
    ///
    /// [`Turn`]: crate::combat::Turn
    Reflect,
    /// `Object_CollideL3` alone, as a list-4 object runs it (the 3-3 spit
    /// balls, `loc_02CB2E`): the weapon collides and is spent, no damage is
    /// taken, and the body does not touch the player (the handler's own
    /// shot does). The handler reads the collision from `Enemy::struck`.
    Collides,
}
impl HitResponse {
    pub(crate) fn weapon_collides(self) -> bool {
        !matches!(self, Self::Pass | Self::BodyOnly)
    }
    pub(crate) fn body_collides(self) -> bool {
        !matches!(self, Self::Pass | Self::WeaponsOnly | Self::Collides)
    }
    pub(crate) fn death_spark(self) -> bool {
        self != Self::NoDeathSpark
    }
}

/// One object type's handler. `kind` is passed where one behavior serves
/// several types (the forest set, `$B4`/`$B8`).
pub(crate) trait Behavior: Send + Sync {
    /// Handler label and address, for reports.
    fn label(&self, kind: u16) -> String;
    /// `obj_frame` of a fresh record, drawn before its first update.
    fn initial_mapping(&self, _kind: u16) -> u16 {
        0
    }
    /// A stage spawn record activated (`Object_SpawnFromRecord`). The common
    /// fields are already reset; `routine` is 0.
    fn activate(&self, _cx: &mut Ctx, enemy: &mut Enemy, _spawn: &Spawn) {
        enemy.state = EnemyState::Waiting;
    }
    /// One handler call while the object is alive.
    fn update(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain);
    /// Future scheduled items, extrapolated for the wider viewport without
    /// allocating objects or advancing their collision/lifetime clocks.
    fn approaching_drops(
        &self,
        _enemy: &Enemy,
        _spawn: &Spawn,
        _camera_fixed: i32,
        _step: i32,
    ) -> Vec<crate::stage_objects::Drop> {
        Vec::new()
    }
    /// One handler call after the lethal hit, while `Enemy::dying()`.
    /// ROM-style handlers keep dispatching their routines (the death routine
    /// was entered by [`Ctx::process_hit`]).
    fn dying(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        self.update(cx, enemy, spawn, terrain);
    }
    /// Weapon contact now. The default follows `Object_ProcessHit`: nothing
    /// collides while `obj_routine` is 0.
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        if enemy.routine == 0 {
            HitResponse::Pass
        } else {
            HitResponse::Accept
        }
    }
    /// Attack-specific damage gate; most handlers use the same gate for
    /// weapons and ninjutsu. Boss1 tests eight-point magic before its guard.
    fn hit_by(&self, enemy: &Enemy, kind: u16, _damage: u8) -> HitResponse {
        self.hit(enemy, kind)
    }
    /// Boss counters can interpret a ninjutsu object differently from its
    /// ordinary damage byte (Func_032F00 returns 8 for types >= $50).
    fn ninjutsu_damage(&self, _kind: u16, damage: u8) -> u8 {
        damage
    }
    /// Apply `Object_Integrate` after each update. The forest set steps its
    /// own motion and returns false.
    fn integrates(&self) -> bool {
        true
    }
    /// A documented native stand-in (not the recovered handler): drawn with
    /// the unported outline.
    fn stand_in(&self) -> bool {
        false
    }
    /// The death routine credits the type's score (`Func_0129C0`). `$94`
    /// frees itself without it.
    fn scores(&self) -> bool {
        true
    }
    /// Body contact writes damage code `$2F` to the player (`obj_damage`)
    /// instead of `Object_ProcessHit`'s 1, so the hit shocks (state 47).
    fn shocks(&self, _kind: u16) -> bool {
        false
    }
    /// The body collides with the player when [`Behavior::hit`] lets it.
    /// False for records that only test weapons (`Object_CollideL3`) and
    /// leave player contact to their own boxes.
    fn touches_player(&self) -> bool {
        true
    }
}

/// Registered behaviors of one stage part, plus its boss.
#[derive(Clone)]
pub(crate) struct Roster {
    types: Vec<Option<Arc<dyn Behavior>>>,
    boss: Option<Arc<dyn Behavior>>,
}
impl Roster {
    pub(crate) fn load(rom: &[u8], stage: crate::stage::StageId) -> Result<Self> {
        let mut roster = Self {
            types: vec![None; 64],
            boss: None,
        };
        forest::register(rom, &mut roster)?;
        round2::register(rom, &mut roster)?;
        round3::register(rom, &mut roster)?;
        round4::register(rom, &mut roster)?;
        round5::register(rom, &mut roster)?;
        round6::register(rom, &mut roster)?;
        round7::register(rom, &mut roster)?;
        crate::bosses::register_parts(rom, &mut roster)?;
        if crate::bosses::record(rom, stage)?.is_some() {
            roster.boss = Some(crate::bosses::fight(rom, stage.round_index())?);
        }
        Ok(roster)
    }
    /// Register `behavior` for the type offset `kind`.
    pub(crate) fn add(&mut self, kind: u16, behavior: Arc<dyn Behavior>) {
        debug_assert!(ported(kind), "${kind:02x} missing from its module's KINDS");
        if let Some(slot) = self.types.get_mut(usize::from(kind / 4)) {
            *slot = Some(behavior);
        }
    }
    pub(crate) fn get(&self, kind: u16) -> Option<&dyn Behavior> {
        if kind == crate::bosses::BOSS_KIND {
            return self.boss.as_deref();
        }
        if !kind.is_multiple_of(4) {
            return None;
        }
        self.types
            .get(usize::from(kind / 4))
            .and_then(|behavior| behavior.as_deref())
    }
    /// `kind`'s body can collide with the player (`Behavior::touches_player`).
    pub(crate) fn touches_player(&self, kind: u16) -> bool {
        self.get(kind)
            .is_none_or(|behavior| behavior.touches_player())
    }
    /// Contact with `kind` shocks the player (`Behavior::shocks`).
    pub(crate) fn shocks(&self, kind: u16) -> bool {
        self.get(kind).is_some_and(|behavior| behavior.shocks(kind))
    }
    /// Drawn with the unported outline: no handler, or a stand-in.
    pub(crate) fn marked(&self, kind: u16) -> bool {
        self.get(kind).is_none_or(|behavior| behavior.stand_in())
    }
    pub(crate) fn initial_mapping(&self, spawn: &Spawn) -> u16 {
        self.get(spawn.kind)
            .map_or(spawn.placeholder_mapping, |b| b.initial_mapping(spawn.kind))
    }
    pub(crate) fn hit(&self, kind: u16, enemy: &Enemy) -> HitResponse {
        self.hit_by(kind, enemy, 0)
    }
    pub(crate) fn ninjutsu_damage(&self, kind: u16, damage: u8) -> u8 {
        self.get(kind)
            .map_or(damage, |behavior| behavior.ninjutsu_damage(kind, damage))
    }
    pub(crate) fn hit_by(&self, kind: u16, enemy: &Enemy, damage: u8) -> HitResponse {
        let response = match self.get(kind) {
            Some(behavior) => behavior.hit_by(enemy, kind, damage),
            // Unported: a hit pauses the placeholder; further hits are spent.
            None if enemy.state == EnemyState::Hurt => HitResponse::Ignore,
            None => HitResponse::Accept,
        };
        // The types whose handlers call the `Object_ProcessHit` variants.
        match (kind, response) {
            (0x0c | 0x60 | 0x68 | 0x70 | 0xdc | 0xf0, HitResponse::Accept) => {
                HitResponse::WeaponsOnly
            }
            (0x8c | 0x90, response) if response != HitResponse::Pass => HitResponse::BodyOnly,
            (0x94 | 0xec, HitResponse::Accept) => HitResponse::NoDeathSpark,
            _ => response,
        }
    }
    pub(crate) fn label(&self, kind: u16) -> String {
        self.get(kind)
            .map_or_else(|| "unported".to_string(), |b| b.label(kind))
    }
}

/// Position and scratch of another object, captured before the update pass
/// (a child reading its parent, e.g. `$94` testing its `$3C` parent's `$34`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) live: bool,
    pub(crate) facing_left: bool,
    pub(crate) high_priority: bool,
    pub(crate) vars: [i32; VARS],
}
impl Snapshot {
    pub(crate) fn of(enemy: &Enemy) -> Self {
        Self {
            x: enemy.x,
            y: enemy.y,
            live: enemy.live(),
            facing_left: enemy.facing_left,
            high_priority: enemy.high_priority,
            vars: enemy.vars,
        }
    }
}

/// A list-5 object allocated during the update pass; `Combat` links it at
/// the tail of the list and runs it later in the same pass.
pub(crate) struct Child {
    pub(crate) kind: u16,
    pub(crate) enemy: Enemy,
}

/// Ordinary object terrain view. `$3854` chooses its signed-result mask from
/// `obj_tile` bit 15, while material queries still return the raw code.
struct PriorityTerrain<'a> {
    terrain: &'a dyn Terrain,
    high_priority: bool,
}
impl Terrain for PriorityTerrain<'_> {
    fn width(&self) -> i32 {
        self.terrain.width()
    }
    fn height(&self) -> i32 {
        self.terrain.height()
    }
    fn solid_at(&self, x: i32, y: i32) -> bool {
        self.terrain.object_solid_at(x, y, self.high_priority)
    }
    fn material_at(&self, x: i32, y: i32) -> crate::movement::Material {
        self.terrain.material_at(x, y)
    }
}

/// One `Objects_RunEnemyList` step for `enemy`: activation of a dormant
/// spawn record, then the type's update (or death sequence), then
/// `Object_Integrate` and `Object_UpdateVisibility` for ROM-style types.
pub(crate) fn run(
    roster: &Roster,
    cx: &mut Ctx,
    enemy: &mut Enemy,
    spawn: &Spawn,
    ticks: u64,
    terrain: &dyn Terrain,
) {
    let behavior = roster.get(spawn.kind);
    cx.spawn_flags = spawn.flags;
    if enemy.state == EnemyState::Dormant {
        // `Object_SetTileBase` before the handler's own tile writes.
        enemy.high_priority = spawn_priority(spawn.flags) != 0;
    }
    let terrain = &PriorityTerrain {
        terrain,
        high_priority: enemy.high_priority,
    };
    match enemy.state {
        EnemyState::Dormant => {
            // The forest leader places itself from the camera and starts
            // the arena's gate/rock timers. Wider visibility must not bring
            // that scripted entrance forward or change its world position.
            let active = if spawn.kind == 0xb8 {
                spawn.active_at(cx.camera, ticks)
            } else {
                spawn.active_in_view(cx.camera, ticks, cx.view)
            };
            if !active {
                return;
            }
            // Object_AllocL5 failure leaves the spawn record unused so the
            // proximity scan retries it on a later update.
            if cx.live + cx.children.len() >= LIST5_CAPACITY {
                return;
            }
            // Object_SpawnFromRecord ($12718): routine, timer and scratch cleared.
            enemy.home_x = enemy.x;
            enemy.motion = Motion::default();
            enemy.timer = 0;
            enemy.counter = 0;
            enemy.phase = 0;
            enemy.vulnerable = false;
            enemy.routine = 0;
            enemy.vars = [0; VARS];
            match behavior {
                Some(behavior) => behavior.activate(cx, enemy, spawn),
                None => {
                    enemy.mapping = spawn.placeholder_mapping;
                    enemy.state = EnemyState::Waiting;
                }
            }
            if enemy.live() {
                cx.live += 1;
            }
        }
        EnemyState::Defeated => {
            if enemy.dying {
                if enemy.reaction == Some(Reaction::Death) && behavior.is_none_or(|b| b.scores()) {
                    cx.award_pending_score(enemy);
                }
                enemy.age += 1;
                enemy.flash = enemy.flash.saturating_sub(1);
                match behavior {
                    Some(behavior) => behavior.dying(cx, enemy, spawn, terrain),
                    None => unported_dying(cx, enemy),
                }
                enemy.damage_taken = 0;
                integrate(behavior, cx, enemy);
            }
            return;
        }
        _ => {}
    }
    enemy.age += 1;
    enemy.flash = enemy.flash.saturating_sub(1);
    match behavior {
        Some(behavior) => behavior.update(cx, enemy, spawn, terrain),
        None => unported_update(cx, enemy),
    }
    enemy.damage_taken = 0;
    integrate(behavior, cx, enemy);
    if let Some(tile) = enemy.tile {
        enemy.high_priority = tile_priority(tile);
    }
}

/// `obj_tile` bit 15: the object is on the high sprite-priority layer.
pub(crate) const fn tile_priority(tile: u16) -> bool {
    tile & 0x8000 != 0
}

/// `Object_SetTileBase`'s priority: spawn flag bit 14 lifted into `obj_tile`
/// bit 15 (`asl.w #1` / `andi.w #$8000`).
pub(crate) const fn spawn_priority(flags: u16) -> u16 {
    (flags << 1) & 0x8000
}

fn integrate(behavior: Option<&dyn Behavior>, cx: &Ctx, enemy: &mut Enemy) {
    if behavior.is_some_and(|b| b.integrates()) && enemy.live() {
        enemy.motion.step(&mut enemy.x, &mut enemy.y);
        enemy.offscreen = cx.outside_view(enemy.x, enemy.y);
    }
}

/// Types without a handler stand at their spawn point in their placeholder
/// pose; a hit pauses them briefly (native only).
fn unported_update(cx: &mut Ctx, enemy: &mut Enemy) {
    match enemy.reaction.take() {
        Some(Reaction::Hurt) => {
            cx.spark(enemy, false);
            enemy.flash = 8;
            enemy.timer = 16;
            enemy.state = EnemyState::Hurt;
            return;
        }
        Some(Reaction::Deflect) => {
            let sound = cx.shared.deflect_sound;
            cx.play(sound);
            cx.spark(enemy, false);
        }
        _ => {}
    }
    if enemy.state == EnemyState::Hurt {
        enemy.timer = enemy.timer.saturating_sub(1);
        if enemy.timer == 0 {
            enemy.state = EnemyState::Waiting;
        }
    }
}
/// Unported death: the shared ordinary blast, then removal.
fn unported_dying(cx: &mut Ctx, enemy: &mut Enemy) {
    if enemy.reaction.take() == Some(Reaction::Death) {
        enemy.phase = 0;
        enemy.counter = 0;
        enemy.timer = 0;
        if enemy.death_spark {
            cx.spark(enemy, true);
        }
        let blast = cx.shared.blasts[0];
        cx.explode(enemy.x, enemy.y, blast);
        enemy.mapping = 0;
    }
    Ctx::remove(enemy);
}

/// Native outline marking an unported type or a stand-in: its hitbox, or a
/// 32x48 box above the feet when the pose has none.
pub(crate) fn marker_box(mappings: &BTreeMap<u16, Mapping>, enemy: &Enemy) -> Hitbox {
    let hitbox = crate::combat::enemy_hitbox(mappings, enemy);
    if hitbox.half_width > 0 && hitbox.half_height > 0 {
        hitbox
    } else {
        Hitbox {
            x: enemy.x,
            y: enemy.y - 24,
            half_width: 16,
            half_height: 24,
        }
    }
}
