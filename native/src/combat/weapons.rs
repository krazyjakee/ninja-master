//! Player weapons (issue #42): sword objects, the somersault spread, guard,
//! the recovered handlers.
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
    /// Button B accepted by `$22F7E` in `posture`. Returns the melee or throw
    /// choice, or `None` for the spread, which keeps state 17.
    pub fn start_player_attack(
        &mut self,
        posture: AttackPosture,
        player: (i32, i32),
        facing_left: bool,
    ) -> Option<Attack> {
        if self.game_over || (self.weapons.force_throw == Some(true) && self.hud.ammo == 0) {
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
            AttackPosture::Spread if self.weapons.force_throw != Some(false) => {
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
            || self.hud.ammo == 0
            || match self.weapons.force_throw {
                Some(throw) => !throw,
                None => self.find_nearest_enemy(player, facing_left) < 64,
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
        self.hud.ammo -= 1;
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
    /// `$2319E..$23270`: the first B of a somersault needs a shuriken, plays
    /// effect 4 (`$B2`) and creates eight object-1 shurikens while list 3
    /// has room. Ammo is checked once: each star subtracts one and
    /// `$23270` clamps a negative count to zero, so 1-7 shurikens still
    /// throw eight.
    fn fire_spread(&mut self, player: (i32, i32), facing_left: bool) {
        if self.hud.ammo == 0 {
            return;
        }
        if self.weapons.spread_fired {
            self.fire_more_spread(player, facing_left);
            return;
        }
        self.weapons.spread_fired = true;
        let throw = self.rules.cues.get(SoundEvent::SpreadThrow);
        self.weapons.sounds.push(throw);
        let mut ammo = i32::from(self.hud.ammo);
        for index in 0..8 {
            if self.free_attack_slots() == 0 {
                break;
            }
            ammo -= 1;
            self.spawn_spread_shot(index, player, facing_left);
        }
        self.hud.ammo = ammo.max(0) as u16;
    }
    /// `$231F8..$2326C`: a later B in the same somersault (`plr_ctrl` bit
    /// 13) starts again at direction 0 and adds stars while a shuriken and
    /// a list-3 record remain, playing effect 4 after the first.
    fn fire_more_spread(&mut self, player: (i32, i32), facing_left: bool) {
        for index in 0..8 {
            if self.hud.ammo == 0 || self.free_attack_slots() == 0 {
                break;
            }
            self.hud.ammo -= 1;
            self.spawn_spread_shot(index, player, facing_left);
            if index == 0 {
                let throw = self.rules.cues.get(SoundEvent::SpreadThrow);
                self.weapons.sounds.push(throw);
            }
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
            facing_left,
            order,
        });
    }
    /// Sword objects and spread shurikens, after the enemies moved.
    pub(super) fn update_player_weapons(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
        event: &mut CombatEvents,
    ) {
        let view = self.screen_view(camera.0);
        let shuriken_hit = self.rules.cues.get(SoundEvent::ShurikenHit);
        let mut walls = Vec::new();
        let mut splashes = Vec::new();
        let steering = (self.weapons.spin_frames, self.weapons.spread);
        for shot in &mut self.weapons.shots {
            shot.age += 1;
            // Func_024BE0, as for the thrown shuriken.
            if let Some(mut turn) = shot.turn.filter(|turn| turn.left > 0)
                && shot.age.is_multiple_of(2)
            {
                let (mapping, flips, vx, vy) = steer(&steering, &mut turn);
                shot.turn = Some(turn);
                shot.mapping = mapping;
                shot.flips = Some(flips);
                shot.velocity_x = vx;
                shot.velocity_y = vy;
            }
            // Its held hit did not land: the terrain test where it met the
            // object comes before the next step (`loc_024DD0`).
            if std::mem::take(&mut shot.untested) {
                Self::spread_terrain_at(terrain, shot, &mut walls, &mut splashes);
                if shot.age == u16::MAX {
                    continue;
                }
            }
            // `Object_Integrate` moves it a whole step, and only where the
            // step ends do the objects' `Object_CollideL3` (next frame) and
            // `Shuriken_Update`'s terrain test meet it.
            shot.x += shot.velocity_x;
            shot.y += shot.velocity_y;
            let (x, y) = (shot.x >> 16, shot.y >> 16);
            let hitbox = Self::spread_box(&self.mappings, shot);
            let target = self.enemies.iter_mut().find(|enemy| {
                Self::attack_targetable(&self.roster, &self.spawns, enemy)
                    && enemy.high_priority == shot.high_priority
                    && hitbox.collides(enemy_hitbox(&self.mappings, enemy))
                    && {
                        let kind = self.spawns[enemy.spawn].kind;
                        if self.roster.hit(kind, enemy) == HitResponse::Reflect {
                            // $12984 turns shurikens off the 4-1 guard's
                            // shield; $2B126 turns them off Boss2's field.
                            shot.turn = shot.turn.or(Some(if kind == 0x18 {
                                Turn::guard(shot.direction, self.ticks)
                            } else {
                                Turn::away(shot.direction, y, enemy.y)
                            }));
                            false
                        } else {
                            true
                        }
                    }
            });
            if let Some(enemy) = target {
                // +$06 = 1 at $24ECC, 3 powered.
                let strike = Strike {
                    origin: (x, y),
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
                if shot.held.is_none() {
                    shot.age = u16::MAX;
                }
            }
            // Spent or waiting for its hit, which frees it first.
            if shot.age != u16::MAX && shot.held.is_none() {
                Self::spread_terrain_at(terrain, shot, &mut walls, &mut splashes);
            }
            if shot.age == u16::MAX {
                continue;
            }
            // +$3A = 255 at $24EDC; offscreen objects are removed. A held
            // shuriken waits for its hit.
            if shot.held.is_none()
                && (shot.age >= 255
                    || !(camera.0 + view.0 - 64..=camera.0 + view.1 + 64).contains(&x)
                    || !(camera.1 - 64..=camera.1 + 288).contains(&y))
            {
                shot.age = u16::MAX;
            }
        }
        self.weapons.shots.retain(|shot| shot.age != u16::MAX);
        for (x, y, left) in splashes {
            self.splash_shuriken((x, y), left, event);
        }
        for wall in walls {
            self.strike_wall(wall, event);
        }
        self.weapons.swing_box = None;

        let Some(mut swing) = self.weapons.swing.take() else {
            return;
        };
        let spec = swing.kind.spec();
        let age = swing.age;
        swing.age += 1;
        if age > 0
            && self
                .weapons
                .player_state
                .is_some_and(|state| !spec.states.contains(&state))
        {
            return; // $25162/$24C8E delete the object with the state.
        }
        let diving = swing.kind == Sword::AirStab && self.weapons.diving;
        // PlayerAttack_Run ($24A8E..$24AAA) counts +$3A down after the
        // routine and frees the record when it reaches zero, before
        // Object_LoadHitbox: the update `life - 1` after creation leaves
        // no box (1-1: a slash made on frame 167 is gone on 181). The dive
        // holds +$3A at 11 ($25342).
        if age + 1 >= spec.life && !diving {
            return;
        }
        // $24CEC/$25530: the swing sound plays on the mapping word with bit 15
        // set when +$3A & 3 is 3: effect 3 for objects 9/11/13/17 and the
        // mounted 3/5 ($24D02), effect $1A for the powered 10/12/14/18 and
        // 4/6, the DASH ATTACK voice for object 19 ($25544). The air stab's
        // words have no bit 15; its voice came with the attack.
        if age == spec.sound_at {
            let cue = match swing.kind {
                Sword::Standing
                | Sword::Crouching
                | Sword::Airborne
                | Sword::Hanging
                | Sword::Horse
                | Sword::Surf => Some(SoundEvent::Slash),
                Sword::StandingPowered
                | Sword::CrouchingPowered
                | Sword::AirbornePowered
                | Sword::HangingPowered
                | Sword::HorsePowered
                | Sword::SurfPowered => Some(SoundEvent::PoweredSlash),
                Sword::DashKick => Some(SoundEvent::DashAttack),
                Sword::AirStab | Sword::SurfStab => None,
            };
            if let Some(cue) = cue {
                event.sound_ids.push(self.rules.cues.get(cue));
            }
        }
        // Outside its active entries the object shows mapping 0, whose
        // empty box `Object_CollideList` still tests as a point.
        let hitbox = self.sword_box(swing.kind, age, diving, player, facing_left);
        self.weapons.swing_box = Some(hitbox);
        // $24C9A (objects 9/13 and every powered object, whose
        // routines all end in loc_024C86), $251F6 (crouching object
        // 11), $252F2 (air stab), $254E0 (dash kick).
        let cue = self.rules.cues.get(swing.kind.hit_sound());
        // `Object_CollideList` ($1950..$19D2) stops at the first overlap
        // and sets bit 14 on the sword, which `flags & $E800 == $8000`
        // then excludes for every later object this frame: one enemy,
        // the first in list order, per update.
        // Boss2's field clears the sword's bit 14 (`$2B126`): it is
        // neither spent nor heard, and meets the next object.
        let target = self.enemies.iter_mut().find(|enemy| {
            Self::attack_targetable(&self.roster, &self.spawns, enemy)
                && enemy.high_priority == self.weapons.high_priority
                && enemy.attack_serial != swing.serial
                && hitbox.collides(enemy_hitbox(&self.mappings, enemy))
                && self.roster.hit(self.spawns[enemy.spawn].kind, enemy) != HitResponse::Reflect
        });
        if let Some(enemy) = target {
            let strike = Strike {
                origin: self.weapons.sword_origin(swing.kind, player, facing_left),
                damage: spec.damage,
                cue,
                shuriken: false,
                bounce: matches!(swing.kind, Sword::AirStab | Sword::SurfStab),
                hold: None,
                serial: swing.serial,
            };
            swing.held = Self::strike_enemy(
                &self.roster,
                &self.spawns[enemy.spawn],
                enemy,
                strike,
                (self.deferring, Some(&mut self.holds)),
                event,
            );
            // Landed at once: bit 14 hides the sword from the list-6
            // and list-4 checks for the rest of the frame, and every
            // sword but the dash kick is spent. Deferred, the sword runs
            // on until `land_strikes` sees whether the hit lands.
            if swing.held.is_none() {
                self.weapons.swing_box = None;
                if swing.kind != Sword::DashKick {
                    return;
                }
            }
        }
        self.weapons.swing = Some(swing);
    }
    /// `Shuriken_Update`'s terrain test (`$24DF8..$24E66`) where a spread
    /// shuriken stands: a stopping material or a negative X ends it.
    fn spread_terrain_at(
        terrain: &impl Terrain,
        shot: &mut SpreadShot,
        walls: &mut Vec<WallHit>,
        splashes: &mut Vec<(i32, i32, bool)>,
    ) {
        let at = (shot.x >> 16, shot.y >> 16);
        match PlayerWeapons::shuriken_terrain(terrain, at, &mut shot.in_water) {
            ShurikenTerrain::Fly => {}
            ShurikenTerrain::Splash => splashes.push((at.0, at.1, shot.velocity_x < 0)),
            ShurikenTerrain::Leave => shot.age = u16::MAX,
            ShurikenTerrain::Wall => {
                walls.push(WallHit {
                    at,
                    velocity: (shot.velocity_x, shot.velocity_y),
                    mapping: shot.mapping,
                    high_priority: shot.high_priority,
                    order: shot.order,
                });
                shot.age = u16::MAX;
            }
        }
    }
    /// `$24E24..$24E66`: entering material 15 plays effect `$27` and
    /// allocates list-1 type `$24` (`Func_022D2E`) at the shuriken with
    /// `+$2A` = `$18` moving right, `$24` left: the splash of wading
    /// (`combat::ambient::splash`).
    pub(super) fn splash_shuriken(
        &mut self,
        (x, y): (i32, i32),
        left: bool,
        event: &mut CombatEvents,
    ) {
        event
            .sound_ids
            .push(self.rules.cues.get(SoundEvent::ShurikenSplash));
        let wade = crate::movement::SplashKind::Wade { left };
        if let Some(splash) = super::ambient::splash(&self.rules.ambient, (x, y), wade.group()) {
            self.effects.push(splash);
        }
    }
    /// `$24E0A..$24E22`: a shuriken on a stopping material. Powered
    /// (`plr_status` bit 7) it is removed. Otherwise `Frame_Counter` bit 0
    /// picks: clear sticks it for 32 ticks (`loc_024C6C`), set knocks it back
    /// (`Func_024AFE`). The combat tick count stands in for the counter.
    pub(super) fn strike_wall(&mut self, wall: WallHit, event: &mut CombatEvents) {
        if self.weapons.powered {
            return;
        }
        let (vx, vy) = wall.velocity;
        let leftward = vx < 0;
        let mut spent = SpentShuriken {
            x: wall.at.0,
            y: wall.at.1,
            mapping: wall.mapping,
            flip: if leftward { 0x800 } else { 0 },
            motion: Motion::default(),
            life: 32,
            spin: None,
            high_priority: wall.high_priority,
            order: wall.order,
        };
        if self.ticks % 2 == 1 {
            // Func_024AFE: effect 8, a quarter of the speed back, 4 px/tick
            // up ($3A 32) or, if it was rising, no vertical speed ($3A 24),
            // under $FF0014 gravity; +$28 is the spread's (or throw's)
            // starting frame, mirrored when thrown left.
            event
                .sound_ids
                .push(self.rules.cues.get(SoundEvent::ShurikenBounce));
            let frame = usize::from(wall.mapping - 0xca);
            spent.spin = Some((
                if vx < 0 { -1 } else { 1 },
                if leftward { 14 - frame } else { frame },
            ));
            spent.motion.vx = -vx >> 2;
            spent.motion.vy = if vy < 0 { 0 } else { -4 << 16 };
            spent.motion.gravity = GRAVITY;
            if vy < 0 {
                spent.life = 24;
            }
        }
        // PlayerAttack_Run counts +$3A down in the same update.
        spent.life -= 1;
        self.weapons.spent.push(spent);
    }
    /// Stuck and knocked-back shurikens: `Func_024B5E` turns the spin every
    /// fourth tick, the object moves, then `PlayerAttack_Run` counts `+$3A`
    /// down and frees it at zero.
    pub(super) fn update_spent_shurikens(&mut self) {
        let frames = self.weapons.spin_frames;
        for spent in &mut self.weapons.spent {
            if let Some((step, index)) = &mut spent.spin
                && spent.life & 3 == 0
            {
                let next = *index as i32 + *step;
                *index = if next < 0 {
                    27
                } else if next >= 28 {
                    0
                } else {
                    next as usize
                };
                let frame = frames[*index];
                spent.mapping = 0xca + (frame & 7);
                spent.flip = frame & 0x1800;
            }
            spent.motion.step(&mut spent.x, &mut spent.y);
            spent.life -= 1;
        }
        self.weapons.spent.retain(|spent| spent.life > 0);
    }
    /// The sword object's hitbox `age` updates after creation, if its mapping
    /// entry is active then. The object follows the player each update.
    pub(super) fn swing_hitbox(
        &self,
        kind: Sword,
        age: u16,
        diving: bool,
        player: (i32, i32),
        facing_left: bool,
    ) -> Option<Hitbox> {
        let spec = kind.spec();
        if !spec.active.contains(&age) && !(diving && age >= *spec.active.start()) {
            return None;
        }
        let (x, y) = self.weapons.sword_origin(kind, player, facing_left);
        Some(
            self.mappings[&spec.mapping]
                .bounds
                .placed(x, y, facing_left, false),
        )
    }
    /// The box `Object_LoadHitbox` leaves the sword object `age` updates
    /// after creation: its active mapping's ([`Self::swing_hitbox`]), or
    /// else mapping 0's, which is empty (`0,0,0,0`) but still meets any box
    /// around the object's origin in `Object_CollideList`.
    pub(super) fn sword_box(
        &self,
        kind: Sword,
        age: u16,
        diving: bool,
        player: (i32, i32),
        facing_left: bool,
    ) -> Hitbox {
        self.swing_hitbox(kind, age, diving, player, facing_left)
            .unwrap_or_else(|| {
                let (x, y) = self.weapons.sword_origin(kind, player, facing_left);
                self.mapping_hitbox(0, x, y, facing_left)
            })
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
            if !Self::weapon_targetable(&self.roster, &self.spawns, enemy)
                || enemy.high_priority != self.weapons.high_priority
            {
                continue;
            }
            if screen.intersects(enemy_hitbox(&self.mappings, enemy)) {
                // Attacker types $50 and above leave no lethal spark.
                Self::hurt_enemy(
                    &self.roster,
                    enemy,
                    &self.spawns[enemy.spawn],
                    damage,
                    false,
                );
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
    /// List 3's shurikens, last allocated first: thrown ones (with their
    /// own priority), spread ones in their `$24EB2` poses and spent ones
    /// (both with the player's, `player_high`). A shuriken that meets a
    /// wall stays the same record, so a spent one keeps its place.
    pub fn draw_player_attacks(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        scene_tiles: &[Tile],
        camera: (i32, i32),
        player_high: bool,
    ) {
        enum Shuriken {
            Thrown(usize),
            Spread(usize),
            Spent(usize),
        }
        let thrown = self
            .projectiles
            .iter()
            .enumerate()
            .filter(|(_, shot)| shot.alive && !shot.hidden && !shot.enemy)
            .map(|(i, shot)| (shot.order, Shuriken::Thrown(i)));
        let spread = (self.weapons.shots.iter().enumerate())
            .map(|(i, shot)| (shot.order, Shuriken::Spread(i)));
        let spent = (self.weapons.spent.iter().enumerate())
            .map(|(i, spent)| (spent.order, Shuriken::Spent(i)));
        let mut listed: Vec<(u64, Shuriken)> = thrown.chain(spread).chain(spent).collect();
        listed.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        for (_, shuriken) in listed {
            match shuriken {
                Shuriken::Thrown(i) => {
                    let shot = &self.projectiles[i];
                    frame.mark_high_sprites(shot.high_priority);
                    self.draw_projectile(frame, palettes, scene_tiles, camera, shot);
                }
                Shuriken::Spread(i) => {
                    let shot = &self.weapons.shots[i];
                    let (base, left) = match shot.flips {
                        Some(flips) => (flips & 0x1000, flips & 0x800 != 0),
                        None => (0, shot.velocity_x < 0),
                    };
                    frame.mark_high_sprites(player_high);
                    self.draw_object(
                        frame,
                        palettes,
                        scene_tiles,
                        shot.mapping,
                        base,
                        ((shot.x >> 16) - camera.0, (shot.y >> 16) - camera.1),
                        left,
                    );
                }
                Shuriken::Spent(i) => {
                    let spent = &self.weapons.spent[i];
                    frame.mark_high_sprites(player_high);
                    self.draw_object(
                        frame,
                        palettes,
                        scene_tiles,
                        spent.mapping,
                        spent.flip & 0x1000,
                        (spent.x - camera.0, spent.y - camera.1),
                        spent.flip & 0x800 != 0,
                    );
                }
            }
        }
        frame.mark_high_sprites(false);
    }
}

/// A list-3 player attack object (`Object_List3`) as the list-6 and list-4
/// collision helpers (`$194A`) see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttackObject {
    /// A thrown shuriken (list-3 type 0), by `Combat::projectiles` index.
    Shot(usize),
    /// A somersault spread shuriken (type 1), by `PlayerWeapons::shots` index.
    Spread(usize),
    /// The sword object (types 9 and up).
    Sword,
}
impl AttackObject {
    /// `cmpi.w #4,obj_type` / `bls` (`$297A6`): types 0 and 1 are shurikens.
    pub(crate) fn shuriken(self) -> bool {
        !matches!(self, Self::Sword)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlayerAttack {
    pub(crate) object: AttackObject,
    pub(crate) hitbox: Hitbox,
    pub(crate) high_priority: bool,
}

impl Combat {
    /// A shot's box where it stands (`Object_LoadHitbox`): a player
    /// shuriken's, flipped by its drawn `obj_tile` bits, or the box an enemy
    /// shot's last update loaded ([`super::ShotBox`]).
    pub(super) fn shot_box(mappings: &BTreeMap<u16, Mapping>, shot: &Projectile) -> Hitbox {
        if let (true, super::ShotBox::Fixed(bounds)) = (shot.enemy, shot.box_load) {
            return bounds.placed(shot.x, shot.y, false, false);
        }
        let (mapping, horizontal, vertical) = match shot.box_frame {
            Some(frame) if shot.enemy => frame,
            _ => shot.drawn_frame(),
        };
        // Mapping 0 and any empty record load `0,0,0,0`: a point.
        mappings
            .get(&mapping)
            .map_or(Hitbox::NONE, |m| m.bounds)
            .placed(shot.x, shot.y, horizontal, vertical)
    }
    /// A spread shuriken's box where it stands (`Object_LoadHitbox`).
    fn spread_box(mappings: &BTreeMap<u16, Mapping>, shot: &SpreadShot) -> Hitbox {
        let (horizontal, vertical) = shot.tile_flips();
        mappings.get(&shot.mapping).map_or(Hitbox::NONE, |m| {
            m.bounds
                .placed(shot.x >> 16, shot.y >> 16, horizontal, vertical)
        })
    }
    /// The player's live attack objects and their hitboxes, in list order.
    pub(crate) fn player_attacks(&self) -> Vec<PlayerAttack> {
        let mut attacks = Vec::new();
        for (index, shot) in self.projectiles.iter().enumerate() {
            if shot.enemy || !shot.alive {
                continue;
            }
            attacks.push(PlayerAttack {
                object: AttackObject::Shot(index),
                hitbox: Self::shot_box(&self.mappings, shot),
                high_priority: shot.high_priority,
            });
        }
        for (index, shot) in self.weapons.shots.iter().enumerate() {
            attacks.push(PlayerAttack {
                object: AttackObject::Spread(index),
                hitbox: Self::spread_box(&self.mappings, shot),
                high_priority: shot.high_priority,
            });
        }
        if let Some(hitbox) = self.weapons.swing_box {
            attacks.push(PlayerAttack {
                object: AttackObject::Sword,
                hitbox,
                high_priority: self.weapons.high_priority,
            });
        }
        attacks
    }
    pub(super) fn attack_damage(&self, attack: AttackObject) -> u8 {
        match attack {
            AttackObject::Shot(i) => self.projectiles[i].damage,
            AttackObject::Spread(i) => self.weapons.shots[i].damage,
            AttackObject::Sword => self.weapons.swing.map_or(1, |s| s.kind.spec().damage),
        }
    }
    /// A shuriken that collided with a stage object: `Shuriken_Update`
    /// removes it and plays its hit voice (`$24DE0`). Returns the command.
    /// The sword is not spent; the object decides what it plays.
    pub(crate) fn spend_attack(&mut self, object: AttackObject) -> Option<u8> {
        match object {
            AttackObject::Shot(index) => self.projectiles.get_mut(index)?.alive = false,
            AttackObject::Spread(index) => self.weapons.shots.get_mut(index)?.age = u16::MAX,
            AttackObject::Sword => return None,
        }
        Some(self.rules.cues.get(SoundEvent::ShurikenHit))
    }
    /// A list-4 object's `Object_CollideL3` also marks a sword as hit:
    /// `$252E4` spends the air stab and sets the player's rebound flag.
    pub(super) fn spend_projectile_attack(
        &mut self,
        object: AttackObject,
        event: &mut CombatEvents,
    ) {
        if object != AttackObject::Sword {
            event.sound_ids.extend(self.spend_attack(object));
            return;
        }
        let Some(swing) = self.weapons.swing else {
            return;
        };
        event
            .sound_ids
            .push(self.rules.cues.get(swing.kind.hit_sound()));
        event.bounce |= matches!(swing.kind, Sword::AirStab | Sword::SurfStab);
        self.weapons.swing_box = None;
        if swing.kind != Sword::DashKick {
            self.weapons.swing = None;
        }
    }
    /// The [`super::Strike::hold`]s of the attacks waiting for their hit.
    pub(super) fn held_attacks(&self) -> Vec<u32> {
        let shots = self
            .projectiles
            .iter()
            .filter(|shot| !shot.enemy && shot.alive);
        let spread = self
            .weapons
            .shots
            .iter()
            .filter(|shot| shot.age != u16::MAX);
        shots
            .filter_map(|shot| shot.held)
            .chain(spread.filter_map(|shot| shot.held))
            .chain(self.weapons.held_sword)
            .chain(self.weapons.swing.and_then(|swing| swing.held))
            .collect()
    }
    /// Free the held attacks whose hit `landed`, and release the others,
    /// which run on: a shuriken's terrain test at the step it met the
    /// object on comes first ([`Projectile::untested`]). A landed hit frees the sword too (`$24C92`, `$251EE`,
    /// `$252E4`), except the dash kick, which only plays its sound
    /// (`$254D8`) and stays with bit 14 hiding it from the list-6 and
    /// list-4 checks until its update. A replaced sword ends either way.
    pub(super) fn free_held_attacks(&mut self, landed: &[u32]) {
        for shot in self.projectiles.iter_mut().filter(|shot| !shot.enemy) {
            if let Some(hold) = shot.held.take() {
                if landed.contains(&hold) {
                    shot.alive = false;
                } else {
                    shot.untested = shot.alive;
                }
            }
        }
        for shot in &mut self.weapons.shots {
            if let Some(hold) = shot.held.take() {
                if landed.contains(&hold) {
                    shot.age = u16::MAX;
                } else {
                    shot.untested = true;
                }
            }
        }
        self.weapons.drop_spent_shots();
        self.weapons.held_sword = None;
        if let Some(swing) = &mut self.weapons.swing
            && swing.held.take().is_some_and(|hold| landed.contains(&hold))
        {
            self.weapons.swing_box = None;
            if swing.kind != Sword::DashKick {
                self.weapons.swing = None;
            }
        }
    }
    /// Drop spent attacks after stage objects (after `spend_attack`, which
    /// keeps shuriken indices).
    pub(crate) fn remove_spent_attacks(&mut self) {
        // Stage objects run before the next player update. Ordinary expired
        // shots are spent here; the Hydra hand's synthetic list-4 contact
        // must survive until Player_CheckDamage tests it.
        for shot in &mut self.projectiles {
            if !shot.alive && !shot.keep_contact {
                shot.touching = false;
            }
        }
        self.retain_projectiles(true);
        self.weapons.shots.retain(|shot| shot.age != u16::MAX);
    }
    /// Hitbox of mapping `id` placed at (x, y) (`$18F4` then the offsets).
    pub(crate) fn mapping_hitbox(&self, id: u16, x: i32, y: i32, left: bool) -> Hitbox {
        mapping_hitbox(&self.mappings, id, x, y, left)
    }
    /// Test hook: a shot's box ([`Self::shot_box`]).
    /// The mapping's own hitbox record, relative to the object.
    pub(crate) fn mapping_bounds(&self, id: u16) -> Hitbox {
        self.mappings.get(&id).map_or(Hitbox::NONE, |m| m.bounds)
    }
    /// ORs `$8000` into the player's `obj_tile` (`$22C4C`, `Boss5`): the
    /// player and its weapons join the high sprite-priority layer.
    pub(crate) fn raise_player_priority(&mut self) {
        self.player_priority_raised = true;
        self.weapons.high_priority = true;
    }
    /// Test hook: the player's `obj_tile` bit 15 as a part would set it.
    pub(crate) fn player_high_priority(&self) -> bool {
        self.weapons.high_priority
    }
    /// The player's full body hitbox from the frame shown on the previous
    /// update (`Object_LoadHitbox` via `Player_FrameTable`, `$23ADE`).
    pub(crate) fn player_body(&self, player: (i32, i32), facing_left: bool) -> Hitbox {
        let mapping = self
            .player_mappings
            .get(self.player_frame)
            .copied()
            .unwrap_or(0);
        mapping_hitbox(&self.mappings, mapping, player.0, player.1, facing_left)
    }
}
