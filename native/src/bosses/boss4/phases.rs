//! Gunboat, flying-machine and defeat phase transitions.
use super::*;

impl Boss4 {
    pub(super) fn routine(&self, cx: &mut Ctx, e: &mut Enemy) {
        loop {
            match e.routine {
                0 => {
                    // $2DBB4: the gunboat surges up out of the water.
                    Self::draw_hull(cx, GUNBOAT_MAP);
                    self.build_parts(cx, e, 0);
                    // ori.w #$800 on the second part: the ram stays stowed.
                    if let Some(p) = part(cx.effects, RAM_PART) {
                        p.flags |= parts::HIDDEN;
                    }
                    e.phase = GUNBOAT;
                    e.step_x = self.health[0];
                    (e.home_x, e.launch_y) = self.starts[0];
                    Self::set_waves(e, &self.waves[0]);
                    e.vars[T] = self.rise_ticks;
                    e.vars[S] = 0;
                    e.routine = 4;
                    cx.effect_sound(self.intro_sound);
                    return;
                }
                4 => {
                    // $2DCBE
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.launch_y = e.y;
                        Self::set_wave(e, Y_WAVE, 8, 8);
                        e.vars[Y_ANGLE] = 0;
                        e.routine = 8;
                    } else if e.vars[S] == 0 && e.vars[Y_ANGLE] >= 0x600 {
                        e.launch_y = e.y + (e.launch_y - e.y) / 2;
                        Self::set_wave(e, Y_WAVE, 0x0c, 0x30);
                        e.vars[S] = 1;
                    }
                    return;
                }
                8 => {
                    // $2DD18: in to X $140.
                    if e.home_x >= 0x140 {
                        e.routine = 0x0c;
                        e.vars[T] = 0x400;
                        e.vars[S] = 1;
                    } else {
                        e.home_x += 2;
                    }
                    return;
                }
                0x0c => {
                    // $2DD38: two volleys of three shells, sinking between.
                    if e.vars[S] != 0 {
                        e.vars[T] -= 0x10;
                        if e.vars[T] != 0 {
                            if e.vars[T] & 0xff == 0 {
                                self.cannon(cx, e);
                            }
                            return;
                        }
                        e.vars[T] = 0x20;
                        if e.vars[S] < 0 {
                            e.routine = 0x10;
                            return;
                        }
                        e.vars[S] = 0;
                    }
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x400;
                        e.vars[S] = -1;
                    }
                    e.launch_y += 1;
                    return;
                }
                0x10 => {
                    // $2DDFE
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x210;
                        e.routine = 0x14;
                    }
                    e.home_x -= 2;
                    e.launch_y -= 1;
                    return;
                }
                0x14 => {
                    // $2DE18: three homing missiles, then lunge or return.
                    if e.vars[T] == 0 {
                        if Self::list4_empty(cx) {
                            e.routine = if cx.random() & 1 != 0 {
                                Self::set_wave(e, X_WAVE, 0, 0x20);
                                e.vars[X_ANGLE] = 0;
                                cx.effect_sound(self.sounds.lunge);
                                0x18
                            } else {
                                8
                            };
                            e.vars[T] = 0x200;
                        }
                        return;
                    }
                    e.vars[T] -= 8;
                    if e.vars[T] & 0xff == 0 {
                        self.launch_missile(cx, e, 0, (e.vars[T] >> 8) as usize);
                    }
                    return;
                }
                0x18 => {
                    // $2DEB4: sway, then the long lunge.
                    if e.vars[T] >> 8 >= 2 {
                        e.vars[X_ANGLE] += 4;
                        if e.vars[X_ANGLE] >= 0x200 {
                            Self::set_wave(e, X_WAVE, 0, 0x5f);
                            e.vars[X_ANGLE] = 0x200;
                            e.home_x -= 0x7e;
                            e.vars[T] = 0x100;
                        }
                        return;
                    }
                    e.home_x += if e.vars[X_ANGLE] < 0x400 { -2 } else { 2 };
                    e.vars[X_ANGLE] += 0x10;
                    if e.vars[X_ANGLE] >= 0x780 {
                        e.home_x = e.x;
                        // move.l #0,$44: the sway stops.
                        Self::set_wave(e, X_WAVE, 0, 0);
                        e.vars[X_ANGLE] = 0;
                        e.routine = 0x20;
                    }
                    return;
                }
                0x1c => e.routine = 0x20,
                0x20 => {
                    // $2DF26: to X $150, then the charge.
                    if Self::toward(&mut e.home_x, 0x150, 2) {
                        self.charge_colors(cx);
                        e.vars[T] = 0x80;
                        e.routine = 0x24;
                        cx.effect_sound(self.sounds.surge);
                    }
                    return;
                }
                0x24 => {
                    // $2DF86: the ram slides out of the bow while the steam
                    // runs, and the bow glows at its end.
                    if let Some(p) = part(cx.effects, RAM_PART) {
                        p.flags &= !parts::HIDDEN;
                        step_toward(&mut p.origin.0, self.parts.ram.0);
                    }
                    e.vars[T] -= 1;
                    let t = e.vars[T];
                    if t == 0 {
                        self.charge_end(cx, e);
                        e.vars[T] = 0x28;
                        e.routine = 0x28;
                    } else if self.steam(cx, e, t) {
                        return;
                    } else if t < 0x40 && (t == 8 || t == 4) {
                        let glow = self.parts.glow[usize::from(t == 4)];
                        if let Some(p) = part(cx.effects, GLOW) {
                            p.mapping = glow;
                            if t == 8 {
                                p.origin.1 = self.parts.glow_y.0;
                            }
                        }
                    }
                    return;
                }
                0x28 => {
                    // $2E106: the surge, then the ram is stowed again and
                    // either loop follows.
                    if e.vars[T] != 0 {
                        e.vars[T] -= 1;
                        self.charge_fade(cx, e.vars[T]);
                        if e.vars[T] != 0 {
                            if e.vars[T] & 1 == 0 {
                                e.home_x += 1;
                            }
                            return;
                        }
                    }
                    if let Some(p) = part(cx.effects, RAM_PART) {
                        if step_toward(&mut p.origin.0, self.parts.ram.1) {
                            return;
                        }
                        p.flags |= parts::HIDDEN;
                    }
                    e.routine = if cx.random() & 1 != 0 { 8 } else { 0x20 };
                    return;
                }
                0x2c => {
                    // $2E1EA: sinking in smoke and debris, the player held
                    // (`move.b #6,$28(a3)`).
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[T] -= 1;
                    let t = e.vars[T];
                    if t > 0 {
                        if (e.y as u16) < 0x1f0 && t & 1 == 0 {
                            self.smoke(cx, e, 0);
                            if t & 0xf == 0 {
                                cx.effect_sound(self.sounds.sinking);
                            }
                        }
                        if t & 7 == 0 {
                            self.debris(cx, e);
                        }
                        if t & 2 == 0 {
                            e.launch_y += 1;
                            // move.b ($FF001C),d0 / andi.w #3,d0: read only.
                            e.home_x ^= i32::from((*cx.random >> 8) & 3);
                        }
                        // $2E312: plumes of water where it goes under, each
                        // with smoke $80 above.
                        if let Some(at) = self.plumes.at(t, (e.x, e.y), *cx.random) {
                            cx.effect_sound(self.plumes.sound);
                            let water = -crate::special::WATER_FLOW << 16;
                            cx.effect(ambient::plume(&cx.shared.ambient, at, water));
                            self.smoke(cx, e, -self.sinking.plume_smoke);
                        }
                        return;
                    }
                    // $2E202: the flying machine, its hull fire lit and
                    // palette line 3 white for the parts' hit flash.
                    Self::draw_hull(cx, FLYER_MAP);
                    self.build_parts(cx, e, 1);
                    Self::light_last(cx);
                    Self::clear_missiles(cx);
                    e.phase = PART_C;
                    e.step_x = self.parts.health;
                    (e.home_x, e.launch_y) = self.starts[1];
                    Self::set_waves(e, &self.waves[1]);
                    e.vars[T] = 0;
                    e.vars[S] = 0x50;
                    e.routine = 0x30;
                    cx.effect_sound(self.intro_sound);
                    cx.fill_palette(3, 0x0eee);
                    return;
                }
                0x30 => {
                    // $2E3C0 with +$28 as the sub-state.
                    match e.vars[T] {
                        0 => {
                            // $2E3E8: rising, then the first part is pulled
                            // up out of the sea.
                            e.vars[S] -= 1;
                            if e.vars[S] == 0 {
                                self.start_rise(cx, e, 0);
                                e.vars[T] = 4;
                            }
                            e.launch_y -= 2;
                        }
                        4 | 8 => {
                            // $2E41A / $2E484: bobbing with the rising part.
                            let n = usize::from(e.vars[T] == 8);
                            let sub = parts::part_ref(cx.effects, n).map_or(0, |p| p.angle);
                            if sub == 8 && e.counter & 3 == 0 {
                                e.launch_y -= 1;
                            }
                            if sub == 0x0c && e.counter & 1 == 0 {
                                e.launch_y += 1;
                            }
                            if sub == 0 {
                                cx.play(self.voice);
                                if n == 0 {
                                    self.start_rise(cx, e, 1);
                                }
                                e.vars[T] += 4;
                            }
                        }
                        0x0c => {
                            // $2E4D0: arm and gun into place, then they
                            // take their roles.
                            if !Self::approach(cx, &self.parts.poses.intro[..2]) {
                                if let Some(p) = part(cx.effects, ARM_PART) {
                                    p.phase = parts::ARM;
                                    p.counter = self.arm_health;
                                }
                                if let Some(p) = part(cx.effects, GUN_PART) {
                                    p.phase = parts::GUN;
                                }
                                e.vars[T] = 0x10;
                                cx.effect_sound(self.handover);
                            }
                        }
                        0x10 => {
                            // $2E50C
                            let intro = &self.parts.poses.intro;
                            if !Self::approach(cx, &intro[..3]) && !Self::approach(cx, intro) {
                                e.vars[T] = 0x14;
                            }
                        }
                        0x14 => {
                            // $2E53A: the sway widens and it settles at Y $140.
                            let mut moving = false;
                            let x_amp = Self::amp(e, X_WAVE);
                            if x_amp != 8 {
                                Self::set_amp(e, X_WAVE, x_amp + 1);
                                moving = true;
                            }
                            let y_amp = Self::amp(e, Y_WAVE);
                            if y_amp != 0x20 {
                                Self::set_amp(e, Y_WAVE, y_amp + 1);
                                moving = true;
                            }
                            if e.launch_y != 0x140 {
                                e.launch_y -= 1;
                                moving = true;
                            }
                            if !moving {
                                e.vars[T] = 0x18;
                            }
                        }
                        // $2E570: the attack starts on the next update.
                        _ => Self::choose(cx, e),
                    }
                    return;
                }
                0x34 => {
                    // $2E5B0: arm and gun aim while it settles to Y $150.
                    if !Self::list4_empty(cx) {
                        return;
                    }
                    if !Self::approach(cx, &self.parts.poses.aim) {
                        cx.play(self.voice);
                        e.vars[T] = 0x220;
                        e.routine = 0x38;
                    }
                    Self::double_step(&mut e.launch_y, 0x150);
                    return;
                }
                0x38 => {
                    // $2E600: shells at +$29 = $1C and $10 of each $A0,
                    // the arm and gun swinging between their poses.
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x40;
                        e.routine = 0x3c;
                        return;
                    }
                    let low = e.vars[T] & 0xff;
                    if low == 0 {
                        e.vars[T] = (e.vars[T] - 0x100) | 0xa0;
                    }
                    if low == 0x10 || low == 0x1c {
                        // $2E646: only from the gun at rest.
                        let Some(gun) = part(cx.effects, GUN_PART).filter(|p| p.angle == 4) else {
                            e.vars[T] = 0x40;
                            e.routine = 0x3c;
                            return;
                        };
                        gun.origin.0 += self.parts.gun_recoil.0;
                        gun.origin.1 += self.parts.gun_recoil.1;
                        self.shell2(cx, e);
                        return;
                    }
                    let pose = if (0x40..0xc0).contains(&low) {
                        &self.parts.poses.rest[0]
                    } else {
                        &self.parts.poses.aim
                    };
                    Self::approach(cx, pose);
                    return;
                }
                0x3c => {
                    // $2E6C8
                    let gun_down = usize::from(destroyed(cx.effects, GUN_PART));
                    if !Self::approach(cx, &self.parts.poses.rest[gun_down]) {
                        e.vars[T] -= 1;
                        if e.vars[T] == 0 {
                            e.vars[T] = 0x710;
                            e.routine = 0x40;
                        }
                    }
                    if Self::amp(e, X_WAVE) != 0 {
                        Self::set_amp(e, X_WAVE, Self::amp(e, X_WAVE) - 1);
                        Self::set_amp(e, Y_WAVE, Self::amp(e, Y_WAVE) - 1);
                    }
                    Self::double_step(&mut e.launch_y, 0x160);
                    return;
                }
                0x40 => {
                    // $2E732
                    self.salvo(cx, e, 1, 0x44);
                    return;
                }
                0x44 => {
                    // $2E7B2: down to the water and across to the left.
                    if !Self::list4_empty(cx) {
                        return;
                    }
                    if e.launch_y < 0x1c0 {
                        e.launch_y += 1;
                        return;
                    }
                    let gun_down = usize::from(destroyed(cx.effects, GUN_PART));
                    if Self::approach(cx, &self.parts.poses.low[gun_down]) {
                        return;
                    }
                    if e.home_x > 0x40 {
                        e.home_x -= 3;
                        return;
                    }
                    let y_amp = Self::amp(e, Y_WAVE);
                    if y_amp != 0x20 {
                        Self::set_amp(e, Y_WAVE, y_amp + (0x20 - y_amp).signum());
                        return;
                    }
                    Self::set_wave(e, Y_WAVE, 8, 0x20);
                    e.vars[T] = 0x710;
                    e.routine = 0x48;
                    return;
                }
                0x48 | 0x4c => e.routine = 0x50,
                0x50 => {
                    // $2E842
                    self.salvo(cx, e, 2, 0x5c);
                    return;
                }
                0x54 => e.routine = 0x58,
                0x58 => {
                    // $2E8C4: back to ($100, $130), dropping bombs on the
                    // way (`bne Func_02E902`).
                    let x = Self::toward(&mut e.home_x, 0x100, 1);
                    let y = Self::toward(&mut e.launch_y, 0x130, 1);
                    if x && y {
                        e.routine = 0x44;
                    } else {
                        self.bombs(cx, e);
                    }
                    return;
                }
                0x5c => {
                    // $2E988: up, then back to X $100 dropping bombs.
                    let gun_down = usize::from(destroyed(cx.effects, GUN_PART));
                    if Self::approach(cx, &self.parts.poses.rest[gun_down]) {
                        return;
                    }
                    if e.launch_y > 0x140 {
                        e.launch_y -= 2;
                    } else if e.home_x < 0x100 {
                        e.home_x += 2;
                        self.bombs(cx, e);
                    } else {
                        e.vars[T] = 0x20;
                        e.routine = 0x7c;
                    }
                    return;
                }
                0x60 => e.routine = 0x64,
                0x64 => {
                    // $2E9F6: to X $130 and Y $140, the arm drawn in.
                    let mut moving = false;
                    if Self::amp(e, X_WAVE) != 0 {
                        Self::set_amp(e, X_WAVE, Self::amp(e, X_WAVE) - 1);
                        moving = true;
                    }
                    if Self::amp(e, Y_WAVE) >= 0x10 {
                        Self::set_amp(e, Y_WAVE, Self::amp(e, Y_WAVE) - 1);
                        moving = true;
                    }
                    if e.home_x != 0x130 {
                        e.home_x += 1;
                        moving = true;
                    }
                    if e.launch_y != 0x140 {
                        e.launch_y -= 1;
                        moving = true;
                    }
                    if moving
                        || (!destroyed(cx.effects, ARM_PART)
                            && Self::approach(cx, &self.parts.poses.arm[0]))
                    {
                        return;
                    }
                    e.vars[T] = 0x140;
                    e.routine = 0x68;
                    return;
                }
                0x68 => {
                    // $2EA5A: at +$29 = 0 the arm reaches out (effect $3C).
                    e.vars[T] -= 1;
                    if destroyed(cx.effects, ARM_PART) || e.vars[T] == 0 {
                        e.vars[T] = 0x80;
                        e.routine = 0x6c;
                        return;
                    }
                    let low = e.vars[T] & 0xff;
                    if low == 0
                        && let Some(arm) = part(cx.effects, ARM_PART).filter(|p| p.angle == 0)
                    {
                        arm.origin = self.parts.poses.arm[1][0];
                        arm.angle = 4;
                        cx.effect_sound(self.parts.reach_sound);
                    }
                    if low < 0x20 {
                        Self::approach(cx, &self.parts.poses.arm[1]);
                    }
                    return;
                }
                0x6c => {
                    // $2EABC
                    if e.launch_y >= 0x140 {
                        e.launch_y -= 1;
                        return;
                    }
                    if Self::approach(cx, &self.parts.poses.arm[2]) {
                        return;
                    }
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 || destroyed(cx.effects, ARM_PART) {
                        e.vars[T] = 0xc0;
                        e.routine = 0x70;
                        return;
                    }
                    e.home_x += 3;
                    return;
                }
                0x70 => {
                    // $2EAFE: strafing the water while moving left, the
                    // arm shaking; a reaching arm ends it.
                    e.vars[T] -= 1;
                    let reaching =
                        parts::part_ref(cx.effects, ARM_PART).is_some_and(|p| p.angle != 0);
                    if e.vars[T] == 0 || reaching {
                        e.vars[T] = 0x80;
                        e.routine = 0x54;
                        return;
                    }
                    if e.vars[T] & 3 == 0 {
                        self.strafe(cx, e);
                    }
                    if e.vars[T] & 1 == 0
                        && let Some(arm) = part(cx.effects, ARM_PART)
                    {
                        // eori.l #$20001,$36(a1)
                        arm.origin.0 ^= 2;
                        arm.origin.1 ^= 1;
                    }
                    e.home_x -= 3;
                    return;
                }
                0x74 => e.routine = 0x78,
                0x78 => {
                    e.vars[T] = 0x80;
                    e.routine = 0x6c;
                    return;
                }
                0x7c => {
                    // $2EC60
                    if !Self::list4_empty(cx) {
                        return;
                    }
                    if e.launch_y < 0x1a0 {
                        e.launch_y += 1;
                        return;
                    }
                    if Self::amp(e, Y_WAVE) >= 0x10 {
                        Self::set_amp(e, Y_WAVE, Self::amp(e, Y_WAVE) - 1);
                    }
                    e.vars[T] -= 1;
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x100;
                        e.routine = 0x80;
                        cx.effect_sound(self.sounds.charge);
                        self.start_spread_charge(cx, e);
                    }
                    return;
                }
                0x80 => {
                    // $2ECC4 with the list-4 routine $28 launcher: it charges
                    // $70 ticks, fires the spread and pulls the boss to $120.
                    e.vars[T] -= 1;
                    if e.vars[T] == 0x91 {
                        self.release_spread_launcher(cx, e);
                        self.spread(cx, e);
                    } else if (0x4f..0x8f).contains(&e.vars[T]) {
                        e.home_x += (0x120 - e.home_x) >> 4;
                    }
                    if e.vars[T] >= 0x90 && e.vars[T] & 3 == 0 {
                        self.spread_charge_spark(cx, e);
                    }
                    if e.vars[T] == 0 {
                        e.vars[T] = 0x18;
                        e.routine = 0x30;
                    }
                    return;
                }
                0x84 => {
                    // $2ED22: drifting off through explosions.
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[T] -= 1;
                    if e.home_x < -72 {
                        cx.fill_palette(2, 0x0eee);
                        e.vars[T] = 0x80;
                        e.routine = 0x88;
                        return;
                    }
                    e.home_x += e.vars[DRIFT];
                    if e.vars[T] & 7 == 0 {
                        let dx = i32::from(cx.random() as u8 as i8) >> 1;
                        let blast = Self::blast(cx, self.sounds.blast);
                        cx.explode(e.x + dx, e.y, blast);
                    }
                    if e.vars[T] & 0xf == 0 {
                        e.vars[DRIFT] -= 1;
                        e.launch_y += 1;
                    }
                    if e.vars[T] & 3 == 0 {
                        // $2EDE6: smoke every fourth update.
                        self.smoke(cx, e, 0);
                    }
                    return;
                }
                _ => {
                    // $2EDF0: then the round clear request.
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[T] -= 1;
                    if e.vars[T] & 7 == 0 {
                        cx.restore_palette(2);
                    }
                    if e.vars[T] <= 0 {
                        Self::clear_missiles(cx);
                        cx.next_substage();
                        Ctx::remove(e);
                    }
                    return;
                }
            }
        }
    }
    /// `$2E3EE`/`$2E45C`: part `n` of the flying machine is pulled up from
    /// the sea below the boss (`+$32`/`+$34`), to show mapping `+$30`.
    fn start_rise(&self, cx: &mut Ctx, e: &mut Enemy, n: usize) {
        let (below, mapping) = self.parts.rises[n];
        if let Some(p) = part(cx.effects, n) {
            p.angle = 4;
        }
        e.vars[P32] = e.x;
        e.vars[P34] = e.y + below;
        e.vars[P30] = i32::from(mapping);
    }
    /// The `ext.l`/`swap`/`ori.w #1`/`add.w d0,d0` step: two toward
    /// `target` (an odd distance keeps it stepping over).
    fn double_step(value: &mut i32, target: i32) {
        if *value != target {
            *value += (target - *value).signum() * 2;
        }
    }
    /// `$2E054`: effect `$37`, the bow glow stowed, sixteen wakes from X -
    /// `$100` along the water (their tile `$800` flips them).
    fn charge_end(&self, cx: &mut Ctx, e: &Enemy) {
        cx.effect_sound(self.charge_end);
        self.charge_flash(cx);
        if let Some(p) = part(cx.effects, GLOW) {
            p.origin.1 = self.parts.glow_y.1;
        }
        let water = -crate::special::WATER_FLOW << 16;
        for i in (0..16).rev() {
            let at = (e.x + (i << 4) - 0x100, WATER);
            let mut wake = ambient::wake(&cx.shared.ambient, at, 0x20, water, false);
            wake.flip = true;
            cx.effect(wake);
        }
    }
}
