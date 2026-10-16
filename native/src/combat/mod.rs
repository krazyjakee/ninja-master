//! Recovered spawn records and deterministic native combat.
//! Per-type enemy handlers live in `crate::enemies`, bosses in `crate::bosses`.
//! Spawn records are in `spawns`, sprite mappings in `mapping`, the shared
//! object model in `objects`, hits on the player in `hits` and the player's
//! weapons in `weapons`.
pub(crate) mod ambient;
mod draw;
mod enemy_updates;
#[cfg(test)]
mod fidelity_tests;
mod hits;
mod loading;
mod mapping;
mod objects;
mod projectiles;
mod shove;
mod spawns;

mod weapons;

pub(crate) use hits::CombatEvents;
pub use hits::{
    CramReload, HitSource, KNOCKDOWN_IMMUNITY, MOUNTED_HURT_IMMUNITY, PaletteOp, Plane, PlaneOp,
    PlayerHit, Signal, THROWN_IMMUNITY, hit_immunity,
};
pub use mapping::Hitbox;
pub(crate) use mapping::{
    Mapping, draw_pieces, enemy_hitbox, mapping_hitbox, object_visible, pose_hitbox,
};
pub(crate) use objects::{
    Blast, Launch, MechShot, Motion, Reaction, ShotBox, Strike, Sway, Turn, advance,
    in_sound_window, outside_view_in,
};
pub use objects::{Effect, EffectKind, Enemy, EnemyState, Projectile, ShotDown};
pub(crate) use spawns::placeholder_mapping;
pub use spawns::{Spawn, boss_arena, spawn_records, stage_spawns};
pub(crate) use weapons::{AttackObject, PlayerAnchors, PlayerAttack};
pub use weapons::{PlayerWeapons, SpentShuriken};
use weapons::{ShurikenTerrain, WallHit};

#[derive(Clone, Copy, Debug)]
pub(crate) enum PartTarget {
    Forward(usize),
    Deflect,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WeaponContact {
    pub(crate) area: Hitbox,
    pub(crate) target: PartTarget,
    pub(crate) high_priority: bool,
}

use spawns::object_selection;

use crate::{
    assets::musashi,
    enemies::{self, Ctx, HitResponse, Roster, Shared, Snapshot},
    game::Terrain,
    movement::{self, PlayerEvent},
    render::{Frame, Lines, Palette, PaletteRef, Tile, md_color},
    rom::{Result, pointer, word},
    sound_events::SoundEvent,
};
use std::collections::BTreeMap;
use weapons::AttackTarget;

const PLAYER_FRAME_TABLE: usize = 0x23ade;
const PLAYER_FRAME_COUNT: usize = (0x23e06 - PLAYER_FRAME_TABLE) / 4;
/// `Spawn::source` of a list-5 object allocated by another object.
pub(crate) const CHILD_SOURCE: usize = 0;
/// Native marker colour of unported types and stand-ins (issue #51).
pub const UNPORTED_OUTLINE: [u8; 3] = [255, 0, 255];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hud {
    pub health: u8,
    pub max_health: u8,
    /// Spare lives, matching $FF37E0. Zero still permits the current life.
    pub lives: u8,
    pub ammo: u16,
    /// Real decimal score (`Score`).
    pub score: u32,
    /// Score currently shown by the HUD (`Score_Display`).
    pub score_display: u32,
    pub timer_seconds: u32,
}
impl Hud {
    /// `$124DE`: advance the displayed score by ten once per play frame.
    pub fn update_score_display(&mut self) {
        if self.score_display < self.score {
            self.score_display = self.score_display.saturating_add(10).min(self.score);
        }
    }
}

/// The object pass's results for the player: the last `plr_event` write,
/// the air stab's bounce (player `+$40`) and the body contact.
#[derive(Clone, Copy, Default)]
struct Carried {
    player_event: Option<PlayerEvent>,
    bounce: bool,
    contact: Option<PlayerHit>,
    /// `Player_ForceSurfJump`'s drift ([`CombatEvents::surf_jump`]).
    surf_jump: Option<i32>,
}

#[derive(Clone)]
pub struct Combat {
    pub(crate) journal: crate::profile::CombatJournal,
    /// Expanded display width and playable world span for object lifetimes.
    pub(crate) view_width: usize,
    pub(crate) world_bounds: (i32, i32),
    /// A boss handler requested the original screen clamp in this part.
    /// Loading new combat resets this independently of player progression.
    pub(crate) view_screen_clamped: bool,
    /// Weapon contacts of composite parts, rebuilt by their owner each pass.
    pub(crate) weapon_contacts: Vec<WeaponContact>,
    pub spawns: Vec<Spawn>,
    pub enemies: Vec<Enemy>,
    pub projectiles: Vec<Projectile>,
    /// Enemy hit/death effects from the `$16F0` list.
    pub effects: Vec<Effect>,
    pub hud: Hud,
    /// `$6174`, the starting shuriken count `$11870` restores on a respawn.
    initial_ammo: u16,
    pub invulnerability: u16,
    /// `Cheat_Invincible == 'R'`: hits take the shield-absorb path without
    /// requiring or consuming an Ikazuchi charge.
    invincible: bool,
    /// Player `obj_tile` bit 15. `Player_Init` sets it from runtime flags
    /// bit 4 (`$22C4C`, the part's mode bit 8: 4-1, 6-1, 7-1) and a boss
    /// handler ORs it in (`Boss5` `$30654` `ori.w #$8000,8(a3)` every
    /// frame); nothing lowers it until `Player_Init` runs with a new part.
    player_priority_raised: bool,
    pub ticks: u64,
    /// `Camera_Mode >= 8`: the stage entrance camera lock has settled.
    pub(crate) camera_locked: bool,
    pub(crate) mech_hand_contact: bool,
    /// Last [`Enemy::order`] handed out.
    allocations: u64,
    /// Recycled list-4 +$28 words, most recently freed first ($18BC).
    /// The native pool does not reproduce cross-list stale RAM at stage entry.
    projectile_scratch: Vec<u16>,
    /// `Lives` went negative at `$B096`: the game is over.
    pub game_over: bool,
    /// `PlayerState_Death` runs: its `plr_flags` bit 3 and `plr_hurt` 1
    /// keep every hit and the pit test away (`$1FD74`).
    pub dying: bool,
    /// Shared effect, blast and stats tables (`crate::enemies::Shared`).
    rules: Shared,
    /// Per-type handlers of this part and its boss.
    roster: Roster,
    random: u16,
    mappings: BTreeMap<u16, Mapping>,
    /// `Player_FrameTable` mapping IDs, and the frame of the last player
    /// update (`set_player_pose`, `set_player_frame`).
    player_mappings: Vec<u16>,
    player_frame: usize,
    /// Remapped input from this frame's player update, for grab escapes.
    pub(crate) player_input: crate::input::Input,
    bases: BTreeMap<u16, u16>,
    common_tiles: BTreeMap<usize, Tile>,
    /// The `$8001C` puff block. Arenas never load it before a boss's death
    /// (`$2AA34`, `$3204E` re-upload it elsewhere), so there only the shared
    /// puff effect, which stands in for those explosions, draws from it.
    puff_tiles: BTreeMap<usize, Tile>,
    /// Joe's powered resident patterns (`$80000` entry 1, `$ADB8C`), which
    /// replace tiles `$280..$2A9` while powered: the shurikens' art.
    powered_tiles: BTreeMap<usize, Tile>,
    attack_serial: u64,
    enemy_projectile_damage: u8,
    sound_commands: [u8; 3],
    /// Player weapons and ninjutsu resources (issue #42).
    pub weapons: PlayerWeapons,
    /// The Ikazuchi shield's frames (`ninjutsu_fx::shield_mapping`).
    pub shield_art: crate::ninjutsu_fx::Shield,
    /// Body contact from objects outside this module (the list-6 devices of
    /// `crate::stage_objects`, which call `Object_CollidePlayer` and set the
    /// player's bit 14), applied by the next `check_player` with the
    /// enemies' contact.
    pub(crate) pending_damage: u8,
    /// The owner of an enemy shot that struck the player on the last tick
    /// without being blocked or met by the shield (`$36900`), for its next
    /// update: the 2-1 guard's kick (`$15AD8`).
    landed_shot: Option<usize>,
    /// The owner of the shot `check_player_damage` spent on this tick.
    struck_owner: Option<usize>,
    /// A list-6 write after the object pass, consumed by the next
    /// `check_player`. An object pass write replaces it (native order).
    pending_player_event: Option<PlayerEvent>,
    /// `Player_PowerDown` reloaded Joe's art on this update (`power_down`):
    /// `Game` takes it as a lag frame (`crate::game::HIT_LAG`).
    pub(crate) art_reloaded: bool,
    /// `Func_00915A`, selected by the part record's word 14 (#63).
    ambient: ambient::Ambient,
    /// A list-4 object outside this module touching the player (the
    /// time-bomb flames of `crate::stage_objects`). The next `check_player`
    /// tests it after its own shots, as `Object_CollideL4` walks one list.
    pub(crate) pending_hit: Option<PlayerHit>,
    /// `plr_flags` bits 6 and 7: riding the horse or the surfboard. Set by
    /// `crate::special` before each `tick`.
    pub(crate) mount: Option<crate::special::Mount>,
    /// `Player_CheckDamage` ran on the last `tick` (no immunity, no death).
    checked: bool,
    /// What the last object pass left for the next player update.
    carried: Carried,
    /// The player's `obj_flags` bit 11 this frame (the immunity blink):
    /// `Object_CollideList` skips a hidden player ([`Ctx::player_body`]).
    player_hidden: bool,
    /// A handler's own `Object_CollidePlayer` damage this pass
    /// ([`Ctx::contact_player`]), before the enemies' body contact.
    object_contact: Option<u8>,
    /// The list-1 `$27` pusher of `Func_037244` ([`Signal::Shove`]).
    shove: Option<shove::Shove>,
    /// The object pass runs after the player: weapon hits wait a pass.
    deferring: bool,
    /// The last [`Strike::hold`] handed out.
    holds: u32,
    /// Player `plr_status` (`+$2B`) bit 3. `Func_0129C0` discards an enemy's
    /// pending points while the scripted player routine holds it.
    score_suppressed: bool,
    /// `Game_Options` bits 2-3 (0 NORMAL, 1 EASY, 2 HARD, 3 EXPERT), the
    /// column boss handlers read from their timer tables (`Data_02C480`).
    /// The game mirrors `GameOptions::level` before each tick.
    pub(crate) level: u8,
    pub(crate) shurikens_disabled: bool,
    pub(crate) infinite_shurikens: bool,
    /// Intact list-6 item boxes, refreshed before the player's update.
    item_boxes: Vec<AttackTarget>,
    /// `$FF3D05`/`$FF3D06`, the 3-1 `$FC` counters (`enemies::round3`).
    pub(crate) mutants: enemies::round3::Mutants,
}

/// Link objects allocated during the update pass into the list: a finished
/// child's slot is reused, otherwise a synthetic spawn record is added.
/// Returns their indices in allocation order.
fn adopt(
    rules: &enemies::Shared,
    all: &mut Vec<Enemy>,
    spawns: &mut Vec<Spawn>,
    children: Vec<enemies::Child>,
) -> Vec<usize> {
    let mut adopted = Vec::with_capacity(children.len());
    for child in children {
        let Some(stats) = rules.stats.get(usize::from(child.kind / 4)).copied() else {
            continue;
        };
        let spawn = Spawn {
            source: CHILD_SOURCE,
            x: child.enemy.x,
            y: child.enemy.y,
            flags: 0x8000,
            kind: child.kind,
            handler: stats.handler,
            score_bcd: stats.score_bcd,
            contact_damage: stats.contact_damage,
            health_counter: stats.health_counter,
            placeholder_mapping: if enemies::ported(child.kind) {
                0
            } else {
                stats.placeholder_mapping
            },
        };
        let mut enemy = child.enemy;
        let free = all
            .iter()
            .position(|old| spawns[old.spawn].source == CHILD_SOURCE && !old.live());
        let index = match free {
            Some(index) => {
                let slot = all[index].spawn;
                spawns[slot] = spawn;
                enemy.spawn = slot;
                all[index] = enemy;
                index
            }
            None => {
                enemy.spawn = spawns.len();
                spawns.push(spawn);
                all.push(enemy);
                all.len() - 1
            }
        };
        adopted.push(index);
    }
    adopted
}

impl Combat {
    /// `Player_SpawnDust` (`$228EC`): allocate list-9 type 5 at the player's
    /// position. Wall kicks pass a position raised by 16 px (`$213D4`).
    pub(crate) fn spawn_player_dust(&mut self, (x, y): (i32, i32)) {
        let mut dust = Effect::new(EffectKind::Dust, x, y, self.rules.dust_seq[0]);
        dust.motion.vy = -0x10000;
        // Func_03696A toggles obj_flags bit 11 before showing each frame.
        dust.blink = true;
        self.effects.push(dust);
    }

    /// `Func_036FF2` outside the enemy pass (the 5-2 floor fire): puffs in
    /// the shared effect list; returns the sound commands.
    pub(crate) fn small_blast(&mut self, at: (i32, i32), camera: (i32, i32)) -> Vec<u8> {
        let mut sounds = Vec::new();
        enemies::small_blast(
            &self.rules,
            &mut self.random,
            &mut self.effects,
            &mut sounds,
            camera,
            at,
        );
        sounds
    }
    /// `$21546..$2158A`: with more than 4 ticks of immunity left, the stepper
    /// hides Joe (`obj_flags` bit 11) whenever `plr_ticks` is even, unless
    /// the shield is up or `state` never flickers
    /// (`movement::immunity_flickers`). `ticks` is the player's update count.
    pub fn player_visible(&self, state: usize, ticks: u64) -> bool {
        self.invulnerability <= 4
            || ticks % 2 == 1
            || self.weapons.shield != 0
            || !movement::immunity_flickers(state)
    }
    /// The player's hide bit as this frame's update left it, for the
    /// object pass's own player collisions ([`Ctx::player_body`]).
    pub(crate) fn set_player_hidden(&mut self, hidden: bool) {
        self.player_hidden = hidden;
    }
    /// Mirror `Cheat_Invincible` from the session options before play.
    pub(crate) fn set_invincible(&mut self, invincible: bool) {
        self.invincible = invincible;
    }
    /// Mirror player `+$2B` bit 3 for `Func_0129C0`. The scripted player
    /// routine is not otherwise part of combat, so its owner sets this bit.
    pub(crate) fn queue_player_event(&mut self, event: PlayerEvent) {
        self.pending_player_event = Some(event);
    }
    /// Keep progression when reloading section-local spawns and art.
    pub fn inherit_progress(&mut self, previous: &Self) {
        self.view_width = previous.view_width;
        self.hud = previous.hud.clone();
        self.ticks = previous.ticks;
        self.invulnerability = previous.invulnerability;
        self.game_over = previous.game_over;
        self.player_frame = previous.player_frame;
        self.weapons.inherit(&previous.weapons);
    }

    pub(crate) fn screen_view(&self, camera_x: i32) -> (i32, i32) {
        let (left, right) = self.world_bounds;
        Frame::world_view(self.view_width, camera_x - left, right - left)
    }
    pub(crate) fn approaching_pickups(
        &self,
        camera_fixed: i32,
        step: i32,
    ) -> Vec<crate::stage_objects::Drop> {
        self.enemies
            .iter()
            .filter(|enemy| enemy.live())
            .flat_map(|enemy| {
                let spawn = &self.spawns[enemy.spawn];
                self.roster
                    .get(spawn.kind)
                    .map(|behavior| behavior.approaching_drops(enemy, spawn, camera_fixed, step))
                    .unwrap_or_default()
            })
            .collect()
    }
    /// A BCD score word as points.
    pub(crate) fn score(bcd: u16) -> u32 {
        (0..4)
            .map(|n| u32::from((bcd >> (n * 4)) & 15) * 10u32.pow(n))
            .sum()
    }
    /// Health is gone (`Player_Die`, the pit at `$1FD84` or Mijin without a
    /// spare life): the death sequence owns the player until the life is
    /// spent. The death handler also clears the shield (`plr_shield`).
    pub fn die(&mut self) {
        self.hud.health = 0;
        self.dying = true;
        self.weapons.shield = 0;
    }
    /// `Mode_Game` state 8 after a lost life: `Game_InitAmmo` reselects the
    /// starting shuriken count, `Game_RefillHealth` refills health and the
    /// ninjutsu stock. The caller has already taken the life.
    pub fn new_life(&mut self) {
        for shot in self.projectiles.drain(..) {
            if shot.scratch_slot {
                self.projectile_scratch.push(shot.animation_counter);
            }
        }
        self.weapons.new_life();
        self.hud.health = self.hud.max_health;
        self.hud.ammo = self.initial_ammo;
        self.invulnerability = 0;
        self.dying = false;
        self.carried = Carried::default();
        self.shove = None;
    }
    /// Call before player movement: spawn -> enemy/projectiles -> player attacks/damage.
    /// Positions/camera are pixels. Simulation does not call SDL or wall-clock time.
    pub fn tick(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
    ) -> CombatEvents {
        self.tick_with_spikes(player, facing_left, camera, None, terrain)
    }
    /// Ordinary `tick` plus the spike contact recorded by the preceding
    /// player terrain probe. Kept separate so combat-only callers need no
    /// player-movement state. It is [`Self::update_objects`] and then
    /// [`Self::check_player`], back to back.
    pub(crate) fn tick_with_spikes(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        spikes: Option<PlayerHit>,
        terrain: &impl Terrain,
    ) -> CombatEvents {
        let mut event = self.update_objects(player, facing_left, camera, terrain, false);
        let checked = self.check_player(player, facing_left, camera, spikes);
        event.sound_ids.extend(checked.sound_ids);
        event.signals.extend(checked.signals);
        CombatEvents {
            sound_ids: event.sound_ids,
            signals: event.signals,
            ..checked
        }
    }
    /// The object pass `Game_RunFrame` runs after `Player_Update` and the
    /// camera, with that camera: `Func_00915A`, `Boss_Update`,
    /// `Objects_UpdateEnemies` and the list-4 shots (`Func_01CC0A`). What it
    /// leaves for the player waits for the next [`Self::check_player`], as
    /// the ROM's does for the next `Player_Update`: the body contact
    /// (`$1285C` writes the player's `6(a3)` and `7(a3)`), the last
    /// `plr_event` write and the air stab's bounce. Returns the sounds and
    /// signals.
    ///
    /// `after_player` says the pass runs where the ROM's does, after the
    /// player update; the caller then runs the list-6 objects and
    /// [`Self::update_attacks`]. Those weapon passes find the attacks'
    /// overlaps where `PlayerAttacks_Update` has just moved them, which the
    /// objects' own `Object_ProcessHit` (`$127C0`) finds on the next frame,
    /// so the hits land on the next pass, after the objects' update
    /// ([`Enemy::strike`]). Otherwise the pass runs before the player
    /// (`tick`) and moves the attacks itself, and their hits land at once,
    /// and then updates the effects.
    pub(crate) fn update_objects(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
        after_player: bool,
    ) -> CombatEvents {
        let mut event = CombatEvents::default();
        if self.game_over {
            return event;
        }
        self.deferring = after_player;
        self.ticks += 1;
        self.hud.timer_seconds = (self.ticks / 60).min(u64::from(u32::MAX)) as u32;
        // Game_RunFrame: Func_00915A runs before the bosses and the enemies.
        let random = &mut self.random;
        self.ambient.tick(
            &self.rules.ambient,
            camera,
            || crate::enemies::next_random(random),
            &mut self.effects,
        );
        self.update_enemies(player, facing_left, camera, terrain, &mut event);
        for enemy in &mut self.enemies {
            enemy.struck = false;
        }
        self.land_strikes(&mut event);
        // Func_01CC0A: list 4.
        let body = self.player_body(player, facing_left);
        self.update_projectiles(true, body, camera, terrain, &mut event);
        if !after_player {
            self.run_attacks(player, facing_left, camera, terrain, &mut event);
        }
        // Shootable list-4 objects' own `Object_CollideL3`.
        self.hit_weapon_parts(camera, &mut event);
        self.shoot_down(&mut event);
        // A shot that touched the body and expired in the same update is
        // still tested by the damage check below.
        self.retain_projectiles(true);
        let mut contact: Option<PlayerHit> = None;
        let mut touch = |damage: u8, shocked: bool| {
            if contact.is_none() {
                contact = Some(PlayerHit {
                    shocked,
                    ..PlayerHit::contact(damage)
                });
            }
        };
        // A handler's own collision set the player's bit 14 first.
        if let Some(damage) = self.object_contact.take() {
            touch(damage, false);
        }
        for enemy in &self.enemies {
            // $1285C: the player test only runs when no weapon collided.
            if enemy.struck
                || !Self::body_targetable(&self.roster, &self.spawns, enemy)
                || enemy.high_priority != self.weapons.high_priority
            {
                continue;
            }
            if body.collides(enemy_hitbox(&self.mappings, enemy)) {
                let spawn = &self.spawns[enemy.spawn];
                // EnemyA8 ($19072) writes code $2F over ProcessHit's 1.
                touch(spawn.contact_damage, self.roster.shocks(spawn.kind));
                break;
            }
        }
        for shot in &self.projectiles {
            if shot.enemy && shot.touching && shot.body_contact {
                touch(shot.damage, false);
            }
        }
        if !after_player {
            self.update_effects(player, facing_left, camera, terrain, &mut event);
        }
        self.carried = Carried {
            player_event: event.player_event.take(),
            bounce: std::mem::take(&mut event.bounce),
            contact,
            surf_jump: event.surf_jump.take(),
        };
        event
    }
    /// `PlayerAttacks_Update` (list 3) after the list-6 objects: thrown and
    /// spread shurikens and the sword, when [`Self::update_objects`] ran
    /// after the player, then the effects ([`Self::update_effects`]).
    /// Returns the sounds and the effects' signals; a bounce waits for the
    /// player.
    pub(crate) fn update_attacks(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
    ) -> CombatEvents {
        let mut event = CombatEvents::default();
        if self.game_over {
            return event;
        }
        self.run_attacks(player, facing_left, camera, terrain, &mut event);
        self.carried.bounce |= std::mem::take(&mut event.bounce);
        self.update_effects(player, facing_left, camera, terrain, &mut event);
        event
    }
    /// `Func_0366A6`: list 1, then list 9, after every other object list
    /// of the frame, so a record any of them allocated gets its first
    /// update in the frame that allocates it.
    fn update_effects(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
        event: &mut CombatEvents,
    ) {
        let view = self.screen_view(camera.0);
        let mut cx = Ctx {
            weapon_contacts: &mut self.weapon_contacts,
            shared: &self.rules,
            mappings: &self.mappings,
            bases: &self.bases,
            effects: &mut self.effects,
            projectiles: &mut self.projectiles,
            projectile_scratch: &mut self.projectile_scratch,
            random: &mut self.random,
            sounds: &mut event.sound_ids,
            score: &mut self.hud.score,
            score_suppressed: self.score_suppressed,
            level: self.level,
            children: Vec::new(),
            others: Vec::new(),
            live: 0,
            attacks: &[],
            player,
            player_input: self.player_input,
            camera,
            view,
            camera_locked: self.camera_locked,
            mech_hand_contact: self.mech_hand_contact,
            star_damage: self.enemy_projectile_damage,
            throw_sound: self.sound_commands[1],
            current: 0,
            spawn_flags: 0,
            signals: Vec::new(),
            player_event: None,
            landed_shot: None,
            mutants: &mut self.mutants,
            player_body: None,
            player_state: self.weapons.player_state,
            player_airborne: self.weapons.airborne,
            player_facing_left: facing_left,
            player_at: player,
            player_contact: None,
            surf_jump: None,
        };
        // $127FE/$1283A pass the attacker to $36900/$368FA immediately;
        // the target's death routine and score wait for its next update.
        for enemy in &mut self.enemies {
            if let Some(((x, y), lethal)) = enemy.hit_spark.take() {
                let (kind, mapping) = if lethal {
                    (EffectKind::KillSpark, self.rules.kill_seq[0])
                } else {
                    (EffectKind::HitSpark, self.rules.hit_seq[0])
                };
                cx.effect(Effect::new(kind, x, y, mapping));
            }
        }
        cx.update_effects(terrain);
        let signals = std::mem::take(&mut cx.signals);
        event.signals.extend(signals);
        // List 1 also holds the pusher, allocated after the effects.
        if let Some(shove) = &mut self.shove {
            match shove.tick(player.0) {
                Some(push) => event.signals.push(Signal::PushVelocity(push)),
                None => self.shove = None,
            }
        }
    }
    fn run_attacks(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
        event: &mut CombatEvents,
    ) {
        // List 3 before any shuriken of this update reaches a wall.
        self.update_spent_shurikens();
        let body = self.player_body(player, facing_left);
        self.update_projectiles(false, body, camera, terrain, event);
        self.retain_projectiles(true);
        self.update_player_weapons(player, facing_left, camera, terrain, event);
    }
    /// `Player_ApplyEvent` and `Player_CheckDamage` (`$1FD94`, `$21C30`) at
    /// the start of `Player_Update`: what the last [`Self::update_objects`]
    /// left, the list-6 devices' contact and flames, `spikes` from the
    /// player's last terrain probe and the enemy shots touching the body.
    /// Returns what the player update applies.
    pub(crate) fn check_player(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        spikes: Option<PlayerHit>,
    ) -> CombatEvents {
        let mut event = CombatEvents::default();
        if self.game_over {
            return event;
        }
        let carried = std::mem::take(&mut self.carried);
        // An object's `plr_event` write replaces a list-6 one queued before.
        event.player_event = carried.player_event.or(self.pending_player_event.take());
        event.bounce = carried.bounce;
        event.surf_jump = carried.surf_jump;
        // Player_ApplyEvent ($1FD94): while plr_hurt is nonzero it only
        // counts down, and Player_CheckDamage does not run on that update,
        // even the one that takes it to zero.
        let immune = self.invulnerability > 0;
        self.invulnerability = self.invulnerability.saturating_sub(1);
        let pending_contact = std::mem::take(&mut self.pending_damage);
        let mut contact = carried.contact;
        let mut touch = |damage: u8, shocked: bool| {
            if contact.is_none() {
                contact = Some(PlayerHit {
                    shocked,
                    ..PlayerHit::contact(damage)
                });
            }
        };
        // `pending_damage` uses zero as "none" for the list-6 devices, all
        // of which deal damage. Enemy and list-4 body contacts above retain
        // zero as a real hit, as `Object_ProcessHit` does.
        if pending_contact > 0 {
            touch(pending_contact, false);
        }
        if event.player_event == Some(PlayerEvent::Stun) {
            touch(1, false);
        }
        let pending = self.pending_hit.take();
        // PlayerState_Death's plr_flags bit 3 skips the check ($1FD74).
        self.checked = !immune && !self.dying;
        let (hit, struck) = if !self.checked {
            (None, None)
        } else {
            self.check_player_damage(contact, spikes, pending, player, facing_left, &mut event)
        };
        let absorbed = hit.is_some() && self.absorb_with_shield(&mut event);
        match struck {
            Some((at, Some(blast))) => enemies::explode(
                &self.rules,
                &mut self.random,
                &mut self.effects,
                &mut event.sound_ids,
                at,
                blast,
                self.weapons.high_priority,
            ),
            Some((at, None)) => {
                self.shot_spark(at, event.blocked > 0, camera, &mut event);
                if event.blocked == 0 && self.weapons.shield == 0 {
                    self.landed_shot = self.struck_owner;
                }
            }
            None => {}
        }
        if event.player_event == Some(PlayerEvent::GrabDamage)
            && !self.dying
            && self.subtract_event_health(1)
        {
            event.lethal = true;
        }
        if let Some(mut hit) = hit.filter(|_| !absorbed) {
            hit.damage = self.power_down(hit.damage);
            let damage = hit.damage;
            // The thrown sequence subtracts the damage at its slam.
            let thrown = matches!(
                event.player_event,
                Some(PlayerEvent::ThrownLeft | PlayerEvent::ThrownRight)
            );
            if !thrown && damage > 0 {
                self.hud.health = self.hud.health.saturating_sub(damage);
            }
            if !thrown && damage > 0 && self.hud.health == 0 {
                // $21EAA: Player_Die, with neither the voice nor immunity.
                self.die();
                event.lethal = true;
            } else {
                event.player_damage = true;
                event.hit = Some(hit);
                // The mounted hurt states hold their own immunity.
                if !thrown
                    && self.mount.is_none()
                    && let Some(ticks) = hit_immunity(damage)
                {
                    self.invulnerability = ticks;
                }
                // $21ED0: effect $31 for damage code $2F, else voice 4.
                event.sound_ids.push(if hit.shocked {
                    self.rules.cues.get(SoundEvent::Shocked)
                } else {
                    self.sound_commands[2]
                });
            }
        }
        self.retain_projectiles(false);
        event
    }
    /// Enemy objects with their own `Object_CollideL3` ([`ShotDown`]):
    /// the first player weapon over one destroys it, each weapon once.
    fn shoot_down(&mut self, event: &mut CombatEvents) {
        let mut attacks = self.player_attacks();
        for index in 0..self.projectiles.len() {
            let shot = &self.projectiles[index];
            let mech_orb = matches!(
                shot.mech,
                Some(MechShot::Orb {
                    launched: Some(_),
                    hit: false,
                    ..
                })
            );
            if !shot.enemy || !shot.alive || (shot.shot_down.is_none() && !mech_orb) {
                continue;
            }
            let down = shot.shot_down;
            let high_priority = shot.high_priority;
            let hitbox = Self::shot_box(&self.mappings, shot);
            let Some(hit) = attacks.iter().position(|attack| {
                attack.high_priority == shot.high_priority && attack.hitbox.collides(hitbox)
            }) else {
                continue;
            };
            let attack = attacks.remove(hit);
            let (x, y) = (shot.x, shot.y);
            let shot = &mut self.projectiles[index];
            if let Some(MechShot::Orb { hit, .. }) = &mut shot.mech {
                *hit = true;
                shot.no_collision = true;
            } else {
                shot.alive = false;
            }
            shot.touching = false;
            self.spend_projectile_attack(attack.object, event);
            if let Some(down) = down {
                if let Some(blast) = down.blast {
                    enemies::explode(
                        &self.rules,
                        &mut self.random,
                        &mut self.effects,
                        &mut event.sound_ids,
                        (x, y),
                        blast,
                        high_priority,
                    );
                } else if down.frames > 0 || down.smoke.is_some() {
                    self.effects.push(down.puff((x, y), self.random));
                    event.sound_ids.extend(down.sound);
                }
            }
        }
        self.weapons.drop_spent_shots();
    }
    fn hit_weapon_parts(&mut self, camera: (i32, i32), event: &mut CombatEvents) {
        let mut attacks = self.player_attacks();
        for index in 0..self.weapon_contacts.len() {
            let contact = self.weapon_contacts[index];
            let Some(i) = attacks.iter().position(|a| {
                a.high_priority == contact.high_priority && a.hitbox.collides(contact.area)
            }) else {
                continue;
            };
            let attack = attacks[i];
            if let PartTarget::Forward(target) = contact.target {
                let Some(enemy) = self.enemies.get(target) else {
                    continue;
                };
                if attack.hitbox.collides(enemy_hitbox(&self.mappings, enemy)) {
                    continue;
                }
                let damage = self.attack_damage(attack.object);
                let spawn = &self.spawns[enemy.spawn];
                let before = self.enemies[target].health;
                Self::hurt_enemy(&self.roster, &mut self.enemies[target], spawn, damage, true);
                if self.enemies[target].health < before {
                    let kind = if attack.object.shuriken() {
                        crate::profile::HitKind::Shuriken
                    } else {
                        crate::profile::HitKind::Melee
                    };
                    event.hits.push(crate::profile::Hit {
                        target: spawn.kind,
                        defeated: self.enemies[target].health == 0,
                        kind,
                    });
                }
                self.spend_projectile_attack(attack.object, event);
            } else {
                self.spend_projectile_attack(attack.object, event);
                self.shot_spark((contact.area.x, contact.area.y), true, camera, event);
            }
            attacks.remove(i);
        }
        self.weapons.drop_spent_shots();
    }
    /// The next allocation serial: `Object_AllocFromList` links each new
    /// record at the tail of its list, so the serial orders a list.
    pub(crate) fn allocate(&mut self) -> u64 {
        self.allocations += 1;
        self.allocations
    }
    /// The sprite list of an enemy record: list 9 for [`Enemy::back`],
    /// list 8 for the boss record (`Boss_Jumps` runs from list 8), list 5
    /// otherwise.
    pub(crate) fn enemy_list(&self, enemy: &Enemy) -> SpriteList {
        if enemy.back {
            SpriteList::Back
        } else if self.spawns[enemy.spawn].kind == crate::bosses::BOSS_KIND {
            SpriteList::Bosses
        } else {
            SpriteList::Enemies
        }
    }
    /// A material-15 splash of the player's update (list-1 type `$24`,
    /// #63). Wading only splashes when `Frame_Counter & 7` is 0 (`$223B4`;
    /// the combat tick count stands in for the counter). Returns the sound
    /// command entering or leaving requests before the allocation.
    pub(crate) fn splash(&mut self, splash: crate::movement::Splash) -> Option<u8> {
        use crate::movement::SplashKind;
        let sound = match splash.kind {
            SplashKind::Wade { .. } if self.ticks & 7 != 0 => return None,
            SplashKind::Wade { .. } => None,
            SplashKind::Enter => Some(self.rules.cues.get(SoundEvent::WaterEnter)),
            SplashKind::Leave => Some(self.rules.cues.get(SoundEvent::WaterLeave)),
        };
        if let Some(effect) = ambient::splash(
            &self.rules.ambient,
            (splash.x, splash.y),
            splash.kind.group(),
        ) {
            self.effects.push(effect);
        }
        sound
    }
    /// The grounded surfer's wake (`Player_StepAnimation`, `$21772`): a
    /// list-1 type-`$24` routine-`$10` splash on every fourth update. Its
    /// first update runs in this frame's later effect pass. `ticks` is
    /// `plr_ticks` of the allocating player update.
    pub(crate) fn surf_wake(&mut self, position: (i32, i32), ticks: u64) {
        // `$FF0190`, the water layer's speed (`$23C4` and `$23C0`).
        let water = -crate::special::WATER_FLOW << 16;
        if let Some(effect) = ambient::surf_wake(&self.rules.ambient, position, ticks, water) {
            self.effects.push(effect);
        }
    }
    /// A back-layer visual driven outside the object pass (the thrown rider).
    pub(crate) fn allocate_back(&mut self, effect: Effect) {
        self.effects.push(effect);
    }
    /// `Object_AllocL1` for a record whose mounted owner drives its updates.
    pub(crate) fn allocate_front(&mut self, effect: Effect) {
        self.effects.push(effect);
    }
    /// `$26D3A`/`$26D64`: a list-1 plume where the thrown rider meets the water,
    /// with the rider's tile word (`ambient::pending_plume`).
    pub(crate) fn pending_plume(&mut self, at: (i32, i32)) {
        let water = -crate::special::WATER_FLOW << 16;
        let plume =
            ambient::pending_plume(&self.rules.ambient, at, water, self.player_high_priority());
        self.effects.push(plume);
    }
    /// `$FF001C`, read without advancing it.
    pub(crate) fn random_word(&self) -> u16 {
        self.random
    }
    /// Advance the native generator for a new random sample.
    pub(crate) fn next_random(&mut self) -> u16 {
        crate::enemies::next_random(&mut self.random)
    }
    pub fn report(&self) -> String {
        let mut text = format!(
            "Combat: {} original spawn records; recovered enemy sequences\n",
            self.spawns.len()
        );
        for s in &self.spawns {
            text.push_str(&format!(
                "  ${:05x}: ({},{}) flags=${:04x} type=${:02x} handler=${:05x} {}\n",
                s.source,
                s.x,
                s.y,
                s.flags,
                s.kind,
                s.handler,
                self.roster.label(s.kind)
            ));
        }
        text
    }
}

/// The object lists as sprite layers. `Objects_BuildSprites` (`$1488`)
/// fills the sprite table walking lists 1 to 9, each from its head along
/// `obj_next`, and an earlier sprite covers a later one.
/// `Object_AllocFromList` (`$1742`) links a new record at the tail of its
/// list, so within a list the older record is in front; only
/// `Func_02BF68` links one at the head ([`Effect::head`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpriteList {
    /// List 1: sparks, debris, splashes, the laser rooms' pieces.
    #[default]
    Effects = 1,
    /// List 2: the player, then the 2-1 horse.
    Player,
    /// List 3: the player's shurikens, sword and ninjutsu objects.
    PlayerAttacks,
    /// List 4: enemy shots, bombs and flames.
    EnemyShots,
    /// List 5: enemies.
    Enemies,
    /// List 6: boxes, lifts and other stage devices.
    StageObjects,
    /// List 7: pickups.
    Pickups,
    /// List 8: the boss record.
    Bosses,
    /// List 9: carriers and decorations (the 2-1 kite, afterimages, dust).
    Back,
}
impl SpriteList {
    /// Painting order: list 9 first, list 1 last.
    pub const BACK_TO_FRONT: [Self; 9] = [
        Self::Back,
        Self::Bosses,
        Self::Pickups,
        Self::StageObjects,
        Self::Enemies,
        Self::EnemyShots,
        Self::PlayerAttacks,
        Self::Player,
        Self::Effects,
    ];
}

// Test hook: `Combat::hurt_enemy` for enemy `index` (tests in other modules).
