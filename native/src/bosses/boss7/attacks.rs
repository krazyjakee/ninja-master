//! Routines `$C` (stand, then pick an attack) and `$10` (the attacks).
use super::*;

impl Boss7 {
    /// `Func_034426` (second form): an uppercut when the player jumps close.
    fn uppercut(&self, cx: &Ctx, e: &mut Enemy) -> bool {
        if e.vars[FLAGS] & PHASE2 == 0
            || cx.player_dx(e.x) >= self.uppercut_range
            || !Self::airborne(cx, e)
        {
            return false;
        }
        Self::face(cx, e);
        e.routine = 0x10;
        e.vars[FLAGS] |= GUARDLESS;
        e.vars[FLAGS] &= !SIGNAL;
        e.vars[SUB] = 4;
        Self::start(e, self.uppercut_script);
        e.motion.vx = Self::sign(e) * self.uppercut_vx;
        true
    }
    /// `Func_034482`: run in at 8 px/tick when 64 px or more away.
    pub(super) fn run(&self, cx: &Ctx, e: &mut Enemy) {
        Self::face(cx, e);
        if cx.player_dx(e.x) < self.run_min {
            return;
        }
        e.motion.vx = -Self::sign(e) * self.run_speed;
        e.mapping = self.run_frame;
        e.vars[FLAGS] |= RUN;
    }
    /// `loc_034342`: the second form's next special: every fourth the hover.
    fn next_special(&self, cx: &Ctx, e: &mut Enemy) {
        e.vars[STEP] = 0;
        Self::face(cx, e);
        let cycle = (Self::misc(e, M_CYCLE) + 1) & 3;
        Self::set_misc(e, M_CYCLE, cycle);
        if cycle == 0 {
            e.vars[SUB] = 0x14;
            Self::start(e, self.hover_script);
        } else {
            e.vars[SUB] = 0x18;
            Self::start(e, self.leap_script);
        }
    }
    /// `$3323A`: stand, then pick an attack (`$3326C`).
    pub(super) fn idle(&self, cx: &mut Ctx, e: &mut Enemy) {
        self.animate(e);
        if matches!(cx.player_state, Some(41 | 42 | 43 | 46)) {
            Self::face(cx, e);
            e.routine = 0x28;
            e.motion.stop();
            e.mapping = 0x2f8;
            return;
        }
        if self.uppercut(cx, e) {
            return;
        }
        e.vars[COUNT] -= if e.vars[FLAGS] & PHASE2 != 0 { 3 } else { 1 };
        if e.vars[COUNT] >= 0 {
            return;
        }
        e.routine = 0x10;
        e.vars[FLAGS] |= RESTART;
        e.vars[FLAGS] &= !SIGNAL;
        if cx.player_dx(e.x) < self.near {
            e.vars[SUB] = 0;
            Self::start(e, self.slash_script);
            self.run(cx, e);
            self.voice(cx, self.run_voice);
        } else if e.vars[FLAGS] & PHASE2 == 0 {
            e.vars[SUB] = 4;
            Self::start(e, self.throw_script);
        } else if cx.random() & 1 == 0 {
            e.vars[SUB] = 8;
            Self::start(e, self.spin_script);
            e.vars[FLAGS] |= RUN;
        } else {
            e.vars[SUB] = 0x10;
            Self::start(e, self.wave_script);
            e.vars[STEP] = 0;
        }
    }
    /// `$33366`: run in, cut (the blade), step back, then jump.
    fn slash(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[FLAGS] & RUN != 0 {
            if cx.player_dx(e.x) >= self.run_min {
                return;
            }
            e.vars[FLAGS] &= !RUN;
            e.motion.vx = 0;
        }
        if Self::done(e) {
            Self::face(cx, e);
            e.vars[FLAGS] &= !DONE;
            e.vars[SUB] = 0x0c;
            Self::start(e, self.after_slash);
            return;
        }
        self.animate(e);
        if e.mapping == self.blade_frame && Self::ticks(e) == 1 && Self::slot_kind(e) == 0 {
            let (dx, rise, ..) = self.blade;
            let at = (e.x + Self::sign(e) * dx, e.y - rise);
            Self::open_slot(e, BLADE, at, false);
        }
        if e.mapping == self.step_frame {
            if Self::ticks(e) == self.step_start {
                e.motion.vx = -Self::sign(e) * self.step_vx;
            } else if Self::ticks(e) == self.step_stop {
                e.motion.vx = 0;
            }
        }
    }
    /// `$33408`: four volleys of three shurikens, then dash or jump.
    fn throw(&self, cx: &mut Ctx, e: &mut Enemy) {
        self.animate(e);
        if e.mapping == self.throw_frame && Self::ticks(e) == 1 {
            cx.effect_sound(self.throw_sound);
            for i in (0..=self.throw_count as usize).rev() {
                self.throw_star(cx, e, i.min(2));
            }
        }
        if !Self::done(e) {
            return;
        }
        Self::face(cx, e);
        if cx.random() & 1 == 0 {
            e.vars[SUB] = 8;
            e.vars[FLAGS] |= RUN;
            Self::start(e, self.dash_after_throw);
        } else {
            e.vars[SUB] = 0x0c;
            Self::start(e, self.jump_script);
        }
    }
    /// `$33482`: wind up, dash at 11 px/tick, brake inside 200 px.
    fn dash(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[FLAGS] & RUN != 0 {
            self.animate(e);
            if !Self::done(e) {
                if e.mapping == self.windup_frame && Self::ticks(e) == 1 {
                    self.voice(cx, self.windup_voice);
                }
                return;
            }
            Self::face(cx, e);
            e.vars[FLAGS] &= !RUN;
            Self::start(e, self.dash_script);
            e.motion.vx = Self::sign(e) * self.dash_vx;
        }
        if e.motion.ax == 0 {
            e.mapping = if e.vars[COUNT] & 1 != 0 {
                self.afterimage
            } else {
                0
            };
            if cx.player_dx(e.x) < self.brake_range {
                e.motion.ax = -Self::sign(e) * self.brake;
            } else {
                return;
            }
        }
        self.animate(e);
        if (e.motion.vx >> 16).abs() < 1 {
            e.motion.vx = 0;
            e.motion.ax = 1;
        }
        if e.mapping == self.dash_end {
            e.mapping = if e.motion.fx & 1 != 0 {
                self.afterimage
            } else {
                0
            };
        }
        if Self::done(e) {
            e.motion.vx = 0;
            e.motion.ax = 0;
            self.stand(e);
        }
    }
    /// `$33578`: retreating jump, the shuriken fan, landing.
    fn jump(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.motion.vy >> 16 >= 0 {
            cap_speed(e, MAX_SPEED);
        }
        self.animate(e);
        if Self::done(e) {
            if e.motion.gravity != 0 || e.motion.vy >> 16 != 0 {
                return;
            }
            e.vars[FLAGS] |= RESTART;
            e.motion.vx = 0;
            if e.vars[FLAGS] & LANDED == 0 {
                Self::face(cx, e);
                e.vars[FLAGS] |= LANDED;
                Self::start(e, self.landing_script);
                return;
            }
            e.vars[FLAGS] &= !LANDED;
            if e.vars[FLAGS] & MIJIN != 0 {
                e.routine = 0x18;
                e.vars[SUB] = 0;
            } else {
                self.stand(e);
            }
            return;
        }
        if e.vars[FLAGS] & LANDED != 0 {
            self.uppercut(cx, e);
            return;
        }
        if Self::ticks(e) != 1 {
            return;
        }
        if e.mapping == self.jump_frame {
            // Away from the player unless the arena wall is too close.
            let left = e.home_x;
            let keep = if e.facing_left {
                e.x - self.room <= left + self.margin
            } else {
                e.x + self.room <= left + self.far_side
            };
            e.motion.vx = if keep { self.jump_vx } else { -self.jump_vx };
            e.motion.vy = self.jump_vy;
            e.motion.gravity = self.gravity;
            cx.effect_sound(self.jump_sound);
        } else if e.mapping == self.jump2_frame {
            e.motion.vx = if e.motion.vx >> 16 >= 0 {
                self.jump2_vx
            } else {
                -self.jump2_vx
            };
            e.motion.vy = self.jump2_vy;
        } else if Self::cursor(e) * 2 == self.fan_cursor {
            cx.effect_sound(self.fan_sound);
            for i in (0..=self.fan_count as usize).rev() {
                self.fan_star(cx, e, i.min(7));
            }
        }
    }
    /// `$336F6` (second form): the uppercut with aura variant 1.
    fn uppercut_attack(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.motion.vy >> 16 >= 0 {
            cap_speed(e, MAX_SPEED);
        }
        self.animate(e);
        if Self::done(e) {
            e.vars[FLAGS] &= !GUARDLESS;
            if e.motion.gravity != 0 || e.motion.vy != 0 {
                return;
            }
            if e.vars[FLAGS] & RUN == 0 {
                Self::face(cx, e);
                e.vars[FLAGS] |= RUN;
                Self::start(e, self.landing_script);
                return;
            }
            e.vars[FLAGS] &= !(RUN | LANDED);
            self.stand(e);
            return;
        }
        if e.vars[FLAGS] & RUN == 0 && e.mapping == self.uppercut_frame && Self::ticks(e) == 1 {
            e.motion.vx = 0;
            e.motion.vy = self.uppercut_vy;
            e.motion.gravity = self.uppercut_gravity;
            cx.effect_sound(self.jump_sound);
            e.vars[FLAGS] |= AURA1;
        }
    }
    /// `$337AC` (second form): the spinner (`$1A` variant 0), then hold.
    fn spin(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.mapping == self.hold_frame {
            e.vars[COUNT] -= 1;
            if e.vars[COUNT] <= 0 {
                self.next_special(cx, e);
            }
            return;
        }
        self.animate(e);
        if !Self::done(e) {
            return;
        }
        if e.vars[FLAGS] & RUN != 0 {
            e.vars[FLAGS] &= !RUN;
            self.voice(cx, self.spin_voice);
            if Self::slot_kind(e) == 0 {
                let (dx, dy) = self.aura_offsets[0];
                let at = (e.x + if e.facing_left { -dx } else { dx }, e.y + dy);
                let flip = e.facing_left;
                Self::open_slot(e, SPINNER, at, flip);
            } else {
                e.vars[FLAGS] |= SIGNAL;
            }
        }
        if e.vars[FLAGS] & SIGNAL != 0 {
            e.vars[FLAGS] &= !SIGNAL;
            e.mapping = self.hold_frame;
            e.vars[COUNT] = self.hold;
        }
    }
    /// `$33804` (second form): the shadow wave, then the hold with variant 3.
    fn wave(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[STEP] == 0 {
            self.animate(e);
            if !Self::done(e) {
                if Self::ticks(e) == 1 && Self::slot_kind(e) == 0 {
                    e.vars[FLAGS] &= !SIGNAL;
                    let (at, flip) = ((e.x, e.y - self.wave.0), e.facing_left);
                    Self::open_slot(e, WAVE, at, flip);
                    cx.effect_sound(self.wave_sound);
                }
                return;
            }
            e.mapping = self.hold_frame;
            if e.vars[FLAGS] & SIGNAL != 0 {
                e.vars[FLAGS] &= !SIGNAL;
                e.vars[STEP] = 4;
                Self::start(e, self.wave_end_script);
                cx.effect_sound(self.wave_hit_sound);
                e.vars[FLAGS] |= AURA3;
            }
            return;
        }
        self.animate(e);
        if Self::done(e) {
            if e.vars[FLAGS] & SIGNAL != 0 || Self::slot_kind(e) == 0 {
                e.vars[FLAGS] &= !SIGNAL;
                self.next_special(cx, e);
            }
        } else if e.mapping == self.wave_hold {
            self.uppercut(cx, e);
        }
    }
    /// `$338AE`: jump and hover; then the energy rain, or death.
    fn hover(&self, cx: &mut Ctx, e: &mut Enemy) {
        match e.vars[STEP] {
            0 => {
                self.animate(e);
                if Self::done(e) {
                    e.vars[STEP] = 4;
                    e.motion.vy = 0;
                    e.motion.gravity = 0;
                    e.vars[COUNT] = self.hover;
                    if e.vars[FLAGS] & DYING == 0 {
                        cx.effect_sound(self.hover_sound);
                        e.flash = if e.mapping == self.guard[0] {
                            self.flash.1
                        } else {
                            self.flash.0
                        };
                    }
                } else if e.mapping == self.jump_frame && Self::ticks(e) == 1 {
                    e.motion.vy = self.hover_vy;
                    e.motion.gravity = self.gravity;
                    cx.effect_sound(self.jump_sound);
                }
            }
            4 => {
                e.vars[COUNT] -= 1;
                if e.vars[COUNT] > 0 {
                    return;
                }
                e.mapping += 1;
                if e.vars[FLAGS] & DYING != 0 {
                    e.routine = 0x24;
                    e.vars[COUNT] = self.death_step;
                    e.phase = 0;
                    self.voice(cx, self.death_voice);
                    // `$3395E`: stop every sound after BOSS DEATH is queued.
                    cx.play(0xff);
                } else {
                    e.vars[STEP] = 8;
                    e.counter = 0;
                    e.vars[COUNT] = self.rain_interval;
                    e.motion.vy = self.rain_rise;
                }
            }
            8 => {
                e.vars[COUNT] -= 1;
                if e.vars[COUNT] > 0 {
                    return;
                }
                e.vars[COUNT] = self.rain_interval;
                match self.rain.get(usize::from(e.counter)) {
                    Some(&index) => {
                        e.counter += 1;
                        self.energy(cx, (e.x, e.y), index, true);
                        e.phase = e.phase.wrapping_add(1);
                        if e.phase & 7 == 4 {
                            cx.effect_sound(self.rain_sound);
                        }
                    }
                    None => {
                        e.vars[STEP] = 0x0c;
                        e.motion.vy = 0;
                        e.motion.gravity = self.gravity;
                        e.mapping = self.first;
                        Self::start(e, self.landing_script);
                    }
                }
            }
            _ => self.land(cx, e),
        }
    }
    /// `loc_033A3C`: land, finish the script, stand.
    fn land(&self, cx: &mut Ctx, e: &mut Enemy) {
        cap_speed(e, MAX_SPEED);
        if e.motion.gravity != 0 || e.motion.vy != 0 {
            return;
        }
        self.animate(e);
        if Self::done(e) {
            self.stand(e);
        } else {
            self.uppercut(cx, e);
        }
    }
    /// `$33A78` (second form): the diving leap with aura variant 2.
    fn leap(&self, cx: &mut Ctx, e: &mut Enemy) {
        match e.vars[STEP] {
            0 => {
                self.animate(e);
                if Self::done(e) {
                    e.vars[STEP] = 4;
                } else if e.mapping == self.jump_frame && Self::ticks(e) == 1 {
                    Self::face(cx, e);
                    let d = cx.player_dx(e.x);
                    let (_, count, vx, vy) = *self
                        .leap
                        .iter()
                        .find(|leap| d >= leap.0)
                        .unwrap_or(&self.leap[2]);
                    e.vars[COUNT] = count;
                    e.motion.vx = Self::sign(e) * vx;
                    e.motion.gravity = self.gravity;
                    e.motion.vy = vy;
                    cx.effect_sound(self.jump_sound);
                }
            }
            4 => {
                if e.vars[COUNT] != 0 {
                    e.vars[COUNT] -= 1;
                    if e.vars[COUNT] == 0 {
                        Self::start(e, self.dive_script);
                    }
                    return;
                }
                self.animate(e);
                if Self::done(e) {
                    e.vars[STEP] = 8;
                    e.motion.vx = Self::sign(e) * self.dive_vx;
                    e.motion.vy = self.dive_vy;
                    e.vars[FLAGS] |= AURA2;
                }
            }
            8 => {
                cap_speed(e, MAX_SPEED);
                if e.vars[FLAGS] & SIGNAL != 0 {
                    e.vars[FLAGS] &= !SIGNAL;
                    e.vars[STEP] = 0x0c;
                    Self::start(e, self.bounce_script);
                    e.motion.vx = -Self::sign(e) * self.bounce_vx;
                    e.motion.vy = self.bounce_vy;
                    e.motion.gravity = self.gravity;
                } else if e.motion.vy >> 16 == 0 {
                    e.vars[STEP] = 0x10;
                    Self::start(e, self.landing_script);
                    e.motion.vx = 0;
                    Self::face(cx, e);
                    self.animate(e);
                }
            }
            0x0c => {
                self.animate(e);
                if Self::done(e) && e.motion.gravity == 0 && e.motion.vy == 0 {
                    e.vars[STEP] = 0x10;
                    Self::start(e, self.landing_script);
                    e.motion.vx = 0;
                    Self::face(cx, e);
                }
            }
            _ => {
                self.animate(e);
                if Self::done(e) {
                    self.stand(e);
                } else {
                    self.uppercut(cx, e);
                }
            }
        }
    }
    /// `$33344`: the attack of `+$2A`.
    pub(super) fn attack(&self, cx: &mut Ctx, e: &mut Enemy) {
        let second = e.vars[FLAGS] & PHASE2 != 0;
        match (second, e.vars[SUB]) {
            (_, 0) => self.slash(cx, e),
            (false, 4) => self.throw(cx, e),
            (false, 8) => self.dash(cx, e),
            (_, 0x0c) => self.jump(cx, e),
            (true, 4) => self.uppercut_attack(cx, e),
            (true, 8) => self.spin(cx, e),
            (true, 0x10) => self.wave(cx, e),
            (_, 0x14) => self.hover(cx, e),
            (true, 0x18) => self.leap(cx, e),
            _ => self.stand(e),
        }
    }
}
