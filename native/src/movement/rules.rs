//! What each original state allows: the +$2A handler bits, the attack
//! postures and the airborne and hang button actions.
use super::state;

/// States handled by `PlayerState_Death` (`$204E4`) and
/// `PlayerState_DeathHidden` (`$204E0`).
pub fn dying(state: usize) -> bool {
    use state::*;
    matches!(
        state,
        MIJIN_DEATH_SURF
            | DEATH
            | MIJIN_DEATH
            | PIT_DEATH
            | THROWN_DEATH
            | HORSE_DEATH
            | SURF_DEATH
    )
}

/// `Player_StepAnimation` (`$21546..$2158A`): the states in which immunity
/// hides Joe on alternate updates. The hurt, knockdown, shocked and death
/// states (40..=47), the horse hurt and death (50, 54), the surf death (62),
/// the dash slash (29) and every ninjutsu state (64 and up) never flicker.
pub fn immunity_flickers(state: usize) -> bool {
    !matches!(state, 29 | 40..=47 | 50 | 54 | 62 | 64..)
}

/// `PlayerState_DeathHidden` sets `obj_flags` bit 11 on every update: the
/// player sprite is not drawn in states 42 and 54.
pub fn death_hidden(state: usize) -> bool {
    matches!(state, state::MIJIN_DEATH | state::HORSE_DEATH)
}

/// Attack started by combat on this tick; posture selects the original state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attack {
    Slash,
    Throw,
}

/// The branch `$22F7E` takes for B in the state its handler left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackPosture {
    /// States 0-4, 26, 27: slash 5 or throw 7.
    Standing,
    /// States 8-11, 15: 12 or 14.
    Crouching,
    /// State 16 without Down: 18 or 22.
    Airborne,
    /// State 25 without Down: 37 or 39.
    WallAir,
    /// State 16 or 25 with Down held (`$2306C`): state 20.
    AirStab,
    /// State 28 (`$23280`): state 29.
    DashKick,
    /// State 17: the eight-way spread `$2319E`, without a state change.
    Spread,
    /// State 32 (`$230D6`): 34 or 36.
    Hang,
    /// Mounted on the horse, state 48 or 51 without Down (`$230F6`):
    /// throw 49, slash 52 (object 3) or powered slash 55 (object 4).
    Horse,
    /// On the surfboard, state 56 or 59 without Down (`$23142`): throw 57,
    /// slash 61 (object 5) or powered slash 63 (object 6).
    Surf,
}

/// `$22F7E..$23004` for the ordinary (non-alternate) player. `down` is A4+$02
/// above zero. A native air stab also needs the player to be airborne.
/// Native air throws accept another attack before their animation finishes.
pub fn attack_posture(state: usize, down: bool, grounded: bool) -> Option<AttackPosture> {
    use state::*;
    Some(match state {
        0..=4 | 26 | 27 => AttackPosture::Standing,
        8..=11 | 15 => AttackPosture::Crouching,
        JUMP | WALL_CLING if down && !grounded => AttackPosture::AirStab,
        JUMP => AttackPosture::Airborne,
        AIR_THROW if !grounded => AttackPosture::Airborne,
        WALL_CLING => AttackPosture::WallAir,
        SOMERSAULT => AttackPosture::Spread,
        RUN => AttackPosture::DashKick,
        HANG => AttackPosture::Hang,
        _ => return None,
    })
}

/// `plr_ctrl` bit 10 after the state handler of `state` ran: `Player_Attack`
/// (`$22F7E`) refuses B while it is set, and `Player_Physics` clears it
/// only after the buttons (`$21462`). These handlers set it before their
/// end test, so B is refused on every update they run, the one on which
/// they end their state included (the new state's handler runs from the
/// next update):
/// the ground attacks (`$20C74` slash and combo, `$20CFC` powered slash and
/// throw, `$20D18` crouch attacks), the air attacks (`$20D34`), the wall
/// attacks (`$20D90`), the air stab (`$20E14`, also run by the mounted
/// stabs), unused state 21 (`$20E98`), the hang attacks (`$20ED8`, through
/// `plr_event`), the dash kick (`$20F02`), the mounted attacks (`$20C2C`,
/// `$20C50`), the deaths (`$204FC`) and every ninjutsu state, whose updates
/// end in `Player_Freeze` (`$21A62`; `Ninjutsu_Finish`, `$2192E`, returns
/// through it). The hurt, knockdown, shocked, thrown and mounted hurt
/// handlers set it only after their end test (`$202DA`, `$2032C`, `$203D2`,
/// `$2036E`, `$20396`), and state 25 only on the ground and after its end
/// test (`$21328`), so B is taken on the update those end. `ended` is
/// `plr_status` bit 0 and `grounded` the last update's floor contact
/// (`plr_status` bit 5 clear).
/// Native air throws omit this lock while airborne to accept repeated shots.
pub fn attack_locked(state: usize, ended: bool, grounded: bool) -> bool {
    use state::*;
    match unpowered(state) {
        AIR_THROW => grounded,
        SLASH | THROW | CROUCH_SLASH | CROUCH_THROW | AIR_SLASH | AIR_STAB | 21 | COMBO
        | DASH_KICK | HANG_SLASH | HANG_THROW | WALL_SLASH | WALL_THROW => true,
        // 49 and 52 the horse attacks, 57 and 61 the surf attacks, 53 and
        // 60 the mounted stabs.
        HORSE_ATTACK | HORSE_SLASH | HORSE_STAB | SURF_ATTACK | SURF_SLASH | SURF_STAB => true,
        // 50 and 58: the mounted hurt states.
        HURT | KNOCKDOWN | SHOCKED | THROWN | 50 | 58 => !ended,
        WALL_CLING => grounded && !ended,
        id => dying(id) || id >= NINJUTSU,
    }
}

/// The hurt, knockdown and shocked states. Their handlers set +$2A bits
/// 9..11 (`ori.w #$E00`, `$202DA`, `$2032C`), so no jump, attack or
/// ninjutsu starts in them.
pub fn hurt(state: usize) -> bool {
    matches!(state, state::HURT | state::KNOCKDOWN | state::SHOCKED)
}

/// `Player_ApplyHit` (`$21E3C..$21E80`): the state a hit of `damage` enters.
pub fn hit_state(damage: u8, shocked: bool) -> usize {
    if damage >= 2 {
        state::KNOCKDOWN
    } else if shocked {
        state::SHOCKED
    } else {
        state::HURT
    }
}

/// `$23404` rejects the jump when the state handler set +$2A bit 9: airborne,
/// hurt, thrown (`$203D2` `ori.w #$E00`) and airborne-attack states, plus
/// every ninjutsu state.
pub fn jump_allowed(state: usize) -> bool {
    use state::*;
    let state = unpowered(state);
    !matches!(
        state,
        SOMERSAULT
            | AIR_SLASH
            | AIR_STAB
            | AIR_THROW
            | WALL_CLING
            | DASH_KICK
            | WALL_SLASH
            | WALL_THROW
            | THROWN
    ) && !hurt(state)
        && state < NINJUTSU
        && !dying(state)
}

/// `Player_JumpButton` (`$23404..$2347E`): C enters state 16 at cursor 0
/// in the state the handler left, `cursor` being its cursor. State 16
/// itself jumps again only in its landing slot 4 (`$2344E`); otherwise
/// bit 9 refuses, and the hang, horse and surf states (`andi.w #$78`) have
/// their own C. No floor is tested: a ground state stepping down a small
/// drop (`plr_ctrl` bit 8), or one the last update carried past an edge,
/// still jumps.
pub fn starts_jump(state: usize, cursor: u16) -> bool {
    use state::*;
    match unpowered(state) {
        JUMP => cursor >> 8 == 4,
        id if matches!(id & 0x78, HANG | HORSE | SURF) => false,
        id => jump_allowed(id),
    }
}

/// `$22E8E` rejects A while +$2A bit 11 is set: attacks, hurt, thrown
/// (`$203D2`) and airborne attacks. Jumping, somersault, wall cling and guard
/// states may cast.
pub fn ninjutsu_allowed(state: usize) -> bool {
    use state::*;
    let state = unpowered(state);
    !matches!(
        state,
        SLASH
            | THROW
            | CROUCH_SLASH
            | CROUCH_THROW
            | AIR_SLASH
            | AIR_STAB
            | AIR_THROW
            | COMBO
            | DASH_KICK
            | WALL_SLASH
            | WALL_THROW
            | HANG_SLASH
            | HANG_THROW
            | THROWN
    ) && !hurt(state)
        && state < NINJUTSU
        && !dying(state)
}

/// +$2A bit 15 (`$1FFF4`, `$20C74`, `$20D12`, `$20D2E`): hits from the facing
/// side are blocked (`$21CFC`). Guards and sword swings set it.
pub fn guards(state: usize) -> bool {
    use state::*;
    matches!(
        unpowered(state),
        GUARD
            | GUARD_WALK
            | CROUCH_GUARD
            | CROUCH_GUARD_WALK
            | SLASH
            | COMBO
            | CROUCH_SLASH
            | AIR_SLASH
            | HANG_SLASH
    )
}

/// Airborne jump presses accepted inside the state-16/25 handlers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `$21098`: state 17, Y-4, vy -5 px/tick.
    Somersault,
    /// `$21102..$21112`: turn away from the wall, state 25 at cursor 0.
    WallCling,
    /// `Player_TryHang` (`$21118`) from the airborne handlers: state 32.
    Hang,
}

/// C in the hang states (`$234F2`, states 32-39 by `and.w #$78`; the wall
/// attacks 37-39 never reach it because they block the jump button).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HangExit {
    /// Down: Y + 8, then `$218DA` falls at cursor `$300`.
    Drop,
    /// Up under 2, 6 or 14 with room above: state 16 at cursor `$100`.
    Climb,
}

/// States whose C press is the hang exit.
pub fn hang_exits(state: usize) -> bool {
    use state::*;
    matches!(unpowered(state), HANG | HANG_SLASH | HANG_THROW)
}
