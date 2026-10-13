//! The player's movement: recovered constants, terrain materials, jump and
//! run control, the original state/animation model and the native `Player`,
//! all independent of SDL.
mod animation;
mod control;
mod material;
mod player;
mod rules;
pub mod state;

pub use crate::assets::{PlayerAnimation as Sequence, sequence, sequences, step_cursor};
pub use animation::{AnimationInput, PlayerAnimator};
pub use material::Material;
pub(crate) use player::WallProbe;
pub use player::{Player, PlayerEvent, PlayerEvents, PlayerHooks, Splash, SplashKind};

pub use rules::{Action, Attack, AttackPosture, death_hidden, dying, guards, immunity_flickers};

pub const ROM_UNIT: i32 = 65536;
pub const NATIVE_UNIT: i32 = 256;
/// $217B0: ordinary horizontal speed returned by $217AE.
pub const WALK_SPEED: i32 = 448;
/// $217DC: running horizontal speed returned by $217DA.
pub const DASH_SPEED: i32 = 768;
/// $217C6: slower movement helper; not the ordinary walking path.
pub const SLOW_SPEED: i32 = 192;
/// $1FC56: acceleration +$3400 in 16.16 units, exactly 52 in 8.8.
pub const GRAVITY: i32 = 52;
/// $20FF2: ordinary jump, with player +$29 bit 1 clear and material != 12.
pub const JUMP_SPEED: i32 = -1200;
/// $20FC8/$21020: subsequent ascending ticks eligible for zero gravity.
pub const JUMP_HOLD_TICKS: u8 = 11;
/// $2346E/$214FE/$20FB0 and state-16 sequence $237B2 ($40 phase increment).
pub const TAKEOFF_DELAY_TICKS: u8 = 4;
/// $220BE..$220D2: threshold applied BEFORE integration (not a post-step cap).
pub const FALL_THRESHOLD: i32 = 7 * NATIVE_UNIT;
/// $20FDE/$213E4: jump with +$29 bit 1 set, which Fushin sets at $2097A.
pub const FUSHIN_JUMP_SPEED: i32 = -1536;
/// $21088: a somersault needs |vy| < $17000 (1.4375 px/tick) when C is pressed.
pub const SOMERSAULT_WINDOW: i32 = 368;
/// $210A6: somersault vertical velocity $FFFB0000, -5 px/tick.
pub const SOMERSAULT_SPEED: i32 = -1280;
/// $210A2 and $212C4: the somersault lifts Y by 4 on its first two updates.
pub const SOMERSAULT_LIFT: i32 = 4;
/// $21E36: non-lethal damage sets vy to -2 px/tick.
pub const HURT_HOP: i32 = -512;
/// $202EC..$20300: hurt state 40 moves 1 px/tick away from the facing side.
pub const KNOCKBACK_SPEED: i32 = 256;
/// $21E4A: a hit during the somersault moves Joe 15 px down.
pub const SOMERSAULT_HIT_DROP: i32 = 15;
/// $2033C..$20350: the knockdown (44) moves 3 px/tick away from the facing
/// side for its first three frame slots, then 1 px/tick.
pub const KNOCKDOWN_SPEED: i32 = 768;
pub const KNOCKDOWN_FAST_SLOTS: u16 = 3;
/// $21D58: a guarded hit sets +$32 to damage * 8 recoil ticks.
pub const GUARD_RECOIL_PER_DAMAGE: u8 = 8;
/// $23280: the dash kick hops with vy -2 px/tick.
pub const DASH_KICK_HOP: i32 = -512;
/// $20F24..$20F44: dash kick 3 px/tick airborne, 2 px/tick on the ground.
pub const DASH_KICK_AIR_SPEED: i32 = 768;
pub const DASH_KICK_GROUND_SPEED: i32 = 512;
/// $2308E..$230A2: the air stab lifts Y by 16 and falls at least 2 px/tick.
pub const AIR_STAB_LIFT: i32 = 16;
pub const AIR_STAB_MIN_FALL: i32 = 512;
/// $20E84..$20E92: 3 px/tick forward while falling, 1 px/tick back while rising.
pub const AIR_STAB_SPEED: i32 = 768;
/// $20E60: an air-stab hit (+$40, set at $252EA) bounces at -4 px/tick.
pub const AIR_STAB_BOUNCE: i32 = -1024;
/// `Player_TryHang` (`$21176..$21184`): the feet snap to `(y & ~7) + 8`. With
/// the rising hitbox top (-46) and the probe 8 px above it, that is 64 px
/// below the top of the 8-px ceiling row the probe touched.
pub const HANG_BELOW_CEILING: i32 = 64;
/// Hitbox top of the hang frames (75-81, and `$1F` written at `$2118E`). The
/// entry's head row, `(y & ~7) - 46`, is the same row below the hang Y.
pub const HANG_HEAD: i32 = 54;
/// `PlayerState_Hang` (`$20206..$20234`): the wall probe 24 px ahead, 16 px
/// below the hitbox top, stops the hand-over-hand movement.
pub const HANG_WALL_AHEAD: i32 = 24;
pub const HANG_WALL_DROP: i32 = 16;
/// `Player_HangPhaseNext` (`$202B2`): the phase a stopped hang returns to.
pub use crate::assets::hang_phase_next;
/// `Player_HangSteps` (`$2028E`): pixels moved on the first update of each
/// cursor phase (sequence speed 32, 8 updates a phase). The loop at `$20268`
/// probes every 8 px past the wall probe and shortens the step at a wall.
pub use crate::assets::hang_steps;
/// `$23584`: C + Down lowers Y by 8 and falls.
pub const HANG_DROP: i32 = 8;
/// `$234F2..$23552`: C + Up needs the space 25 px above the hitbox top free
/// of 1, 3, 4 and 5.
pub const CLIMB_CLEARANCE: i32 = 25;
/// `$23564`: climbing through vy `$FFFAC000`, -5.25 px/tick; `$2355A` with
/// +$29 bit 1 (Fushin) `$FFF9C000`, -6.25 px/tick.
pub const CLIMB_SPEED: i32 = -1344;
pub const FUSHIN_CLIMB_SPEED: i32 = -1600;
/// `$2357A` `move.l #$70008,plr_count`: 7 hold ticks, and plr_timer 8
/// updates without floor or ceiling collision (`$214A0`).
pub const CLIMB_HOLD_TICKS: u8 = 7;
pub const CLIMB_PASS_TICKS: u8 = 8;
/// `Player_JumpButton` (`$23478..$234EE`): C in states 8 and 9 drops through
/// the floor when `plr_floor` is not 0, 1, 3, 4, 5, 12 or 14 and the pixel
/// 17 px below the feet (`$234C4` `addi.w #$11,d1`) is not 1, 3, 4 or 5.
/// `$234E8` `move.w #$D,plr_timer`: 13 updates without floor or ceiling
/// collision, then `$218DA` falls.
pub const DROP_CLEARANCE: i32 = 17;
pub const DROP_PASS_TICKS: u8 = 13;
/// `Player_FloorContact` (`$22094..$220A6`): on the update the floor
/// probes first miss, `addq.w #7,d1` probes 7 px lower at the last (right)
/// probe column. A floor there sets `plr_ctrl` bit 8, the step-down flag:
/// the ground handlers and `$218DA` do not fall while it is set, and the
/// next floor contact clears it (`$21FD2`).
pub const STEP_DOWN: i32 = 7;
/// `PlayerState_Crouch` (`$1FF7A`) and `PlayerState_CrouchGuard` (`$200EC`):
/// `subi.w #$10,d1`, the headroom probe 16 px above the hitbox top plus vy.
pub const CROUCH_HEADROOM: i32 = 16;
/// `Ninjutsu_Finish` (`$219D6`): `subi.w #$38,d1`, 56 px above the feet.
pub const NINJUTSU_HEADROOM: i32 = 56;
