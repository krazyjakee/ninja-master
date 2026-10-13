//! The routines around the fight: the entrance (0, 4, 8), the parry pose
//! (`$14`), MIJIN (`$18`), the knock-back (`$20`) and the ending (`$24`, `$2C`).
use super::*;
use crate::bosses::fade_white_lines;

impl Boss7 {
    /// $343A8 forces event 5 except for the death/shock classes in $344EC.
    fn hold_player(cx: &mut Ctx) {
        if !matches!(cx.player_state, Some(41 | 42 | 43 | 46)) {
            cx.player_event(crate::movement::PlayerEvent::Idle);
        }
    }

    /// `$33160`
    pub(super) fn init(&self, cx: &mut Ctx, e: &mut Enemy) {
        Self::hold_player(cx);
        e.vars = [0; crate::enemies::VARS];
        e.home_x = cx.camera.0 + self.arena;
        e.y = cx.camera.1;
        e.motion = Motion {
            gravity: self.fall_gravity,
            ..Motion::default()
        };
        Self::start(e, self.intro_script);
        e.mapping = self.first;
        e.vars[HEALTH] = self.health;
        e.health = POOL;
        e.step_x = 0;
        e.counter = 0;
        e.phase = 0;
        e.timer = 0;
        e.body = None;
        e.launch_y = 0;
        e.routine = 4;
        cx.palette_op(PaletteOp::Load(1, self.first_palette));
    }
    /// `$331D6`: fall, then the landing script.
    pub(super) fn fall(&self, cx: &mut Ctx, e: &mut Enemy) {
        Self::hold_player(cx);
        cap_speed(e, MAX_SPEED);
        if e.motion.vy != 0 {
            return;
        }
        self.animate(e);
        if Self::done(e) {
            e.routine = 8;
            e.vars[SUB] = 0;
            e.vars[COUNT] = 0;
            e.timer = 0;
            e.counter = 0;
            e.phase = 0;
            cx.effect_sound(self.intro_sound);
        }
    }
    /// `$33212` waiting for the intro object `$1F970`, run here.
    pub(super) fn intro(&self, cx: &mut Ctx, e: &mut Enemy) {
        Self::hold_player(cx);
        cx.plane_op(crate::combat::PlaneOp::FinalBossScroll {
            mode: if e.vars[SUB] < 2 { 3 } else { 7 },
            amplitude: e.timer,
            phase: e.counter,
        });
        e.counter = e.counter.wrapping_add(0x20);
        match e.vars[SUB] {
            0 => {
                // $1F9B2: VDP $0B = 3, the wave amplitude grows to $7F00.
                e.timer = e.timer.saturating_add(0x100).min(0x7f00);
                if e.timer >= 0x7f00 {
                    e.vars[SUB] = 1;
                    e.vars[COUNT] = 0;
                }
            }
            1 => {
                // $1FA04: the amplitude swings $5000..$7F00; lines 2-3 fade
                // one step every 8 ticks.
                if e.phase == 0 {
                    e.timer -= 0x100;
                    if e.timer < 0x5000 {
                        e.phase = 1;
                    }
                } else {
                    e.timer += 0x100;
                    if e.timer >= 0x7f00 {
                        e.phase = 0;
                    }
                }
                e.vars[COUNT] += 1;
                if e.vars[COUNT] >= 8 {
                    e.vars[COUNT] = 0;
                    let fade = Self::present(e, P_INTRO);
                    if fade < 15 {
                        Self::set_present(e, P_INTRO, fade + 1);
                        cx.palette_op(PaletteOp::FadeTo(2, [0; 16]));
                        cx.palette_op(PaletteOp::FadeTo(3, [0; 16]));
                    } else {
                        e.vars[SUB] = 2;
                    }
                }
            }
            2 => {
                // $1FA7E: Boss_LoadArt(round + 1), VDP $0B = 7.
                cx.plane_op(crate::combat::PlaneOp::BossArt(7));
                e.timer = 0;
                e.vars[SUB] = 3;
                e.vars[COUNT] = 0;
            }
            _ => {
                // $1FAD2: fade back, then release the boss ($1FB06).
                e.vars[COUNT] += 1;
                if e.vars[COUNT] >= 8 {
                    e.vars[COUNT] = 0;
                    let fade = Self::present(e, P_INTRO);
                    if fade > 0 {
                        Self::set_present(e, P_INTRO, fade - 1);
                        cx.palette_op(PaletteOp::Restore(2));
                        cx.palette_op(PaletteOp::Restore(3));
                    } else {
                        e.vars[SUB] = 0;
                        self.stand(e);
                    }
                }
            }
        }
    }
    /// `$33C50`: the parry pose; a broken guard plays `$2FA/$2FB`.
    pub(super) fn parry_pose(&self, cx: &mut Ctx, e: &mut Enemy) {
        self.animate(e);
        if e.motion.vx >> 16 != 0 {
            if cx.player_state == Some(crate::movement::state::DASH_KICK) {
                return;
            }
            e.motion.vx = 0;
        } else if e.vars[FLAGS] & BROKEN != 0 && !Self::done(e) {
            // $33C8A sets body-contact bit 14 with zero damage: recoil,
            // not PlayerState_Thrown (the audit originally called it a throw).
            if e.mapping != self.broken_frame && Self::ticks(e) == 10 {
                cx.contact_player(0);
            }
            if e.mapping == self.broken_frame && Self::ticks(e) == 1 {
                self.voice(cx, self.broken_cry);
            }
            return;
        }
        if Self::done(e) {
            e.vars[FLAGS] &= !BROKEN;
            Self::set_misc(e, M_PARRIES, 0);
            self.stand(e);
        }
    }
    /// $33F02: wait for a grounded ordinary pose, then reappear behind Joe.
    pub(super) fn teleport(&self, cx: &mut Ctx, e: &mut Enemy) {
        Self::face(cx, e);
        if e.vars[SUB] == 0 {
            if cx.player_state.is_some_and(|state| state >= 0x40) || Self::airborne(cx, e) {
                return;
            }
            e.motion.stop();
            e.vars[SLOT] = 0;
            e.vars[FLAGS] &= SECOND | PHASE2;
            e.vars[SUB] = 4;
            e.y = cx.player.1;
            e.x = (cx.player.0 + if cx.player_facing_left { 40 } else { -40 })
                .clamp(cx.camera.0 + 32, cx.camera.0 + 288);
            Self::start(e, 0x10);
            return;
        }
        self.animate(e);
        if Self::done(e) {
            e.routine = 0x10;
            e.vars[SUB] = 0;
            Self::start(e, self.slash_script);
            self.run(cx, e);
            self.voice(cx, self.run_voice);
        } else if Self::ticks(e) >= 20 && Self::ticks(e) & 1 != 0 {
            e.mapping = 0;
        }
    }
    /// $3463A runs even when no weapon was recorded this frame.
    pub(super) fn special_reaction(&self, cx: &Ctx, e: &mut Enemy, terrain: &dyn Terrain) {
        if !matches!(e.routine, 0x0c | 0x10 | 0x14)
            || e.vars[FLAGS] & MIJIN != 0
            || !cx
                .player_state
                .is_some_and(|state| state == 0x41 || state >= 0x43)
        {
            return;
        }
        if terrain.object_solid_at(e.x, e.y, e.high_priority) {
            e.routine = 0x1c;
            e.vars[SUB] = 0;
            Self::start(e, 0x10);
            e.mapping = 0;
        } else {
            e.vars[HEALTH] -= 8;
            self.knock(cx, e);
        }
    }
    /// $343BA / $1F520: a two-pixel shove, with the leading foot blocked
    /// by solid terrain. State 29 also retains the recent-hit timer.
    pub(super) fn shove(&self, cx: &mut Ctx, e: &mut Enemy, terrain: &dyn Terrain) {
        let state = cx.player_state.unwrap_or(1);
        let mut shove = self.player_classes.get(state) == Some(&1);
        if e.vars[SHOVE_TIMER] > 0 && e.flash == 0 {
            if state == 29 {
                if cx.player_dx(e.x) < 128 {
                    e.vars[SHOVE_TIMER] = 1;
                    shove = true;
                } else {
                    e.vars[SHOVE_TIMER] = 0;
                }
            } else {
                e.vars[SHOVE_TIMER] -= 1;
            }
        }
        if !shove || cx.player_dx(e.x) >= 112 {
            return;
        }
        let dx: i32 = if cx.player_facing_left { 2 } else { -2 };
        let width = cx.player_body.map_or(15, |body| body.half_width);
        if !terrain.object_solid_at(
            cx.player_at.0 + width * dx.signum() + dx,
            cx.player_at.1 - 1,
            cx.current_high_priority(),
        ) {
            cx.signals.push(crate::combat::Signal::PushPlayer(dx));
        }
    }
    /// `$33CD6`: MIJIN.
    pub(super) fn mijin(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[SUB] != 0 {
            Self::hold_player(cx);
        }
        match e.vars[SUB] {
            0 => {
                if e.vars[FLAGS] & RETREAT == 0 && cx.player_dx(e.x) < self.retreat {
                    // Jump clear first; the jump returns here.
                    Self::face(cx, e);
                    e.routine = 0x10;
                    e.vars[FLAGS] |= RETREAT;
                    e.vars[FLAGS] &= !DONE;
                    e.vars[SUB] = 0x0c;
                    Self::start(e, self.after_slash);
                    return;
                }
                if Self::airborne(cx, e) {
                    return;
                }
                Self::hold_player(cx);
                e.vars[SUB] = 4;
                e.mapping = self.mijin_frame;
                self.start_pillar(cx, e);
                Self::face(cx, e);
                e.vars[FLAGS] &= !(RETREAT | MIJIN);
                e.vars[COUNT] = self.mijin_step;
                cx.effect_sound(self.mijin_sound);
            }
            4 => {
                // $33D8C: lines 1-3 one step whiter every 7 ticks.
                e.vars[COUNT] -= 1;
                if e.vars[COUNT] > 0 {
                    return;
                }
                e.vars[COUNT] = self.mijin_step;
                let white = Self::present(e, P_WHITE);
                if white < 15 {
                    Self::set_present(e, P_WHITE, white + 1);
                    fade_white_lines(cx, 1..=3);
                } else {
                    // Data_036560 row 0 from now on.
                    e.vars[SUB] = 8;
                }
            }
            8 => {
                // $33E14: back to Data_036560 row 0 and the stage palette.
                e.vars[COUNT] -= 1;
                if e.vars[COUNT] > 0 {
                    return;
                }
                e.vars[COUNT] = self.mijin_step;
                let white = Self::present(e, P_WHITE);
                if white > 0 {
                    Self::set_present(e, P_WHITE, white - 1);
                    cx.palette_op(PaletteOp::FadeTo(1, self.second_palette));
                    cx.palette_op(PaletteOp::Restore(2));
                    cx.palette_op(PaletteOp::Restore(3));
                } else {
                    e.vars[SUB] = 0x0c;
                    e.mapping = self.mijin_after;
                    Self::release_pillar(cx);
                    e.vars[FLAGS] &= !SIGNAL;
                    let at = (e.x, e.y);
                    Self::open_slot(e, AURA, at, false);
                    cx.effect_sound(self.aura_sound);
                }
            }
            _ => {
                // $33EB6: the aura's end starts the second form.
                if e.vars[FLAGS] & SIGNAL == 0 {
                    return;
                }
                e.vars[FLAGS] &= !(SIGNAL | BROKEN);
                e.vars[FLAGS] |= PHASE2;
                Self::set_misc(e, M_PARRIES, 0);
                // $1FBB8: Boss_LoadArt(round + 2).
                cx.plane_op(crate::combat::PlaneOp::BossArt(8));
                self.stand(e);
            }
        }
    }
    /// `$33FC2`: knocked back; on landing MIJIN, death or stand.
    pub(super) fn knocked(&self, cx: &Ctx, e: &mut Enemy) {
        if e.vars[COUNT] != 0 {
            e.vars[COUNT] -= 1;
            return;
        }
        e.mapping = self.hurt_frame;
        cap_speed(e, MAX_SPEED);
        if e.motion.gravity != 0 || e.motion.vy != 0 {
            return;
        }
        // Func_034536
        e.vars[SUB] = 0;
        e.vars[STEP] = 0;
        Self::set_misc(e, M_PARRIES, 0);
        e.vars[FLAGS] &= !(SIGNAL | LANDED | BROKEN | RUN | MIJIN | DYING | RETREAT);
        e.motion.stop();
        Self::face(cx, e);
        if e.vars[FLAGS] & SECOND == 0 {
            if e.vars[HEALTH] < 0 {
                e.routine = 0x18;
                e.vars[FLAGS] |= MIJIN | SECOND;
                e.vars[HEALTH] = self.second_health;
                e.health = self.second_health as u16 + 1;
                e.mapping = self.base;
                return;
            }
        } else if e.vars[HEALTH] < 0 {
            e.routine = 0x10;
            e.vars[SUB] = 0x14;
            e.vars[FLAGS] |= DYING;
            Self::start(e, self.death_script);
            return;
        }
        self.stand(e);
    }
    /// `$34072`: the final white-out with explosions everywhere.
    pub(super) fn fade_out(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.game_flag(0x1002);
        cx.player_event(crate::movement::PlayerEvent::Freeze);
        e.vars[COUNT] -= 1;
        if e.vars[COUNT] <= 0 {
            e.vars[COUNT] = self.fade_step;
            let white = Self::present(e, P_WHITE);
            if white < 15 {
                Self::set_present(e, P_WHITE, white + 1);
                fade_white_lines(cx, 1..=3);
            } else {
                e.routine = 0x2c;
                e.vars[COUNT] = 0;
                cx.effect_sound(self.end_sound);
                return;
            }
        }
        // List-1 $39: an explosion at a random point of the screen.
        let x = cx.camera.0 + i32::from(cx.random() % 320);
        let y = cx.camera.1 + i32::from(cx.random() % 224);
        animate(
            cx,
            (x, y),
            Motion::default(),
            0x2000,
            0,
            self.script(self.boom_script),
            false,
        );
        e.phase = e.phase.wrapping_add(1);
        if e.phase == 8 {
            e.phase = 0;
            cx.effect_sound(self.boom_sound);
        }
    }
    /// `$34140`: 128 ticks, then `Game_Requests` bit 1.
    pub(super) fn finish(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.game_flag(0x1000);
        cx.player_event(crate::movement::PlayerEvent::Freeze);
        e.vars[COUNT] += 1;
        if e.vars[COUNT] & 0x80 != 0 {
            cx.next_substage();
            Ctx::remove(e);
            e.mapping = 0;
        }
    }
}
