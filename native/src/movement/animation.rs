//! `PlayerAnimator`, the native state and button handlers using the asset
//! catalog's `$235F6` sequences and `$214CA` cursor stepper.
use super::{
    Player, hang_phase_next,
    rules::{Action, Attack, AttackPosture, HangExit, attack_posture, dying, hurt},
    state,
};

use crate::assets::{sequence, step_cursor};

/// Top of the player's hitbox relative to the feet for a displayed frame ID:
/// `hitbox_y - hitbox_h` of its `Sprite_Mappings` record, reached through
/// `Player_FrameTable` (`$23ADE`). `Object_LoadHitbox` reloads it with every
/// frame, so the ceiling probe (`Func_022494`) and `Player_TryHang` read the
/// frame shown on the previous update. A test pins the table to the ROM.
pub fn body_top(frame: usize) -> i32 {
    crate::assets::player_body_top(frame)
}

/// What the original handlers read on one tick. `grounded_before` and
/// `velocity_y_before` are the values before this tick's movement, matching
/// the handlers' use of +$2B bit 5 and +$1C before the `$19D4` integration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimationInput {
    pub grounded_before: bool,
    pub velocity_y_before: i32,
    pub jump_requested: bool,
    /// Held horizontal direction (-1, 0, 1), A4+$04.
    pub direction: i32,
    /// A new horizontal press this tick, A4+$08.
    pub direction_edge: bool,
    pub running: bool,
    pub crouching: bool,
    /// Down held (A4+$02 above zero), also while airborne.
    pub down: bool,
    /// B held (A4+$00 bit 4): the guard handlers `$23360` and `$229C2`.
    pub attack_held: bool,
    /// B newly pressed: the combo check `$20CDE`.
    pub attack_pressed: bool,
    /// Any of A, B or C newly pressed: `$21428` then skips the held-B handler.
    pub buttons_pressed: bool,
    /// Airborne jump press accepted by the state-16/25 handler.
    pub action: Option<Action>,
    /// Guard recoil +$32 is running (`$20076`, `$20192`).
    pub recoil: bool,
    pub attack: Option<Attack>,
    /// Technique started by `$22E8E` (0 Ikazuchi .. 3 Mijin).
    pub ninjutsu: Option<usize>,
    /// Non-lethal damage taken this tick: the hurt state it enters
    /// (`hit_state`).
    pub hurt: Option<usize>,
    /// `plr_ctrl` bit 6 as the handlers see it: contact on the last update.
    pub ceiling: bool,
    /// `Player_HangStop` (`$2029E`): no direction, or a wall ahead.
    pub hang_stop: bool,
    /// `plr_flags` bit 0: `$21804` returns to the hang (`$21834`).
    pub hanging: bool,
    /// C accepted by the hang exit this tick.
    pub hang_exit: Option<HangExit>,
    /// C while crouching dropped through the floor (`$23478`).
    pub drop_through: bool,
    /// A low ceiling keeps the crouching guard down (`$20116`).
    pub low_ceiling: bool,
    /// `plr_ctrl` bit 8 from the last update: stepping down a small drop.
    pub step_down: bool,
    /// `plr_ctrl` bit 1 from the last update: a wall stopped the motion.
    pub against_wall: bool,
    /// A wall stopped the motion on this update, on the ground.
    pub wall_stop: bool,
    /// `$22062..$2207E`: this update's physics met the floor after an
    /// airborne update, at vy 3 px/tick or less.
    pub landed: bool,
    /// `plr_status` bit 7 (`Player_PowerUp`): the handlers pick the powered
    /// states (`$1FF10`, `$20054`, `$23398`, `Player_ChooseAttack`).
    pub powered: bool,
}
impl AnimationInput {
    /// Handler inputs from a player that has just moved, with the support and
    /// vertical velocity it had before moving.
    pub fn after(player: &Player, grounded_before: bool, velocity_y_before: i32) -> Self {
        Self {
            grounded_before,
            velocity_y_before,
            jump_requested: player.jump_requested,
            direction: player.direction,
            direction_edge: player.direction_edge,
            running: player.running,
            crouching: player.crouching,
            down: player.down_held,
            attack_held: player.attack_held,
            attack_pressed: player.attack_pressed,
            buttons_pressed: player.buttons_pressed,
            action: player.action,
            recoil: player.recoil,
            attack: None,
            ninjutsu: None,
            hurt: None,
            ceiling: player.ceiling_contact,
            hang_stop: player.hang_stop,
            hanging: player.hanging,
            hang_exit: player.hang_exit,
            drop_through: player.drop_through,
            low_ceiling: player.low_ceiling,
            step_down: player.step_down,
            against_wall: player.against_wall,
            wall_stop: false,
            landed: player.landed,
            powered: player.powered,
        }
    }
}

/// Native form of the player's +$2C state, +$2E cursor and +$2A bit 0 end
/// flag. Each tick applies the damage entry, the state handler, the button
/// handler and finally the stepper, in the original `$1FC36..$214CA` order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerAnimator {
    pub state: usize,
    pub cursor: u16,
    pub ended: bool,
    /// Frame ID displayed by the last tick.
    pub frame: usize,
    /// +$2A bit 14, set by the running handler at `$1FF46`.
    run_latch: bool,
    /// +$2A bit 12 as the last tick left it: `Player_EnterNeutral` clears
    /// it (`$2180E`) and `$218DA` sets it when falling (`$218FE`).
    somersault: Option<bool>,
}
impl Default for PlayerAnimator {
    fn default() -> Self {
        Self {
            state: state::IDLE,
            cursor: 0,
            ended: false,
            frame: 1,
            run_latch: false,
            somersault: None,
        }
    }
}
impl PlayerAnimator {
    fn set(&mut self, id: usize, cursor: u16) {
        self.state = id;
        self.cursor = cursor;
        self.ended = false;
        if id != state::RUN {
            self.run_latch = false;
        }
    }
    /// `Player_Update`'s forced-input table: select a state and keep its
    /// cursor only while the current high-byte slot is at most `last_slot`.
    pub(crate) fn force(&mut self, id: usize, last_slot: u8) {
        self.state = id;
        self.ended = false;
        if self.cursor >> 8 > u16::from(last_slot) {
            self.cursor = 0;
        }
        if id != state::RUN {
            self.run_latch = false;
        }
    }
    /// Airborne as the ground handlers test it (`$1FE90`): +$2B bit 5
    /// without the step-down flag.
    fn airborne(input: &AnimationInput) -> bool {
        !input.grounded_before && !input.step_down
    }
    /// `$218DA`: leaving the ground enters state 16 at the rising or falling
    /// pose. While stepping down (`$218E0`) it settles instead.
    fn fall(&mut self, input: &AnimationInput) {
        if input.step_down {
            return self.settle(input);
        }
        let cursor = if input.velocity_y_before < 0 {
            0x100
        } else {
            self.somersault = Some(true); // $218FE
            0x300
        };
        self.set(state::JUMP, cursor);
    }
    /// `$21804`, the common return to the ground states.
    fn settle(&mut self, input: &AnimationInput) {
        self.somersault = Some(false); // $2180E: +$2A bits 12 and 13
        if input.hanging {
            return self.set(state::HANG, 0); // $21834 -> Player_EnterHang
        }
        if Self::airborne(input) {
            return self.fall(input); // $21836
        }
        // $21842: the first tick after running keeps state 28 at cursor zero.
        if std::mem::take(&mut self.run_latch) && self.state == state::RUN {
            return self.set(state::RUN, 0);
        }
        if input.crouching {
            self.set(state::CROUCH, 0); // $2184A -> $218C6
        } else {
            self.stand(input);
        }
    }
    /// State 16 after the handlers meets the floor slowly (`$22062`). Natively
    /// the preparation (slot 0) is left alone, so a jump accepted while
    /// stepping down still launches.
    fn lands(&self, input: &AnimationInput) -> bool {
        input.landed && self.state == state::JUMP && self.cursor >> 8 != 0
    }
    /// Whether this update's physics sends state 16 to neutral (`lands`).
    pub fn lands_neutral(&self, input: &AnimationInput) -> bool {
        let mut next = self.clone();
        next.handle(input);
        next.buttons(input);
        next.lands(input)
    }
    /// `$22062..$2207E`: state 16 meeting the floor slowly calls
    /// `Player_EnterNeutral` from the physics, skipping the landing frames.
    /// Bit 5 is already clear, so it stands, runs on with the run latch
    /// (`$21842`, the native run control) or crouches with Down (`$2184A`).
    fn land(&mut self, input: &AnimationInput) {
        self.somersault = Some(false); // $2180E
        if input.running {
            self.set(state::RUN, 0);
            self.run_latch = true;
        } else if input.down {
            self.set(state::CROUCH, 0);
        } else {
            self.stand(input);
        }
    }
    /// `$21852..$21868`: state 1 at cursor 0, or `$100` while powered
    /// (`plr_status` bit 7), written on every idle update: frame 1, or 48
    /// powered, is held and never stepped.
    fn stand(&mut self, input: &AnimationInput) {
        self.set(state::IDLE, if input.powered { 0x100 } else { 0 });
    }
    /// Ground handlers `$1FE90` (1, 2), `$1FF1A` (28) and `$1FF4E` (8, 9).
    fn ground(&mut self, input: &AnimationInput) {
        if Self::airborne(input) {
            return self.fall(input);
        }
        match self.state {
            state::RUN => {
                if input.running {
                    self.run_latch = true;
                } else {
                    self.settle(input);
                }
            }
            state::CROUCH | state::CRAWL => {
                if !input.crouching {
                    self.settle(input); // $1FF98
                } else if input.direction != 0 {
                    if input.direction_edge {
                        self.cursor = 0; // $1FFB4
                    }
                    self.state = state::CRAWL; // $1FFB8 keeps the cursor
                } else {
                    self.set(state::CROUCH, 0); // $218C6
                }
            }
            _ => {
                if input.crouching {
                    self.set(state::CROUCH, 0); // $1FEAC -> $218C6
                } else if input.running {
                    // $1FEC4/$1FEE6 on the accepted edge. A run that continues
                    // through a landing also keeps state 28 natively.
                    if self.state != state::RUN {
                        self.set(state::RUN, 0);
                    }
                    self.run_latch = true;
                } else if input.direction != 0 {
                    if input.direction_edge {
                        self.cursor = 0; // $1FEC4
                    }
                    // $1FF06 keeps the cursor; $1FF10 walks in 26 powered.
                    self.state = if input.powered {
                        state::WALK_POWERED
                    } else {
                        state::WALK
                    };
                } else {
                    self.settle(input);
                }
            }
        }
    }
    /// State 16 handler `$20F5E`: preparation, launch, rise, fall and landing.
    fn jump(&mut self, input: &AnimationInput) {
        if self.ended {
            return self.settle(input); // $20F6A
        }
        if self.cursor >> 8 == 0 || self.cursor == 0x100 {
            return; // $20F6E preparation frames; $20FB0 launch tick
        }
        if input.velocity_y_before < 0 {
            self.cursor = 0x100; // $21014: hold frame 29 while rising
        } else if !input.grounded_before {
            self.cursor = 0x300; // $2104C: hold frame 28 while falling
        } else if self.cursor >> 8 < 4 {
            self.cursor = 0x400; // $21066: landing frame 14, then $FF
        }
    }
    /// `PlayerState_Hang` (`$201C0`): losing the ceiling falls (`$218DA`).
    /// Phase 7 wraps to 1 (`$20254`); a stopped hang moves to the next rest
    /// phase (`Player_HangStop`). Both act on a phase's first update only.
    fn hang(&mut self, input: &AnimationInput) {
        if !input.ceiling {
            return self.fall(input);
        }
        if self.cursor & 0xff != 0 {
            return;
        }
        let phase = usize::from(self.cursor >> 8);
        if input.hang_stop {
            self.cursor = u16::from(hang_phase_next()[phase & 7]) << 8;
        } else if phase == 7 {
            self.cursor = 0x100;
        }
    }
    /// The standing and crouching guards hold their first frame (130, 109);
    /// powered, `$2002C`, `$20148`, `$201A4` and `$23398` hold the second
    /// (131, 113) with cursor `$100`.
    fn guard_cursor(input: &AnimationInput) -> u16 {
        if input.powered { 0x100 } else { 0 }
    }
    /// Guard handler `$1FFDC` for states 3, 4 and 27.
    fn guard(&mut self, input: &AnimationInput) {
        use state::*;
        if Self::airborne(input) {
            return self.fall(input);
        }
        if !input.attack_held {
            return self.settle(input); // $20000..$20004
        }
        let walk = if input.powered {
            GUARD_WALK_POWERED
        } else {
            GUARD_WALK
        };
        if input.recoil {
            return self.set(walk, 0); // $2007A..$2008C
        }
        if input.down {
            return self.set(CROUCH_GUARD, Self::guard_cursor(input)); // $2000C -> $2013C
        }
        if input.direction == 0 {
            return self.set(GUARD, Self::guard_cursor(input)); // $20020: held
        }
        if input.direction_edge {
            self.cursor = 0; // $20038
        }
        self.state = walk; // $2004A..$20058 keep the cursor
    }
    /// Crouching guard `$200A4` for states 10, 11 and 15.
    fn crouch_guard(&mut self, input: &AnimationInput) {
        use state::*;
        if Self::airborne(input) {
            return self.fall(input);
        }
        if !input.attack_held {
            return self.set(CROUCH, 0); // $200CE -> $218C6
        }
        if input.recoil {
            return self.set(CROUCH_GUARD, Self::guard_cursor(input)); // $20196
        }
        if !input.down && !input.low_ceiling {
            return self.set(GUARD, Self::guard_cursor(input)); // $20124 -> $20020
        }
        if input.direction == 0 {
            return self.set(CROUCH_GUARD, Self::guard_cursor(input)); // $2013C
        }
        if input.direction_edge {
            self.cursor = 0; // $20154
        }
        // $20166..$20174
        self.state = if input.powered {
            CROUCH_GUARD_WALK_POWERED
        } else {
            CROUCH_GUARD_WALK
        };
    }
    /// Wall cling and wall jump `$212FE` (state 25).
    fn wall(&mut self, input: &AnimationInput) {
        if self.ended {
            return self.settle(input); // $2130A
        }
        if input.grounded_before {
            if self.cursor >> 8 < 3 {
                self.cursor = 0x300; // $21332: landing frame 14
            }
            return;
        }
        if self.cursor >> 8 == 0 || self.cursor == 0x100 {
            return; // $2134C hang (frame 129); $213C2 launch
        }
        if input.against_wall {
            // $21364: a wall met in flight continues as state 16.
            self.state = state::JUMP;
            return self.jump(input);
        }
        // $21394/$2139E: frame 29 while rising, 28 while falling.
        self.cursor = if input.velocity_y_before < 0 {
            0x100
        } else {
            0x200
        };
    }
    /// Enter a death state at `cursor` (`Player_Die` `$21A94`, `$1FD84`,
    /// `$20418`, `$20746`).
    pub fn die(&mut self, state: usize, cursor: u16) {
        self.set(state, cursor);
    }
    /// Damage entry and state handler (`$1FD20`, `$1FDF8`), before the buttons.
    pub fn handle(&mut self, input: &AnimationInput) {
        use state::*;
        // PlayerState_Death never changes the state; its plr_flags bit 3
        // skips the damage entry ($1FD74).
        if dying(self.state) {
            return;
        }
        if let Some(id) = input.hurt {
            self.set(id, 0); // $21E54..$21E80, $21ECA
        }
        match self.state {
            SLASH | COMBO => {
                if self.ended {
                    self.settle(input); // $20C80
                } else if self.state == SLASH && input.attack_pressed {
                    self.state = COMBO; // $20CDE..$20CEE keeps the cursor
                }
            }
            // PlayerState_SlashPowered (`$20CF6`) has no combo.
            SLASH_POWERED | THROW | AIR_SLASH | AIR_SLASH_POWERED | AIR_THROW => {
                if self.ended {
                    self.settle(input); // $20D08, $20D40
                } else if input.grounded_before
                    && matches!(self.state, AIR_SLASH | AIR_SLASH_POWERED | AIR_THROW)
                {
                    // $20D4C..$20D5C: landing continues as the ground attack
                    // (the state minus 13, and 2 more for the throw).
                    self.state = match self.state {
                        AIR_SLASH => SLASH,
                        AIR_SLASH_POWERED => SLASH_POWERED,
                        _ => THROW,
                    };
                }
            }
            WALL_SLASH | WALL_SLASH_POWERED | WALL_THROW => {
                if self.ended {
                    self.settle(input); // $20D96 -> $2130A
                }
            }
            HANG_SLASH | HANG_SLASH_POWERED | HANG_THROW => {
                if self.ended {
                    self.set(HANG, 0); // $20EF2 -> Player_EnterHang
                }
            }
            CROUCH_SLASH | CROUCH_SLASH_POWERED | CROUCH_THROW => {
                if self.ended {
                    self.set(CROUCH, 0); // $20D24 -> $218C6
                }
            }
            HURT | KNOCKDOWN | SHOCKED => {
                if self.ended {
                    // $202C2..$202D0, $20314..$20322
                    if input.grounded_before {
                        self.set(CROUCH, 0);
                    } else {
                        self.settle(input);
                    }
                }
            }
            THROWN => {
                if self.ended {
                    self.settle(input); // `$203B2..$203C8`
                }
            }
            SOMERSAULT | DASH_KICK => {
                // $212AC..$212B8, $20F08..$20F1A: a grounded end crouches.
                if self.ended {
                    if input.grounded_before {
                        self.set(CROUCH, 0);
                    } else {
                        self.settle(input);
                    }
                }
            }
            AIR_STAB => {
                // $20E1E..$20E42: the end, the ground or a wall (+$2B bit 1)
                // settles. Touching a ceiling in frame slot 0 skips the wall.
                let walled = input.against_wall && !(input.ceiling && self.cursor >> 8 == 0);
                if self.ended || input.grounded_before || walled {
                    self.settle(input);
                } else if input.velocity_y_before >= 0 && self.cursor >> 8 == 2 {
                    self.cursor = 0x200; // $20E54: frame 97 while diving
                }
            }
            WALL_CLING => self.wall(input),
            HANG => self.hang(input),
            GUARD | GUARD_WALK | GUARD_WALK_POWERED => self.guard(input),
            CROUCH_GUARD | CROUCH_GUARD_WALK | CROUCH_GUARD_WALK_POWERED => {
                self.crouch_guard(input)
            }
            JUMP => self.jump(input),
            id if id >= NINJUTSU => {
                if self.ended {
                    self.settle(input); // $2192E resumes through $21804
                }
            }
            _ => self.ground(input),
        }
        match input.action {
            Some(Action::Somersault) => self.set(SOMERSAULT, 0), // $21098
            Some(Action::WallCling) => self.set(WALL_CLING, 0),  // $21108
            Some(Action::Hang) => self.set(HANG, 0),             // $217FA
            None => {}
        }
    }
    /// Button handler `$21428`: A, then B, then C, then held B; only one runs.
    fn buttons(&mut self, input: &AnimationInput) {
        use state::*;
        // $204FA sets plr_ctrl bits 9-11: no jump, attack or ninjutsu.
        if dying(self.state) {
            return;
        }
        if let Some(kind) = input.ninjutsu {
            return self.set(NINJUTSU + kind, 0); // $22F4A..$22F5E
        }
        if let Some(attack) = input.attack {
            // `Player_ChooseAttack` (`$232CC`) returns the sign of
            // `plr_status`: a powered melee takes the powered state.
            let slash = |ordinary: usize, powered: usize, throw: usize| match attack {
                Attack::Slash if input.powered => powered,
                Attack::Slash => ordinary,
                Attack::Throw => throw,
            };
            let next = match attack_posture(self.state, input.down, input.grounded_before) {
                Some(AttackPosture::Standing) => Some(slash(SLASH, SLASH_POWERED, THROW)),
                Some(AttackPosture::Crouching) => {
                    Some(slash(CROUCH_SLASH, CROUCH_SLASH_POWERED, CROUCH_THROW))
                }
                Some(AttackPosture::Airborne) => {
                    Some(slash(AIR_SLASH, AIR_SLASH_POWERED, AIR_THROW))
                }
                Some(AttackPosture::WallAir) => {
                    Some(slash(WALL_SLASH, WALL_SLASH_POWERED, WALL_THROW))
                }
                Some(AttackPosture::AirStab) => Some(AIR_STAB),
                Some(AttackPosture::DashKick) => Some(DASH_KICK),
                Some(AttackPosture::Hang) => {
                    Some(slash(HANG_SLASH, HANG_SLASH_POWERED, HANG_THROW))
                }
                // `Special` starts the mounted attacks itself.
                Some(AttackPosture::Spread | AttackPosture::Horse | AttackPosture::Surf) | None => {
                    None
                }
            };
            if let Some(next) = next {
                self.set(next, 0); // $2330A/$2330E, $23332/$23336
            }
            return;
        }
        match input.hang_exit {
            Some(HangExit::Drop) => return self.set(JUMP, 0x300), // $23584 -> $218DA
            Some(HangExit::Climb) => return self.set(JUMP, 0x100), // $23572
            None => {}
        }
        if input.drop_through {
            return self.set(JUMP, 0x300); // $234EC -> $218DA
        }
        // $2346E; hurt sets +$2A bit 9, which blocks the jump handler.
        if input.jump_requested && !hurt(self.state) {
            return self.set(JUMP, 0);
        }
        if input.attack_held && !input.buttons_pressed {
            // $23360: holding B turns ordinary ground states into guards,
            // the powered ones while powered (26 always guards as 27).
            let (next, cursor) = match self.state {
                IDLE => (GUARD, Self::guard_cursor(input)),
                WALK if !input.powered => (GUARD_WALK, 0),
                WALK | WALK_POWERED => (GUARD_WALK_POWERED, 0),
                CROUCH => (CROUCH_GUARD, Self::guard_cursor(input)),
                CRAWL if !input.powered => (CROUCH_GUARD_WALK, 0),
                CRAWL => (CROUCH_GUARD_WALK_POWERED, 0),
                _ => return,
            };
            self.set(next, cursor);
        }
    }
    /// The somersault-used bit if the last tick went through
    /// `Player_EnterNeutral` or `$218DA`.
    pub fn take_somersault(&mut self) -> Option<bool> {
        self.somersault.take()
    }
    /// The state the state handler and the button handler leave for
    /// `Player_Physics`, without stepping the sequence.
    pub fn resolve(&self, input: &AnimationInput) -> usize {
        let mut next = self.clone();
        next.handle(input);
        next.buttons(input);
        next.state
    }
    /// Advance one tick and return the displayed frame ID.
    pub fn tick(&mut self, input: &AnimationInput) -> usize {
        self.somersault = None;
        self.handle(input);
        self.buttons(input);
        if self.lands(input) {
            self.land(input);
        }
        // $2243C..$22450: Player_Physics' wall stop clears the run latch
        // (+$2A bit 14) and, on the ground, walks at cursor 0.
        if input.wall_stop && std::mem::take(&mut self.run_latch) {
            self.set(state::WALK, 0);
        }
        let sequence = sequence(self.state);
        let (frame, ended) = step_cursor(sequence.speed, &sequence.frames, &mut self.cursor);
        self.ended = ended;
        self.frame = frame;
        frame
    }
}
