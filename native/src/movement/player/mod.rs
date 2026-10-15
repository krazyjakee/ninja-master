//! `Player`: the native form of the original player object's update, with
//! its damage entry, state and button handlers, motion and collision.
mod camera;
mod collision;
mod event;
mod handlers;
mod update;
mod water;

pub use collision::WallProbe;
pub use event::PlayerEvent;
pub use water::{Splash, SplashKind};

use crate::{
    WIDTH,
    combat::{HitSource, PlayerHit},
    game::Terrain,
    input::{Button, Input},
    movement::{
        AIR_STAB_BOUNCE, AIR_STAB_LIFT, AIR_STAB_MIN_FALL, AIR_STAB_SPEED, Action, AnimationInput,
        Attack, AttackPosture, CLIMB_CLEARANCE, CLIMB_HOLD_TICKS, CLIMB_PASS_TICKS, CLIMB_SPEED,
        CROUCH_HEADROOM, DASH_KICK_AIR_SPEED, DASH_KICK_GROUND_SPEED, DASH_KICK_HOP,
        DROP_CLEARANCE, DROP_PASS_TICKS, FALL_THRESHOLD, FUSHIN_CLIMB_SPEED, FUSHIN_JUMP_SPEED,
        GRAVITY, GUARD_RECOIL_PER_DAMAGE, HANG_DROP, HANG_HEAD, HURT_HOP, JUMP_HOLD_TICKS,
        JUMP_SPEED, KNOCKBACK_SPEED, KNOCKDOWN_FAST_SLOTS, KNOCKDOWN_SPEED, Material, NATIVE_UNIT,
        NINJUTSU_HEADROOM, PlayerAnimator, ROM_UNIT, SLOW_SPEED, SOMERSAULT_HIT_DROP,
        SOMERSAULT_LIFT, STEP_DOWN,
        animation::body_top,
        control::{JumpControl, RunControl, launch_velocity_16},
        dying,
        material::wall_probe_height,
        rules::{
            HangExit, attack_locked, attack_posture, hang_exits, hit_state, ninjutsu_allowed,
            starts_jump,
        },
        state,
    },
};

const UNIT: i32 = NATIVE_UNIT;
/// Half the native occupancy width and the layout clamp.
pub(crate) const HALF_WIDTH: i32 = 8;
/// `Player_CollideWalls` uses the displayed mapping's `obj_hitbox_w`.
/// Every ordinary player frame has the recovered value 15; the zero-width
/// ninjutsu frames do not run ordinary movement collision.
const WALL_HALF_WIDTH: i32 = 15;
const BODY_HEIGHT: i32 = 56;
/// `$215E4`: the shocked flash shows player frame 40, whose `Player_FrameTable`
/// entry is mapping `$13` with the DMA list at `$241D0`.
const SHOCK_FLASH_FRAME: usize = 40;

/// Results of the damage entry that reach the player on this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerEvents {
    /// `plr_event` (+$28), written by an enemy, boss or stage object.
    pub event: Option<PlayerEvent>,
    /// Non-lethal damage ($21DB0): the hurt, knockdown or shocked state
    /// with its hop and knockback.
    pub hit: Option<PlayerHit>,
    /// Damage a guard cancelled ($21D4E): damage * 8 recoil ticks.
    pub blocked: u8,
    /// The air stab struck an enemy (+$40, `$252EA`).
    pub bounce: bool,
}

/// The parts of the button handler `$21428` that belong to other systems.
pub trait PlayerHooks {
    /// B accepted by `$22F7E` in `posture`: melee or throw, or `None` when no
    /// state change follows (the somersault spread). Facing is the direction
    /// the state handler set this update, before the button handler runs.
    fn attack(&mut self, posture: AttackPosture, facing_left: bool) -> Option<Attack>;
    /// A accepted by `$22E8E` in `state`: the technique (0..=3) that starts.
    fn ninjutsu(&mut self, state: usize) -> Option<usize>;
    /// Health subtraction delayed by the thrown sequence until cursor `$600`.
    /// Returns true when it entered the thrown-death state.
    fn damage(&mut self, _damage: u8) -> bool {
        false
    }
}
/// Hooks for movement-only simulation: B and A start nothing.
pub struct NoHooks;
impl PlayerHooks for NoHooks {
    fn attack(&mut self, _: AttackPosture, _: bool) -> Option<Attack> {
        None
    }
    fn ninjutsu(&mut self, _: usize) -> Option<usize> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    /// Fixed-point world position, 256 units per pixel; Y is the feet.
    pub x: i32,
    pub y: i32,
    /// The X step (`obj_vel_x`) the last update's motion took, 256 units
    /// per pixel: its wall probe's result, before a wall stopped it.
    pub velocity_x: i32,
    pub velocity_y: i32,
    /// `plr_push`: an external X push, 256 units per pixel, that the next
    /// update adds to the velocity once (`Player_Physics`, `$21466`).
    pub push: i32,
    /// `obj_accel_y` as this update's `Object_Integrate` adds it, 256 units
    /// per pixel: `Player_Update`'s `$3400` (`$1FC56`), cleared by the jump
    /// holds (`$2102A`, `$213B6`) and `Player_Freeze`.
    pub accel_y: i32,
    pub grounded: bool,
    pub facing_left: bool,
    pub moving: bool,
    pub running: bool,
    pub preparing_jump: bool,
    /// The tick on which a grounded jump was accepted ($2346E).
    pub jump_requested: bool,
    /// The wall-kick launch ran this tick (`PlayerState_WallKick` `$213C2`).
    /// `Game` consumes this to request player sound 7 and the dust object.
    pub wall_kicked: bool,
    /// Held horizontal direction (-1, 0, 1) and whether it was newly pressed.
    pub direction: i32,
    pub direction_edge: bool,
    /// Down held on the ground: original states 8/9 ($218C6, $1FF4E).
    pub crouching: bool,
    /// Down held anywhere (A4+$02 above zero).
    pub down_held: bool,
    /// Up held (A4+$02 below zero). The pad cannot report both; natively
    /// Down wins when a keyboard holds both.
    pub up_held: bool,
    /// Up newly pressed this update (`act_pressed` bit 0).
    pub up_pressed: bool,
    /// B held and newly pressed; any of A/B/C newly pressed.
    pub attack_held: bool,
    pub attack_pressed: bool,
    pub buttons_pressed: bool,
    /// Airborne jump press accepted this tick (somersault or wall cling).
    pub action: Option<Action>,
    /// Guard recoil was applied this tick.
    pub recoil: bool,
    /// +$29 bit 1, set when Fushin completes ($2097A): -6 px/tick jumps.
    pub fushin: bool,
    /// `plr_status` bit 7 (`Player_PowerUp`, `$22890`): the handlers pick
    /// the powered states. `Combat::weapons` owns the flag; `Game` copies it
    /// here before each update.
    pub powered: bool,
    /// Original +$2C/+$2E state and sequence, stepped once per tick.
    pub animator: PlayerAnimator,
    jump_control: JumpControl,
    run_control: RunControl,
    /// +$2A bit 12: the somersault was used or refused in this airborne phase.
    somersault_spent: bool,
    /// +$32 in the guard states: remaining 1 px/tick recoil ticks.
    guard_recoil: u8,
    /// `plr_hurt` written by this update's state handler: the knockdown's
    /// 64 (`$20306`). The game hands it to `Combat::invulnerability`.
    pub immunity_hold: Option<u16>,
    /// `$20F4A`: the grounded dash slash handler adds 1 to `plr_hurt` on
    /// every update. The damage test runs first and counts it down, so the
    /// countdown freezes and, from zero, no damage test runs at all.
    pub immunity_step: bool,
    /// +$32 = 1 in the hurt states: the hit came during a somersault
    /// (`$21E44`), so `PlayerState_Hurt` leaves the X velocity at 0.
    somersault_hit: bool,
    /// +$40: bounce requested by an air-stab hit.
    bounce: bool,
    /// `plr_floor` (+$3C): the material under the feet, or empty in the air.
    pub floor: Material,
    /// `plr_ctrl` bit 6: the last update's head probe (`Func_022494`) found a
    /// `$4C7E` ceiling material (1-6, 14).
    pub ceiling_contact: bool,
    /// `plr_floor+1`: the material that probe read.
    pub ceiling: Material,
    /// Pixel row of that probe.
    ceiling_row: i32,
    /// `plr_flags` bit 0: set by the hang handlers, cleared by the jump
    /// handler (`$20F5E`) and by damage (`$21DFC`).
    pub hanging: bool,
    /// `Player_HangStop` (`$2029E`) ran this update.
    pub hang_stop: bool,
    /// C in a hang state this update (`$234F2`).
    pub hang_exit: Option<HangExit>,
    /// C while crouching dropped through the floor this update (`$23478`).
    pub drop_through: bool,
    /// The crouching handlers found a low ceiling (`$1FF60..$1FF92`,
    /// `$200E6..$2011E`) or a ninjutsu ended under one (`$219CC`): Joe
    /// stays down without Down and +$2A bit 9 refuses the jump.
    pub low_ceiling: bool,
    /// A lift's ceiling probe forced the crouch last update
    /// ([`Player::duck_under_lift_ceiling`]).
    lift_ceiling: bool,
    /// `plr_ctrl` bit 8 (`$22094..$220A6`): the floor was lost with another
    /// within 7 px below. The ground states keep going until it lands.
    pub step_down: bool,
    /// `$22062..$2207E`: this update met the floor after an airborne one,
    /// at vy 3 px/tick or less. State 16 then enters neutral at once.
    pub landed: bool,
    /// `plr_ctrl` bit 1 (`Func_0223F6`, `$22428`): a wall stopped the
    /// horizontal motion on this update. The handlers read it on the next.
    pub against_wall: bool,
    /// The wall probes' record from this update (`plr_floor+2`/`+3` and
    /// `plr_flags` bit 2), which the next update's C press reads.
    pub wall_probe: Option<WallProbe>,
    /// The wall probe's pixel when the material it recorded is 15.
    water_probe: Option<(i32, i32)>,
    /// `plr_floor` as the entry splash compares it (`$21F46`): the feet
    /// probe of the last update that ran the floor collision.
    feet_probe: Material,
    /// Material-15 splashes of this update, in the original's order
    /// (`crate::movement::player::water`). The game allocates them.
    pub splashes: Vec<Splash>,
    /// `plr_timer` after a climb or a drop-through: updates left without floor or ceiling
    /// collision (`Player_Physics` `$214A0`).
    pass_through: u8,
    /// Touching spikes (material 5) through the floor, a wall or the head
    /// probe this update. `Player_CheckDamage` (`$21C44..$21C94`) then deals
    /// damage code 2 (`$2F`, the shocked state, in 7-2); the caller applies it.
    pub spikes: bool,
    /// The spike hit came from the head probe, after the higher-priority feet
    /// and wall tests missed. It pushes Joe down after applying the hit.
    pub(crate) head_spikes: bool,
    pub ticks: u64,
    pub camera_x: i32,
    pub camera_y: i32,
    /// `$FF0146`, the part's runtime scroll flags (`stage::RuntimeFlags`):
    /// bit 15 centres the camera, bits 12-14 raise its vertical target.
    pub scroll_flags: u16,
    /// Camera follow state of `Func_0225A2`: `plr_ctrl` bit 4 (look ahead
    /// to the right), and `act_x_held` / `act_y_held` with the vertical
    /// direction they count.
    look_right: bool,
    x_held: u8,
    y_held: u8,
    previous_direction_x: i32,
    direction_y: i32,
    /// Damage saved by `Player_ApplyHit` for the thrown slam at cursor `$600`.
    thrown_damage: u8,
    /// The horizontal sign is based on the facing saved when the throw began.
    thrown_rightward: bool,
    /// `plr_event` 6 sets `plr_flags` bit 5 for this update.
    pub(crate) silence_sounds: bool,
    pub(crate) spawn: (i32, i32),
}
impl Player {
    pub fn new(x: i32, y: i32) -> Self {
        Self {
            x: x * UNIT,
            y: y * UNIT,
            velocity_x: 0,
            velocity_y: 0,
            push: 0,
            accel_y: GRAVITY,
            grounded: false,
            facing_left: false,
            moving: false,
            running: false,
            preparing_jump: false,
            jump_requested: false,
            wall_kicked: false,
            direction: 0,
            direction_edge: false,
            crouching: false,
            down_held: false,
            up_held: false,
            up_pressed: false,
            attack_held: false,
            attack_pressed: false,
            buttons_pressed: false,
            action: None,
            recoil: false,
            fushin: false,
            powered: false,
            animator: PlayerAnimator::default(),
            jump_control: JumpControl::default(),
            run_control: RunControl::default(),
            somersault_spent: false,
            guard_recoil: 0,
            somersault_hit: false,
            immunity_hold: None,
            immunity_step: false,
            bounce: false,
            floor: Material::EMPTY,
            ceiling_contact: false,
            ceiling: Material::EMPTY,
            ceiling_row: 0,
            hanging: false,
            hang_stop: false,
            hang_exit: None,
            drop_through: false,
            low_ceiling: false,
            lift_ceiling: false,
            step_down: false,
            landed: false,
            against_wall: false,
            wall_probe: None,
            water_probe: None,
            feet_probe: Material::EMPTY,
            splashes: Vec::new(),
            pass_through: 0,
            spikes: false,
            head_spikes: false,
            ticks: 0,
            camera_x: (x - WIDTH as i32 / 2).max(0),
            camera_y: (y - 160).clamp(0, 320),
            scroll_flags: 0,
            look_right: false,
            x_held: 0,
            y_held: 0,
            previous_direction_x: 0,
            direction_y: 0,
            thrown_damage: 0,
            thrown_rightward: false,
            silence_sounds: false,
            spawn: (x, y),
        }
    }
    /// The frame `Player_StepAnimation` shows: the animator's, except in
    /// the shocked state (47) while its cursor is in slot 0, where every
    /// fourth `plr_ticks` shows frame 40 (mapping `$13`, DMA `$241D0`,
    /// `$215C6..$215EE`). The hitbox keeps the animator's frame.
    pub fn display_frame(&self) -> usize {
        let state = &self.animator;
        if state.state == state::SHOCKED && state.cursor >> 8 == 0 && self.ticks.is_multiple_of(4) {
            SHOCK_FLASH_FRAME
        } else {
            state.frame
        }
    }
    pub fn position(&self) -> (i32, i32) {
        (self.x.div_euclid(UNIT), self.y.div_euclid(UNIT))
    }
    /// Where the last update left `obj_x`/`obj_y` before `Object_Integrate`
    /// (in `Objects_BuildSprites`, after the attacks' update) added the
    /// velocities: the handlers' direct writes are in (the air stab's lift,
    /// a landing's snap to the floor), the step is not. A wall that stopped
    /// the step had subtracted it from `obj_x` (`$22414..$2242E`).
    pub fn unmoved_position(&self) -> (i32, i32) {
        (
            (self.x - self.velocity_x).div_euclid(UNIT),
            (self.y - self.velocity_y).div_euclid(UNIT),
        )
    }
    /// `obj_x + obj_vel_x`, `obj_y + obj_vel_y` before `Object_Integrate`
    /// adds `obj_accel_y` to the velocity: where the slashes and the dash
    /// kick stand (`$24CBA..$24CD8`, `$25506..$2551E`). In the air that is
    /// the moved position less this update's gravity. On the floor
    /// `Player_CollideVertical` leaves `obj_vel_y` at `-obj_accel_y`
    /// (`$2207E..$2208E`), so the feet do not move but the sum is the
    /// gravity above them; material 12 leaves it at 0.
    pub fn stepped_position(&self) -> (i32, i32) {
        let lead = if self.grounded && self.floor.0 == 12 {
            0
        } else {
            self.accel_y
        };
        (self.x.div_euclid(UNIT), (self.y - lead).div_euclid(UNIT))
    }
    /// Where list 3 finds the player this frame.
    pub(crate) fn anchors(&self) -> crate::combat::PlayerAnchors {
        crate::combat::PlayerAnchors {
            unmoved: self.unmoved_position(),
            stepped: self.stepped_position(),
        }
    }
    /// A movement-only update. Without a game to run the pit death, a fall
    /// 64 px past the terrain starts again at the spawn point.
    pub fn tick(&mut self, input: Input, terrain: &impl Terrain) {
        self.tick_with(input, terrain, PlayerEvents::default(), &mut NoHooks);
        if self.y > (terrain.height() + 64) * UNIT {
            *self = Self::new(self.spawn.0, self.spawn.1);
        }
    }
    /// `Player_Die` (`$21A94`) on foot: state 41 at cursor 0 with the
    /// velocities cleared; ordinary gravity (`$3400`, `$21B18`) pulls the
    /// body to the ground.
    /// `$28FB8..$28FC8`: the ridden lift found solid ground 64 px above
    /// the feet, and states 1-4 become their crouching forms (+7). The
    /// original's crouch handler stands Joe up again until its own ceiling
    /// probe reaches the beam, so he flickers between the two poses;
    /// natively the lift's probe also keeps him down, like `low_ceiling`.
    pub fn duck_under_lift_ceiling(&mut self) {
        if self.animator.state < state::SLASH {
            self.animator.state += state::CROUCH - state::IDLE;
        }
        self.lift_ceiling = true;
    }
    pub fn die(&mut self) {
        self.animator.die(state::DEATH, 0);
        self.velocity_y = 0;
        self.halt();
    }
    /// `Player_ApplyEvent` (`$1FD84`): below the screen, state 43 at cursor
    /// 0. The fall goes on.
    pub fn fall_into_pit(&mut self) {
        self.animator.die(state::PIT_DEATH, 0);
        self.halt();
    }
    /// Mijin without a spare life (`$20746`): state 42 normally or state 30
    /// on surf, and `Player_Freeze` clears the velocities. The Mijin update's
    /// stepper moves the cursor to `$20` before the death handler first runs.
    pub(crate) fn mijin_death(&mut self, death_state: usize) {
        self.animator.die(death_state, crate::death::SPEED);
        self.velocity_y = 0;
        self.halt();
    }
    /// A death drops any queued jump, run, guard recoil or hang.
    fn halt(&mut self) {
        self.velocity_x = 0;
        self.jump_control = JumpControl::default();
        self.run_control.stop();
        self.guard_recoil = 0;
        self.hanging = false;
        self.preparing_jump = false;
        self.jump_requested = false;
    }
}
