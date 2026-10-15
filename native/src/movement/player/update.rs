use super::*;

impl Player {
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
