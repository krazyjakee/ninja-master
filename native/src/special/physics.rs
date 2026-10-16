use super::*;

impl Special {
    /// `$1B8E`: the camera advances by `$FF0180` once the arming tick
    /// passed. Returns the displayed camera X.
    pub(super) fn scroll(&mut self) -> i32 {
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
    pub(super) fn tick_warnings(&mut self, camera_x: i32, events: &mut SpecialEvents) {
        if self.warnings.tick(camera_x) {
            events.sounds.push(self.warning_sound);
        }
    }
    pub(super) fn water(&self) -> i32 {
        self.floor + self.bob[usize::from((self.bob_phase >> 5) & 63)] * UNIT
    }
    /// Current autoscroll contribution to the rider's velocity.
    pub(super) fn flow(&self) -> i32 {
        if self.scrolling { self.step } else { 0 }
    }
    pub(super) fn launch(&mut self, speed: i32, hold: bool) {
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
    pub(super) fn integrate(
        &mut self,
        jump_held: bool,
        jump_handler: bool,
        was_rising: bool,
    ) -> i32 {
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
    pub(super) fn force_jump(&mut self, drift: i32) {
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
    pub(super) fn start_hurt(&mut self) {
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
    pub(super) fn check(
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
    pub(super) fn wake(&self, position: (i32, i32), ticks: u64, combat: &mut Combat) {
        if self.mount == Mount::Surfboard && !self.airborne {
            combat.surf_wake(position, ticks);
        }
    }
    pub(super) fn land(&mut self, player: &mut Player) {
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
}
