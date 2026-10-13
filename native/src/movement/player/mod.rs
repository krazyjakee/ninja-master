//! `Player`: the native form of the original player object's update, with
//! its damage entry, state and button handlers, motion and collision.
mod camera;
mod collision;
mod event;
mod handlers;
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
    /// One player update in the original order: damage entry, state handler,
    /// button handler (`hooks` resolves B and A), motion, sequence stepper.
    /// Motion follows the state the handlers saw at the start of the update.
    pub fn tick_with(
        &mut self,
        input: Input,
        terrain: &impl Terrain,
        events: PlayerEvents,
        hooks: &mut impl PlayerHooks,
    ) {
        use state::*;
        if input.pressed(Button::Start) {
            *self = Self::new(self.spawn.0, self.spawn.1);
            return;
        }
        // PlayerState_Death blocks every button and clears the X velocity;
        // the body only falls.
        let event = (!dying(self.animator.state))
            .then_some(events.event)
            .flatten();
        self.silence_sounds = event == Some(PlayerEvent::Freeze);
        let mut input = if dying(self.animator.state) {
            Input::default()
        } else {
            input
        };
        if let Some((forced, state, last_slot)) = event.and_then(PlayerEvent::forced) {
            let start = Button::Start.mask();
            input = Input {
                held: forced.held | input.held & start,
                pressed: forced.pressed | input.pressed & start,
            };
            self.animator.force(state, last_slot);
        }
        self.ticks += 1;
        self.splashes.clear();
        let grounded_before = self.grounded;
        let velocity_y_before = self.velocity_y;
        // Player_ApplyHit ($21DB0) runs before the state handler: the
        // handler and the motion see the state it enters at cursor 0.
        let thrown_hit = events.hit.filter(|_| {
            matches!(
                event,
                Some(PlayerEvent::ThrownLeft | PlayerEvent::ThrownRight)
            )
        });
        let thrown = thrown_hit.is_some();
        if let Some(hit) = thrown_hit {
            self.enter_thrown(hit, event == Some(PlayerEvent::ThrownLeft));
        } else if let Some(hit) = events.hit {
            // $21D64: a shot turns Joe toward it, so the knockback carries
            // him away from it. Contact and spikes keep his facing.
            if let HitSource::Shot { x } = hit.source {
                self.facing_left = x < self.position().0;
            }
            // $21E3C..$21E50: +$32 = 1 and Y + 15 for a somersault hit.
            self.somersault_hit = self.animator.state == SOMERSAULT;
            if self.somersault_hit {
                self.y += SOMERSAULT_HIT_DROP * UNIT;
            }
        }
        let reaction = (!thrown)
            .then_some(events.hit)
            .flatten()
            .map(|hit| hit_state(hit.damage, hit.shocked));
        let mut state = reaction.unwrap_or(self.animator.state);
        self.immunity_hold = match state {
            KNOCKDOWN => Some(crate::combat::KNOCKDOWN_IMMUNITY),
            THROWN => Some(crate::combat::THROWN_IMMUNITY),
            _ => None,
        };
        let cursor = if reaction.is_some() {
            0
        } else {
            self.animator.cursor
        };
        if state == THROWN && self.thrown_step(cursor, hooks) {
            state = THROWN_DEATH;
        }
        // A powered state moves, collides and permits as its base state.
        let state = unpowered(state);
        // `plr_ctrl` bit 10 from this update's state handler: B is refused
        // even when the handler ends its state.
        let b_locked = attack_locked(state, self.animator.ended, grounded_before);
        // $21364: wall-jump flight that met a wall on the last update runs
        // the state-16 handler instead (after $212FE's bit clear below).
        let against_wall_before = self.against_wall;
        let flight_hit =
            state == WALL_CLING && cursor > 0x100 && !grounded_before && against_wall_before;
        let direction = i32::from(input.held(Button::Right)) - i32::from(input.held(Button::Left));
        self.direction_edge = direction != 0 && direction != self.direction;
        self.direction = direction;
        self.down_held = input.held(Button::Down);
        self.up_held = input.held(Button::Up) && !self.down_held;
        self.up_pressed = input.pressed(Button::Up);
        self.attack_held = input.held(Button::Attack);
        // $21428 runs one of A, B, C in that order, so A hides B and C.
        // Event 6 sets the ROM's ninjutsu refusal bits ($1FD06), but
        // Player_HandleButtons still reads B and C during the boss fall.
        let ninjutsu_locked = event == Some(PlayerEvent::Freeze);
        let ninjutsu_pressed = input.pressed(Button::Ninjutsu);
        self.attack_pressed = input.pressed(Button::Attack) && !ninjutsu_pressed;
        let jump_pressed = input.pressed(Button::Jump) && !ninjutsu_pressed && !self.attack_pressed;
        self.buttons_pressed =
            ninjutsu_pressed || input.pressed(Button::Attack) || input.pressed(Button::Jump);
        self.action = None;
        self.drop_through = false;
        self.wall_kicked = false;
        let ceiling_before = self.ceiling_contact;
        if state == JUMP {
            self.hanging = false; // $20F5E clears plr_flags bit 0
        }
        if reaction.is_some() {
            // $21DCC/$21E36: vy -2 px/tick, or 0 when hanging ($21DFC) or
            // at shocked head spikes ($21E74). `Player_CheckDamage` then
            // pushes every head-spike hit down 8 px at +1 px/tick ($21C96).
            let head_spikes = events
                .hit
                .is_some_and(|hit| matches!(hit.source, HitSource::Spikes { head: true }));
            let hanging = std::mem::take(&mut self.hanging);
            self.velocity_y = if head_spikes {
                self.y += 8 * UNIT;
                UNIT
            } else if hanging {
                0
            } else {
                HURT_HOP
            };
            self.jump_control = JumpControl::default();
            self.run_control.stop();
        }
        let guarding = matches!(state, GUARD | GUARD_WALK | CROUCH_GUARD | CROUCH_GUARD_WALK);
        if events.blocked > 0 {
            self.guard_recoil = events.blocked.saturating_mul(GUARD_RECOIL_PER_DAMAGE);
        }
        if !guarding {
            self.guard_recoil = 0;
        }
        self.recoil = self.guard_recoil > 0;
        if self.recoil {
            self.guard_recoil -= 1; // $20076, $20192
        }
        if state != AIR_STAB {
            self.bounce = false;
        }
        self.bounce |= events.bounce;

        let mut passing = self.pass_through > 0;
        let (initial_x, initial_feet) = self.position();
        let initial_floor = if self.velocity_y >= 0 && !passing {
            Self::floor_below(terrain, initial_x, initial_feet)
        } else {
            None
        };
        self.grounded = initial_floor.is_some();
        let supported = self.grounded;
        // $20F08..$20F4A: past the sequence end test, with +$2A bit 5 clear,
        // the branch that also picks the 2 px/tick ground speed below.
        self.immunity_step = state == DASH_KICK && !self.animator.ended && supported;
        if supported {
            self.somersault_spent = false; // $21804 clears +$2A bits 12 and 13
        }
        if state == WALL_CLING {
            // $212FE: every update in state 25 clears bits 12 and 13, so the
            // wall jump always has its somersault.
            self.somersault_spent = false;
        }
        let state = if flight_hit { JUMP } else { state };
        // $20E1E..$20E42: an air stab stopped by a wall ends at once
        // (Player_EnterNeutral clears X velocity), unless the head probe
        // touched a ceiling while it shows its first frame.
        let stab_walled =
            state == AIR_STAB && against_wall_before && !(ceiling_before && cursor >> 8 == 0);
        if stab_walled {
            self.bounce = false;
        }
        let horizontal = self
            .run_control
            .tick(direction, self.facing_left, self.grounded);
        // $21054: state 16 back on the floor clears the run latch (+$2A bit
        // 14) on each landing update, so a running jump lands walking. A
        // slow landing ($22062) never gets here and runs on.
        if state == JUMP
            && grounded_before
            && velocity_y_before >= 0
            && cursor >> 8 != 0
            && cursor != 0x100
        {
            self.run_control.stop();
        }
        if matches!(state, WALL_CLING | WALL_SLASH | WALL_THROW) && !grounded_before {
            // $21314: airborne, state 25 sets the run latch and the wall
            // attacks leave it for the state 16 that follows them.
            self.run_control.latch();
        }
        // $20FB8 (state 16 after its launch tick) and $21374 (wall-jump
        // flight) test act_pressed bit 5 themselves, before $21428 picks one
        // of A, B and C: an A or B pressed with it runs after the cling.
        let air_jump_pressed = input.pressed(Button::Jump);
        if air_jump_pressed
            && !grounded_before
            && ((state == JUMP && cursor >> 8 != 0 && cursor != 0x100)
                || (state == WALL_CLING && cursor > 0x100))
        {
            self.action = self.air_press(terrain, direction);
        }
        // Player_TryHang from the jump ($2100A), air attacks ($20D60), wall
        // attacks ($20DA8) and the wall-jump flight ($2138A). A C press in
        // states 16 and 25 returns before it.
        let tries_hang = match state {
            JUMP => cursor >> 8 != 0 && cursor != 0x100 && !air_jump_pressed,
            WALL_CLING => cursor > 0x100 && !air_jump_pressed,
            AIR_SLASH | AIR_THROW | WALL_SLASH | WALL_THROW => true,
            _ => false,
        };
        let entered_hang =
            tries_hang && !grounded_before && !self.animator.ended && self.try_hang(terrain);
        if entered_hang {
            self.action = Some(Action::Hang);
            self.jump_control.cancel_hold();
        }
        self.hang_stop = false;
        let mut hang_dx = 0;
        let hang_fell = state == HANG && (!ceiling_before || self.y < 0); // $201C0 -> $218DA
        if state == HANG && !hang_fell {
            self.hanging = true; // $201D8
            hang_dx = self.hang_motion(terrain, direction, cursor);
        }
        if matches!(state, HANG_SLASH | HANG_THROW) {
            // $20EDC..$20EEC: plr_flags bit 0, Y & ~7 and no velocity. The
            // attack keeps the hang without testing the ceiling.
            self.hanging = true;
            self.y = (self.y.div_euclid(UNIT) & !7) * UNIT;
            self.velocity_y = 0;
        }
        // $1FF60..$1FF92 and $200E6..$2011E: the crouching handlers probe
        // 16 px above the hitbox top plus vy. A blocking material keeps Joe
        // crouched without Down and sets +$2A bit 9, refusing C. The end of
        // a ninjutsu probes 56 px above the feet ($219CC) and crouches.
        let (x, feet) = self.position();
        let head = feet + body_top(self.animator.frame) + self.velocity_y.div_euclid(UNIT);
        let step_down_before = self.step_down;
        let on_ground = self.grounded || step_down_before;
        let lift_ceiling = std::mem::take(&mut self.lift_ceiling);
        self.low_ceiling = on_ground
            && (lift_ceiling && state < NINJUTSU
                || match state {
                    CROUCH | CRAWL | CROUCH_GUARD | CROUCH_GUARD_WALK => terrain
                        .material_at(x, head - CROUCH_HEADROOM)
                        .blocks_standing(),
                    id if id >= NINJUTSU && self.animator.ended => terrain
                        .material_at(x, feet - NINJUTSU_HEADROOM)
                        .blocks_standing(),
                    _ => false,
                });
        // The state handler runs before the button handler: resolve the
        // buttons against the state it leaves.
        let pending = self.jump_control.is_pending();
        self.jump_requested = false;
        let down = self.down_held || self.low_ceiling;
        // Only the crouching handlers set bit 9; Ninjutsu_Finish forces Down.
        let jump_blocked = self.low_ceiling && state < NINJUTSU;
        self.crouching = on_ground && down && !pending;
        self.running = horizontal.running && !self.crouching;
        let mut pose = AnimationInput::after(self, grounded_before, velocity_y_before);
        pose.hurt = reaction;
        let mut probe = self.animator.clone();
        probe.handle(&pose);
        let handler_state = probe.state;
        // $20F88..$20FA4 (jump), and the ground handlers, turn before
        // Player_HandleButtons. A throw must see this update's facing.
        let steers = matches!(
            state,
            IDLE | WALK
                | RUN
                | CROUCH
                | CRAWL
                | GUARD
                | GUARD_WALK
                | CROUCH_GUARD
                | CROUCH_GUARD_WALK
                | JUMP
                | SOMERSAULT
                | HANG
        ) || (state == AIR_THROW && !grounded_before);
        if direction != 0 && steers {
            self.facing_left = direction < 0;
        }
        if self.action == Some(Action::WallCling) {
            self.facing_left = direction > 0; // $21102 turns away from the wall
        }
        // Only the state-16 handler launches (`$20FB0`): a preparation an
        // air attack replaced on the last update never takes off.
        if handler_state != JUMP {
            self.jump_control.cancel_pending();
        }
        let mut started = None;
        let mut ninjutsu = None;
        if ninjutsu_pressed {
            if !ninjutsu_locked && !self.fushin && ninjutsu_allowed(handler_state) {
                ninjutsu = hooks.ninjutsu(handler_state);
            }
        } else if self.attack_pressed
            && !b_locked
            && let Some(posture) = attack_posture(handler_state, self.down_held, grounded_before)
        {
            let attack = hooks.attack(posture, self.facing_left);
            if attack.is_some() {
                started = Some(posture);
            }
            pose.attack = attack;
        }
        // $234F2: C in a hang state. Down drops; Up climbs through a ceiling
        // other than 1, 3, 4 and 5 when the space 25 px above the hitbox
        // top is not one of them either.
        self.hang_exit = None;
        if jump_pressed && hang_exits(handler_state) {
            let (x, feet) = self.position();
            let above = terrain
                .material_at(x, feet - HANG_HEAD - CLIMB_CLEARANCE)
                .for_player();
            if self.down_held {
                self.hang_exit = Some(HangExit::Drop);
            } else if self.up_held && !self.ceiling.full_block() && !above.full_block() {
                self.hang_exit = Some(HangExit::Climb);
                self.pass_through = CLIMB_PASS_TICKS;
                passing = true;
            }
        }
        // $23478..$234EE: C in states 8 and 9 drops through a floor other
        // than 0, 1, 3, 4, 5, 12 and 14 when the pixel 17 px below the feet
        // is not 1, 3, 4 or 5, unless plr_timer is already running.
        if jump_pressed
            && matches!(handler_state, CROUCH | CRAWL)
            && !jump_blocked
            && self.pass_through == 0
            && !matches!(self.floor.0, 0 | 1 | 3 | 4 | 5 | 12 | 14)
        {
            let (x, feet) = self.position();
            let below = feet + self.velocity_y.div_euclid(UNIT) + DROP_CLEARANCE;
            if !terrain.material_at(x, below).for_player().full_block() {
                self.drop_through = true;
                self.pass_through = DROP_PASS_TICKS;
                passing = true;
            }
        }
        let jump = self.jump_control.tick(
            jump_pressed
                && starts_jump(handler_state, probe.cursor)
                && !self.drop_through
                && !jump_blocked,
            input.held(Button::Jump),
            self.grounded,
            self.velocity_y,
        );
        self.preparing_jump = jump.preparing;
        self.jump_requested = jump.requested;
        self.crouching = on_ground && down && !jump.preparing && !jump.launch;
        self.running = horizontal.running && !self.crouching;
        if matches!(started, Some(AttackPosture::DashKick)) {
            self.run_control.stop(); // state 29 ends in 8, then 1
        }

        let facing = if self.facing_left { -1 } else { 1 };
        let mut dx = if self.recoil {
            -facing * KNOCKBACK_SPEED // $20090..$2009E: 1 px/tick backwards
        } else {
            match state {
                id if dying(id) => 0,                                // $2050C
                _ if entered_hang => 0,                              // $217F4
                _ if self.drop_through => 0,                         // $218DA
                HANG if self.hang_exit == Some(HangExit::Drop) => 0, // $218DA
                HANG => hang_dx,
                HANG_SLASH | HANG_THROW => 0, // $20EE8
                // $21804/$218C6 clear +$18 when these states end or land;
                // $20D3E ends an air attack before its X step.
                HURT | KNOCKDOWN | SHOCKED | DASH_KICK | SOMERSAULT | AIR_STAB | AIR_SLASH
                | AIR_THROW | WALL_SLASH | WALL_THROW
                    if self.animator.ended && reaction.is_none() =>
                {
                    0
                }
                AIR_STAB if self.grounded || stab_walled => 0,
                // $20CDA, $20D0C, $20D28: ground attacks stand still.
                SLASH | THROW | COMBO | CROUCH_SLASH | CROUCH_THROW => 0,
                // $202EC..$20300; +$32 (a somersault hit) keeps X still.
                HURT | SHOCKED if self.somersault_hit => 0,
                HURT | SHOCKED => -facing * KNOCKBACK_SPEED,
                THROWN => self.thrown_velocity(cursor),
                // $2033C..$20350: 3 px/tick for the first three slots.
                KNOCKDOWN if cursor >> 8 < KNOCKDOWN_FAST_SLOTS => -facing * KNOCKDOWN_SPEED,
                KNOCKDOWN => -facing * KNOCKBACK_SPEED,
                // $2005C, $20178: guard walks use the 0.75 px/tick helper.
                GUARD | GUARD_WALK | CROUCH_GUARD | CROUCH_GUARD_WALK => direction * SLOW_SPEED,
                DASH_KICK => {
                    facing
                        * if self.grounded {
                            DASH_KICK_GROUND_SPEED
                        } else {
                            DASH_KICK_AIR_SPEED
                        }
                }
                AIR_STAB => {
                    if self.velocity_y < 0 && !self.bounce {
                        -facing * KNOCKBACK_SPEED
                    } else {
                        facing * AIR_STAB_SPEED
                    }
                }
                WALL_CLING | WALL_SLASH | WALL_THROW => {
                    if self.grounded || (state == WALL_CLING && cursor >> 8 == 0) {
                        0 // $21340, $2134C
                    } else {
                        // $213F6..$21422: 2 px/tick away from the wall, 3 when
                        // holding away and 1 when holding toward it.
                        let speed = if direction == 0 {
                            2
                        } else if (direction < 0) == self.facing_left {
                            3
                        } else {
                            1
                        };
                        facing * speed * NATIVE_UNIT
                    }
                }
                // $2106C: preparation and landing clear horizontal velocity.
                JUMP if self.grounded && !jump.launch && !jump.requested => 0,
                id if id >= NINJUTSU => 0, // $21A62
                _ if jump.preparing && !jump.requested => 0,
                _ if self.crouching => direction * SLOW_SPEED, // State 9 crawl uses $217C4.
                _ => horizontal.velocity,
            }
        };
        // $2204E..$22062: a conveyor floor (3/4) adds +-1 px/tick to the
        // velocity the state handler chose.
        if let Some(floor) = initial_floor {
            dx += floor.conveyor() * UNIT;
        }
        // $21466..$2146C: Player_Physics adds and clears plr_push.
        dx += std::mem::take(&mut self.push);
        // $224F8..$22512: hanging (plr_flags bit 0) under a conveyor pushes
        // the other way. Natively from the last update's contact material.
        if self.hanging && ceiling_before && self.hang_exit.is_none() {
            dx -= self.ceiling.conveyor() * UNIT;
        }
        // $222C2..$22336: the wall probes reach 56 px up, 40 crouching or
        // late in the dash kick, 17 in the somersault and the air stab.
        let wall_height = wall_probe_height(handler_state, probe.cursor);
        // Code 3 deliberately walks through walls and past the right edge
        // before its sender requests the next substage (`$1C018`).
        let forced_exit = event == Some(PlayerEvent::WalkRight);
        let x_before_walls = self.x;
        let dx = self.collide_walls(terrain, dx, wall_height, forced_exit);
        self.velocity_x = dx;
        let (x, feet) = self.position();
        // The wall probe side: motion, else the held direction, else facing.
        let side = if dx != 0 {
            dx.signum()
        } else if direction != 0 {
            direction
        } else {
            facing
        };
        let wall_spikes = Self::column_has(
            terrain,
            Self::leading_column(x, side) + side,
            feet,
            wall_height,
            Material::hurts,
        );
        // Support after horizontal movement: walking off an edge falls from
        // here. A jump already accepted still launches ($20FB0).
        let floor = if self.velocity_y >= 0 && !passing {
            Self::floor_below(terrain, x, feet)
        } else {
            None
        };
        self.grounded = floor.is_some();
        if self.grounded {
            self.velocity_y = 0;
        }
        if supported && !self.grounded && !jump.launch && self.velocity_y >= 0 {
            // $22094..$220A6: the first update without a floor probes 7 px
            // lower at the last probe column. Found, the ground states carry
            // on; otherwise $218FE falls and spends the somersault.
            let row = feet + self.velocity_y.div_euclid(UNIT) + STEP_DOWN;
            self.step_down = terrain.stands_on(Self::probes(x)[2], row);
            // $218FE: walking off a ledge. A preparing jump is state 16,
            // which never falls through $218DA and keeps its somersault.
            if !self.step_down && !jump.preparing {
                self.somersault_spent = true;
            }
        }
        if jump.launch {
            // $20FCE..$21006: Fushin's flag selects the higher jump; standing
            // in material 12 selects the lower one. In the air plr_floor is
            // 0, the ordinary jump.
            let material = floor.map_or(0, |floor| floor.0);
            self.velocity_y = launch_velocity_16(self.fushin, material) / (ROM_UNIT / UNIT);
            self.grounded = false;
        }
        // Vertical writes the handlers make before `$19D4` integrates.
        let mut hang = false;
        let bounce = state == AIR_STAB && std::mem::take(&mut self.bounce);
        match state {
            WALL_CLING if !self.grounded && cursor >> 8 == 0 => hang = true, // $2134C
            WALL_CLING if !self.grounded && cursor == 0x100 => {
                // $213C2..$213F4: wall jump with the eleven-tick hold.
                self.wall_kicked = true;
                self.velocity_y = if self.fushin {
                    FUSHIN_JUMP_SPEED
                } else {
                    JUMP_SPEED
                };
                self.jump_control.start_hold(JUMP_HOLD_TICKS);
            }
            WALL_CLING if self.grounded => self.velocity_y = 0,
            SOMERSAULT if cursor == 0x40 => self.lift(terrain, SOMERSAULT_LIFT), // $212C4
            AIR_STAB if bounce => self.velocity_y = AIR_STAB_BOUNCE,             // $20E5A
            // $218DA spends the somersault when it falls with vy >= 0.
            HANG if hang_fell && self.velocity_y >= 0 => self.somersault_spent = true,
            _ => {}
        }
        match self.hang_exit {
            Some(HangExit::Drop) => {
                self.y += HANG_DROP * UNIT; // $23584
                self.velocity_y = 0;
                self.somersault_spent = true; // $218DA
            }
            Some(HangExit::Climb) => {
                // $23556..$2357A: clears +$2A bits 12 and 13.
                self.velocity_y = if self.fushin {
                    FUSHIN_CLIMB_SPEED
                } else {
                    CLIMB_SPEED
                };
                self.jump_control.start_hold(CLIMB_HOLD_TICKS);
                self.somersault_spent = false;
            }
            None => {}
        }
        if self.drop_through {
            // $218DA with vy >= 0: plr_count cleared, the somersault spent.
            self.jump_control.cancel_hold();
            self.somersault_spent = true;
        }
        match started {
            Some(AttackPosture::DashKick) => {
                self.velocity_y = DASH_KICK_HOP; // $23280
                self.grounded = false;
            }
            Some(AttackPosture::AirStab) => {
                self.lift(terrain, AIR_STAB_LIFT); // $2308E
                self.velocity_y = self.velocity_y.max(AIR_STAB_MIN_FALL); // $23094..$230A2
                self.jump_control.cancel_hold();
            }
            Some(_) => self.jump_control.cancel_hold(),
            None => {}
        }
        // Func_022494 runs after the handlers and before `$19D4` integrates:
        // 8 px above the hitbox top of the frame shown last update, plus the
        // Y velocity, at the X the update started from.
        let (_, feet) = self.position();
        // Player_TryHang writes hitbox_h $1F (`$2118E`): with hitbox_y -23,
        // the hang frames' top.
        let top = if entered_hang {
            -HANG_HEAD
        } else {
            body_top(self.animator.frame)
        };
        let row = feet + top + self.velocity_y.div_euclid(UNIT) - 8;
        if passing {
            // $214A0..$214BA: plr_timer skips the probe and clears contact.
            self.pass_through -= 1;
            self.ceiling_contact = false;
        } else {
            self.ceiling = terrain.material_at(initial_x, row).for_player();
            self.ceiling_contact = self.ceiling.ceiling_contact();
            self.ceiling_row = row;
        }
        // $224F2..$2259C: 1, 3, 4 and 5 stop the rise; nothing else does.
        // Natively a grounded player is left alone: the original's snap
        // moves the feet by one pixel there, which the next floor contact
        // takes back.
        let mut bonked = false;
        if !passing && !self.grounded && self.ceiling_contact && self.ceiling.blocks_ceiling() {
            let physics_pose = AnimationInput {
                attack: pose.attack,
                ninjutsu,
                hurt: reaction,
                ceiling: ceiling_before,
                step_down: step_down_before,
                against_wall: against_wall_before,
                ..AnimationInput::after(self, grounded_before, velocity_y_before)
            };
            let physics_state = self.animator.resolve(&physics_pose);
            bonked = self.bonk(physics_state, ceiling_before);
        }
        // obj_accel_y as `$19D4` will add it: Func_0225A2 sees Y before that.
        let mut camera_accel = GRAVITY;
        let mut probe_velocity_y = self.velocity_y;
        if !self.grounded {
            // Original free-fall cap is applied before this frame's integration.
            if self.velocity_y >= FALL_THRESHOLD {
                self.velocity_y = FALL_THRESHOLD - GRAVITY;
            }
            // The hold cleared obj_accel_y in the handler, before a bonk.
            let mut gravity =
                if !jump.launch && jump.gravity == 0 && self.velocity_y >= 0 && !bonked {
                    GRAVITY
                } else {
                    jump.gravity
                };
            // Only the state-16 and state-25 handlers run the hold ($21020).
            if !matches!(state, JUMP | WALL_CLING) || started.is_some() || reaction.is_some() {
                gravity = GRAVITY;
            }
            if entered_hang {
                gravity = GRAVITY; // Player_TryHang returns before the hold
            }
            if hang {
                // $2134C writes vy = -$3400, cancelling this tick's gravity.
                self.velocity_y = 0;
                gravity = 0;
            } else if gravity == 0 {
                camera_accel = 0; // the hold cleared obj_accel_y
            }
            probe_velocity_y = self.velocity_y;
            self.velocity_y += gravity;
        }
        // Player_Physics ($214A0): plr_timer skips Player_CollideVertical;
        // rising outside the somersault takes Player_CollideCeiling, which
        // only splashes. The wall probes ($2217E) run after it.
        let rising = self.velocity_y < 0 && state != SOMERSAULT;
        if !passing && rising {
            self.splashes.extend(self.leave_water(terrain, x));
        }
        self.move_vertically(
            terrain,
            x,
            floor,
            passing,
            !grounded_before,
            probe_velocity_y,
        );
        // `cmpi.w #3,obj_vel_y` reads the velocity the last update moved by.
        self.landed = !grounded_before && self.grounded && velocity_y_before < 4 * UNIT;
        if !passing && !rising {
            let entry = self.enter_water(terrain, x);
            self.splashes.extend(entry);
        }
        if let Some((x, y)) = self.water_probe
            && direction != 0
        {
            let left = direction < 0;
            self.splashes.push(Splash {
                kind: SplashKind::Wade { left },
                x,
                y,
            });
        }
        // $21C44..$21C94: spikes under grounded feet, at the wall probe, or
        // at the head probe 8 px above the body ($224B8).
        let feet_spikes = self.grounded && self.floor.hurts();
        let head_spikes = self.ceiling_contact && self.ceiling.hurts();
        self.spikes = feet_spikes || wall_spikes || head_spikes;
        // `Player_CheckDamage` tests feet and walls first. The head-only
        // branch is the one that applies the extra push-down.
        self.head_spikes = !feet_spikes && !wall_spikes && head_spikes;
        let mut pose = AnimationInput {
            attack: pose.attack,
            ninjutsu,
            hurt: reaction,
            ceiling: ceiling_before,
            step_down: step_down_before,
            against_wall: against_wall_before,
            wall_stop: self.against_wall && self.grounded,
            ..AnimationInput::after(self, grounded_before, velocity_y_before)
        };
        // $22062: `Player_EnterNeutral` clears obj_vel_x before
        // `Player_CollideWalls` and the integration, so a slow landing in
        // state 16 does not step. Native probes the landing after the X
        // step: keep it only where the floor was not under the old X.
        if self.animator.lands_neutral(&pose) {
            let (_, feet) = self.position();
            let old_x = x_before_walls.div_euclid(UNIT);
            if Self::floor_below(terrain, old_x, feet).is_some() {
                self.x = x_before_walls;
                self.velocity_x = 0;
                self.against_wall = false;
                pose.wall_stop = false;
            }
        }
        self.animator.tick(&pose);
        if self.animator.state != THROWN && self.animator.state != THROWN_DEATH {
            self.thrown_damage = 0;
        }
        // Player_EnterNeutral clears the somersault-used bit; $218DA sets
        // it again when Joe falls.
        if let Some(spent) = self.animator.take_somersault() {
            self.somersault_spent = spent;
        }
        self.accel_y = camera_accel;
        self.follow_camera(input, camera_accel);
        // The part limits (`$1A64`, Y from -24) are applied by the game;
        // standalone, keep X inside the layout.
        self.camera_x = self
            .camera_x
            .clamp(0, (terrain.width() - WIDTH as i32).max(0));
    }
    /// Ninjutsu mode `$22A96`: only the player's sequence advances while
    /// `$21A62` holds its motion. Returns true once the sequence has ended.
    pub fn tick_frozen(&mut self) -> bool {
        // `Mode_Ninjutsu` runs `Player_Update`, which counts `plr_ticks`.
        self.ticks += 1;
        self.velocity_x = 0;
        self.velocity_y = 0;
        self.push = 0;
        self.accel_y = 0; // Player_Freeze clears obj_accel_x/obj_accel_y
        self.jump_requested = false;
        self.action = None;
        let pose = AnimationInput {
            grounded_before: self.grounded,
            ..AnimationInput::default()
        };
        self.animator.tick(&pose);
        self.animator.ended
    }
}
