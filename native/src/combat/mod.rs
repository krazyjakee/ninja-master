//! Recovered spawn records and deterministic native combat.
//! Per-type enemy handlers live in `crate::enemies`, bosses in `crate::bosses`.
//! Spawn records are in `spawns`, sprite mappings in `mapping`, the shared
//! object model in `objects`, hits on the player in `hits` and the player's
//! weapons in `weapons`.
pub(crate) mod ambient;
mod hits;
mod mapping;
mod objects;
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

    /// Spawns, object graphics bases and resident art of any part.
    pub fn load_stage(rom: &[u8], stage: crate::stage::StageId) -> Result<Self> {
        let mut spawns = stage_spawns(rom, stage)?;
        // Every loadable `$43222` record (959 ids): player, projectiles,
        // effects and all enemy poses, so no handler can select a missing one.
        let mut mappings = BTreeMap::new();
        for id in 0..crate::rom::SPRITE_MAPPING_COUNT {
            if let Ok(mapping) = Mapping::load(rom, id) {
                mappings.insert(id, mapping);
            }
        }
        for id in [1, 0xca, 0xd2, 0xd3, 0xd5, 0xd6, 0xd7, 0xd8, 0x35b] {
            if !mappings.contains_key(&id) {
                return Err(format!("combat mapping ${id:x} does not load"));
            }
        }
        let player_mappings = (0..PLAYER_FRAME_COUNT)
            .map(|frame| word(rom, PLAYER_FRAME_TABLE + frame * 4))
            .collect::<Result<Vec<_>>>()?;
        if let Some(id) = player_mappings.iter().find(|id| !mappings.contains_key(id)) {
            return Err(format!("player mapping ${id:x} does not load"));
        }
        let rules = Shared::load(rom)?;
        let roster = Roster::load(rom, stage)?;
        let prefix = object_selection(rom, stage)?;
        let object_root = pointer(rom, 0x80008)?;
        let indices = pointer(rom, object_root)?;
        let groups = pointer(rom, object_root + 4)?;
        let mut bases = BTreeMap::new();
        let mut base: u16 = 0;
        for i in 0..4 {
            let p = prefix + i * 8;
            if word(rom, p)? == 0xffff {
                break;
            }
            let kind = word(rom, p + 6)?;
            bases.insert(kind, 0x2300u16.wrapping_add(base));
            let mut group = pointer(rom, groups + pointer(rom, indices + usize::from(kind))?)?;
            let mut terminated = false;
            for _ in 0..256 {
                let destination = word(rom, group)?;
                let descriptor = word(rom, group + 2)?;
                group += 4;
                if destination == 0xffff {
                    base = base
                        .checked_add(descriptor)
                        .ok_or("object graphics base overflow")?;
                    terminated = true;
                    break;
                }
            }
            if !terminated {
                return Err("unterminated object graphics group".into());
            }
        }
        // $1C628 deliberately shares type-$B4 graphics for the $B8 entry.
        if let Some(&base) = bases.get(&0xb4) {
            bases.insert(0xb8, base);
        }
        let mut common_tiles = BTreeMap::new();
        let mut puff_tiles = BTreeMap::new();
        // Resident art: all eight $8001C blocks ($4D8..$575: projectiles,
        // sparks and explosion puffs), Joe, the HUD, and the hit sparks
        // $3C422/$3C4B8/$3C4EC ($5B0..$5FF) loaded by $A2EA..$A308.
        let common = pointer(rom, 0x8001c)?;
        let puffs = pointer(rom, common + 0x1c)?;
        let mut sources = Vec::new();
        for i in 0..8 {
            sources.push(pointer(rom, common + i * 4)?);
        }
        sources.extend([
            pointer(rom, pointer(rom, 0x80000)?)?,
            0x3bffa,
            0x3c422,
            0x3c4b8,
            0x3c4ec,
        ]);
        let mode = spawns::mode_flags(rom, stage)?;
        // $A32E: over the font's last cells, mounted stages load the warning
        // sign ($3C646, tiles $30..$38, `crate::special::Warnings`), the
        // others the "GO" markers ($3C570, tiles $30..$3C).
        sources.push(if mode.mounted() { 0x3c646 } else { 0x3c570 });
        // $A30E: arenas (`$FF39F7` & 3) skip the puff block, whose tiles
        // $4D8..$4F9 hold boss or miniboss art there instead.
        let resident_puffs = mode.resident_puffs();
        for source in sources {
            let first = usize::from(word(rom, source)?);
            let count = usize::from(word(rom, source + 2)?) + 1;
            if first + count > 2048 {
                return Err("combat resident art exceeds VRAM".into());
            }
            let (data, _) = musashi(rom, source + 4, 65536)?;
            let bytes = data
                .get(..count * 32)
                .ok_or("truncated combat resident art")?;
            for (i, tile) in bytes.as_chunks::<32>().0.iter().enumerate() {
                if source == puffs {
                    puff_tiles.insert(first + i, Tile(*tile));
                    if !resident_puffs {
                        continue;
                    }
                }
                common_tiles.insert(first + i, Tile(*tile));
            }
        }
        let mut powered_tiles = BTreeMap::new();
        {
            let source = pointer(rom, pointer(rom, 0x80000)? + 4)?;
            let first = usize::from(word(rom, source)?);
            let count = usize::from(word(rom, source + 2)?) + 1;
            let (data, _) = musashi(rom, source + 4, 65536)?;
            let bytes = data
                .get(..count * 32)
                .ok_or("truncated powered resident art")?;
            for (i, tile) in bytes.as_chunks::<32>().0.iter().enumerate() {
                powered_tiles.insert(first + i, Tile(*tile));
            }
        }
        let mut enemies: Vec<Enemy> = spawns
            .iter()
            .enumerate()
            .map(|(index, s)| {
                Enemy::new(
                    index,
                    s.x,
                    s.y,
                    roster.initial_mapping(s),
                    u16::from(s.health_counter) + 1,
                )
            })
            .collect();
        // Boss_Setup ($2A3AE) at stage load: Boss_Spawn allocates the list-8
        // record from Boss_Records[Round_Index] in boss arenas. It runs every
        // frame from the start (Boss_Update), not through spawn activation.
        if let Some(record) = crate::bosses::record(rom, stage)? {
            let (spawn, enemy) = crate::bosses::spawn(&record, spawns.len());
            spawns.push(spawn);
            enemies.push(enemy);
        }
        let max_health = *rom.get(0x615f).ok_or("missing default health")?;
        Ok(Self {
            weapon_contacts: Vec::new(),
            spawns,
            enemies,
            projectiles: Vec::new(),
            effects: Vec::new(),
            rules,
            roster,
            random: 0,
            mappings,
            player_mappings,
            player_frame: 1,
            player_input: crate::input::Input::default(),
            bases,
            common_tiles,
            puff_tiles,
            powered_tiles,
            hud: Hud {
                health: max_health,
                max_health,
                lives: rom[0x615e],
                ammo: u16::from(rom[0x6174]),
                score: 0,
                score_display: 0,
                // Native session timer: the original HUD does not show a stage clock.
                timer_seconds: 0,
            },
            initial_ammo: u16::from(rom[0x6174]),
            view_width: crate::WIDTH,
            world_bounds: (0, i32::MAX),
            view_screen_clamped: false,
            invulnerability: 0,
            invincible: false,
            player_priority_raised: crate::stage::sections(
                rom,
                stage.round_index(),
                stage.substage_index(),
            )?
            .get(stage.part_index())
            .is_some_and(|section| crate::stage::ModeFlags(section.raw[7]).player_high_priority()),
            ticks: 0,
            camera_locked: false,
            mech_hand_contact: false,
            allocations: 0,
            projectile_scratch: Vec::new(),
            game_over: false,
            dying: false,
            attack_serial: 0,
            enemy_projectile_damage: *rom.get(0x1fbfa).ok_or("missing projectile damage")?,
            sound_commands: [
                SoundEvent::Throw.command(rom)?,
                SoundEvent::EnemyStar.command(rom)?,
                SoundEvent::PlayerHurt.command(rom)?,
            ],
            weapons: PlayerWeapons::load(rom)?,
            shield_art: crate::ninjutsu_fx::Shield::load(rom)?,
            pending_damage: 0,
            landed_shot: None,
            struck_owner: None,
            pending_player_event: None,
            art_reloaded: false,
            ambient: ambient::Ambient::new(
                crate::stage::sections(rom, stage.round_index(), stage.substage_index())?
                    .get(stage.part_index())
                    .map_or(0, |section| section.raw[14]),
            ),
            pending_hit: None,
            mount: None,
            checked: false,
            carried: Carried::default(),
            player_hidden: false,
            object_contact: None,
            shove: None,
            deferring: false,
            holds: 0,
            score_suppressed: false,
            level: 0,
            item_boxes: Vec::new(),
            mutants: enemies::round3::Mutants::default(),
        })
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
    /// Enemy-side reaction to a hit ($127C0). The pose, knockback, sparks and
    /// death sequence start in the enemy's next update from `reaction`.
    fn hurt_enemy(
        roster: &Roster,
        enemy: &mut Enemy,
        spawn: &Spawn,
        damage: u8,
        show_death_spark: bool,
    ) {
        // Handlers call $127C0 only in live states.
        if !Self::hittable(enemy) {
            return;
        }
        enemy.spark_handled = false;
        let response = roster.hit(spawn.kind, enemy);
        match response {
            HitResponse::Pass
            | HitResponse::Ignore
            | HitResponse::BodyOnly
            | HitResponse::Reflect
            | HitResponse::Collides => return,
            HitResponse::Deflect => {
                enemy.reaction = Some(Reaction::Deflect);
                return;
            }
            HitResponse::Accept | HitResponse::WeaponsOnly | HitResponse::NoDeathSpark => {}
        }
        let taken = enemy.health.min(u16::from(damage));
        enemy.health -= taken;
        enemy.damage_taken = enemy.damage_taken.saturating_add(taken);
        if enemy.health == 0 {
            enemy.state = EnemyState::Defeated;
            enemy.dying = true;
            enemy.death_spark = show_death_spark && response.death_spark();
            enemy.pending_score = Self::score(spawn.score_bcd);
            enemy.reaction = Some(Reaction::Death);
        } else {
            enemy.state = EnemyState::Hurt;
            enemy.reaction = Some(Reaction::Hurt);
        }
    }
    /// A player attack met `enemy`: land the hit now, or on the next pass
    /// when this one runs after the player (see `update_objects`). A
    /// deferred hit by an attack that its hit frees returns the
    /// [`Strike::hold`] from `holds` that the attack keeps until then.
    fn strike_enemy(
        roster: &Roster,
        spawn: &Spawn,
        enemy: &mut Enemy,
        mut strike: Strike,
        (defer, holds): (bool, Option<&mut u32>),
        event: &mut CombatEvents,
    ) -> Option<u32> {
        if defer {
            if let Some(holds) = holds {
                *holds = holds.wrapping_add(1);
                strike.hold = Some(*holds);
            }
            enemy.strike = Some(strike);
            strike.hold
        } else {
            Self::land_strike(roster, spawn, enemy, strike, event);
            None
        }
    }
    fn land_strike(
        roster: &Roster,
        spawn: &Spawn,
        enemy: &mut Enemy,
        strike: Strike,
        event: &mut CombatEvents,
    ) {
        enemy.struck = true;
        if strike.serial != 0 {
            enemy.attack_serial = strike.serial;
        }
        Self::hurt_enemy(roster, enemy, spawn, strike.damage, true);
        match enemy.reaction {
            Some(Reaction::Death) if enemy.death_spark => {
                enemy.hit_spark = Some((strike.origin, true));
                enemy.spark_handled = true;
            }
            Some(Reaction::Hurt) => {
                enemy.hit_spark = Some((strike.origin, false));
                enemy.spark_handled = true;
            }
            _ => {}
        }
        if !strike.shuriken || enemy.reaction != Some(Reaction::Deflect) {
            event.sound_ids.push(strike.cue);
        }
        event.bounce |= strike.bounce;
    }
    /// The hits the last pass found, after this pass's object updates:
    /// each object still able to take one does, and skips its player test.
    /// The attack is freed with it, as its own update does in the ROM
    /// (`$24DD6` shurikens, `$24C92` swords, `$252E4` the air stab), after
    /// the frame it was drawn meeting the object. An attack whose hit does
    /// not land (the object no longer takes hits, or a new life cleared the
    /// attack) runs on: a shuriken flies on, a sword keeps its routine.
    fn land_strikes(&mut self, event: &mut CombatEvents) {
        let live = self.held_attacks();
        let mut landed = Vec::new();
        for enemy in &mut self.enemies {
            let Some(strike) = enemy.strike.take() else {
                continue;
            };
            if strike.hold.is_some_and(|hold| !live.contains(&hold))
                || !Self::weapon_targetable(&self.roster, &self.spawns, enemy)
            {
                continue;
            }
            let spawn = &self.spawns[enemy.spawn];
            Self::land_strike(&self.roster, spawn, enemy, strike, event);
            landed.extend(strike.hold);
        }
        self.free_held_attacks(&landed);
    }
    /// A weapon target without a hit waiting from the last pass: one
    /// `Object_CollideL3` overlap per object update.
    fn attack_targetable(roster: &Roster, spawns: &[Spawn], enemy: &Enemy) -> bool {
        enemy.strike.is_none() && Self::weapon_targetable(roster, spawns, enemy)
    }
    /// A live object that the player's weapons collide with now.
    fn weapon_targetable(roster: &Roster, spawns: &[Spawn], enemy: &Enemy) -> bool {
        Self::hittable(enemy)
            && roster
                .hit(spawns[enemy.spawn].kind, enemy)
                .weapon_collides()
    }
    /// A live object whose body collides with the player's now.
    fn body_targetable(roster: &Roster, spawns: &[Spawn], enemy: &Enemy) -> bool {
        let kind = spawns[enemy.spawn].kind;
        Self::hittable(enemy)
            && roster.touches_player(kind)
            && roster.hit(kind, enemy).body_collides()
    }
    fn hittable(enemy: &Enemy) -> bool {
        !matches!(
            enemy.state,
            EnemyState::Dormant | EnemyState::Emerging | EnemyState::Defeated
        )
    }
    /// Enemy state machines and effects, in object-list order.
    fn update_enemies(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
        event: &mut CombatEvents,
    ) {
        self.weapon_contacts.clear();
        let others: Vec<Snapshot> = self.enemies.iter().map(Snapshot::of).collect();
        let live = self
            .enemies
            .iter()
            .filter(|enemy| {
                enemy.live() && self.spawns[enemy.spawn].kind != crate::bosses::BOSS_KIND
            })
            .count();
        // List 3 as `Func_012BFE` scans it: thrown and spread shurikens and
        // the sword object, which follows the player.
        let mut attacks: Vec<(i32, i32, bool)> = self
            .projectiles
            .iter()
            .filter(|shot| !shot.enemy)
            .map(|shot| (shot.x, shot.y, shot.high_priority))
            .chain(
                self.weapons
                    .shots
                    .iter()
                    .map(|shot| (shot.x >> 16, shot.y >> 16, shot.high_priority)),
            )
            .collect();
        attacks.extend(
            self.weapons
                .sword_position(player, facing_left)
                .map(|(x, y)| (x, y, self.weapons.high_priority)),
        );
        // `Object_CollidePlayer`'s view of the player: where its update
        // left it before `Object_Integrate`, unless hidden or raised.
        let player_at = self
            .weapons
            .anchors
            .map_or(player, |anchors| anchors.unmoved);
        let player_body = (!self.player_hidden && !self.weapons.high_priority)
            .then(|| self.player_body(player_at, facing_left));
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
            others,
            live,
            attacks: &attacks,
            // `Object_DistanceToPlayer` ($126F2) reads the player's obj_x/
            // obj_y before `Object_Integrate` adds this update's step
            // (`Objects_BuildSprites`): where the last update left it.
            player: player_at,
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
            landed_shot: self.landed_shot.take(),
            mutants: &mut self.mutants,
            player_body,
            player_state: self.weapons.player_state,
            player_airborne: self.weapons.airborne,
            player_facing_left: facing_left,
            player_at,
            player_contact: None,
            surf_jump: None,
        };
        for (index, enemy) in self.enemies.iter_mut().enumerate() {
            let spawn = &self.spawns[enemy.spawn];
            cx.current = index;
            let dormant = enemy.state == EnemyState::Dormant;
            let allocated = cx.children.len();
            enemies::run(&self.roster, &mut cx, enemy, spawn, self.ticks, terrain);
            // Boss5 runs before list 5: its parts read this frame's position
            // and commands, not the previous frame's snapshot.
            if spawn.handler == 0x30654 {
                cx.others[index] = Snapshot::of(enemy);
            }
            // A spawn record's activation, then the records its update
            // allocated, join the tail of the list in that order.
            if dormant && enemy.state != EnemyState::Dormant {
                self.allocations += 1;
                enemy.order = self.allocations;
            }
            for child in &mut cx.children[allocated..] {
                self.allocations += 1;
                child.enemy.order = self.allocations;
            }
        }
        // `Object_AllocFromList` links a new record at the tail of list 5
        // and `Objects_RunEnemyList` ($126C2) reads each record's `obj_next`
        // after its handler returns, so what the pass allocated runs in the
        // same pass, after every older record (and so do its own children).
        // Their parents have already updated: they see them as they are now.
        while !cx.children.is_empty() {
            let adopted = adopt(
                &self.rules,
                &mut self.enemies,
                &mut self.spawns,
                std::mem::take(&mut cx.children),
            );
            cx.live += adopted.len();
            cx.others = self.enemies.iter().map(Snapshot::of).collect();
            for index in adopted {
                let enemy = &mut self.enemies[index];
                cx.current = index;
                let allocated = cx.children.len();
                enemies::run(
                    &self.roster,
                    &mut cx,
                    enemy,
                    &self.spawns[enemy.spawn],
                    self.ticks,
                    terrain,
                );
                for child in &mut cx.children[allocated..] {
                    self.allocations += 1;
                    child.enemy.order = self.allocations;
                }
            }
        }
        let Ctx {
            signals,
            player_event,
            player_contact,
            surf_jump,
            ..
        } = cx;
        self.object_contact = player_contact;
        event.surf_jump = surf_jump;
        for signal in signals {
            match signal {
                // The raise precedes this update's player collisions.
                Signal::RaisePlayerPriority => self.raise_player_priority(),
                // Func_0373E6: the part box against the player's.
                Signal::Shove(part) => {
                    if self.player_body(player, facing_left).collides(part) {
                        self.shove = Some(shove::Shove::new(part));
                    }
                }
                _ => event.signals.push(signal),
            }
        }
        event.player_event = player_event;
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
    /// `Shuriken_Update`'s terrain test (`$24DF8..$24E66`) where a thrown
    /// shuriken stands, with its own material table rather than `$3854`'s
    /// D2: a stopping material or a negative X ends it.
    fn shuriken_terrain_at(
        terrain: &impl Terrain,
        shot: &mut Projectile,
        walls: &mut Vec<WallHit>,
        splashes: &mut Vec<(i32, i32, bool)>,
    ) {
        let at = (shot.x, shot.y);
        match PlayerWeapons::shuriken_terrain(terrain, at, &mut shot.in_water) {
            ShurikenTerrain::Fly => {}
            ShurikenTerrain::Splash => splashes.push((shot.x, shot.y, shot.leftward())),
            ShurikenTerrain::Leave => shot.alive = false,
            ShurikenTerrain::Wall => {
                walls.push(WallHit {
                    at,
                    velocity: ((shot.velocity_x << 16) + shot.motion.vx, shot.motion.vy),
                    mapping: shot.mapping,
                    high_priority: shot.high_priority,
                    order: shot.order,
                });
                shot.alive = false;
            }
        }
    }
    /// $18BC pushes a freed record on the free-list head without clearing
    /// +$28. Keep that word for the next native list-4 allocation.
    fn retain_projectiles(&mut self, touching: bool) {
        self.projectiles.retain(|shot| {
            let keep = shot.alive || (touching && shot.touching);
            if !keep && shot.scratch_slot {
                self.projectile_scratch.push(shot.animation_counter);
            }
            keep
        });
    }
    /// One update of the enemy shots (list 4) or of the thrown shurikens.
    fn update_projectiles(
        &mut self,
        enemy_shots: bool,
        body: Hitbox,
        camera: (i32, i32),
        terrain: &impl Terrain,
        event: &mut CombatEvents,
    ) {
        let view = self.screen_view(camera.0);
        let shuriken_hit = self.rules.cues.get(SoundEvent::ShurikenHit);
        let mut walls = Vec::new();
        let mut splashes = Vec::new();
        for shot in &mut self.projectiles {
            if shot.enemy != enemy_shots {
                continue;
            }
            shot.touching = false;
            // Removed by an enemy this tick (deflected, or its owner died).
            if !shot.alive {
                continue;
            }
            shot.age = shot.age.saturating_add(1);
            if let Some(segments) = &shot.sparkle_trail
                && shot.age.is_multiple_of(2)
            {
                // $1B646 calls $1AE84 before movement. The shot drives
                // these list-9 records even after its battery is destroyed.
                let mut start = 0;
                for &(mapping, ticks) in segments {
                    let mut sparkle = Effect::new(EffectKind::Pieces, shot.x, shot.y, mapping);
                    sparkle.base = 0x8000;
                    sparkle.high_priority = true;
                    sparkle.back = true;
                    sparkle.delay = start + 1;
                    sparkle.counter = i32::from(ticks);
                    self.effects.push(sparkle);
                    start += ticks;
                }
            }
            if shot
                .flip_period
                .is_some_and(|n| n > 0 && shot.age.is_multiple_of(n))
            {
                shot.base ^= 0x1000;
            }
            if let Some((period, mapping)) = shot.trail.filter(|(period, _)| *period > 0)
                && shot.age.is_multiple_of(period)
            {
                let kind = if mapping == 0 {
                    EffectKind::Puff
                } else {
                    EffectKind::Pieces
                };
                let mut trail = Effect::new(kind, shot.x, shot.y, mapping);
                trail.base = shot.base;
                trail.back = true;
                trail.flip = shot.leftward() != shot.faces_left;
                trail.blink = mapping != 0;
                trail.counter = 8;
                self.effects.push(trail);
            }
            shot.animation_counter = shot.animation_counter.wrapping_add(1);
            // $1CDE4 reads the visibility flag left by the preceding sprite
            // pass, before motion. Allocation clears it even off screen.
            if shot.enemy && !shot.left_edge_only && shot.offscreen {
                shot.alive = false;
                continue;
            }
            // `Object_LoadHitbox` at the top of most list-4 updates reads the
            // frame and flips the last update left drawn (enemy handlers that
            // animate a shot of theirs do so after it, in the ROM's handler).
            let held = if shot.enemy {
                shot.held_frame.take().unwrap_or_else(|| shot.drawn_frame())
            } else {
                shot.drawn_frame()
            };
            // Func_024BE0: a shuriken Boss2's field turned steers on every
            // other update and flies on its 16.16 velocity.
            if let Some(mut turn) = shot.turn.filter(|turn| turn.left > 0)
                && shot.age.is_multiple_of(2)
            {
                let (mapping, flips, vx, vy) = self.weapons.steer(&mut turn);
                shot.turn = Some(turn);
                shot.mapping = mapping;
                shot.velocity_x = 0;
                shot.motion.vx = vx;
                shot.motion.vy = vy;
                shot.base = (shot.base & !0x1000) | (flips & 0x1000);
                shot.faces_left = shot.leftward() != (flips & 0x800 != 0);
            }
            // $1D204: only the ninja star (projectile 4) cycles its mappings.
            if shot.enemy && (0x35b..=0x35e).contains(&shot.mapping) {
                shot.mapping = 0x35b + ((shot.animation_counter >> 1) & 3);
            }
            if let Some(frames) = shot.frames {
                let (mapping, flip) = frames[usize::from((shot.age >> 2) & 3)];
                shot.mapping = mapping;
                // Drawn flipped exactly when the frame says so.
                shot.faces_left = shot.leftward() != flip;
            }
            if let Some(mut sway) = shot.sway.take() {
                let stays = sway.update(shot);
                shot.sway = Some(sway);
                if !stays {
                    shot.alive = false;
                    continue;
                }
            }
            if !shot.enemy && std::mem::take(&mut shot.untested) {
                // Its hit held and did not land: `Shuriken_Update` found
                // no collision bit (`loc_024DD0`) and tests the terrain
                // where the step ended before the shuriken moves on.
                Self::shuriken_terrain_at(terrain, shot, &mut walls, &mut splashes);
                if !shot.alive {
                    continue;
                }
            }
            let moving = shot.motion != Motion::default();
            if !shot.enemy {
                // `Object_Integrate` in `Objects_BuildSprites` moves the
                // shuriken a whole step, and only where the step ends does
                // anything meet it: the objects' `Object_CollideL3` on the
                // next frame, then `Shuriken_Update`'s terrain test.
                shot.x += shot.velocity_x;
                if moving {
                    shot.motion.step(&mut shot.x, &mut shot.y);
                }
                let hitbox = Self::shot_box(&self.mappings, shot);
                for enemy in &mut self.enemies {
                    if !Self::attack_targetable(&self.roster, &self.spawns, enemy)
                        || shot.high_priority != enemy.high_priority
                        || !hitbox.collides(enemy_hitbox(&self.mappings, enemy))
                    {
                        continue;
                    }
                    let kind = self.spawns[enemy.spawn].kind;
                    if self.roster.hit(kind, enemy) == HitResponse::Reflect {
                        // `Shuriken_Init` left +$28 0 or $E. The guard's
                        // shield and Boss2's field select different turns.
                        let direction = if shot.leftward() { 0xe } else { 0 };
                        shot.turn = shot.turn.or(Some(if kind == 0x18 {
                            Turn::guard(direction, self.ticks)
                        } else {
                            Turn::away(direction, shot.y, enemy.y)
                        }));
                        continue;
                    }
                    let strike = Strike {
                        origin: (shot.x, shot.y),
                        damage: shot.damage,
                        cue: shuriken_hit, // $24DE0
                        shuriken: true,
                        bounce: false,
                        hold: None,
                        serial: 0,
                    };
                    shot.held = Self::strike_enemy(
                        &self.roster,
                        &self.spawns[enemy.spawn],
                        enemy,
                        strike,
                        (self.deferring, Some(&mut self.holds)),
                        event,
                    );
                    shot.alive = shot.held.is_some();
                    break;
                }
                // Spent or waiting for its hit, which frees it first.
                if shot.alive && shot.held.is_none() {
                    Self::shuriken_terrain_at(terrain, shot, &mut walls, &mut splashes);
                }
            } else if std::mem::take(&mut shot.walled) {
                if shot.bounce_dart() {
                    shot.motion.step(&mut shot.x, &mut shot.y);
                    continue;
                }
                if let Some(sound) = shot.wall_sound.take() {
                    event.sound_ids.push(sound);
                }
                // `$1CDE4`: the handler meets the wall where the last step
                // left the shot (drawn there on that frame) and frees it
                // with `$3690C`'s spark. List 1 runs after list 4, so the
                // spark has its first update (hidden, `$35F`) on this frame
                // (the effect pass) and is first drawn on the next.
                if let Some(blast) = shot.contact_blast {
                    enemies::explode(
                        &self.rules,
                        &mut self.random,
                        &mut self.effects,
                        &mut event.sound_ids,
                        (shot.x, shot.y),
                        blast,
                        shot.high_priority,
                    );
                } else {
                    let mut spark = Effect::new(
                        EffectKind::WallSpark,
                        shot.x,
                        shot.y,
                        self.rules.dust_seq[0],
                    );
                    spark.blink = true;
                    spark.high_priority = shot.high_priority;
                    spark.base = if shot.high_priority { 0x8000 } else { 0 };
                    self.effects.push(spark);
                }
                shot.alive = false;
            } else {
                // `Object_Integrate` in `Objects_BuildSprites` moves the shot
                // a whole step, and only where it ends is it tested: by the
                // player's `Player_CheckDamage` (`Object_CollideL4`, strict,
                // no size check) on the next frame, then by its own next
                // update's terrain test there. A shot that crosses the body
                // inside a step misses it.
                shot.x += shot.velocity_x;
                if moving {
                    shot.motion.step(&mut shot.x, &mut shot.y);
                }
                // The box this update loaded, met where the step ends.
                shot.box_frame = Some(match shot.box_load {
                    ShotBox::Before => held,
                    ShotBox::After | ShotBox::Fixed(_) => shot.drawn_frame(),
                });
                shot.held_frame = Some(shot.drawn_frame());
                // The player's damage check decides whether it hits.
                shot.touching = shot.high_priority == self.weapons.high_priority
                    && !shot.no_collision
                    && Self::shot_box(&self.mappings, shot).collides(body);
                let material = terrain.material_at(shot.x, shot.y);
                // Enemy shots pass platforms; a wall leaves `$3690C`.
                let terrain_hit = material.object_solid(shot.high_priority)
                    && material != movement::Material::PLATFORM;
                shot.walled = !shot.passes_terrain && terrain_hit;
            }
            // A held shuriken waits for its hit.
            if shot.held.is_some() {
                continue;
            }
            // Enemy shots use Object_UpdateVisibility: ±256 px from the
            // screen centre on either axis. Specialized handlers may also
            // install an explicit lifetime.
            let outside = if shot.left_edge_only {
                shot.x < camera.0 + view.0
            } else if shot.enemy {
                shot.offscreen = outside_view_in((shot.x, shot.y), camera, view);
                false
            } else {
                shot.x < camera.0 + view.0 - 64 || shot.x > camera.0 + view.1 + 64
            };
            if shot.life != u16::MAX && shot.age >= shot.life || outside {
                shot.alive = false;
            }
            if let Some(home) = shot.home {
                // Back within 8 pixels of home after turning ($1A934).
                let returning = (shot.motion.vx < 0) == (shot.motion.ax < 0);
                if returning && (home - shot.x).abs() < 8 {
                    shot.alive = false;
                }
            }
            if !shot.enemy && moving && (shot.y < camera.1 - 64 || shot.y > camera.1 + 288) {
                shot.alive = false;
            }
        }
        for (x, y, left) in splashes {
            self.splash_shuriken((x, y), left, event);
        }
        for wall in walls {
            self.strike_wall(wall, event);
        }
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
        for contact in self.weapon_contacts.clone() {
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
                Self::hurt_enemy(&self.roster, &mut self.enemies[target], spawn, damage, true);
                self.spend_projectile_attack(attack.object, event);
            } else {
                self.spend_projectile_attack(attack.object, event);
                self.shot_spark((contact.area.x, contact.area.y), true, camera, event);
            }
            attacks.remove(i);
        }
        self.weapons.drop_spent_shots();
    }
    /// The records of sprite list `list` this module holds, painted last
    /// linked first so that the first record wins overlaps
    /// (`Objects_BuildSprites`, see [`SpriteList`]). The player's shurikens
    /// are [`Self::draw_player_attacks`]; Boss4's list-1 parts are
    /// [`Self::draw_boss_parts`].
    pub fn draw_list(
        &self,
        frame: &mut Frame,
        list: SpriteList,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
    ) {
        match list {
            SpriteList::Enemies | SpriteList::Bosses => {}
            // List 4 holds the enemies' shots (the player's are
            // `draw_player_attacks`). A shot that expired touching the body
            // waits for the damage check.
            SpriteList::EnemyShots => {
                for shot in self
                    .projectiles
                    .iter()
                    .rev()
                    .filter(|shot| shot.alive && !shot.hidden && !shot.offscreen && shot.enemy)
                {
                    frame.mark_high_sprites(shot.high_priority);
                    self.draw_projectile(frame, palettes, scene_tiles, camera, shot);
                }
                frame.mark_high_sprites(false);
            }
            _ => {}
        }
        self.draw_effects(frame, list, palettes, scene_tiles, camera);
        self.draw_enemies(frame, palettes, scene_tiles, camera, list);
    }
    /// The effect records of `list` in reverse link order: the ones linked
    /// at the list's tail newest first, then the ones linked at its head
    /// ([`Effect::head`]) oldest first. The ambient spawners' priority
    /// sprites are [`Self::draw_front`] and Boss4's list-1 parts
    /// [`Self::draw_boss_parts`].
    fn draw_effects(
        &self,
        frame: &mut Frame,
        list: SpriteList,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
    ) {
        let listed = |effect: &&Effect| {
            effect.sprite_list() == list
                && !effect.in_front()
                && !(list == SpriteList::Effects && effect.kind == EffectKind::BossPart)
                // Drawn on the player's patterns (`Game::draw`).
                && !matches!(
                    effect.kind,
                    EffectKind::ThrownRider | EffectKind::HorseWreck | EffectKind::SurfSlashBoard
                )
        };
        let tail = self.effects.iter().rev().filter(|e| !e.head).filter(listed);
        let head = self.effects.iter().filter(|e| e.head).filter(listed);
        for effect in tail.chain(head) {
            frame.mark_high_sprites(effect.high_priority);
            self.draw_effect(frame, palettes, scene_tiles, camera, effect);
        }
        frame.mark_high_sprites(false);
    }
    /// Boss4's list-1 parts (`bosses::boss4::parts`), newest first. They
    /// keep the place they had before the lists were drawn in order, after
    /// list 4 (4-3 is being worked on separately).
    pub fn draw_boss_parts(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
    ) {
        for effect in self.effects.iter().rev().filter(|effect| {
            effect.kind == EffectKind::BossPart && effect.sprite_list() == SpriteList::Effects
        }) {
            frame.mark_high_sprites(effect.high_priority);
            self.draw_effect(frame, palettes, scene_tiles, camera, effect);
        }
        frame.mark_high_sprites(false);
    }
    fn draw_projectile(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
        shot: &Projectile,
    ) {
        // Enemy weapons may use their thrower's graphics base; art that
        // faces left (the sickle) mirrors while moving right.
        let left = shot.leftward() != shot.faces_left;
        self.draw_object(
            frame,
            palettes,
            scene_tiles,
            shot.mapping,
            shot.base,
            (shot.x - camera.0, shot.y - camera.1),
            left,
        );
    }
    fn draw_effect(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
        effect: &Effect,
    ) {
        if effect.delay > 0
            || effect.flags & 0x880 != 0
            || effect.mapping == 0
            || (effect.blink && !effect.age.is_multiple_of(2))
            || !self.mappings.contains_key(&effect.mapping)
        {
            return;
        }
        self.draw_mapping(
            frame,
            palettes,
            scene_tiles,
            effect.mapping,
            effect.base,
            (effect.x - camera.0, effect.y - camera.1),
            effect.flip,
            // Both sequences use $4D8..$4F9, whose contents vary by stage.
            matches!(effect.kind, EffectKind::Puff | EffectKind::Debris),
        );
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
    /// Indices of the enemies of `list`, last allocated first: painted in
    /// this order, the first record wins overlaps.
    pub(crate) fn paint_order(&self, list: SpriteList) -> impl Iterator<Item = usize> + use<> {
        let mut listed: Vec<(u64, usize)> = self
            .enemies
            .iter()
            .enumerate()
            .filter(|(_, enemy)| self.enemy_list(enemy) == list)
            .map(|(index, enemy)| (enemy.order, index))
            .collect();
        listed.sort_unstable_by(|a, b| b.cmp(a));
        listed.into_iter().map(|(_, index)| index)
    }
    /// The enemies of `list` in reverse list order (the first record wins
    /// overlaps). Freed slots are reused, so list order is the allocation
    /// order, not the index.
    fn draw_enemies(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
        list: SpriteList,
    ) {
        for enemy in self.paint_order(list).map(|index| &self.enemies[index]) {
            let visible = match enemy.state {
                EnemyState::Dormant | EnemyState::Emerging => false,
                EnemyState::Defeated => enemy.dying,
                _ => true,
            };
            if !visible {
                continue;
            }
            let kind = self.spawns[enemy.spawn].kind;
            if enemy.mapping != 0 {
                let mut actor_palettes = *palettes;
                let mut flashed = *palettes[1].colors();
                // The samurai write CRAM line 1 themselves (`PaletteOp`).
                // Natively the flash is the actor's own copy of the line.
                if enemy.flash > 0 && !matches!(kind, 0xb4 | 0xb8) {
                    self.flash_palette(&mut flashed);
                    actor_palettes[1] = PaletteRef::Fixed(&flashed);
                }
                let base = enemy
                    .tile
                    .or_else(|| self.bases.get(&kind).copied())
                    .unwrap_or(0x2300);
                frame.mark_high_sprites(enemy.high_priority);
                self.draw_object(
                    frame,
                    &actor_palettes,
                    scene_tiles,
                    enemy.mapping,
                    base,
                    (enemy.x - camera.0, enemy.y - camera.1),
                    enemy.facing_left,
                );
                frame.mark_high_sprites(false);
            }
            // Issue #51: an unported type (or a documented stand-in) is
            // outlined instead of passing off a guessed pose as recovered.
            if self.roster.marked(kind) {
                let area = enemies::marker_box(&self.mappings, enemy);
                Self::draw_outline(frame, area, camera, UNPORTED_OUTLINE);
            }
        }
    }
    /// The effects `Effect::in_front` keeps out of `draw`: the game paints
    /// them after the high-priority plane.
    pub fn draw_front(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
    ) {
        for effect in self.effects.iter().filter(|effect| effect.in_front()) {
            self.draw_object(
                frame,
                palettes,
                scene_tiles,
                effect.mapping,
                effect.base,
                (effect.x - camera.0, effect.y - camera.1),
                effect.flip,
            );
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
    /// `$1BEC4`: the white hit flash over colours 0..7 of an actor's line.
    fn flash_palette(&self, line: &mut Palette) {
        line[..8].fill(md_color(self.rules.flash_color));
    }
    /// One-pixel outline of `area` (world coordinates).
    fn draw_outline(frame: &mut Frame, area: Hitbox, camera: (i32, i32), color: [u8; 3]) {
        let left = area.x - area.half_width - camera.0;
        let top = area.y - area.half_height - camera.1;
        let (width, height) = (area.half_width * 2, area.half_height * 2);
        frame.overlay_rect(left, top, width, 1, color);
        frame.overlay_rect(left, top + height - 1, width, 1, color);
        frame.overlay_rect(left, top, 1, height, color);
        frame.overlay_rect(left + width - 1, top, 1, height, color);
    }
    /// `$FF001C`, read without advancing it.
    pub(crate) fn random_word(&self) -> u16 {
        self.random
    }
    /// Advance the native generator for a new random sample.
    pub(crate) fn next_random(&mut self) -> u16 {
        crate::enemies::next_random(&mut self.random)
    }
    /// Draw mapping `id` with attribute base `base` at screen `position`,
    /// from the resident art first (Joe's powered patterns while powered),
    /// then the scene's tiles.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_object(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        id: u16,
        base: u16,
        position: (i32, i32),
        left: bool,
    ) {
        self.draw_mapping(
            frame,
            palettes,
            scene_tiles,
            id,
            base,
            position,
            left,
            false,
        );
    }
    /// [`Self::draw_object`]; `puffs` selects the separate explosion art,
    /// including in arenas where the scene uses those tile indices for bosses.
    #[allow(clippy::too_many_arguments)]
    fn draw_mapping(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        id: u16,
        base: u16,
        position: (i32, i32),
        left: bool,
        puffs: bool,
    ) {
        let Some(mapping) = self.mappings.get(&id) else {
            return;
        };

        draw_pieces(
            frame,
            &mapping.pieces,
            palettes,
            base,
            position,
            left,
            crate::render::SatWriter::Object,
            |index| {
                self.weapons
                    .powered
                    .then(|| self.powered_tiles.get(&index))
                    .flatten()
                    .or_else(|| puffs.then(|| self.puff_tiles.get(&index)).flatten())
                    .or_else(|| self.common_tiles.get(&index))
                    .or_else(|| scene_tiles.get(index))
            },
        );
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
