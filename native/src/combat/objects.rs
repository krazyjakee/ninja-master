//! The shared object model (issue #25, #51): 16.16 motion, launches,
//! blasts, the `$16F0` effect objects, enemies and projectiles. Per-type
//! recovered values.
use super::{Hitbox, SpriteList};
use crate::{
    enemies,
    rom::{Result, long, word},
};

/// `$4B52`: `(x, y)` lies in the window (0..$160 right of and below the
/// camera) that `$4B2E` requires before it requests an effect sound.
pub(crate) fn in_sound_window((x, y): (i32, i32), camera: (i32, i32)) -> bool {
    (0..0x160).contains(&(x - camera.0)) && (0..0x160).contains(&(y - camera.1))
}

/// `Object_UpdateVisibility` (`$17A4`): `obj_flags` bit 7, set when `(x, y)`
/// is 256 or more pixels from the screen centre on either axis.
pub(crate) fn outside_view_in((x, y): (i32, i32), camera: (i32, i32), view: (i32, i32)) -> bool {
    x <= camera.0 + view.0 - 96 || x >= camera.0 + view.1 + 96 || (camera.1 + 112 - y).abs() >= 256
}

/// Knockback/death launch installed by a handler (16.16 pixels per tick).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Launch {
    pub(crate) vx: i32,
    pub(crate) vy: i32,
    pub(crate) gravity: i32,
}
impl Launch {
    pub(crate) fn read(rom: &[u8], vy: usize, gravity: usize, vx: usize) -> Result<Self> {
        Ok(Self {
            vx: long(rom, vx)?,
            vy: long(rom, vy)?,
            gravity: i32::from(word(rom, gravity)?),
        })
    }
}
/// `$36DB6` explosion parameters: vertical/horizontal velocity shifts and sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Blast {
    pub(crate) y_shift: u32,
    pub(crate) x_shift: u32,
    pub(crate) sound: u8,
}

/// Object-list motion in 16.16 pixels (`Object_Integrate` `$19D4`):
/// acceleration (`+$14`/`+$16`, words added to the long velocity), then
/// velocity (`+$18`/`+$1C`) into the position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Motion {
    pub(crate) fx: u16,
    pub(crate) fy: u16,
    pub(crate) vx: i32,
    pub(crate) vy: i32,
    /// Horizontal acceleration `+$14`.
    pub(crate) ax: i32,
    /// Vertical acceleration `+$16`.
    pub(crate) gravity: i32,
}
impl Motion {
    pub(crate) fn step(&mut self, x: &mut i32, y: &mut i32) {
        self.vx = self.vx.saturating_add(self.ax);
        self.vy = self.vy.saturating_add(self.gravity);
        advance(x, &mut self.fx, self.vx);
        advance(y, &mut self.fy, self.vy);
    }
    pub(crate) fn stop(&mut self) {
        self.vx = 0;
        self.vy = 0;
        self.ax = 0;
        self.gravity = 0;
    }
}
pub(crate) fn advance(value: &mut i32, fraction: &mut u16, velocity: i32) {
    let full = (i64::from(*value) << 16) + i64::from(*fraction) + i64::from(velocity);
    *value = (full >> 16) as i32;
    *fraction = full as u16;
}

/// Hit outcome recorded by `hurt_enemy` and applied by the enemy's next update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reaction {
    Hurt,
    Death,
    Deflect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    /// `Player_SpawnDust` (`$228EC`): list-9 type 5, mappings `$35F..$362`.
    Dust,
    /// `$36E82` child: one puff of the shared death explosion.
    Puff,
    /// `$36F64`: type-$24 debris.
    Debris,
    /// `$36A38`: type-$24 sparkle.
    Sparkle,
    /// `$16032`: the type-$24 head/hat that bounces away.
    Head,
    /// `$36952`: non-lethal hit spark.
    HitSpark,
    /// `$36978`: lethal hit spark; also list-1 type `$21`, which a shot that
    /// hits the player leaves (`$36900`).
    KillSpark,
    /// `$36948` (list-1 type 1): the spark `$368EC` leaves, with effect 8,
    /// when the guard blocks a shot or the shield absorbs it.
    DeflectSpark,
    /// `$3690C` (list-1 type 5): a projectile striking solid terrain.
    WallSpark,
    /// `$151EE`: falling leaf announcing a hidden enemy.
    Leaf,
    /// `$36794`: radial sparkle when a type-$44 leaves.
    Poof,
    /// `Func_031A7A`: list-1 `$1B`, placed 16 px from a point and flying
    /// back into it while animating through `Data_03051A`.
    ChargeSparkle,
    /// A handler-specific piece with its own mapping (e.g. the `$2C`
    /// wreckage of `$1625E`): falls under its motion, optionally bouncing,
    /// for `counter` ticks.
    Pieces,
    /// `$37D1C`: one section of Boss7's growing MIJIN pillar, driven by
    /// its boss until the release signal drains it upward.
    Boss7Pillar,
    /// `$37944`: Karura's seven animated death sparks.
    KaruraSpark,
    /// `$383E6`: Karura's shed feathers, fluttering before settling.
    KaruraFeather,
    /// `$37E48`: spark/puff animation emitted by Karura's falling body.
    KaruraTrail,
    /// Muzzle flash (list-1 type `$19`, `Data_0303DE` frames `$36D..$36F`).
    Flash,
    /// `$9210` (list-1 type `$32`): a `$916C` leaf (1-1) or flake (7-1).
    AmbientLeaf,
    /// $38EEE: kabuki-room petal, swaying 40 px while falling.
    Petal,
    /// `$92BE` (list-1 type `$2E`): a `$9244` water drip (1-2).
    Drip,
    /// `$93F0` (list-9 type `$34`): a `$940C` pebble (6-1).
    Pebble,
    /// `$22D2E` (list-1 type `$24`): a material-15 splash.
    Splash,
    /// `$22D4E` (list-1 or list-9 type `$24`, routine `$10`): a wake
    /// splash drifting left with the water (`combat::ambient::wake`).
    Wake,
    /// `$22E34` (list-1 type `$24`, routine 8): a plume of water thrown up
    /// where the 4-3 gunboat sinks (`combat::ambient::plume`).
    Plume,
    /// `Func_014592` (list-9 type `$3C`): the 4-1 end guard's intro
    /// jet-skis and glide flame, driven by the guard's own update
    /// (`enemies::round4::escort`), not by `Ctx::update_effects`.
    GuardPart,
    /// `Func_026CCE` (list-9 type `$2B`, routine `$7C`): the rider thrown
    /// off in a surf death, driven by `special::thrown` after each death
    /// update, not by `Ctx::update_effects`, and drawn by `Game::draw`.
    ThrownRider,
    /// Mounted horse wreck, driven and drawn by special::thrown.
    HorseWreck,
    /// Powered surf-slash board, driven and drawn by the mounted player.
    SurfSlashBoard,
    /// `$15858` (list-9 type `$35`): the 2-1 guard's hurt pose flying
    /// off, its palette line toggled (`bchg #6`) on every update, for
    /// `counter` more updates.
    Ghost,
    /// `$1C36A` (list-1 `$A0`, routine `$30`): a boulder of the wall the
    /// 1-1 samurai leader bursts through (`enemies::forest::props`).
    Boulder,
    /// `$1C446` (list-1 `$A0`, routine `$38`): a blinking chip of a boulder
    /// that met the ground.
    Chip,
    /// `$1C4EA` (list-1 `$A0`, routine `$34`): a half of the rock gate that
    /// drops behind the player at the 1-1 leader's arena.
    Gate,
    /// `$262D6` (list-1 type `$2B` routine `$0C`, Boss2's charge): a spark
    /// circling `origin` at radius `counter` + 8, `angle` advancing by
    /// `phase` a tick; the radius and `counter` shrink by one each tick,
    /// the mapping steps down every eighth, and it ends at `counter` 0.
    Orbit,
    /// Boss4's list-9 sparks around the flying machine's spread launcher.
    Boss4Charge,
    /// Boss4's attached list-4 spread launcher, driven by the boss update.
    Boss4Launcher,
    /// `$26386` (list-1 type `$2B` routine `$1C`, Boss2's hurt burst) and
    /// `$26274` (routine 0, a shot-down orb's puff): a frame every `period`
    /// ticks for `phase` frames, the H/V flips (`angle`) rotating on each
    /// even count of `counter`.
    Flare,
    /// A list-1 or list-9 record a boss runs from its own update (Boss4's
    /// type-`$2B` routine-`$68` parts, `bosses::boss4::parts`): the effect
    /// pass leaves it alone.
    BossPart,
    /// Boss5's `$31962` smoke or `$31A54` trail, with its ROM attribute sequence.
    MechParticle,
}
/// Short-lived effect objects from the `$16F0` list (dispatch `$365A2`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    pub kind: EffectKind,
    pub x: i32,
    pub y: i32,
    pub mapping: u16,
    pub flip: bool,
    pub(crate) base: u16,
    pub(crate) motion: Motion,
    /// The list-1/9 pass that gives it its first update, counting the
    /// allocating frame's as 1 (0 is that one too): a record a controller
    /// allocates on a later frame. Hidden until then.
    pub(crate) delay: u16,
    pub(crate) age: u16,
    pub(crate) phase: u8,
    pub(crate) counter: i32,
    pub(crate) angle: u16,
    pub(crate) alive: bool,
    /// Drawn only on even ages (a blinking piece).
    pub(crate) blink: bool,
    /// A list-9 record, drawn behind every other object (see [`Enemy::back`]).
    pub(crate) back: bool,
    /// The sprite list of a record that is in neither list 1 nor list 9
    /// ([`Effect::sprite_list`]): a part drawn for a list-4 or list-6 record.
    pub(crate) list: SpriteList,
    /// Linked at the head of its list rather than the tail: `Func_02BF68`
    /// (Boss2's afterimages) puts the record before every other list-9
    /// record, where `Object_AllocFromList` appends it.
    pub(crate) head: bool,
    /// The point an [`EffectKind::Orbit`] circles (its parent's position
    /// plus the handler's offset).
    pub(crate) origin: (i32, i32),
    /// Ticks per frame of a flare, or a puff (0 keeps the shared two-tick rate).
    pub(crate) period: u16,
    /// The sprite's priority bit: the piece stays over the high-priority
    /// plane cells painted after it (the 4-3 wake splashes). Opt-in; other
    /// effects keep drawing low.
    pub(crate) high_priority: bool,
    /// `obj_flags` of an [`EffectKind::BossPart`]: bit 11 keeps it out of
    /// the sprite table (`Objects_BuildSprites` skips `$880`), bit 3 marks
    /// a destroyed part.
    pub(crate) flags: u16,
    /// `obj_tile` bit 15 of a record whose pieces carry their own priority:
    /// `Object_DrawSprite` (`$156E`) flips each piece's by it (the
    /// thrown rider's plumes, `combat::ambient::pending_plume`).
    pub(crate) tile_priority: bool,
}
impl Effect {
    /// A priority sprite of the ambient spawners (the 1-1 leaf, `$E106`):
    /// drawn over the high-priority plane rather than behind it.
    pub(crate) fn in_front(&self) -> bool {
        super::ambient::owns(self.kind) && self.base & 0x8000 != 0
    }
    /// The object list the record is in: list 9 when `back`, else `list`
    /// (list 1 unless a part is drawn for a record of another list).
    pub(crate) fn sprite_list(&self) -> SpriteList {
        if self.back {
            SpriteList::Back
        } else {
            self.list
        }
    }
    pub(crate) fn new(kind: EffectKind, x: i32, y: i32, mapping: u16) -> Self {
        Self {
            kind,
            x,
            y,
            mapping,
            flip: false,
            base: 0,
            motion: Motion::default(),
            delay: 0,
            age: 0,
            phase: 0,
            counter: 0,
            angle: 0,
            alive: true,
            blink: false,
            back: false,
            list: SpriteList::Effects,
            head: false,
            origin: (0, 0),
            period: 0,
            high_priority: false,
            flags: 0,
            tile_priority: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnemyState {
    Dormant,
    Emerging,
    Waiting,
    Throwing,
    Recovering,
    /// Jumping, dropping or leaping under gravity (types $14 and $44).
    Airborne,
    Hurt,
    /// No longer hittable. While `Enemy::dying()` its original
    /// death sequence is still drawn; afterwards the object is removed.
    Defeated,
}
/// Native stand-in for an original `$54`-byte object record (`obj_*` in
/// `spawn` is the `Spawn_Records` slot that `Spawn_ActivateNearby` activated
/// (`spawn_flags` bit 15), or a synthetic record for a child or boss;
/// `x`/`y` are the integer words of `obj_x`/`obj_y`; `state` is the native
/// life cycle (the forest set also uses it for `obj_routine`); `routine` is
/// `obj_routine` of ROM-style handlers (`crate::enemies`); `mapping` is
/// `obj_frame` (`Sprite_Mappings` index); `facing_left` is the horizontal
/// flip bit ($800) of the object's base attribute (+8): mappings and
/// hitboxes mirror while it is set. Each handler derives it differently
/// `obj_health` + 1; `timer` plays the role of `obj_timer`; `vars` are the
/// handler scratch words. `age` and `attack_serial` are native bookkeeping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enemy {
    pub spawn: usize,
    pub x: i32,
    pub y: i32,
    pub state: EnemyState,
    pub mapping: u16,
    pub facing_left: bool,
    /// `obj_tile` bit 15, initially copied from spawn flag bit 14 and copied
    /// from a parent into allocated children.
    pub(crate) high_priority: bool,
    pub health: u16,
    pub(crate) timer: u16,
    pub(crate) age: u64,
    pub(crate) attack_serial: u64,
    /// A player weapon collided with it in this tick's weapon pass
    /// (`Object_CollideL3` found an overlap), so `Object_ProcessHit` skips
    /// its player test (`$1285C`). Native bookkeeping, cleared every tick.
    pub(crate) struck: bool,
    /// A player attack's overlap found by `Combat::update_attacks`, which
    /// lands on the next object pass, after the object's update.
    pub(crate) strike: Option<Strike>,
    pub(crate) motion: Motion,
    pub(crate) phase: u8,
    pub(crate) counter: u16,
    pub(crate) home_x: i32,
    pub(crate) launch_y: i32,
    pub(crate) step_x: i32,
    pub(crate) vulnerable: bool,
    pub(crate) flash: u16,
    pub(crate) dying: bool,
    pub(crate) reaction: Option<Reaction>,
    /// The weapon pass emits the spark when damage lands, before the death routine.
    pub(crate) spark_handled: bool,
    pub(crate) hit_spark: Option<((i32, i32), bool)>,
    /// Whether the fatal weapon leaves `$36900`. `Func_0128E4` and
    /// attacker types `$50` and above suppress it.
    pub(crate) death_spark: bool,
    /// Decimal points waiting for the handler's death update
    /// (`Func_0129C0`), cleared there even when scoring is suppressed.
    pub(crate) pending_score: u32,
    /// Health the hits recorded since the handler's last update took
    /// (`Combat::hurt_enemy`); see [`Enemy::health_before_hit`].
    pub(crate) damage_taken: u16,
    /// `obj_routine` (`+$04`) of a ROM-style handler; 0 = uninitialised.
    pub(crate) routine: u16,
    /// `obj_saved_routine` (`+$44`), the routine a hit interrupted.
    pub(crate) saved_routine: u16,
    /// `obj_hurt_routine` (`+$46`) and `obj_death_routine` (`+$36`).
    pub(crate) hurt_routine: u16,
    pub(crate) death_routine: u16,
    /// Handler scratch (`obj_var*`); each handler documents its indices.
    pub(crate) vars: [i32; enemies::VARS],
    /// `obj_flags` bit 7 after the last integration (`Object_UpdateVisibility`).
    pub(crate) offscreen: bool,
    /// `$3C` parent: index into `Combat::enemies` of the allocating object.
    pub(crate) parent: Option<usize>,
    /// Base attribute (`obj_tile` without the flip) when the handler sets
    /// one other than its type's `Object_GfxBases` entry (children copy
    /// their parent's).
    pub(crate) tile: Option<u16>,
    /// Collision box relative to (x, y) when not taken from the mapping.
    pub(crate) body: Option<Hitbox>,
    /// An armored region relative to (x, y) with its own `Object_CollideL3`
    /// apart from the body's, which deflects player shurikens
    /// ([`crate::enemies::HitResponse::Rebound`]): Boss4's hull while a
    /// weapon part takes the hits (`$2DA58`).
    pub(crate) armor: Option<Hitbox>,
    /// The armor's (or a rebounding body's) collision of this object update
    /// is spent: `Object_CollideList` stops at the first overlap.
    pub(crate) armor_struck: bool,
    /// Drawn as a list-9 record, such as the 2-1 kite that the `$08`
    /// spawner stands in for. `Objects_BuildSprites` (`$1488`) walks
    /// lists 1..9 in order, so these sprites sit behind every other object.
    pub(crate) back: bool,
    /// Allocation serial: `Object_AllocFromList` links each new record at
    /// the tail of its list and `Objects_BuildSprites` writes the lists in
    /// link order, so earlier allocations cover later ones. 0 = never live.
    pub(crate) order: u64,
}
/// A player attack meeting an enemy: its damage and hit sound. A deflected
/// shuriken makes no hit sound (`$24DE0`); a sword's plays anyway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Strike {
    /// Attacker `obj_x`/`obj_y`, copied by `$36918` for the hit effect.
    pub(crate) origin: (i32, i32),
    pub(crate) damage: u8,
    pub(crate) cue: u8,
    pub(crate) shuriken: bool,
    pub(crate) board: bool,
    /// The air stab: landing sets the player's `+$40` (`$252EA`).
    pub(crate) bounce: bool,
    /// The attack object held until the hit lands (`Combat::land_strikes`):
    /// in the ROM it stays in list 3 until its own next update sees the
    /// collision bit the object's `Object_CollideL3` set, and frees itself.
    pub(crate) hold: Option<u32>,
    /// The sword's native attack serial, recorded on the object when the
    /// hit lands ([`Enemy::attack_serial`]); 0 for shurikens.
    pub(crate) serial: u64,
    /// The shuriken's deflection flag (`$0100`) is set as well: its update
    /// takes `Func_024AFE`'s branch before the hit test (`loc_024DD0`), so
    /// it plays the rebound sound, not its hit voice (`$24DE0`).
    pub(crate) rebound: bool,
}

impl Enemy {
    pub(crate) fn new(spawn: usize, x: i32, y: i32, mapping: u16, health: u16) -> Self {
        Self {
            spawn,
            x,
            y,
            state: EnemyState::Dormant,
            mapping,
            facing_left: false,
            high_priority: false,
            health,
            timer: 0,
            age: 0,
            attack_serial: 0,
            struck: false,
            strike: None,
            motion: Motion::default(),
            phase: 0,
            counter: 0,
            home_x: x,
            launch_y: y,
            step_x: 0,
            vulnerable: false,
            flash: 0,
            dying: false,
            reaction: None,
            spark_handled: false,
            hit_spark: None,
            death_spark: true,
            pending_score: 0,
            damage_taken: 0,
            routine: 0,
            saved_routine: 0,
            hurt_routine: 0,
            death_routine: 0,
            vars: [0; enemies::VARS],
            offscreen: false,
            parent: None,
            tile: None,
            body: None,
            armor: None,
            armor_struck: false,
            back: false,
            order: 0,
        }
    }
    /// `health` before the hits recorded since the handler's last update:
    /// ROM handlers subtract `6(a5)` themselves and some read the old value
    /// first (Boss2's `Func_02BF46`, Boss5's `Func_030D78`).
    pub(crate) fn health_before_hit(&self) -> u16 {
        self.health.saturating_add(self.damage_taken)
    }
    /// `obj_tile` bit 12, the vertical flip `Object_LoadHitbox` (`$18F4`)
    /// negates the box's Y offset by. Only a handler's own `tile` carries
    /// it: no stage's `Object_GfxBases` base reaches bits 11 or 12.
    pub(crate) fn vertical_flip(&self) -> bool {
        self.tile.is_some_and(|tile| tile & 0x1000 != 0)
    }
    /// Defeated but still playing its death sequence before removal.
    pub fn dying(&self) -> bool {
        self.dying
    }
    /// Remaining ticks of the white palette flash ($1BEC4).
    pub fn flash(&self) -> u16 {
        self.flash
    }
    /// In the object list: activated and not yet removed.
    pub fn live(&self) -> bool {
        match self.state {
            EnemyState::Dormant => false,
            EnemyState::Defeated => self.dying,
            _ => true,
        }
    }
}
/// `Func_024BE0` (shuriken routine `$10`): Boss2's field turns a player
/// shuriken away from the boss, one of `Data_024BA8`'s 28 directions every
/// other update, four times, then routine `$14` flies it on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn {
    /// `+$28`: the direction index.
    pub(crate) direction: u8,
    /// `+$2A`: +1 or -1, away from the boss.
    pub(crate) step: i8,
    /// `+$38`: turns left.
    pub(crate) left: u8,
}
impl Turn {
    /// `$12984..$129B6`: the 4-1 guard's raised shield starts four
    /// steering updates; `Frame_Counter & 3` picks +1, -1, -1 or -3.
    pub(crate) fn guard(direction: u8, tick: u64) -> Self {
        Self {
            direction,
            step: -((tick & 3) as i8) | 1,
            left: 4,
        }
    }
    /// `$2B17A..$2B196`: the field at `field_y` meets a shuriken at `y`
    /// heading along `direction` (`Shuriken_Init` leaves 0 right, `$E` left).
    pub(crate) fn away(direction: u8, y: i32, field_y: i32) -> Self {
        Self {
            direction,
            step: if y + 0x18 - field_y < 0 { -1 } else { 1 },
            left: 4,
        }
    }
    /// One turn: the new direction's `Data_024BA8` word.
    pub(crate) fn advance(&mut self, frames: &[u16; 28]) -> u16 {
        let next = i32::from(self.direction) + i32::from(self.step);
        self.direction = if next < 0 {
            27
        } else if next >= 28 {
            0
        } else {
            next as u8
        };
        self.left = self.left.saturating_sub(1);
        frames[usize::from(self.direction)]
    }
}

/// `$1D144` (list-4 `$03`, the 5-x grenade flames), after its box load:
/// every update adds the X speed `$36` to the offset `$32` and rewrites the
/// X word to `$2E` plus its whole pixels, minus on odd `$28` (the handler's
/// count, [`Projectile::age`]), and turns `obj_tile` bit 11, so one flame
/// is drawn on each side in turn, mirrored on the left; `Object_Integrate`
/// then adds the same speed, the flame's velocity, with the X fraction the
/// rewrite keeps. Every fourth update shows the next frame of
/// `Data_01D0D4`, and the one after the last frees the flame: mapping 0
/// (VRAM tile 0, blank) and its point box for the first three updates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sway {
    /// `$2E`: the X it sways about.
    pub(crate) base: i32,
    /// `$32`: the 16.16 offset.
    pub(crate) offset: i32,
    pub(crate) frames: Vec<u16>,
}
impl Sway {
    /// One update of `$1D182`; false when the flame frees itself.
    pub(crate) fn update(&mut self, shot: &mut Projectile) -> bool {
        self.offset = self.offset.wrapping_add(shot.motion.vx);
        let odd = shot.age % 2 == 1;
        let offset = self.offset >> 16;
        shot.x = self.base + if odd { -offset } else { offset };
        shot.faces_left = shot.leftward() != odd;
        if shot.age.is_multiple_of(4) {
            match self.frames.get(usize::from(shot.age / 4 - 1)) {
                Some(&frame) => shot.mapping = frame,
                None => return false,
            }
        }
        true
    }
}

/// Where a list-4 handler's `obj_hitbox` comes from, which is the box
/// `Player_CheckDamage` (`$21C30`) and the handler's own tests meet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ShotBox {
    /// `Object_LoadHitbox` (`$18F4`, through `$1DD96`) at the top of the
    /// update, before the routine advances the frame or turns the flips:
    /// the box of the frame shown after the previous update, one update
    /// behind the one drawn. Most handlers.
    #[default]
    Before,
    /// `Object_LoadHitbox` after the routine set this update's frame (the
    /// sickle's `$1BA44`, Boss5's `$3060A`, the battery's `$1B650`), or no
    /// load at all, where the box left in the record is stale (the
    /// commando's `$1DC4A`): the frame drawn.
    After,
    /// The handler writes `obj_hitbox` itself, or loads it only once, and
    /// the box stays: centre offset and half sizes, never flipped.
    Fixed(Hitbox),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MechShot {
    Flame,
    Orb {
        index: u8,
        launched: Option<u16>,
        hit: bool,
    },
    Rock {
        delay: u16,
        bounced: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projectile {
    pub(crate) mech: Option<MechShot>,
    pub x: i32,
    pub y: i32,
    /// Whole pixels per tick, moved in one step.
    pub velocity_x: i32,
    pub enemy: bool,
    pub mapping: u16,
    pub damage: u8,
    /// `obj_tile` bit 15, copied from the allocating object (or Joe for a
    /// player weapon).
    pub(crate) high_priority: bool,
    pub(crate) age: u16,
    /// Star handler $1D1D6 increments the recycled record's uncleared +$28 word.
    pub(crate) animation_counter: u16,
    /// Allocated through the list-4 pool, whose scratch survives removal.
    pub(crate) scratch_slot: bool,
    /// Visibility from the previous sprite pass; a new record starts visible.
    pub(crate) offscreen: bool,
    pub(crate) alive: bool,
    /// Further 16.16 motion applied after `velocity_x` (aimed or falling shots).
    pub(crate) motion: Motion,
    /// Base attribute of the art (0 = resident projectile art).
    pub(crate) base: u16,
    /// The art faces left, so it is mirrored while moving right.
    pub(crate) faces_left: bool,
    /// Removed after this many ticks; `u16::MAX` means the handler has no
    /// fixed lifetime and relies on `Object_UpdateVisibility`.
    pub(crate) life: u16,
    /// The handler has no terrain test (e.g. `$1CE0E` users, the `$1A8CC` chain).
    pub(crate) passes_terrain: bool,
    /// The handler skips `Object_UpdateVisibility` and frees itself only
    /// once left of the camera (`Func_01776E`'s `cmp.w Camera_X`, list-4
    /// `$2C` bombs, mines and debris, which may start right of the view).
    pub(crate) left_edge_only: bool,
    /// `Func_01776E` (list-4 `$2C` bombs, mines and debris): the player's
    /// collision bit makes the object's own update explode it (`$36D9A`)
    /// and free it, instead of the `$368EC`/`$36900` spark of other shots.
    pub(crate) contact_blast: Option<Blast>,
    /// `Combat::enemies` index of the throwing object, when it tracks its shot.
    pub(crate) owner: Option<usize>,
    /// Returns to this X and is removed there once its acceleration has
    /// turned it around (the `$1A8CC` kusarigama chain); hitting the player
    /// spends its damage and reverses it instead of removing it.
    pub(crate) home: Option<i32>,
    /// Four (mapping, horizontal flip) frames selected by `(age >> 2) & 3`.
    pub(crate) frames: Option<[(u16, bool); 4]>,
    /// The grenade flames' side-to-side X and frames ([`Sway`]).
    pub(crate) sway: Option<Sway>,
    /// An enemy shot's box overlaps the player's body where this tick's
    /// step ended; `Combat::check_player_damage` decides whether it counts.
    pub(crate) touching: bool,
    /// Boss7's persistent `$1A` aura: variant and the collision bit left
    /// by Player_CheckDamage (including a blocked or shielded hit).
    pub(crate) boss7_aura: Option<(u8, bool)>,
    /// An expired synthetic list-4 hitbox that must survive the intervening
    /// stage-object pass until the next player damage check.
    pub(crate) keep_contact: bool,
    /// `obj_flags` bit 13: no further collision (the `$1A8CC` chain after
    /// its hit).
    pub(crate) no_collision: bool,
    /// Where the handler's box comes from ([`ShotBox`]).
    pub(crate) box_load: ShotBox,
    /// The frame and `obj_tile` flips the record holds for the next
    /// update's `Object_LoadHitbox`: what the last update left drawn, or
    /// before the first update what the allocator left (`None`: as the
    /// shot was built; frame 0 where the spawner leaves `obj_frame` clear
    /// and the handler sets it after loading, [`Self::unset_frame`]).
    pub(crate) held_frame: Option<(u16, bool, bool)>,
    /// The frame and flips of the box the last update loaded (`None`
    /// before the first update: the frame drawn).
    pub(crate) box_frame: Option<(u16, bool, bool)>,
    /// An enemy shot whose step ended in a wall: its next update frees it
    /// with the `$3690C` spark before it moves (`$1CDE4`).
    pub(crate) walled: bool,
    /// The handler's one-shot cue when the pending terrain impact runs
    /// (Boss6's flagged needle at `$1E754`).
    pub(crate) wall_sound: Option<u8>,
    /// A native stand-in for a handler that sets the player's bit 14 itself
    /// (`Object_CollidePlayer`): its touch is body contact, not a shot.
    pub(crate) body_contact: bool,
    /// A player shuriken's `+$2D`: the previous `Func_024AB0` result was
    /// the splash material, so staying in it does not splash again.
    pub(crate) in_water: bool,
    /// A player shuriken Boss2's field turned ([`Turn`]); once turned it
    /// is never turned again.
    pub(crate) turn: Option<Turn>,
    /// The handler clears its collision bit before reading it (Boss2's
    /// orbs, `$25F90`): touching the player neither frees it nor leaves a
    /// spark, though the player takes its damage.
    pub(crate) stays: bool,
    /// On touching the player the handler leaves `Player_SpawnDust` at its
    /// own place instead of a spark (Boss2's shots, `$25E80`).
    pub(crate) contact_dust: bool,
    /// Its own `Object_CollideL3` lets a player weapon destroy it.
    pub(crate) shot_down: Option<ShotDown>,
    /// Boss4's stored 2048-step missile heading ($34), retained at launch.
    pub(crate) missile_heading: Option<u16>,
    /// $19C2E: a launched dart reverses, drops and becomes harmless on impact.
    pub(crate) dart: bool,
    /// Period and mapping of a fading projectile trail (0 uses puff art).
    pub(crate) trail: Option<(u16, u16)>,
    /// `$1AE84`: stationary, high-priority list-9 sparkles every other
    /// shot update, with (mapping, ticks) runs from `$1B10A`.
    pub(crate) sparkle_trail: Option<Vec<(u16, u16)>>,
    /// Boss4's pre-integration water contact: true leaves a wake.
    pub(crate) water: Option<bool>,
    /// Period of a projectile's alternating vertical tile flip.
    pub(crate) flip_period: Option<u16>,
    /// Spawn the shot-down puff on player contact as well (Boss4 missiles).
    pub(crate) contact_puff: bool,
    /// A resting Boss2 orb's centre line (`$2A`) and bob angle (`$2C`).
    pub(crate) rest: Option<i32>,
    pub(crate) angle: u16,
    /// A player shuriken that met an enemy, waiting for the hit to land
    /// ([`Strike::hold`]): drawn where it met it, neither moved nor tested.
    pub(crate) held: Option<u32>,
    /// Its held hit did not land: `Shuriken_Update` (`loc_024DD0`) found
    /// no collision bit and still tests the terrain where the step ended,
    /// before the shuriken moves on.
    pub(crate) untested: bool,
    /// An armored object set its deflection flag (`$0100`, [`Strike::rebound`]):
    /// its next update enters `Func_024AFE` (`loc_024DD0`) where it stands.
    pub(crate) deflected: bool,
    /// A native collision stand-in with no sprite of its own (Boss7's body
    /// box, whose mapping is the boss's own frame): not drawn, so it takes
    /// no place in the sprite table.
    pub(crate) hidden: bool,
    /// Allocation serial of a player shuriken (`Combat::allocate`): list 3
    /// is drawn in allocation order across the native containers.
    pub(crate) order: u64,
}

/// What a player weapon leaves of a shootable enemy object: Boss2's
/// `$2603A` orb flare, or Boss4's `$2F3B4` missile smoke. The attack's
/// collision response is handled separately (including the air-stab rebound).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShotDown {
    /// A projectile whose handler uses the shared explosion ($1B4AE).
    pub(crate) blast: Option<Blast>,
    pub(crate) first: u16,
    pub(crate) frames: u8,
    pub(crate) period: u16,
    pub(crate) sound: Option<u8>,
    /// `$2F3B4`: (tile, vertical speed, gravity) for type-$1E smoke
    /// instead of the orb's spinning flare.
    pub(crate) smoke: Option<(u16, i32, i32)>,
}
impl ShotDown {
    /// The puff at (x, y). `$2623A` reads the random word `$FF001C` without
    /// advancing it and ORs `$8080`: both bytes negative, each / 64 px a
    /// tick, up and to the left.
    pub(crate) fn puff(self, (x, y): (i32, i32), random: u16) -> Effect {
        if let Some((tile, rise, gravity)) = self.smoke {
            let mut smoke = Effect::new(EffectKind::Debris, x, y, self.first);
            smoke.base = tile & !0x800;
            smoke.flip = tile & 0x800 != 0;
            smoke.high_priority = tile & 0x8000 != 0;
            smoke.back = random & 0x100 != 0;
            smoke.motion = Motion {
                vx: crate::special::WATER_FLOW << 16,
                vy: rise << 16,
                gravity,
                ..Motion::default()
            };
            smoke.age = 1;
            return smoke;
        }
        let r = random | 0x8080;
        let mut puff = Effect::new(EffectKind::Flare, x, y, self.first);
        puff.motion = Motion {
            vx: i32::from(r as u8 as i8) << 10,
            vy: i32::from((r >> 8) as u8 as i8) << 10,
            ..Motion::default()
        };
        puff.period = self.period;
        puff.counter = i32::from(self.period);
        puff.phase = self.frames;
        puff
    }
}
impl Projectile {
    pub(crate) fn bounce_dart(&mut self) -> bool {
        if !self.dart || self.damage == 0 {
            return false;
        }
        let flip = self.tile_flips().0;
        self.dart = false;
        self.mapping += 1;
        self.motion.vx = -((self.velocity_x << 16) + self.motion.vx) >> 2;
        self.velocity_x = 0;
        // $19C2E reverses motion without changing obj_tile: the broken
        // arrow keeps pointing in its launch direction while falling back.
        self.faces_left = self.leftward() != flip;
        self.motion.gravity = 0x3400;
        self.life = self.age.saturating_add(16);
        self.damage = 0;
        self.no_collision = true;
        self.passes_terrain = true;
        self.stays = true;
        true
    }
    pub(crate) fn new(
        x: i32,
        y: i32,
        velocity_x: i32,
        enemy: bool,
        mapping: u16,
        damage: u8,
    ) -> Self {
        Self {
            x,
            y,
            velocity_x,
            mech: None,
            enemy,
            mapping,
            damage,
            high_priority: false,
            age: 0,
            animation_counter: 0,
            scratch_slot: false,
            offscreen: false,
            alive: true,
            motion: Motion::default(),
            base: 0,
            faces_left: false,
            life: u16::MAX,
            passes_terrain: false,
            wall_sound: None,
            left_edge_only: false,
            contact_blast: None,
            owner: None,
            home: None,
            frames: None,
            sway: None,
            touching: false,
            boss7_aura: None,
            keep_contact: false,
            no_collision: false,
            box_load: ShotBox::Before,
            held_frame: None,
            box_frame: None,
            walled: false,
            body_contact: false,
            in_water: false,
            turn: None,
            stays: false,
            contact_dust: false,
            shot_down: None,
            missile_heading: None,
            dart: false,
            trail: None,
            sparkle_trail: None,
            water: None,
            flip_period: None,
            contact_puff: false,
            rest: None,
            angle: 0,
            held: None,
            untested: false,
            deflected: false,
            hidden: false,
            order: 0,
        }
    }
    pub(crate) fn with_art(mut self, base: u16, faces_left: bool) -> Self {
        self.base = base;
        self.faces_left = faces_left;
        self
    }
    pub(crate) fn with_motion(mut self, motion: Motion) -> Self {
        self.motion = motion;
        self
    }
    /// Moving left (hitboxes mirror).
    pub(crate) fn leftward(&self) -> bool {
        self.velocity_x < 0 || (self.velocity_x == 0 && self.motion.vx < 0)
    }
    /// The shot's `obj_tile` flip bits 11 and 12, which `Object_LoadHitbox`
    /// applies to its box: the flips it is drawn with. For a player
    /// shuriken the throw's facing (`Shuriken_Init` copies the player's) or
    /// the pose flips `Func_024BE0` last turned it to; for an enemy shot
    /// its direction mirrored for art that faces left, a frame's own flip
    /// ([`Self::frames`]) or the `$1000` of its `base`.
    pub(crate) fn tile_flips(&self) -> (bool, bool) {
        (self.leftward() != self.faces_left, self.base & 0x1000 != 0)
    }
    /// The frame drawn and its `obj_tile` flips.
    pub(crate) fn drawn_frame(&self) -> (u16, bool, bool) {
        let (horizontal, vertical) = self.tile_flips();
        (self.mapping, horizontal, vertical)
    }
    /// The spawner leaves `obj_frame` 0 and the handler's first routine
    /// sets the frame after `Object_LoadHitbox`: the first update's box is
    /// mapping 0's (`0,0,0,0`), a point.
    pub(crate) fn unset_frame(mut self) -> Self {
        self.held_frame = Some((0, false, false));
        self
    }
    /// The handler loads its box after setting the frame ([`ShotBox::After`]).
    pub(crate) fn box_after(mut self) -> Self {
        self.box_load = ShotBox::After;
        self
    }
}
