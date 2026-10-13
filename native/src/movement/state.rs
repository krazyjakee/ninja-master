//! Original player state IDs. Object word +$2C is both the `$1FDF8` handler
//! index and the `$235F6` animation index, so each state has one sequence.
pub const IDLE: usize = 1;
pub const WALK: usize = 2;
/// Held B: guard `$1FFDC` (frame 130) and guard walk.
pub const GUARD: usize = 3;
pub const GUARD_WALK: usize = 4;
pub const SLASH: usize = 5;
/// The powered-up states (`plr_status` bit 7, `Player_PowerUp`). Each
/// shares its base state's handler except 6 (`PlayerState_SlashPowered`,
/// which has no combo), and has its own sequence; see `unpowered`.
pub const SLASH_POWERED: usize = 6;
pub const THROW: usize = 7;
pub const CROUCH: usize = 8;
pub const CRAWL: usize = 9;
/// Held B while crouching: `$200A4`.
pub const CROUCH_GUARD: usize = 10;
pub const CROUCH_GUARD_WALK: usize = 11;
pub const CROUCH_SLASH: usize = 12;
pub const CROUCH_SLASH_POWERED: usize = 13;
pub const CROUCH_THROW: usize = 14;
pub const CROUCH_GUARD_WALK_POWERED: usize = 15;
pub const JUMP: usize = 16;
/// `$212A6`, entered from `$21098`.
pub const SOMERSAULT: usize = 17;
pub const AIR_SLASH: usize = 18;
pub const AIR_SLASH_POWERED: usize = 19;
/// Down + B in the air: `$20E14`.
pub const AIR_STAB: usize = 20;
pub const AIR_THROW: usize = 22;
/// B again during a slash: `$20CEE` keeps the cursor.
pub const COMBO: usize = 24;
/// Wall cling and wall jump: `$212FE`, entered from `$21108`.
pub const WALL_CLING: usize = 25;
/// `PlayerState_Stand` (`$1FF10`) and `PlayerState_Guard` (`$20054`) walk
/// in these while powered.
pub const WALK_POWERED: usize = 26;
pub const GUARD_WALK_POWERED: usize = 27;
pub const RUN: usize = 28;
/// B while running: `$23280`, handler `$20F02`.
pub const DASH_KICK: usize = 29;
/// Attacks from state 25 (`$230A6`).
pub const WALL_SLASH: usize = 37;
pub const WALL_SLASH_POWERED: usize = 38;
pub const WALL_THROW: usize = 39;
/// `Player_ApplyHit` (`$21E54..$21E80`) enters one of three hurt states:
/// 40 for a hit of 0 or 1, 44 (the knockdown, `PlayerState_Knockdown`)
/// for 2 or more and 47 (`PS_Shocked`, run by `PlayerState_Hurt`) for
/// damage code `$2F` below 2.
pub const HURT: usize = 40;
pub const KNOCKDOWN: usize = 44;
/// `plr_event` 1/2 plus a colliding hit enters the lift, turn and slam
/// sequence (`PlayerState_Thrown`, `$203A8`).
pub const THROWN: usize = 45;
pub const SHOCKED: usize = 47;
/// Ceiling hang `$201C0`, entered by `Player_TryHang` (`$21118`).
pub const HANG: usize = 32;
/// B while hanging (`$230D6`): slash `$20ED2` (35 is the powered-up
/// slash, alternate art) and throw `$20ED8`.
pub const HANG_SLASH: usize = 34;
pub const HANG_SLASH_POWERED: usize = 35;
pub const HANG_THROW: usize = 36;
/// `$22F4A`: 64 + $FF37E7 selects Ikazuchi, Kariu, Fushin or Mijin.
pub const NINJUTSU: usize = 64;
/// Round 2-1 mounted states (`$205B8`, `$20C5E`, `$20E2A`).
pub const HORSE: usize = 48;
pub const HORSE_ATTACK: usize = 49;
pub const HORSE_JUMP: usize = 51;
pub const HORSE_SLASH: usize = 52;
pub const HORSE_STAB: usize = 53;
pub const HORSE_SLASH_POWERED: usize = 55;
/// Round 4-1 mounted states (`$2064E`, `$20C38`, `$20E20`).
pub const SURF: usize = 56;
pub const SURF_ATTACK: usize = 57;
pub const SURF_JUMP: usize = 59;
pub const SURF_STAB: usize = 60;
pub const SURF_SLASH: usize = 61;
pub const SURF_SLASH_POWERED: usize = 63;
/// Mounted ninjutsu variants: 68..=71 horse, 72..=75 surf.
pub const HORSE_NINJUTSU: usize = 68;
pub const SURF_NINJUTSU: usize = 72;
/// Death states, all run by `PlayerState_Death` (`$204E4`, see
/// `crate::death`). Mijin on the surfboard without a spare life
/// (`$20758`).
pub const MIJIN_DEATH_SURF: usize = 30;
/// `Player_Die` (`$21B12`): a lethal hit on foot.
pub const DEATH: usize = 41;
/// Mijin without a spare life (`$20746`); `PlayerState_DeathHidden`.
pub const MIJIN_DEATH: usize = 42;
/// Below the screen (`Player_ApplyEvent`, `$1FD84`).
pub const PIT_DEATH: usize = 43;
/// A lethal slam at thrown cursor `$600` (`$20418`); the cursor is kept.
pub const THROWN_DEATH: usize = 46;
/// `Player_Die` on the horse (`$21AEA`); `PlayerState_DeathHidden`.
pub const HORSE_DEATH: usize = 54;
/// `Player_Die` on the surfboard (`$21AB0`).
pub const SURF_DEATH: usize = 62;

/// The ordinary state a powered-up state stands for: 6, 13, 15, 19, 26,
/// 27, 35, 38, 55 and 63 become 5, 12, 11, 18, 2, 4, 34, 37, 52 and 61.
/// The motion, collision and permission rules are the base state's
/// (`Player_StateTable` shares the handlers); only the sequence and the
/// sword object differ. Other states map to themselves.
pub const fn unpowered(state: usize) -> usize {
    match state {
        SLASH_POWERED => SLASH,
        CROUCH_SLASH_POWERED => CROUCH_SLASH,
        CROUCH_GUARD_WALK_POWERED => CROUCH_GUARD_WALK,
        AIR_SLASH_POWERED => AIR_SLASH,
        WALK_POWERED => WALK,
        GUARD_WALK_POWERED => GUARD_WALK,
        HANG_SLASH_POWERED => HANG_SLASH,
        WALL_SLASH_POWERED => WALL_SLASH,
        HORSE_SLASH_POWERED => HORSE_SLASH,
        SURF_SLASH_POWERED => SURF_SLASH,
        other => other,
    }
}
