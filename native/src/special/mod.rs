//! Round 2-1 (horseback) and round 4-1 (surfing): the autoscrolling mounted
//! special stages (issue #43).
//!
//! Both substages set part-record mode bit 7 (mounted player; bit 8 selects the
//! surfboard) and autoscroll word `$0507`. The rules below were measured with
//! Genesis Plus GX from `obj/emu/states/r2-1.state` and `r4-1.state`
//! routines. The tests replay the recorded input tapes and compare positions
//! with the recorded values.
//!
//! The end guards are ported object handlers: 2-1's `$1C` (`$15308`,
//! `crate::enemies::round2`) and 4-1's `$18` (`$13F3A`,
//! `crate::enemies::round4`). Each alone moves, collides, dies and requests
//! the next substage; this module only stops the autoscroll when its first
//! update clears `$FF0180`. Wave ramps now come from the real list-6 type-4 records in
//! `stage_objects`, and the 4-1 floating debris is the list-4 `$2C` object
//! of the ported `$3C` spawner, which collides through `Combat`.
use crate::{
    combat::Combat,
    game::Terrain,
    input::{Button, Input},
    movement::{
        AIR_STAB_BOUNCE, AIR_STAB_MIN_FALL, AIR_STAB_SPEED, Attack, AttackPosture, DASH_SPEED,
        FALL_THRESHOLD, FUSHIN_JUMP_SPEED, GRAVITY, JUMP_SPEED, KNOCKBACK_SPEED, Player,
        WALK_SPEED, state,
    },
    ninjutsu::Kind,
    sound_events::Table,
    stage::{ModeFlags, Section, StageId},
};

pub(crate) mod thrown;
pub mod warning;
pub use warning::Warnings;

const UNIT: i32 = 256;
/// Screen X limits. Before moving, a rider whose integer screen X (against
/// the camera already advanced this tick) is right of 288 or left of 32 is
/// pushed back by the excess in whole pixels. Matches every unobstructed
/// frame of the recorded 2-1 and 4-1 movement tapes.
pub const SCREEN_LEFT: i32 = 32;
pub const SCREEN_RIGHT: i32 = 288;
/// Held C suppresses gravity for this many ticks after the launch tick, as in
/// the ordinary jump. The launch tick itself already applies gravity, so the
/// recorded first velocity is -4.6875 + 0.203125 = -4.484375 px/tick.
pub const HOLD_TICKS: u32 = 11;
/// Surfboard jumps crouch (frame 192) for four ticks before the launch.
pub const SURF_PREPARE_TICKS: u8 = 4;
/// Recorded ramp launch: -4.0 px/tick (-3.796875 after the launch tick's gravity).
pub const RAMP_LAUNCH: i32 = -4 * UNIT;
/// `$292FA` sets `plr_count` to 25 for a wave-ramp jump, versus 11 for
/// an ordinary jump. `$21286` spends it only while rising with C held.
const RAMP_HOLD_TICKS: u8 = 25;
/// The mounted attack sequences of `$235F6`: state, speed and frames.
/// `PlayerState_HorseAttack` (`$20C5E`) and `PlayerState_SurfAttack`
/// (`$20C38`) run all three of a mount's attacks and return to the ride
/// when the stepper ends the sequence (`plr_status` bit 0).
/// A mounted attack state's speed and frames from the user's cartridge.
fn attack_sequence(state: usize) -> (u8, &'static [usize]) {
    let animation = crate::assets::sequence(state);
    (animation.speed, &animation.frames)
}
/// Updates until the `$214CA` cursor passes the terminator: 16 for the
/// throws, 14 for the slashes and 18 for the powered surf slash.
pub fn attack_ticks(state: usize) -> u8 {
    let (speed, frames) = attack_sequence(state);
    (frames.len() * 256).div_ceil(usize::from(speed)) as u8
}
/// State 53/60 advances by `$40`; falling holds cursor `$0200` so the stab
/// remains active until the rider lands.
const STAB_STEP: u16 = 0x40;
/// State 60's six animation entries; the handler consumes the end bit on
/// the update after the cursor reaches the terminator ($214FA/$20E24).
const SURF_STAB_END: u16 = 6 * 256;
/// Hurt states 50/58 hold the rider in place for seventeen ticks.
pub const HURT_TICKS: u8 = 17;
/// Surfboard bob: `+$48` advances by `$20` per tick and the rider stands at
/// water line + a sine offset decoded from the cartridge. Recorded on every grounded
/// surf frame of the 4-1 tapes (thousands of frames, no exception).
pub const SURF_BOB_STEP: u16 = 0x20;
/// Floating debris, mines and ramps move with the fast water layer. The
/// debris itself is the list-4 `$2C` object of the `$3C` spawner
/// (`crate::enemies::round4`), on the surfer's high priority layer.
pub const WATER_FLOW: i32 = -8;
/// `$235D6`: the surf jump has its own effect, rather than effect 5.
const SURF_JUMP_EFFECT: u8 = 0x27;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mount {
    Horse,
    Surfboard,
}
impl Mount {
    /// `Player_Init` (`$22C60`, `$22CD8`) chooses by the substage, not by
    /// the part record: the horse in 2-1 (`Round_Index` 1, `Substage_Index`
    /// 0), the surfboard in every round-4 substage but 4-2 (4-1 and the
    /// Boss4 arena 4-3), with `plr_flags` bit 7 and `plr_bob` = 8.
    pub fn for_substage(id: StageId) -> Option<Self> {
        match (id.round_index(), id.substage_index()) {
            (1, 0) => Some(Self::Horse),
            (3, substage) if substage != 1 => Some(Self::Surfboard),
            _ => None,
        }
    }
    /// Part-record mode word: bit 7 mounted (`$A334`, `$377D2`), bit 8 the
    /// surfboard (its raised sprite priority, `$22C4C`). Only 2-1 and 4-1
    /// set it; 4-3 surfs without it (no warning signs, no raised priority).
    pub fn from_mode(mode: u16) -> Option<Self> {
        let flags = ModeFlags(mode);
        if !flags.mounted() {
            None
        } else if flags.player_high_priority() {
            Some(Self::Surfboard)
        } else {
            Some(Self::Horse)
        }
    }
    /// Player states (`+$2C`, animations in `$235F6`): ride, attack, hurt, jump.
    pub fn states(self) -> [usize; 4] {
        match self {
            Self::Horse => [48, 49, 50, 51],
            Self::Surfboard => [56, 57, 58, 59],
        }
    }
    /// Down+B in the air: horse state 53, surf state 60.
    fn stab_state(self) -> usize {
        match self {
            Self::Horse => 53,
            Self::Surfboard => 60,
        }
    }
    /// `$22F4A`: selection plus 4 for horse or 8 for surf.
    fn ninjutsu_state(self, kind: Kind) -> usize {
        (match self {
            Self::Horse => 68,
            Self::Surfboard => 72,
        }) + kind.index()
    }
    /// B on the ride or in a jump (`$230F6`, `$23142`): the throw, the
    /// slash and the powered slash states.
    fn attack_states(self) -> [usize; 3] {
        match self {
            Self::Horse => [
                state::HORSE_ATTACK,
                state::HORSE_SLASH,
                state::HORSE_SLASH_POWERED,
            ],
            Self::Surfboard => [
                state::SURF_ATTACK,
                state::SURF_SLASH,
                state::SURF_SLASH_POWERED,
            ],
        }
    }
    fn attack_posture(self) -> AttackPosture {
        match self {
            Self::Horse => AttackPosture::Horse,
            Self::Surfboard => AttackPosture::Surf,
        }
    }
}

/// The horse: a second list-2 record that `Player_Init` (`$22C92`, 2-1 only)
/// links at the rider's `obj_next` with tile word `$2000`. Each update
/// `Player_StepAnimation` (`$21634..$21738`) copies the rider's position,
/// velocity, priority and hide bit to it, shows `Player_FrameTable` entry
/// `$90 + (routine & $1C) / 4` on the ground (six phases of four ticks,
/// the routine restarting at `$18`) or `$A0..$A2` rising, near the apex and
/// falling, then counts the routine up by one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Horse {
    /// The record's `obj_routine`, which counts ticks here.
    pub routine: u16,
    /// Player frame ID drawn on palette line 1, right after the rider.
    pub frame: usize,
}
impl Horse {
    /// First gallop frame (`$240 / 4`).
    pub const GALLOP: usize = 0x90;
    /// Rising, near the apex (integer Y velocity -1 or 0) and falling.
    pub const AIRBORNE: [usize; 3] = [0xa0, 0xa1, 0xa2];
    /// Tile word `$2000`: `Object_DrawSprite` XORs it into each piece, so
    /// the horse's pieces move from palette line 0 to line 1.
    pub const PALETTE_LINE: usize = 1;

    fn new() -> Self {
        Self {
            routine: 0,
            frame: Self::GALLOP,
        }
    }

    /// `$20602`: the gallop phase that `PS_Horse` copies to the rider's
    /// cursor before the horse steps; phase 6 reads as 0.
    fn phase(self) -> usize {
        match usize::from(self.routine & 0x1c) >> 2 {
            6.. => 0,
            phase => phase,
        }
    }

    /// `$216BE..$21738` with the rider's airborne bit and 8.8 Y velocity.
    fn step(&mut self, airborne: bool, velocity_y: i32) {
        self.frame = if airborne {
            match velocity_y >> 8 {
                -1..=0 => Self::AIRBORNE[1],
                1.. => Self::AIRBORNE[2],
                _ => Self::AIRBORNE[0],
            }
        } else {
            if self.routine & 0x1c >= 0x18 {
                self.routine = 0;
            }
            Self::GALLOP + self.phase()
        };
        self.routine = self.routine.wrapping_add(1);
    }
}

/// What the game loop must act on after a special-stage tick.
#[derive(Debug, Default)]
pub struct SpecialEvents {
    /// Driver commands (`$5B56` effects and combat sounds).
    pub sounds: Vec<u8>,
    /// Health ran out: `Player_Die` (`$21A94`) enters the mounted death.
    pub lethal: bool,
    /// The combat pass's signals, among them the end guard's
    /// `Signal::NextSubstage` (`$FF0029` bit 1).
    pub signals: Vec<crate::combat::Signal>,
    /// A mounted technique started; the game loop owns its frozen timeline.
    pub ninjutsu: Option<Kind>,
}

/// Horizontal hold/release bytes in the input record. Mounted sprint tests
/// the preceding tap and release against eight ticks (`$20624`, `$20672`).
#[derive(Clone, Default)]
struct TapControl {
    direction: i32,
    held: u8,
    released: u8,
    previous_held: u8,
}

impl TapControl {
    fn new() -> Self {
        Self {
            released: u8::MAX,
            previous_held: u8::MAX,
            ..Self::default()
        }
    }

    fn tick(&mut self, direction: i32) {
        if direction != self.direction {
            if self.direction != 0 {
                self.previous_held = self.held;
                self.released = 0;
            }
            if direction != 0 {
                self.held = 0;
            }
            self.direction = direction;
        }
        if direction == 0 {
            self.released = self.released.saturating_add(1);
        } else {
            self.held = self.held.saturating_add(1);
        }
    }

    fn sprinting(&self) -> bool {
        self.direction > 0 && self.previous_held < 8 && self.released < 8
    }
}

#[derive(Clone)]
pub struct Special {
    bob: [i32; 64],
    pub mount: Mount,
    /// 8.8 camera X; the displayed camera is the integer part.
    pub camera_x: i32,
    /// `$FF0180` as 8.8 pixels per tick (`$0507`: `$5000` 16.16, 0.3125 px).
    pub step: i32,
    pub scrolling: bool,
    /// `$1B8E` holds the camera for the first tick after arming.
    arming: bool,
    floor: i32,
    pub airborne: bool,
    pub velocity_y: i32,
    /// Acceleration actually applied this update (zero while C holds a jump).
    accel_y: i32,
    launch_age: u32,
    hold: bool,
    /// Surfboard crouch ticks left before the launch.
    pub prepare: u8,
    ramp_pending: bool,
    ramp_hold: u8,
    /// `Player_ForceSurfJump` left `PS_SurfJump` at cursor `$100`: the
    /// next update launches without the crouch.
    forced: bool,
    /// `plr_drift`: whole pixels added to the X velocity while rising
    /// after a forced jump; `Player_EnterNeutral` clears it.
    drift: i32,
    pub attack: u8,
    /// The running attack's state: 49/52/55 or 57/61/63.
    pub attack_state: usize,
    stab: bool,
    stab_cursor: u16,
    velocity_x: i32,
    tap: TapControl,
    pub hurt: u8,
    /// Ending a jump, attack or hurt costs one tick without horizontal motion.
    freeze: bool,
    /// `plr_ctrl` bit 10 on the next update: the mounted attack handlers
    /// (`$20C2C`, `$20C50`) set it before they end their state, so B is
    /// refused on the update an attack ends
    /// ([`crate::movement::rules::attack_locked`]). A ninjutsu's finish
    /// update, which sets it through `Player_Freeze`, takes no buttons.
    attack_lockout: bool,
    pub bob_phase: u16,
    /// The 2-1 horse record; `None` on the surfboard, which has none.
    pub horse: Option<Horse>,
    jump_sound: u8,
    /// The obstacle warning signs (`Func_037814`).
    pub warnings: Warnings,
    warning_sound: u8,
    /// The rider thrown off in a mounted death, in list 9 or list 1.
    thrown: Option<thrown::ThrownRider>,
    wreck: Option<thrown::HorseWreck>,
    board: Option<thrown::SlashBoard>,
}

impl Special {
    /// The riding state of a part of substage `id`
    /// ([`Mount::for_substage`]), or `None` on foot. Prepares `player`
    /// (already at the record's start) for riding.
    pub fn for_stage(
        rom: &[u8],
        id: StageId,
        section: &Section,
        player: &mut Player,
    ) -> Option<Self> {
        let mount = Mount::for_substage(id)?;
        // $1B32: word 13 bit 0 arms, bit 2 selects X, bit 4 negates; the high
        // byte shifted left by four is the 16.16 velocity.
        let setup = section.raw[13];
        let mut step = if setup & 5 == 5 {
            (i32::from(setup & 0xff00) << 4) >> 8
        } else {
            0
        };
        if setup & 0x10 != 0 {
            step = -step;
        }
        let effect = |id| Table::Effect.command(rom, id).ok();
        let sine = crate::rom::Sine::load(rom).ok()?;
        let mut special = Self {
            bob: std::array::from_fn(|i| (i32::from(sine.at(i * 16 + 2)) >> 12) + 4),
            mount,
            camera_x: player.camera_x * UNIT,
            step,
            scrolling: step != 0,
            arming: true,
            floor: player.y,
            airborne: false,
            velocity_y: 0,
            accel_y: GRAVITY,
            launch_age: 0,
            hold: false,
            prepare: 0,
            ramp_pending: false,
            ramp_hold: 0,
            forced: false,
            drift: 0,
            attack: 0,
            attack_state: mount.attack_states()[0],
            stab: false,
            stab_cursor: 0,
            velocity_x: 0,
            tap: TapControl::new(),
            hurt: 0,
            freeze: false,
            attack_lockout: false,
            bob_phase: 0,
            horse: (mount == Mount::Horse).then(Horse::new),
            jump_sound: effect(if mount == Mount::Surfboard {
                SURF_JUMP_EFFECT
            } else {
                5
            })?,
            // `Func_0377D2` installs a schedule only with `Stage_ExitFlags`
            // bit 7 (the part's mode bit 7): 2-1 and 4-1, not 4-3.
            warnings: if ModeFlags(section.mode_flags).mounted() {
                Warnings::load(rom, id.round_index() as u8).ok()?
            } else {
                Warnings::none()
            },
            warning_sound: effect(warning::EFFECT)?,
            thrown: match mount {
                Mount::Surfboard => Some(thrown::ThrownRider::load(rom).ok()?),
                Mount::Horse => None,
            },
            wreck: match mount {
                Mount::Horse => Some(thrown::HorseWreck::load(rom).ok()?),
                Mount::Surfboard => None,
            },
            board: match mount {
                Mount::Surfboard => Some(thrown::SlashBoard::load(rom).ok()?),
                Mount::Horse => None,
            },
        };
        if mount == Mount::Surfboard {
            // The record start (Y 192) is the water line plus its initial sine offset.
            special.floor = player.y - special.bob[0] * UNIT;
        }
        player.facing_left = false;
        player.grounded = true;
        special.animate(player, 0);
        Some(special)
    }

    /// `$1B8E`: the camera advances by `$FF0180` once the arming tick
    /// passed. Returns the displayed camera X.
    fn scroll(&mut self) -> i32 {
        if self.arming {
            self.arming = false;
        } else if self.scrolling {
            self.camera_x += self.step;
        }
        self.camera_x >> 8
    }

    /// The list-1 pass runs on the camera this frame scrolled to (the
    /// 2-1 capture allocated its first sign on the frame `Camera_X` became
    /// `$1E0`). `Func_004B2E`'s on-screen test always passes for the sign.
    fn tick_warnings(&mut self, camera_x: i32, events: &mut SpecialEvents) {
        if self.warnings.tick(camera_x) {
            events.sounds.push(self.warning_sound);
        }
    }

    fn water(&self) -> i32 {
        self.floor + self.bob[usize::from((self.bob_phase >> 5) & 63)] * UNIT
    }

    /// Current autoscroll contribution to the rider's velocity.
    fn flow(&self) -> i32 {
        if self.scrolling { self.step } else { 0 }
    }

    fn launch(&mut self, speed: i32, hold: bool) {
        self.airborne = true;
        self.velocity_y = speed;
        self.launch_age = 0;
        self.hold = hold;
        self.prepare = 0;
        self.ramp_pending = false;
        self.ramp_hold = if self.mount == Mount::Surfboard {
            HOLD_TICKS as u8
        } else {
            0
        };
    }

    /// A real list-6 type-4 ramp touched the surfer this frame. Stage objects
    /// update after the rider, so the launch is consumed on the next tick,
    /// matching `$292D4` and the recorded 4-1 timing.
    pub(crate) fn queue_ramp(&mut self) {
        if self.mount == Mount::Surfboard && !self.airborne {
            self.ramp_pending = true;
        }
    }

    /// The surf jump spends its boost only on rising, held-C updates
    /// (`$21286`); releasing C preserves the remaining allowance. Horse
    /// jumps discard it on release (`$211DE`). Only a jump handler can
    /// suppress gravity, including the update on which B replaces it.
    fn integrate(&mut self, jump_held: bool, jump_handler: bool, was_rising: bool) -> i32 {
        self.accel_y = 0;
        self.hold &= jump_held && (jump_handler || self.launch_age == 0);
        let surf_boost = self.mount == Mount::Surfboard
            && self.launch_age > 0
            && self.ramp_hold > 0
            && was_rising
            && jump_held
            && jump_handler;
        // Player_FloorContact ($220BE) caps before Object_Integrate adds
        // gravity. Preserve the first overshoot below the threshold.
        if self.velocity_y >= FALL_THRESHOLD {
            self.velocity_y = FALL_THRESHOLD - GRAVITY;
        }
        if surf_boost {
            self.ramp_hold -= 1;
        } else if self.mount == Mount::Surfboard
            || self.launch_age == 0
            || !(self.hold && self.launch_age <= HOLD_TICKS)
        {
            self.accel_y = GRAVITY;
            self.velocity_y += self.accel_y;
        }
        self.launch_age += 1;
        self.velocity_y
    }

    /// `Player_ForceSurfJump` (`$2292C`, Boss4's `Func_02F1EA` on a
    /// contact from above): `PS_SurfJump` with cursor `$100`, whatever the
    /// rider was doing, and `plr_drift` = `drift`.
    fn force_jump(&mut self, drift: i32) {
        self.forced = true;
        self.drift = drift;
        self.prepare = 0;
        self.ramp_pending = false;
        self.attack = 0;
        self.stab = false;
        self.stab_cursor = 0;
        self.hurt = 0;
        self.freeze = false;
    }

    fn start_hurt(&mut self) {
        // Player_ApplyHit ($21DCC) discards the incoming vertical momentum.
        self.forced = false;
        self.velocity_y = 0;
        self.hold = false;
        self.hurt = HURT_TICKS;
        self.attack = 0;
        self.stab = false;
        self.stab_cursor = 0;
        self.prepare = 0;
        self.ramp_pending = false;
        self.ramp_hold = 0;
    }

    /// Player_CheckDamage consumes the preceding object pass's contacts.
    /// The current object's update pass runs after mounted movement.
    fn check(
        &self,
        player: &Player,
        combat: &mut Combat,
        _terrain: &impl Terrain,
    ) -> crate::combat::CombatEvents {
        let hidden = !combat.player_visible(player.animator.state, player.ticks);
        combat.set_player_hidden(hidden);
        combat.check_player(
            player.position(),
            false,
            (player.camera_x, player.camera_y),
            None,
        )
    }

    /// Apply an object handler's direct autoscroll write in this frame.
    /// Contact, bounce and surf-jump requests remain queued in Combat for
    /// the next player update, just like the ordinary player path.
    pub(crate) fn after_objects(&mut self, events: &crate::combat::CombatEvents) {
        if events.stop_autoscroll() {
            self.scrolling = false;
        }
    }

    /// One player tick: damage from the last object pass, camera and rider.
    pub fn tick(
        &mut self,
        input: Input,
        player: &mut Player,
        combat: &mut Combat,
        terrain: &impl Terrain,
    ) -> SpecialEvents {
        let mut events = SpecialEvents::default();
        self.accel_y = GRAVITY;
        let [ride, _, hurt_state, jump_state] = self.mount.states();
        combat.set_player_pose(
            player.animator.state,
            player.animator.cursor,
            player.animator.frame,
            input.held(Button::Down),
        );
        // The post-player object pass refreshes the player's anchors.
        combat.set_player_anchors(None);
        // `Game_RunFrame` scrolls the camera before the enemies run, so
        // those placed from it (the 2-1 kites) draw where they were put.
        let camera_x = self.scroll();
        player.camera_x = camera_x;
        combat.mount = Some(self.mount);
        combat.weapons.scroll_speed = self.flow() << 8;
        let hits = self.check(player, combat, terrain);
        events.sounds.extend(hits.sound_ids.iter().copied());
        events.signals.extend(hits.signals.iter().copied());
        if hits.player_event == Some(crate::movement::PlayerEvent::Freeze) {
            self.freeze = true;
        }
        if hits.lethal {
            events.lethal = true;
            self.velocity_x = 0;
            self.velocity_y = 0;
            player.velocity_x = 0;
            player.velocity_y = 0;
            // `$FF0180` stays cleared through the death.
            self.scrolling &= !hits.stop_autoscroll();
            return events;
        }
        // The stab handler does not run the jump's held-C gravity rule.
        // On the entry tick, the preceding jump handler still owns gravity.
        if self.stab {
            self.hold = false;
        }
        // The object pass changed the state before this update's
        // `Player_CheckDamage`, whose hit wins.
        if let Some(drift) = hits.surf_jump
            && self.mount == Mount::Surfboard
        {
            self.force_jump(drift);
        }
        if hits.player_damage {
            self.start_hurt();
        }
        if hits.stop_autoscroll() {
            self.scrolling = false;
        }
        self.tick_warnings(camera_x, &mut events);

        // Horizontal. The normal mounted states replace the velocity each
        // tick. Air-stab writes only its integer word, preserving the
        // autoscroll fraction before adding `$FF0180` again (`$20E6C`,
        // `$23142`), which is why the native stab trace gains 5/16 px each
        // update until that fraction carries.
        let direction = i32::from(input.held(Button::Right)) - i32::from(input.held(Button::Left));
        self.tap.tick(direction);
        let stab_handler = self.stab;
        let settled_jump = !self.airborne
            && self.prepare == 0
            && !self.forced
            && player.animator.state == jump_state;
        let settled_stab = self.stab
            && (!self.airborne
                || self.mount == Mount::Surfboard && self.stab_cursor >= SURF_STAB_END);
        if settled_stab {
            self.stab = false;
            self.stab_cursor = 0;
            self.drift = 0; // Player_EnterNeutral clears plr_drift.
        }
        if self.stab && self.velocity_y >= 0 && self.stab_cursor >> 8 == 2 {
            self.stab_cursor = 0x200;
        }
        // $20E46 clamps the diving cursor using the incoming velocity,
        // then $20E5A consumes the rebound flag and skips the X write.
        let rebound = hits.bounce && self.stab;
        if rebound {
            self.velocity_y = AIR_STAB_BOUNCE | (self.velocity_y & 0xff);
        }
        // `Object_DrawSprite` integrates the velocity after the update and
        // the screen limits act with it, so `Player_StepAnimation` sees the
        // X the last update ended on.
        let unmoved_x = player.position().0;
        let screen = (player.x >> 8) - camera_x;
        if screen > SCREEN_RIGHT {
            player.x -= (screen - SCREEN_RIGHT) * UNIT;
        } else if screen < SCREEN_LEFT {
            player.x += (SCREEN_LEFT - screen) * UNIT;
        }
        let frozen = std::mem::take(&mut self.freeze) || self.hurt > 0;
        self.velocity_x = if frozen {
            0
        } else if settled_stab {
            self.flow()
        } else if self.stab {
            let integer = if rebound {
                self.velocity_x & !0xff
            } else if self.velocity_y < 0 {
                -KNOCKBACK_SPEED
            } else {
                AIR_STAB_SPEED
            };
            (integer & !0xff) | (self.velocity_x & 0xff)
        } else {
            let riding = !self.airborne && self.prepare == 0 && self.attack == 0 && !self.forced;
            let sprint = riding
                && self.tap.sprinting()
                && (self.mount == Mount::Horse || !input.held(Button::Down));
            // `$21298`: the drift adds while rising, after the launch update.
            let drift = if self.airborne && self.velocity_y < 0 && !self.forced {
                self.drift * UNIT
            } else {
                0
            };
            self.flow() + direction * if sprint { DASH_SPEED } else { WALK_SPEED } + drift
        };
        if self.stab {
            self.velocity_x += self.flow();
        }
        player.x += self.velocity_x;

        // `$21244` runs before `$21428`: a forced surf jump assigns its
        // launch velocity before B can replace it with a downward kick.
        let forced_launch = self.mount == Mount::Surfboard && std::mem::take(&mut self.forced);
        let prepared_launch = self.mount == Mount::Surfboard && self.prepare == 1;
        if forced_launch || prepared_launch {
            self.launch(
                if player.fushin {
                    FUSHIN_JUMP_SPEED
                } else {
                    JUMP_SPEED
                },
                true,
            );
        }

        let jump_handler = self.attack == 0 && !stab_handler && self.hurt == 0;
        let was_rising = self.velocity_y < 0;

        // Buttons: A, then B, then C, as in `$21428`. The stab's handler
        // (`$20E14`) sets bit 10 on the update it lands too.
        let attack_locked = std::mem::take(&mut self.attack_lockout) || settled_stab;
        let mut pressed_jump = false;
        if input.pressed(Button::Ninjutsu) {
            if self.hurt == 0
                && self.attack == 0
                && !self.stab
                && !attack_locked
                && !player.fushin
                && let Some(index) = combat.begin_ninjutsu()
                && let Some(kind) = Kind::from_index(index)
            {
                events.ninjutsu = Some(kind);
            }
        } else if input.pressed(Button::Attack) {
            // Button priority is decided before the selected action's lock
            // checks. A refused B still consumes a simultaneous C ($21428).
            if self.hurt == 0 && self.attack == 0 && !self.stab && !attack_locked {
                combat.set_player_motion(self.airborne, self.velocity_y);
                if (self.airborne || self.mount == Mount::Surfboard && self.prepare > 0)
                    && input.held(Button::Down)
                    && !input.held(Button::Up)
                {
                    // `Player_Attack` checks PS_SurfJump, even at cursor $0000
                    // before the board launches. `Player_StartMelee` changes
                    // state only after allocating the board attack object.
                    // $2317E writes velocity before attempting that allocation.
                    self.velocity_y = self.velocity_y.max(AIR_STAB_MIN_FALL);
                    if combat
                        .start_player_attack(AttackPosture::AirStab, player.position(), false)
                        .is_some()
                    {
                        self.prepare = 0;
                        self.stab = true;
                        self.stab_cursor = 0;
                    }
                } else {
                    // `Player_ChooseAttack`: the throw, or a melee whose object
                    // the powered flag picks. Nothing starts without a list-3
                    // record (`$232E2`, `$23322`).
                    let [throw, slash, powered] = self.mount.attack_states();
                    let posture = self.mount.attack_posture();
                    let started = combat.start_player_attack(posture, player.position(), false);
                    if let Some(attack) = started {
                        // Replacing PS_SurfJump before cursor $100 cancels launch.
                        self.prepare = 0;
                        self.attack_state = match attack {
                            Attack::Throw => throw,
                            Attack::Slash if combat.weapons.powered => powered,
                            Attack::Slash => slash,
                        };
                        self.attack = attack_ticks(self.attack_state);
                        if self.attack_state == state::SURF_SLASH_POWERED
                            && let Some(board) = &self.board
                        {
                            board.allocate(combat, player.position());
                        }
                    }
                }
                events.sounds.extend(combat.take_player_sounds());
            }
        } else if self.hurt == 0
            && !self.stab
            && !stab_handler
            && !settled_jump
            && !self.airborne
            && self.prepare == 0
            && !self.ramp_pending
            && !self.forced
            && input.pressed(Button::Jump)
        {
            pressed_jump = true;
            // Grounded mounted attacks leave jump unlocked ($20C38).
            self.attack = 0;
            events.sounds.push(self.jump_sound);
            let speed = if player.fushin {
                FUSHIN_JUMP_SPEED
            } else {
                JUMP_SPEED
            };
            match self.mount {
                Mount::Horse => self.launch(speed, true),
                Mount::Surfboard => self.prepare = SURF_PREPARE_TICKS,
            }
        }

        let jump_held = input.held(Button::Jump);
        let airborne_before_physics = self.airborne;
        // The hit entry displays the stopped rider before integration resumes.
        if hits.player_damage {
            self.accel_y = 0;
        }
        match self.mount {
            Mount::Horse => {
                if self.airborne {
                    if !hits.player_damage {
                        player.y += self.integrate(jump_held, jump_handler, was_rising);
                    }
                    if self.velocity_y > 0 && player.y >= self.floor {
                        self.land(player);
                    }
                }
            }
            Mount::Surfboard => {
                let before = self.water();
                self.bob_phase = self.bob_phase.wrapping_add(SURF_BOB_STEP);
                let water = self.water();
                if !forced_launch && self.ramp_pending && !self.airborne {
                    self.launch(RAMP_LAUNCH, false);
                    self.ramp_hold = RAMP_HOLD_TICKS;
                } else if !forced_launch && !prepared_launch && self.prepare > 0 && !pressed_jump {
                    self.prepare -= 1;
                }
                if self.airborne {
                    // The bob keeps running in the air.
                    let contact = self.velocity_y >= 0 && player.y + self.velocity_y >= before;
                    let velocity = if hits.player_damage {
                        0
                    } else {
                        self.integrate(jump_held, jump_handler, was_rising)
                    };
                    player.y += velocity + (water - before);
                    if contact {
                        player.y = water;
                        self.land(player);
                    }
                } else {
                    player.y = water;
                    self.velocity_y = 0; // Ground contact cancels the kick's +2.
                }
            }
        }

        player.down_held = input.held(Button::Down);
        let landed_jump = self.mount == Mount::Surfboard
            && airborne_before_physics
            && !self.airborne
            && !self.stab
            && self.attack == 0
            && self.hurt == 0;
        let state = if let Some(kind) = events.ninjutsu {
            self.mount.ninjutsu_state(kind)
        } else if self.hurt > 0 {
            hurt_state
        } else if self.stab {
            self.mount.stab_state()
        } else if self.attack > 0 {
            self.attack_state
        } else if self.airborne || self.prepare > 0 || landed_jump {
            jump_state
        } else {
            ride
        };
        self.animate(player, direction);
        if landed_jump && self.mount == Mount::Surfboard {
            player.animator.frame = 187;
        } else if self.mount == Mount::Surfboard
            && (settled_stab || settled_jump || frozen)
            && (state == ride || state == jump_state && self.prepare == 0)
        {
            // Neutral's stepper runs before Surf's Down pose selection.
            player.animator.frame = if self.airborne { 187 } else { 183 };
        }
        // Countdowns after the frame is chosen; each end costs one frozen tick.
        if self.attack > 0 {
            self.attack -= 1;
            if self.attack == 0 {
                self.freeze = true;
                self.attack_lockout = true;
            }
        }
        if self.hurt > 0 {
            self.hurt -= 1;
            if self.hurt == 0 {
                self.freeze = true;
            }
        }
        if state == hurt_state {
            // $20358/$20376: plr_hurt = 32 on every update of the state.
            combat.invulnerability = crate::combat::MOUNTED_HURT_IMMUNITY;
        }
        player.animator.state = state;
        player.animator.cursor = if self.stab {
            let cursor = self.stab_cursor;
            self.stab_cursor = self.stab_cursor.wrapping_add(STAB_STEP);
            cursor.wrapping_add(STAB_STEP)
        } else if self.mount == Mount::Surfboard && state == jump_state {
            if self.prepare > 0 {
                u16::from(SURF_PREPARE_TICKS - self.prepare + 1) * STAB_STEP
            } else {
                0x140
            }
        } else {
            0
        };
        player.animator.ended = false;
        if events.ninjutsu.is_some() {
            // The A-press update still runs mounted physics, then the new
            // sequence takes its first `$20` step. Following updates are
            // owned by `Game::tick_ninjutsu` and freeze the world.
            player
                .animator
                .tick(&crate::movement::AnimationInput::default());
            self.velocity_y = 0;
        }
        player.velocity_x = self.velocity_x;
        player.velocity_y = self.velocity_y;
        player.accel_y = self.accel_y;
        player.grounded = !self.airborne;
        player.facing_left = false;
        player.moving = direction != 0;
        player.jump_requested = pressed_jump;
        player.ticks += 1;
        if let Some(horse) = &mut self.horse {
            horse.step(self.airborne, self.velocity_y);
        }
        // The A press sets `plr_flags` bit 4 (`$22F26`), which skips the
        // bob and the wake of that update.
        if events.ninjutsu.is_none() {
            self.wake((unmoved_x, player.position().1), player.ticks, combat);
        }
        events
    }

    /// One tick of the mounted death (states 54 and 62, `Player_Die`
    /// `$21AB0..$21AF6`), which `crate::death` times. The world goes on:
    /// the camera keeps scrolling, enemies, hazards and the end guard keep
    /// moving, and the handler's cleared X velocity leaves the rider where
    /// it fell until the screen's left edge pushes it (Genesis Plus GX, 2-1
    /// and 4-1). A rider killed in the air falls to its floor. The
    /// rider the surf death throws off (`$7C`, list 9) is
    /// [`Special::update_rider`]'s, as is the horse's bouncing wreck
    /// (`$78`, list 1).
    pub fn tick_dying(
        &mut self,
        player: &mut Player,
        combat: &mut Combat,
        terrain: &impl Terrain,
    ) -> SpecialEvents {
        let mut events = SpecialEvents::default();
        self.velocity_x = 0;
        self.accel_y = GRAVITY;
        combat.set_player_pose(
            player.animator.state,
            player.animator.cursor,
            player.animator.frame,
            false,
        );
        combat.set_player_anchors(None);
        let camera_x = self.scroll();
        player.camera_x = camera_x;
        combat.mount = Some(self.mount);
        combat.weapons.scroll_speed = self.flow() << 8;
        let hits = self.check(player, combat, terrain);
        events.sounds.extend(hits.sound_ids.iter().copied());
        events.signals.extend(hits.signals.iter().copied());
        if hits.stop_autoscroll() {
            self.scrolling = false;
        }
        self.tick_warnings(camera_x, &mut events);
        let unmoved_x = player.position().0;
        let screen = (player.x >> 8) - camera_x;
        if screen < SCREEN_LEFT {
            player.x += (SCREEN_LEFT - screen) * UNIT;
        }
        self.hurt = 0;
        self.attack = 0;
        self.stab = false;
        self.stab_cursor = 0;
        self.prepare = 0;
        self.ramp_pending = false;
        let floor = match self.mount {
            Mount::Horse => self.floor,
            Mount::Surfboard => {
                self.bob_phase = self.bob_phase.wrapping_add(SURF_BOB_STEP);
                self.water()
            }
        };
        if self.airborne {
            self.hold = false;
            player.y += self.integrate(false, false, false);
            if self.velocity_y > 0 && player.y >= floor {
                player.y = floor;
                self.airborne = false;
                self.velocity_y = 0;
            }
        } else {
            player.y = floor;
        }
        // The death handler changes nothing in the animator: this is the
        // stepper alone.
        player
            .animator
            .tick(&crate::movement::AnimationInput::default());
        player.velocity_x = self.velocity_x;
        player.velocity_y = self.velocity_y;
        player.accel_y = self.accel_y;
        player.grounded = !self.airborne;
        player.moving = false;
        player.jump_requested = false;
        player.ticks += 1;
        // The surf death keeps `plr_flags` bit 7: the wake goes on.
        self.wake((unmoved_x, player.position().1), player.ticks, combat);
        events
    }

    /// `Player_Die` (`$21AB0`): a list-9 surf rider or list-1 horse wreck,
    /// first updated by the same update's [`Special::update_rider`].
    pub(crate) fn throw_rider(&self, player: &Player, combat: &mut Combat) {
        if let Some(thrown) = &self.thrown {
            thrown.allocate(combat, player.position());
        }
        if let Some(wreck) = &self.wreck {
            wreck.allocate(combat, player.position());
        }
    }
    /// `Func_026CCE`, list 9 after the player's update: the thrown rider
    /// follows the death's cursor (`special::thrown`).
    pub(crate) fn update_rider(
        &self,
        player: &mut Player,
        death: &mut crate::death::Death,
        combat: &mut Combat,
        terrain: &impl Terrain,
    ) {
        if let Some(thrown) = &self.thrown {
            thrown.update(combat, player, death, terrain);
        }
        if let Some(wreck) = &self.wreck {
            wreck.update(combat, death, terrain);
        }
    }

    /// List-9 powered slash board, after player movement and object updates.
    pub(crate) fn update_board(&self, player: &Player, combat: &mut Combat) {
        if let Some(board) = &self.board {
            board.update(combat, player);
        }
    }

    /// `Player_StepAnimation` (`$21740..$217AC`), after the bob: a surfer
    /// on the water (`plr_status` bit 5 clear) at `position` leaves a wake
    /// splash when `plr_ticks` is a multiple of four
    /// (`ambient::surf_wake`). The horse has none.
    fn wake(&self, position: (i32, i32), ticks: u64, combat: &mut Combat) {
        if self.mount == Mount::Surfboard && !self.airborne {
            combat.surf_wake(position, ticks);
        }
    }

    fn land(&mut self, player: &mut Player) {
        if self.mount == Mount::Horse {
            player.y = self.floor;
        }
        self.airborne = false;
        self.velocity_y = 0;
        self.drift = 0;
        // State 53/60 remains visible on the contact update and its handler
        // settles on the next one. Ordinary jumps retain their recorded
        // one-tick mounted landing pause.
        self.freeze = !self.stab;
    }

    /// Restore the mounted neutral/jump pose in the frozen update that
    /// finishes a ninjutsu (`Ninjutsu_Finish`). `PlayerAnimator`'s generic
    /// settle path does not know which mount selected states 68..75. That
    /// update takes no buttons, and `Player_Physics` clears the lock bits
    /// its `Player_Freeze` set, so the next update takes B.
    pub(crate) fn resume_after_ninjutsu(&mut self, player: &mut Player) {
        self.animate(player, 0);
        player.animator.state = if self.airborne {
            self.mount.states()[3]
        } else {
            self.mount.states()[0]
        };
        player.animator.cursor = 0;
        player.animator.ended = false;
        player.velocity_y = self.velocity_y;
    }

    /// The displayed frame (`$23ADE` IDs equal the mapping IDs here).
    fn animate(&self, player: &mut Player, direction: i32) {
        // The `$214CA` stepper: the frame at the cursor before the speed is
        // added, so a throw shows a, b, then c for eight ticks.
        let (speed, frames) = attack_sequence(self.attack_state);
        let elapsed = attack_ticks(self.attack_state).saturating_sub(self.attack);
        let index = (usize::from(elapsed) * usize::from(speed)) >> 8;
        let attack_frame = frames[index.min(frames.len() - 1)];
        player.animator.frame = match self.mount {
            Mount::Horse => {
                if self.hurt > 0 {
                    156
                } else if self.stab {
                    165
                } else if self.attack > 0 {
                    attack_frame
                } else if self.airborne {
                    // Animation 51 [180,163,164,165,181]: rising, apex, falling.
                    if self.velocity_y < -192 {
                        163
                    } else if self.velocity_y >= 288 {
                        165
                    } else {
                        164
                    }
                } else {
                    // Animation 48: 150..155 at a gallop, 172..177 while
                    // Right is held, at the horse's phase (`$20602`).
                    let base = if direction > 0 { 172 } else { 150 };
                    base + self.horse.map_or(0, Horse::phase)
                }
            }
            Mount::Surfboard => {
                if self.hurt > 0 {
                    191
                } else if self.stab {
                    match self.stab_cursor >> 8 {
                        0 | 1 => 189,
                        2 => 188,
                        _ => 187,
                    }
                } else if self.attack > 0 {
                    attack_frame
                } else if self.prepare > 0 {
                    192
                } else if self.airborne {
                    187
                } else if player.down_held {
                    192
                } else if self.tap.sprinting() && self.tap.held > 12 {
                    // $20688: the second press gains speed immediately,
                    // then selects surf sequence slot 2 after twelve ticks.
                    187
                } else {
                    183
                }
            }
        };
    }
}
