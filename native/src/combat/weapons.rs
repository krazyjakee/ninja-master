//! Player weapons (issue #42): sword objects, the somersault spread, guard,
//! the recovered handlers.
mod attacks;
mod draw;
mod update;
pub(crate) use attacks::{AttackObject, PlayerAttack};

use super::{
    Combat, CombatEvents, EnemyState, Hitbox, Mapping, Motion, Projectile, Strike, Turn,
    enemy_hitbox, mapping_hitbox,
};
use crate::{
    enemies::HitResponse,
    game::Terrain,
    movement::{self, Attack, AttackPosture},
    render::{Frame, Lines, Tile},
    rom::{Result, word},
    sound_events::SoundEvent,
};
use std::collections::BTreeMap;

/// [`PlayerWeapons::steer`] from `Data_024BA8` and the `Data_024E92` pairs.
fn steer((frames, spread): &([u16; 28], [(i32, i32); 8]), turn: &mut Turn) -> (u16, u16, i32, i32) {
    let word = turn.advance(frames);
    let frame = word & 7;
    let (x, y) = spread[usize::from(frame ^ 7)];
    let flips = word & 0x1800;
    let vx = if flips & 0x800 != 0 { -x } else { x };
    let vy = if flips & 0x1000 != 0 { -y } else { y };
    (0xca + frame, flips, vx, vy)
}

/// A `Player_FindNearestEnemy` candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AttackTarget {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) high_priority: bool,
}

/// Sword objects created by `$23314`/`$2331A` through the `$249A6` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sword {
    /// Object 9 (`$2511A`): slash 5 and the combo's second cut in state 24.
    Standing,
    /// Object 11 (`$251AE`): crouch slash 12.
    Crouching,
    /// Object 13 (`$25244`): air slash 18 and the wall slash 37.
    Airborne,
    /// Object 15 (`$252C4`): air stab 20.
    AirStab,
    /// Object 8 (`$250B6`): surfboard kick in state 60.
    SurfStab,
    /// Object 19 (`$254B4`): dash kick 29.
    DashKick,
    /// Object 17 (`$25434`): hang slash 34.
    Hanging,
    /// The powered swords (`Player_StartPoweredSlash`, `$2331A`): damage 3
    /// and effect `$1A`. Object 10 (`$2516E`): slash 6.
    StandingPowered,
    /// Object 12 (`$25204`): crouch slash 13.
    CrouchingPowered,
    /// Object 14 (`$25284`): air slash 19 and wall slash 38.
    AirbornePowered,
    /// Object 18 (`$25474`): hang slash 35.
    HangingPowered,
    /// The mounted swords (`$230F6`, `$23142`). Object 3 (`$24F48`): horse
    /// slash 52.
    Horse,
    /// Object 4 (`$24F88`): powered horse slash 55.
    HorsePowered,
    /// Object 5 (`$24FC8`): surf slash 61.
    Surf,
    /// Object 6 (`$25008`): powered surf slash 63.
    SurfPowered,
}
/// The player as list 3 reads it, before `Object_Integrate` (in
/// `Objects_BuildSprites`) moves it this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlayerAnchors {
    /// `obj_x`/`obj_y` ([`crate::movement::Player::unmoved_position`]).
    pub unmoved: (i32, i32),
    /// `obj_x + obj_vel_x`, `obj_y + obj_vel_y`
    /// ([`crate::movement::Player::stepped_position`]).
    pub stepped: (i32, i32),
}
pub(super) struct SwordSpec {
    /// +$2A/+$2C offset from the player, mirrored with the facing.
    offset: (i32, i32),
    /// Mapping whose `$43222` hitbox is active.
    mapping: u16,
    /// Ticks after creation whose `(+$3A >> 2)` entry holds the mapping.
    active: std::ops::RangeInclusive<u16>,
    /// Initial +$3A lifetime.
    life: u16,
    /// +$06 damage byte (`d3` at `$23314`/`$2331A`).
    pub(super) damage: u8,
    /// The update after creation whose mapping word has bit 15 set at
    /// `+$3A & 3 == 3`: the swing sound plays then (`$24CEC`, `$25530`).
    sound_at: u16,
    /// Player states that keep the object alive (`$25158`, `$24C86`).
    states: &'static [usize],
}
impl Sword {
    fn hit_sound(self) -> SoundEvent {
        match self {
            Self::Crouching => SoundEvent::CrouchHit,
            Self::AirStab | Self::SurfStab => SoundEvent::AirStabHit,
            Self::DashKick => SoundEvent::DashKickHit,
            _ => SoundEvent::SlashHit,
        }
    }

    pub(super) fn spec(self) -> SwordSpec {
        use movement::state::*;
        let ordinary = SwordSpec {
            offset: (12, -48),
            mapping: 0xd2,
            active: 4..=11,
            life: 15,
            damage: 1,
            sound_at: 4,
            states: &[SLASH, COMBO],
        };
        let powered = SwordSpec {
            offset: (28, -8),
            mapping: 0xd3,
            active: 8..=11,
            life: 15,
            damage: 3,
            sound_at: 8,
            states: &[SLASH_POWERED],
        };
        match self {
            // $2512E..$25150: (12,-48), life 15, words [0,$D2,$80D2,0].
            Self::Standing => ordinary,
            // $251C2..$251E6: (12,-16), life 15, words [0,0,$80D2,0].
            Self::Crouching => SwordSpec {
                offset: (12, -16),
                active: 4..=7,
                states: &[CROUCH_SLASH],
                ..ordinary
            },
            // $25258..$2527C: (16,-60), life 15, words [0,$D2,$80D2,0].
            Self::Airborne => SwordSpec {
                offset: (16, -60),
                states: &[AIR_SLASH, WALL_SLASH],
                ..ordinary
            },
            // $252D8, $2530E..$2536A: 12 ahead, Y-8, words [0,0,$D5,0];
            // the dive in state 20 keeps +$3A at 11 ($25332..$25342).
            Self::AirStab => SwordSpec {
                offset: (12, -8),
                mapping: 0xd5,
                active: 4..=7,
                states: &[AIR_STAB, HORSE_STAB],
                ..ordinary
            },
            // $250C6..$25118: the board's full box from creation until
            // a hit or leaving the surf jump/stab states; no age window.
            Self::SurfStab => SwordSpec {
                offset: (0, 0),
                mapping: 0xd7,
                active: 0..=0x7ffd,
                life: 0x7fff,
                states: &[SURF_JUMP, SURF_STAB],
                ..ordinary
            },
            // $254CC, $254F8..$25520: (4,-8), life 31, words [0,$D3,$80D3,0..].
            Self::DashKick => SwordSpec {
                offset: (4, -8),
                mapping: 0xd3,
                active: 20..=27,
                life: 31,
                damage: 3,
                sound_at: 20,
                states: &[DASH_KICK],
            },
            // $25448..$2546C: (20,-40), life 15, words [0,0,$80D2,0].
            Self::Hanging => SwordSpec {
                offset: (20, -40),
                active: 4..=7,
                states: &[HANG_SLASH],
                ..ordinary
            },
            // $2517E..$251A6: (28,-8), words [0,$80D3,0,0]: the large
            // 64x64 $D3 box from +$3A = 7 down to 4.
            Self::StandingPowered => powered,
            // $25214..$2523C: (12,-8), words [0,0,$80D6,0].
            Self::CrouchingPowered => SwordSpec {
                offset: (12, -8),
                mapping: 0xd6,
                active: 4..=7,
                sound_at: 4,
                states: &[CROUCH_SLASH_POWERED],
                ..powered
            },
            // $25294..$252BC: (20,-8), words [0,$80D3,0,0].
            Self::AirbornePowered => SwordSpec {
                offset: (20, -8),
                states: &[AIR_SLASH_POWERED, WALL_SLASH_POWERED],
                ..powered
            },
            // $25484..$254AC: (36,+24), words [0,$80D8,0,0].
            Self::HangingPowered => SwordSpec {
                offset: (36, 24),
                mapping: 0xd8,
                states: &[HANG_SLASH_POWERED],
                ..powered
            },
            // $24F5C..$24F80: (16,-68), words [0,$D2,$80D2,0].
            Self::Horse => SwordSpec {
                offset: (16, -68),
                states: &[HORSE_SLASH],
                ..ordinary
            },
            // $24F9C..$24FC0: (20,-24), words [0,$80D3,0,0].
            Self::HorsePowered => SwordSpec {
                offset: (20, -24),
                states: &[HORSE_SLASH_POWERED],
                ..powered
            },
            // $24FDC..$25000: (12,-48), words [0,$D2,$80D2,0].
            Self::Surf => SwordSpec {
                states: &[SURF_SLASH],
                ..ordinary
            },
            // $2501C..$25040: (28,-8), words [0,$80D3,0,0], as object 10.
            Self::SurfPowered => SwordSpec {
                states: &[SURF_SLASH_POWERED],
                ..powered
            },
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Swing {
    kind: Sword,
    age: u16,
    serial: u64,
    /// The overlap its last update found waits for the object's next pass
    /// ([`super::Strike::hold`]): the hit frees it, a hit that does not
    /// land leaves it running.
    held: Option<u32>,
}

/// One object-1 shuriken of the somersault spread (`$24EB2`), in 16.16 pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpreadShot {
    pub x: i32,
    pub y: i32,
    pub velocity_x: i32,
    pub velocity_y: i32,
    pub mapping: u16,
    pub(super) high_priority: bool,
    age: u16,
    /// `+$2D`: the last terrain test found the splash material.
    in_water: bool,
    /// `+$06`: 1, or 3 when thrown powered (`$24ED6`).
    pub(super) damage: u8,
    /// `+$28`: the `Data_024BA8` direction `SpreadShuriken_Init` set.
    direction: u8,
    /// Boss2's field turned it ([`Turn`]), and its pose's flips since.
    turn: Option<Turn>,
    flips: Option<u16>,
    /// Waiting for its hit to land ([`super::Strike::hold`]).
    held: Option<u32>,
    /// Its held hit did not land: the terrain test of the step's end is
    /// still to come ([`Projectile::untested`]).
    untested: bool,
    /// Its deflection flag is set ([`Projectile::deflected`]).
    deflected: bool,
    /// `obj_tile` bit 11 from the player's facing (`$24EBA`).
    facing_left: bool,
    /// Allocation serial ([`Projectile::order`]).
    pub(super) order: u64,
}
impl SpreadShot {
    /// `obj_tile` flip bits 11 and 12, which `Object_LoadHitbox` applies
    /// to its box: the player's facing, or the pose flips `Func_024BE0`
    /// last turned it to.
    fn tile_flips(&self) -> (bool, bool) {
        self.flips.map_or((self.facing_left, false), |flips| {
            (flips & 0x800 != 0, flips & 0x1000 != 0)
        })
    }
}

/// What `Func_024AB0` (`$24ADC`) and `Shuriken_Update` (`$24DF0..$24E72`)
/// make of the material under a player shuriken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShurikenTerrain {
    /// Materials other than 1, 3, 4, 14 and 15: flies on.
    Fly,
    /// Entering material 15: sound `$27` and a list-1 type-`$24` splash;
    /// the shuriken flies on.
    Splash,
    /// Materials 1, 3, 4 and 14 (`$24E0A`): `Combat::strike_wall`.
    Wall,
    /// A negative X (`$24AB2`, value 3): removed.
    Leave,
}

/// List 3's record count (`Object_ListParams` + 1, `$1446`).
const ATTACK_SLOTS: usize = 8;

/// `$FF0014`, the gravity `Func_024AFE` gives a knocked-back shuriken
/// (`$AE` writes `$3400`).
const GRAVITY: i32 = 0x3400;

/// A shuriken on a stopping material, handed to `Combat::strike_wall`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WallHit {
    pub(super) at: (i32, i32),
    /// 16.16 pixels per tick.
    pub(super) velocity: (i32, i32),
    pub(super) mapping: u16,
    pub(super) high_priority: bool,
    /// The shuriken's allocation serial: the record stays where it is in
    /// list 3 and only changes its routine.
    pub(super) order: u64,
}

/// A player shuriken that struck a wall (`$24E0A`): stuck in it
/// (`loc_024C6C`) or knocked back (`Func_024AFE`). Obj_flags bit 13 keeps it
/// out of every collision until its `+$3A` lifetime runs out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpentShuriken {
    pub x: i32,
    pub y: i32,
    pub mapping: u16,
    /// Attribute flip bits (`$0800` horizontal, `$1000` vertical).
    flip: u16,
    motion: Motion,
    /// `+$3A`: removed when it reaches zero.
    life: u16,
    /// `+$2A` step and `+$28` index of the knock-back spin (`Func_024B5E`);
    /// `None` while stuck (routine `$0C` does nothing).
    spin: Option<(i32, usize)>,
    pub(super) high_priority: bool,
    /// Allocation serial of the shuriken it was ([`WallHit::order`]).
    order: u64,
}

/// Player-side weapon and ninjutsu state kept alongside the enemies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerWeapons {
    /// $232C2: None uses enemy distance; Some(true) forces a throw (X/Y),
    /// Some(false) a sword (B) in the secret six-button mode.
    pub(crate) force_throw: Option<bool>,
    /// Player state from the previous update; `None` skips the object checks.
    pub player_state: Option<usize>,
    /// State 20 at cursor $2xx: the stab object stays active ($25332).
    pub diving: bool,
    /// +$2A bit 15: guard states and sword swings block frontal hits.
    pub guarding: bool,
    /// State 29 below cursor `$500` (`$21CB2`): the dash slash cancels the
    /// list-4 hit on both sides, so the shot flies on through Joe.
    pub dash_slashing: bool,
    /// Down held: extends `Player_MeasureEnemy`'s vertical target window.
    pub crouching: bool,
    /// Proximity sampled before the state handler changes facing.
    pub(crate) proximity: Option<i32>,
    /// Player `obj_tile` bit 15, used by `Player_MeasureEnemy` (`$22876`).
    pub high_priority: bool,
    /// `plr_ctrl` bit 5 and the player's Y velocity (256 units per pixel)
    /// when B was pressed: `Shuriken_Init` adds twice the velocity's whole
    /// pixels to an airborne throw's height (`$24D72`).
    pub(crate) airborne: bool,
    pub(crate) velocity_y: i32,
    /// Where list 3 finds the player this frame; `None` takes the position
    /// the update is given.
    pub(crate) anchors: Option<PlayerAnchors>,
    /// `$FF0180` (16.16) while `plr_flags` bit 6 or 7 (horse, surfboard) is
    /// set: added to a thrown shuriken's speed (`$24D8C`). Zero on foot.
    pub(crate) scroll_speed: i32,
    /// +$30: hits the Ikazuchi shield still absorbs ($20C0C, $21D86).
    pub shield: u8,
    /// $FF37E6: techniques left in this life, refilled from $6185 ($11896).
    pub ninjutsu_uses: u8,
    /// $FF37E7: 0 Ikazuchi, 1 Kariu, 2 Fushin, 3 Mijin.
    pub ninjutsu_selected: u8,
    pub shots: Vec<SpreadShot>,
    /// Shurikens stuck in or knocked back from a wall.
    pub spent: Vec<SpentShuriken>,
    ninjutsu_per_life: u8,
    /// +$2A bit 13 ($231F0): the spread already fired in this somersault.
    spread_fired: bool,
    swing: Option<Swing>,
    /// A sword object replaced by a new swing while its hit waited to land
    /// ([`super::Strike::hold`]). Natively only one sword object runs, so
    /// it keeps its list-3 slot until then and ends either way.
    held_sword: Option<u32>,
    /// The sword object's hitbox in its last update (`+$10..$13`, which
    /// `$194A` reads; a point while it shows mapping 0), or `None` while no
    /// sword object runs or bit 14 hides it.
    swing_box: Option<Hitbox>,
    /// Player `+$2A` bit 7 (`plr_status` bit 7), set by the POW item
    /// through `Player_PowerUp` (`$22890`): powered swords and shuriken
    /// damage 3, and shurikens that vanish at walls (#70).
    pub powered: bool,
    /// `$FF37EA`: POW items collected in a row while already powered.
    pub power_bonus: u8,
    /// `$FF37EB`: health items collected in a row at full health.
    pub health_bonus: u8,
    /// `$24E92`: eight (vx, vy) pairs, 16.16 after the `lsl.l #4` at `$24F04`.
    spread: [(i32, i32); 8],
    /// `Data_024BA8`: the knock-back spin's 28 (mapping - `$CA`, flip) words.
    spin_frames: [u16; 28],
    sounds: Vec<u8>,
}
impl PlayerWeapons {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut spread = [(0, 0); 8];
        for (i, pair) in spread.iter_mut().enumerate() {
            *pair = (
                i32::from(word(rom, 0x24e92 + i * 4)?) << 4,
                i32::from(word(rom, 0x24e94 + i * 4)?) << 4,
            );
        }
        let per_life = *rom.get(0x6185).ok_or("missing ninjutsu count")?;
        let mut spin_frames = [0; 28];
        for (i, frame) in spin_frames.iter_mut().enumerate() {
            *frame = word(rom, 0x24ba8 + i * 2)?;
        }
        Ok(Self {
            force_throw: None,
            player_state: None,
            diving: false,
            guarding: false,
            dash_slashing: false,
            crouching: false,
            proximity: None,
            high_priority: false,
            airborne: false,
            velocity_y: 0,
            anchors: None,
            scroll_speed: 0,
            shield: 0,
            ninjutsu_uses: per_life,
            // The round 1-1 state captured $FF37E7 = 0 at the start of play.
            ninjutsu_selected: 0,
            shots: Vec::new(),
            spent: Vec::new(),
            ninjutsu_per_life: per_life,
            spread_fired: false,
            swing: None,
            held_sword: None,
            swing_box: None,
            powered: false,
            power_bonus: 0,
            health_bonus: 0,
            spread,
            spin_frames,
            sounds: Vec::new(),
        })
    }
    pub(super) fn inherit(&mut self, previous: &Self) {
        self.shield = previous.shield;
        self.ninjutsu_uses = previous.ninjutsu_uses;
        self.ninjutsu_selected = previous.ninjutsu_selected;
        // `PowerUp_Saved` (`$22D06`) carries the flag across parts; the two
        // RAM counters are only written by the pickups.
        self.powered = previous.powered;
        self.power_bonus = previous.power_bonus;
        self.health_bonus = previous.health_bonus;
    }
    /// Test hook: an active sword hitbox.
    /// Where the sword object stands: it follows the player each update,
    /// which runs before `Object_Integrate` moves the player. The others add
    /// the player's velocity to its `obj_x`/`obj_y` (`loc_024C86`,
    /// `$24CBA..$24CD8`; the dash kick `$25506..$2551E`), before this
    /// frame's `obj_accel_y` joins the velocity; the air stab adds none
    /// (`$2531C..$2532C`), so it stands where the player was before this
    /// frame's move.
    fn sword_origin(&self, kind: Sword, player: (i32, i32), facing_left: bool) -> (i32, i32) {
        let (dx, dy) = kind.spec().offset;
        let player = match (kind, self.anchors) {
            (Sword::AirStab | Sword::SurfStab, Some(anchors)) => anchors.unmoved,
            (_, Some(anchors)) => anchors.stepped,
            (_, None) => player,
        };
        (player.0 + if facing_left { -dx } else { dx }, player.1 + dy)
    }
    /// The live sword object's position, if any.
    pub(crate) fn sword_position(
        &self,
        player: (i32, i32),
        facing_left: bool,
    ) -> Option<(i32, i32)> {
        self.swing
            .map(|swing| self.sword_origin(swing.kind, player, facing_left))
    }
    /// One `Func_024BE0` turn: the pose (`$CA` + frame, flips `$1800`) and
    /// the 8 px/tick velocity of the new direction (`Data_024E92` pair
    /// `frame ^ 7`, negated by the flips).
    pub(super) fn steer(&self, turn: &mut Turn) -> (u16, u16, i32, i32) {
        steer(&(self.spin_frames, self.spread), turn)
    }
    /// Spread shurikens [`Combat::spend_attack`] spent.
    pub(super) fn drop_spent_shots(&mut self) {
        self.shots.retain(|shot| shot.age != u16::MAX);
    }
    /// `+$06` of a new shuriken: 1, plus 2 while powered (`$24D5C`,
    /// `$24ED6`).
    fn shuriken_damage(&self) -> u8 {
        if self.powered { 3 } else { 1 }
    }
    /// Native dev tools: at least a life's technique count.
    pub(crate) fn refill_ninjutsu(&mut self) {
        self.ninjutsu_uses = self.ninjutsu_uses.max(self.ninjutsu_per_life.max(1));
    }
    /// `Func_024AB0` for a player shuriken at (x, y), updating its `+$2D`
    /// splash memory.
    pub(super) fn shuriken_terrain(
        terrain: &impl Terrain,
        (x, y): (i32, i32),
        in_water: &mut bool,
    ) -> ShurikenTerrain {
        if x < 0 {
            return ShurikenTerrain::Leave;
        }
        let material = terrain.material_at(x, y);
        let was_in_water = std::mem::replace(in_water, material.splashes_shuriken());
        if material.stops_shuriken() {
            ShurikenTerrain::Wall
        } else if *in_water && !was_in_water {
            ShurikenTerrain::Splash
        } else {
            ShurikenTerrain::Fly
        }
    }
    /// `$AF2C -> $11896` refills the technique count for each life.
    pub(super) fn new_life(&mut self) {
        self.shield = 0;
        self.ninjutsu_uses = self.ninjutsu_per_life;
        self.shots.clear();
        self.spent.clear();
        self.swing = None;
        self.held_sword = None;
        self.swing_box = None;
        self.spread_fired = false;
        // The new life's player record starts without the POW bit.
        self.powered = false;
    }
}

impl Combat {
    /// Player sound commands produced outside `tick` (attacks started by the
    /// player's button handler).
    pub fn take_player_sounds(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.weapons.sounds)
    }
    /// Record the player's state and collision frame from the last player
    /// update: before `tick` or `check_player` and the next player update,
    /// and on foot again before `update_objects`.
    pub fn set_player_pose(&mut self, state: usize, cursor: u16, frame: usize, down: bool) {
        self.set_player_frame(frame);
        let weapons = &mut self.weapons;
        weapons.player_state = Some(state);
        weapons.diving = matches!(
            state,
            movement::state::AIR_STAB | movement::state::HORSE_STAB | movement::state::SURF_STAB
        ) && cursor >> 8 == 2;
        weapons.guarding = movement::guards(state);
        weapons.dash_slashing = state == movement::state::DASH_KICK && cursor < 0x500;
        weapons.crouching = down;
        weapons.high_priority = self.player_priority_raised;
        if state != movement::state::SOMERSAULT {
            weapons.spread_fired = false;
        }
    }
    /// Where the attacks' next update finds the player, or `None` for the
    /// position that update is given.
    pub(crate) fn set_player_anchors(&mut self, anchors: Option<PlayerAnchors>) {
        self.weapons.anchors = anchors;
    }
    /// The player's `plr_ctrl` bit 5 and Y velocity (256 units per pixel)
    /// for the next `start_player_attack`.
    pub(crate) fn set_player_motion(&mut self, airborne: bool, velocity_y: i32) {
        self.weapons.airborne = airborne;
        self.weapons.velocity_y = velocity_y;
    }
    /// Refresh the collision frame after the player step for stage objects,
    /// whose overlap becomes pending contact on the following combat update.
    pub(crate) fn set_player_frame(&mut self, frame: usize) {
        debug_assert!(frame < self.player_mappings.len());
        self.player_frame = frame;
    }
    /// Refresh the first pass of `Player_FindNearestEnemy` (`$2277C`):
    /// live list-6 item boxes whose `+$34` high byte is clear.
    pub(crate) fn set_item_box_targets(
        &mut self,
        boxes: impl IntoIterator<Item = (i32, i32, bool)>,
    ) {
        self.item_boxes = boxes
            .into_iter()
            .map(|(x, y, high_priority)| AttackTarget {
                x,
                y,
                high_priority,
            })
            .collect();
    }
    /// `$1FD68`: sample with the previous pose/facing, before the state
    /// handler can turn Joe or move him. The selected attack then stays fixed.
    pub(crate) fn sample_attack_proximity(&mut self, player: (i32, i32), facing_left: bool) {
        self.weapons.proximity = Some(self.find_nearest_enemy(player, facing_left));
    }
    /// Button B accepted by `$22F7E` in `posture`. Returns the melee or throw
    /// choice, or `None` for the spread, which keeps state 17.
    pub fn start_player_attack(
        &mut self,
        posture: AttackPosture,
        player: (i32, i32),
        facing_left: bool,
    ) -> Option<Attack> {
        if self.game_over
            || (self.weapons.force_throw == Some(true)
                && (self.shurikens_disabled || (!self.infinite_shurikens && self.hud.ammo == 0)))
        {
            return None;
        }
        let posture = if self.weapons.force_throw == Some(true) {
            match posture {
                AttackPosture::DashKick => AttackPosture::Standing,
                AttackPosture::AirStab => {
                    if self.mount == Some(crate::special::Mount::Surfboard) {
                        AttackPosture::Surf
                    } else {
                        AttackPosture::Airborne
                    }
                }
                other => other,
            }
        } else {
            posture
        };
        // `Player_ChooseAttack` (`$232CC`): a powered melee takes the
        // powered object; the stab and dash slash have only one.
        let powered = self.weapons.powered;
        let sword = match posture {
            AttackPosture::Spread
                if self.weapons.force_throw != Some(false) && !self.shurikens_disabled =>
            {
                self.fire_spread(player, facing_left);
                return None;
            }
            AttackPosture::Standing if powered => Sword::StandingPowered,
            AttackPosture::Standing => Sword::Standing,
            AttackPosture::Crouching if powered => Sword::CrouchingPowered,
            AttackPosture::Crouching => Sword::Crouching,
            AttackPosture::Airborne | AttackPosture::WallAir | AttackPosture::Spread if powered => {
                Sword::AirbornePowered
            }
            AttackPosture::Airborne | AttackPosture::WallAir | AttackPosture::Spread => {
                Sword::Airborne
            }
            AttackPosture::AirStab if self.mount == Some(crate::special::Mount::Surfboard) => {
                Sword::SurfStab
            }
            AttackPosture::AirStab => Sword::AirStab,
            AttackPosture::DashKick => Sword::DashKick,
            AttackPosture::Hang if powered => Sword::HangingPowered,
            AttackPosture::Hang => Sword::Hanging,
            AttackPosture::Horse if powered => Sword::HorsePowered,
            AttackPosture::Horse => Sword::Horse,
            AttackPosture::Surf if powered => Sword::SurfPowered,
            AttackPosture::Surf => Sword::Surf,
        };
        // $23292: melee below 64 px, otherwise a throw while shurikens remain.
        // The stab ($2306C) and dash kick ($23280) never throw.
        let melee = matches!(sword, Sword::AirStab | Sword::SurfStab | Sword::DashKick)
            || self.shurikens_disabled
            || (!self.infinite_shurikens && self.hud.ammo == 0)
            || match self.weapons.force_throw {
                Some(throw) => !throw,
                None => {
                    self.weapons
                        .proximity
                        .unwrap_or_else(|| self.find_nearest_enemy(player, facing_left))
                        < 64
                }
            };
        let full = self.free_attack_slots() == 0;
        if melee {
            // $23190 requests the surf kick's voice before $2331E attempts
            // to allocate its object. A full list still plays the kiai.
            if sword == Sword::SurfStab {
                self.weapons
                    .sounds
                    .push(self.rules.cues.get(SoundEvent::AirStabKiai));
            }
            if full {
                return None; // $23322: no object, and the state stays
            }
            self.start_swing(sword);
            if sword == Sword::AirStab {
                // $23078: the ordinary kick requests the kiai.
                let kiai = self.rules.cues.get(SoundEvent::AirStabKiai);
                self.weapons.sounds.push(kiai);
            }
            return Some(Attack::Slash);
        }
        // $232D8 passes the spawn height: -47 standing ($23010), -26
        // crouching ($23030), -51 airborne ($23056, $230C0), -33 hanging
        // ($230E0), -64 on the horse ($2310C) and -41 on the surfboard
        // ($23158). $24D38 spawns
        // object 0 16 px ahead at 8 px/tick; $232B8 uses one shuriken.
        let mut rise = match posture {
            AttackPosture::Crouching => 26,
            AttackPosture::Airborne | AttackPosture::WallAir | AttackPosture::Spread => 51,
            AttackPosture::Hang => 33,
            AttackPosture::Horse => 64,
            AttackPosture::Surf => 41,
            _ => 47,
        };
        // $24D6E..$24D7E: airborne, add twice obj_vel_y's high word.
        if self.weapons.airborne {
            rise -= 2 * (self.weapons.velocity_y >> 8);
        }
        // $232B8 takes the shuriken before `Player_StartThrow` allocates the
        // object; with list 3 full ($232E2) it is lost and nothing changes.
        if !self.infinite_shurikens {
            self.hud.ammo -= 1;
        }
        if full {
            return None;
        }
        let direction = if facing_left { -1 } else { 1 };
        // $24D50..$24D5E: damage 1, 3 while powered.
        let mut shot = Projectile::new(
            player.0 + direction * 16,
            player.1 - rise,
            direction * 8,
            false,
            0xca,
            self.weapons.shuriken_damage(),
        );
        // $24D8C..$24DB6: mounted, $FF0180 joins the 8 px/tick before the
        // facing negates it.
        shot.motion.vx = direction * self.weapons.scroll_speed;
        shot.life = 255;
        shot.high_priority = self.weapons.high_priority;
        shot.order = self.allocate();
        self.journal.shurikens += 1;
        self.projectiles.push(shot);
        self.weapons.sounds.push(self.sound_commands[0]);
        Some(Attack::Throw)
    }
    /// State 24 at cursor `$0900` (`$20C8C..$20CD4`) creates object 9 again.
    pub fn start_combo_cut(&mut self) {
        // $20C8C allocates through Object_AllocL3 like any sword.
        if self.free_attack_slots() > 0 {
            self.start_swing(Sword::Standing);
        }
    }
    /// List 3 holds 8 records (`Object_ListParams` = 7 at `$1446`): thrown,
    /// spread and spent shurikens and the sword object. (`$22F0A` raises it
    /// to 20 only while a technique runs, when B starts nothing.)
    pub(super) fn free_attack_slots(&self) -> usize {
        let used = self
            .projectiles
            .iter()
            .filter(|shot| !shot.enemy && shot.alive)
            .count()
            + self.weapons.shots.len()
            + self.weapons.spent.len()
            + usize::from(self.weapons.swing.is_some())
            + usize::from(self.weapons.held_sword.is_some());
        ATTACK_SLOTS.saturating_sub(used)
    }
    fn start_swing(&mut self, kind: Sword) {
        self.attack_serial += 1;
        let replaced = self.weapons.swing.replace(Swing {
            kind,
            age: 0,
            serial: self.attack_serial,
            held: None,
        });
        if let Some(held) = replaced.and_then(|swing| swing.held) {
            self.weapons.held_sword = Some(held);
        }
    }
    /// `$2277C..$2288A`: list 6 item boxes, list 5 enemies except
    /// `$8C..=$94`, then the list-8 boss. Returns the nearest qualifying
    /// Manhattan distance, or `$7FFF` when none qualifies.
    pub(super) fn find_nearest_enemy(&self, player: (i32, i32), facing_left: bool) -> i32 {
        let mut nearest = 0x7fff;
        for &target in &self.item_boxes {
            Self::measure_enemy(
                player,
                facing_left,
                self.weapons.crouching,
                self.weapons.high_priority,
                target,
                &mut nearest,
            );
        }
        for enemy in &self.enemies {
            let spawn = &self.spawns[enemy.spawn];
            if spawn.kind == crate::bosses::BOSS_KIND || (0x8c..=0x94).contains(&spawn.kind) {
                continue;
            }
            if matches!(
                enemy.state,
                EnemyState::Dormant | EnemyState::Emerging | EnemyState::Defeated
            ) || enemy.offscreen
            {
                continue;
            }
            // `obj_tile` bit 15 as the object last left it: the spawn flag,
            // its handler's tile, a parent's (`Func_012DD8`) or an
            // explicit raise such as Karura's `$34C8C`.
            let high_priority = enemy.high_priority;
            Self::measure_enemy(
                player,
                facing_left,
                self.weapons.crouching,
                self.weapons.high_priority,
                AttackTarget {
                    x: enemy.x,
                    y: enemy.y,
                    high_priority,
                },
                &mut nearest,
            );
        }
        // Boss_Setup owns the list-8 object, represented by BOSS_KIND in the
        // native enemy vector rather than the list-5 pass above.
        for enemy in &self.enemies {
            let spawn = &self.spawns[enemy.spawn];
            if spawn.kind != crate::bosses::BOSS_KIND
                || matches!(
                    enemy.state,
                    EnemyState::Dormant | EnemyState::Emerging | EnemyState::Defeated
                )
                || enemy.offscreen
            {
                continue;
            }
            let tile = enemy.tile.unwrap_or(0x2000);
            Self::measure_enemy(
                player,
                facing_left,
                self.weapons.crouching,
                self.weapons.high_priority,
                AttackTarget {
                    x: enemy.x,
                    y: enemy.y,
                    high_priority: tile & 0x8000 != 0,
                },
                &mut nearest,
            );
        }
        nearest
    }
    /// `$22810`: reject targets too far below the feet, more than 16 px
    /// behind Joe, or on the other sprite-priority layer; otherwise retain
    /// the smallest `|dx| + |dy|`.
    pub(super) fn measure_enemy(
        player: (i32, i32),
        facing_left: bool,
        down: bool,
        player_high_priority: bool,
        target: AttackTarget,
        nearest: &mut i32,
    ) {
        let dx = target.x - player.0;
        let above_feet = player.1 - target.y;
        if above_feet < if down { -80 } else { -16 } {
            return;
        }
        if (dx < 0) != facing_left && dx.abs() > 16 {
            return;
        }
        let distance = dx.abs() + above_feet.abs();
        if distance < *nearest && target.high_priority == player_high_priority {
            *nearest = distance;
        }
    }
    /// Native refinement of `$2319E..$23270`: every allocated star costs
    /// one shuriken. The original's first burst overdraws a stock of 1..7;
    /// both first and subsequent bursts now stop at the available stock.
    fn fire_spread(&mut self, player: (i32, i32), facing_left: bool) {
        if self.shurikens_disabled {
            return;
        }
        let mut fired = 0;
        for index in 0..8 {
            if (!self.infinite_shurikens && self.hud.ammo == 0) || self.free_attack_slots() == 0 {
                break;
            }
            if !self.infinite_shurikens {
                self.hud.ammo -= 1;
            }
            self.spawn_spread_shot(index, player, facing_left);
            fired += 1;
        }
        if fired > 0 {
            self.weapons.spread_fired = true;
            self.weapons
                .sounds
                .push(self.rules.cues.get(SoundEvent::SpreadThrow));
        }
    }
    /// `SpreadShuriken_Init` (`$24EB2`) for direction `index` (`+$28`).
    fn spawn_spread_shot(&mut self, index: usize, player: (i32, i32), facing_left: bool) {
        let (vx, vy) = self.weapons.spread[index];
        let vx = if facing_left { -vx } else { vx };
        // $24EEA / $24F0E: +$28 = index ^ 7, mirrored to $E - it facing left.
        let direction = index as u8 ^ 7;
        let direction = if facing_left {
            0xe - direction
        } else {
            direction
        };
        // $24F24..$24F3A: two steps ahead, 24 px above the feet.
        let order = self.allocate();
        self.journal.shurikens += 1;
        self.weapons.shots.push(SpreadShot {
            x: (player.0 << 16) + 2 * vx,
            y: (player.1 << 16) + 2 * vy - (24 << 16),
            velocity_x: vx,
            velocity_y: vy,
            mapping: 0xca + (index as u16 ^ 7),
            high_priority: self.weapons.high_priority,
            age: 0,
            in_water: false,
            damage: self.weapons.shuriken_damage(),
            direction,
            turn: None,
            flips: None,
            held: None,
            untested: false,
            deflected: false,
            facing_left,
            order,
        });
    }
    /// `$22E9C..$22F44`: A needs a remaining use ($FF37E6) and no running
    /// shield (+$30). Every technique except Mijin uses one. Returns 0..=3.
    pub fn begin_ninjutsu(&mut self) -> Option<usize> {
        let weapons = &mut self.weapons;
        if self.game_over || weapons.ninjutsu_uses == 0 || weapons.shield != 0 {
            return None;
        }
        if weapons.ninjutsu_selected != 3 {
            weapons.ninjutsu_uses -= 1;
        }
        self.journal.ninjutsu += 1;
        self.journal.mijin |= weapons.ninjutsu_selected == 3;
        Some(usize::from(weapons.ninjutsu_selected))
    }
    /// Kariu's dragons and Mijin's blast: every live enemy whose hitbox is on
    /// screen takes `damage`. Returns the number struck.
    pub fn strike_visible(&mut self, camera: (i32, i32), damage: u8) -> usize {
        let screen = Hitbox {
            x: camera.0 + 160,
            y: camera.1 + 112,
            half_width: 160,
            half_height: 112,
        };
        let mut hits = 0;
        for enemy in &mut self.enemies {
            let damage = self
                .roster
                .ninjutsu_damage(self.spawns[enemy.spawn].kind, damage);
            if !Self::hittable(enemy)
                || !self
                    .roster
                    .hit_by(self.spawns[enemy.spawn].kind, enemy, damage)
                    .weapon_collides()
                || enemy.high_priority != self.weapons.high_priority
            {
                continue;
            }
            if screen.intersects(enemy_hitbox(&self.mappings, enemy)) {
                // Attacker types $50 and above leave no lethal spark.
                let before = enemy.health;
                Self::hurt_enemy(
                    &self.roster,
                    enemy,
                    &self.spawns[enemy.spawn],
                    damage,
                    false,
                );
                if enemy.health < before {
                    self.journal.hits.push(crate::profile::Hit {
                        target: self.spawns[enemy.spawn].kind,
                        defeated: enemy.health == 0,
                        kind: crate::profile::HitKind::Magic,
                    });
                }
                hits += 1;
            }
        }
        hits
    }
    /// `$20730..$20778`: Mijin spends a spare life and refills health. With
    /// no spare life it is fatal (`$20740..$20768`, state 42).
    pub fn spend_mijin_life(&mut self) -> CombatEvents {
        let mut event = CombatEvents::default();
        if self.hud.lives > 0 {
            self.hud.lives -= 1;
            self.hud.health = self.hud.max_health;
        } else {
            self.die();
            event.lethal = true;
        }
        event
    }
}

#[cfg(test)]
mod armor_tests;
#[cfg(test)]
mod roadmap_tests;
