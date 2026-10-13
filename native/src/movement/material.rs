//! Collision materials and the player's wall-probe rows.
use super::state;

/// `Player_CollideWalls` (`$222C2..$22336`) probes the leading column at
/// the bottom row of the body (`$222C4`), 23 px higher (`$222F0`), then
/// twice more 16 px apart (`$22324`, `$2232E`): 56 px above the feet. The
/// crouching states 8-15 and the dash kick from cursor `$300` skip the
/// third probe (40 px). The somersault and the air stab probe only 17 px
/// up. `plr_flags` bit 3 also selects the 17-px probes; native play does
/// not set it.
pub const WALL_PROBE_STANDING: i32 = 56;
/// The rows, above the feet, of those probes in their order.
pub const WALL_PROBE_ROWS: [i32; 4] = [1, 24, 40, 56];
/// `plr_flags` bit 2 (`$222C6..$22300`): the first wall-probe hit came from
/// one of the first two rows. `Player_TryWallKick` needs it for a cling.
pub const CLING_PROBES: usize = 2;
pub const WALL_PROBE_LOW: i32 = 40;
pub const WALL_PROBE_TUCKED: i32 = 17;

/// The height of the wall probes for the state and cursor
/// `Player_Physics` sees. Natively every row up to it is tested; the
/// original tests the rows listed at `WALL_PROBE_STANDING`.
pub fn wall_probe_height(state: usize, cursor: u16) -> i32 {
    match state {
        state::SOMERSAULT | state::AIR_STAB => WALL_PROBE_TUCKED,
        8..=15 => WALL_PROBE_LOW,
        state::DASH_KICK if cursor >= 0x300 => WALL_PROBE_LOW,
        _ => WALL_PROBE_STANDING,
    }
}

/// The rows above the feet that `Player_CollideWalls` probes, in order,
/// for a `wall_probe_height`: the tucked states jump from the first probe
/// to the last (`$222DE..$222EE` to `$2232E`), 16 px higher.
pub fn wall_probe_rows(height: i32) -> &'static [i32] {
    match height {
        WALL_PROBE_TUCKED => &[1, WALL_PROBE_TUCKED],
        WALL_PROBE_LOW => &[1, 24, WALL_PROBE_LOW],
        _ => &WALL_PROBE_ROWS,
    }
}

/// Low nibble of a `$3854` collision byte after quadrant exclusion. The
/// lookup itself only tags materials in the `$3906` mask (bit 15 of D2);
/// every caller then classifies the code with its own mask. The player's
/// masks below come from `Player_Physics` and apply after
/// `Player_MaterialRemap` (`$21BBA`), which maps 10 and 11 to 0 for both
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Material(pub u8);
impl Material {
    pub const EMPTY: Self = Self(0);
    /// Solid on every side.
    pub const SOLID: Self = Self(1);
    /// One-way floor that can be hung from (ceiling contact, no bump).
    pub const LEDGE: Self = Self(2);
    /// Solid; standing on it adds +1 / -1 px/tick (`$2204E..$22062`).
    pub const CONVEYOR_RIGHT: Self = Self(3);
    pub const CONVEYOR_LEFT: Self = Self(4);
    /// Solid; damages from the floor, walls or ceiling (`$21C44..$21C94`).
    pub const SPIKES: Self = Self(5);
    /// Hang bar: ceiling contact only; not a floor or wall.
    pub const HANG_BAR: Self = Self(6);
    /// One-way floor: not a wall or ceiling, and cannot be hung from.
    pub const PLATFORM: Self = Self(7);
    /// Floor and wall that refuses a wall cling (`$210F2`); no ceiling.
    pub const NO_CLING: Self = Self(8);
    /// Wall that only stops rightward motion (`$22384`).
    pub const WALL_RIGHT: Self = Self(9);
    /// Slows: floor without snapping that the player sinks through (`$21FE0`,
    /// `$2207E`), half horizontal speed (`$22370`), weaker jump (`$20FEA`).
    pub const MIRE: Self = Self(12);
    /// Wall that only stops leftward motion (`$22394`).
    pub const WALL_LEFT: Self = Self(13);
    /// Not solid; entering or wading makes a splash (sound `$27`, object `$24`;
    /// `$21F50`, `Player_CollideCeiling`, `$223A4`).
    pub const SPLASH: Self = Self(15);

    /// `$3906` long `$55BE5ABE`: the high word for objects whose sprite
    /// priority bit (`obj_tile` bit 15) is clear, the low word when set.
    /// `$3854` sets D2 bit 15 for these; most object handlers test it with
    /// `BMI` or `ANDI.W #$8000`.
    pub const OBJECT_SOLID: [u16; 2] = [0x55be, 0x5abe];
    /// `Player_SolidMaterialMask` (`$220EC`): floor probes (`Player_ProbeSolid`).
    pub const PLAYER_FLOOR: u16 = 0x5dbe;
    /// `$2259E`: ceiling contact (`Func_022494`, `plr_ctrl` bit 6).
    pub const PLAYER_CEILING: u16 = 0x4c7e;
    /// `$2248C`: wall probes (`Func_022456`) record the material.
    pub const PLAYER_WALL_PROBE: u16 = 0xa33a;
    /// `$22490`: recorded wall materials that always stop the player.
    pub const PLAYER_WALL: u16 = 0x013a;
    /// `Player_BlockingMaterials` (`$1FFD8`, long `$FFFFAF3A`): the crouch
    /// headroom probe (`$1FF84`) and the end of a ninjutsu (`$219DE`).
    pub const PLAYER_HEADROOM: u16 = 0xaf3a;
    /// `$24ADC` (`Func_024AB0`, `Shuriken_Update`): table value 1 removes a
    /// player shuriken, 2 splashes.
    pub const SHURIKEN_STOP: u16 = 0x401a;

    fn code(self) -> u8 {
        self.0 & 15
    }
    fn in_mask(self, mask: u16) -> bool {
        (mask >> self.code()) & 1 != 0
    }
    /// `Player_MaterialRemap` (`$21BBA`): 10 and 11 read as empty.
    pub fn for_player(self) -> Self {
        match self.code() {
            10 | 11 => Self::EMPTY,
            code => Self(code),
        }
    }
    /// `$38E0..$38F8`: D2 bit 15, the generic solidity ordinary objects test.
    pub fn object_solid(self, high_priority: bool) -> bool {
        self.in_mask(Self::OBJECT_SOLID[usize::from(high_priority)])
    }
    /// Player floor (`Player_ProbeSolid` `$220D8`, `$21FAA`): lands and stands on it.
    pub fn stands_on(self) -> bool {
        self.for_player().in_mask(Self::PLAYER_FLOOR)
    }
    /// Crouch headroom (`$1FF84..$1FF8A`): keeps a crouching player down and
    /// blocks the jump: 1, 3, 4, 5, 8, 9, 13 and 15. The crouching guard
    /// lists the codes that do not block (`$200F6..$20116`: 0, 2, 6, 7, 12,
    /// 14); after the remap of 10 and 11 that is the same set.
    pub fn blocks_standing(self) -> bool {
        self.for_player().in_mask(Self::PLAYER_HEADROOM)
    }
    /// Player head probe (`$224CA..$224D4`, `plr_ctrl` bit 6): touches it from below.
    pub fn ceiling_contact(self) -> bool {
        self.for_player().in_mask(Self::PLAYER_CEILING)
    }
    /// `$224DA..$224F2`: stops a rising player. 2, 6 and 14 are passed through.
    pub fn blocks_ceiling(self) -> bool {
        self.ceiling_contact() && !matches!(self.code(), 2 | 6 | 14)
    }
    /// `Player_TryHang` (`$21126..$21134`): Up while touching it enters the hang.
    pub fn hangable(self) -> bool {
        self.ceiling_contact() && !matches!(self.code(), 5 | 7)
    }
    /// `Player_CollideWalls` (`$22366..$223A2`): stops horizontal motion in
    /// `direction` (-1 left, 0 none, 1 right). 9 and 13 are one-way walls.
    pub fn blocks_horizontal(self, direction: i32) -> bool {
        let m = self.for_player();
        m.in_mask(Self::PLAYER_WALL)
            || (m.code() == 9 && direction > 0)
            || (m.code() == 13 && direction < 0)
    }
    /// `Func_022456` (`$2248C`): the wall probes record it.
    pub fn wall_probe(self) -> bool {
        self.for_player().in_mask(Self::PLAYER_WALL_PROBE)
    }
    /// `Player_TryWallKick` (`$210F0..$210F6`): a blocking wall other than 8.
    pub fn clingable(self, direction: i32) -> bool {
        self.blocks_horizontal(direction) && self.code() != 8
    }
    /// Blocks the player from every side: 1, 3, 4 and 5.
    pub fn full_block(self) -> bool {
        self.stands_on() && self.blocks_ceiling() && self.blocks_horizontal(0)
    }
    /// `Player_CheckDamage` (`$21C44..$21C94`): spikes.
    pub fn hurts(self) -> bool {
        self.code() == 5
    }
    /// Material 12: sinking floor, half speed, weak jump.
    pub fn slows(self) -> bool {
        self.code() == 12
    }
    /// `$2204E..$22062`: px/tick added to the standing player's X velocity.
    pub fn conveyor(self) -> i32 {
        match self.code() {
            3 => 1,
            4 => -1,
            _ => 0,
        }
    }
    /// `Func_024AB0`: a player shuriken stops here (1, 3, 4 and 14). Both
    /// sprite-priority columns of the `$24ADC` table agree.
    pub(crate) fn stops_shuriken(self) -> bool {
        self.in_mask(Self::SHURIKEN_STOP)
    }
    /// `Func_024AB0` value 2: a player shuriken splashes here and flies on.
    pub(crate) fn splashes_shuriken(self) -> bool {
        self.code() == Self::SPLASH.0
    }
    // Whether any recovered classification gives the code a behavior.
}
