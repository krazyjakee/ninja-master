//! Gunboat and flying-machine projectile attacks.
use super::*;

impl Boss4 {
    /// `Func_02F0F6`: list-4 `$34` routine 8, `table` entry `index`.
    pub(super) fn launch_missile(&self, cx: &mut Ctx, e: &Enemy, table: usize, index: usize) {
        let m = self.missiles[table][index];
        cx.effect_sound(self.sounds.missile);
        let (ox, oy) = Self::arc(cx, &m, m.angle);
        let (x, y) = (e.x + m.offset.0 + ox, e.y + m.offset.1 + oy);
        // `$2F108` leaves `obj_frame` 0 (only the cannon's `$2DDA8` sets it).
        let mut missile =
            Projectile::new(x, y, 0, true, self.headings[0], self.missile_damage).unset_frame();
        missile.life = MISSILE_TAG | ((table as u16) << 2) | index as u16;
        missile.passes_terrain = true;
        missile.owner = Some(cx.current);
        // `$2F7B2`/`$2F7EA`: a weapon frees the missile with `$2F3B4`
        // smoke; the only sound is the attacking weapon's hit response.
        missile.shot_down = Some(ShotDown {
            blast: None,
            first: self.sinking.smoke.0,
            frames: 0,
            period: 0,
            sound: None,
            smoke: Some((self.sinking.smoke.1, self.sinking.rise, self.sinking.lift)),
        });
        missile.contact_puff = true;
        missile.missile_heading = Some(0x600);
        self.aim_missile(cx, &mut missile);
        missile.x += m.drift.0;
        missile.y += m.drift.1;
        cx.shoot(missile);
    }
    /// The arc offset at `angle` (`$2F830`: `muls`, `asl.l #2`, `swap`).
    fn arc(cx: &Ctx, m: &Missile, angle: i32) -> (i32, i32) {
        (
            (Self::cosine(cx, angle) * m.radii.0) << 2 >> 16,
            (sine(cx, angle) * m.radii.1) << 2 >> 16,
        )
    }
    /// `$2F860`: the heading toward the player (2048 per turn, `$7F0`).
    fn heading(cx: &Ctx, shot: &Projectile) -> i32 {
        (i32::from(cx.shared.angle_to((shot.x, shot.y), cx.player)) << 3) & 0x7f0
    }
    fn aim_missile(&self, cx: &Ctx, shot: &mut Projectile) {
        let target = Self::heading(cx, shot);
        let current = i32::from(shot.missile_heading.unwrap_or(0x600)) & 0x7f0;
        let delta = (target - current) & 0x7ff;
        let heading = (current
            + match delta {
                0 => 0,
                0x400.. => -0x20,
                _ => 0x20,
            })
            & 0x7ff;
        shot.missile_heading = Some(heading as u16);
        shot.mapping = self.headings[((heading >> 6) & 15) as usize];
        let flip = self.heading_flips[((heading >> 9) & 3) as usize];
        shot.faces_left = (flip & 0x800 != 0) != shot.leftward();
        shot.base = (shot.base & !0x1000) | (flip & 0x1000);
    }
    /// List-4 `$34` routine `$0C` for every missile of this boss: the arc
    /// about its drifting centre, the hover, then the launch at 4 px/tick.
    pub(super) fn steer_missiles(&self, cx: &mut Ctx) {
        let me = cx.current;
        let mut shots = std::mem::take(&mut *cx.projectiles);
        for shot in shots.iter_mut() {
            if shot.owner == Some(me) && shot.alive && shot.water.is_some() && shot.y >= WATER {
                shot.alive = false;
                if shot.water == Some(true) {
                    let water = -crate::special::WATER_FLOW << 16;
                    cx.effect(ambient::wake(
                        &cx.shared.ambient,
                        (shot.x, WATER),
                        32,
                        water,
                        false,
                    ));
                }
                continue;
            }
            if shot.owner != Some(me) || !shot.alive || shot.shot_down.is_none() {
                continue;
            }
            // `$2F7CE` runs before the arc/heading step and frees a missile
            // at the water line, leaving the same routine-8 plume as the
            // sinking hull. This also applies after its launch tag is gone.
            if shot.y >= WATER {
                shot.alive = false;
                let water = -crate::special::WATER_FLOW << 16;
                cx.effect(ambient::plume(&cx.shared.ambient, (shot.x, WATER), water));
                continue;
            }
            if shot.life & 0xff00 != MISSILE_TAG {
                self.missile_exhaust(cx, shot);
                continue;
            }
            let table = usize::from((shot.life >> 2) & 3).min(2);
            let m = self.missiles[table][usize::from(shot.life & 3).min(2)];
            let k = i32::from(shot.age);
            if k == 0 || shot.motion.vx != 0 || shot.motion.vy != 0 {
                continue;
            }
            if k < m.arc {
                let (x0, y0) = Self::arc(cx, &m, m.angle + m.step * (k - 1));
                let (x1, y1) = Self::arc(cx, &m, m.angle + m.step * k);
                shot.x += x1 - x0;
                shot.y += y1 - y0;
                self.missile_exhaust(cx, shot);
                self.aim_missile(cx, shot);
                shot.x += m.drift.0;
                shot.y += m.drift.1;
            } else if k < m.arc + m.hover - 1 {
                self.aim_missile(cx, shot);
                shot.x += m.drift.0;
                shot.y += m.drift.1;
            } else {
                let heading = i32::from(shot.missile_heading.unwrap_or(0x600));
                shot.motion.vx = Self::cosine(cx, heading) << 4;
                shot.motion.vy = sine(cx, heading) << 4;
                // Keep the heading's flip while the motion mirrors the art.
                let flip = self.heading_flips[((heading >> 9) & 3) as usize] & 0x800 != 0;
                shot.faces_left = flip != (shot.motion.vx < 0);
                shot.life = u16::MAX;
                self.missile_exhaust(cx, shot);
            }
        }
        *cx.projectiles = shots;
    }
    fn missile_exhaust(&self, cx: &mut Ctx, shot: &Projectile) {
        if (shot.age + 1) & 3 != 0 {
            return;
        }
        let heading = i32::from(shot.missile_heading.unwrap_or(0x600));
        let at = (
            shot.x + ((-Self::cosine(cx, heading) << 6) >> 16),
            shot.y + ((-sine(cx, heading) << 6) >> 16),
        );
        let mut spark = Effect::new(EffectKind::Dust, at.0, at.1, cx.shared.dust_seq[0]);
        spark.back = true;
        spark.blink = true;
        spark.high_priority = shot.high_priority;
        cx.effect(spark);
    }
    /// Removes the boss's hovering missiles (they have no life of their own).
    pub(super) fn clear_missiles(cx: &mut Ctx) {
        let me = cx.current;
        for shot in cx.projectiles.iter_mut() {
            if shot.owner == Some(me) && shot.life & 0xff00 == MISSILE_TAG {
                shot.alive = false;
            }
        }
    }
    /// `$2DD82`: a cannon shell from the bow gun.
    /// The barrel recoils (`+$36` += 8, its `+$2A` = 8 walks it back) and,
    /// with the shell, a list-1 `$2B` muzzle flash shows for one frame
    /// (routine `$10`, `+$28` = 2).
    pub(super) fn cannon(&self, cx: &mut Ctx, e: &Enemy) {
        if let Some(p) = part(cx.effects, CANNON) {
            p.origin.0 += self.parts.recoil;
            p.angle = 8;
        }
        cx.effect_sound(self.sounds.cannon);
        let (mapping, speed, damage) = self.shell;
        // `$2F6D4`: tile word 0 (not mirrored while it flies left) and no
        // frame before the first `Object_LoadHitbox`.
        let mut shell =
            Projectile::new(e.x - 0xa0, e.y - 0x10, speed, true, mapping, damage).unset_frame();
        shell.faces_left = true;
        shell.flip_period = Some(1);
        shell.trail = Some((4, 0));
        shell.passes_terrain = true;
        if cx.shoot(shell) {
            let (mapping, dx, dy) = self.muzzle;
            cx.overlay(e.x + dx, e.y + dy, mapping, 0, false);
        }
    }
    /// `$2E646`: the phase-2 diagonal shell.
    pub(super) fn shell2(&self, cx: &mut Ctx, e: &Enemy) {
        cx.effect_sound(self.sounds.shell);
        let (mapping, offset, speed, damage) = self.shell2;
        let (x, y) = (e.x + offset.0, e.y + offset.1);
        let mut shell = Projectile::new(x, y, speed.0, true, mapping, damage)
            .with_motion(Motion {
                vy: speed.1 << 16,
                ..Motion::default()
            })
            .unset_frame();
        // $2FA18 clears obj_tile: this left-moving diagonal artwork is
        // already oriented toward its flight, with the tail at the gun.
        shell.faces_left = true;
        shell.passes_terrain = true;
        shell.owner = Some(cx.current);
        shell.water = Some(false);
        shell.trail = Some((1, 0));
        cx.shoot(shell);
    }
    /// `Func_02E902`: every eighth call a bomb lobbed toward the player.
    pub(super) fn bombs(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.vars[T] = (e.vars[T] - 0x20) & 0xffff;
        if e.vars[T] & 0xff != 0 {
            return;
        }
        if (e.vars[T] >> 8) & 3 == 0 {
            cx.effect_sound(self.sounds.bomb);
        }
        let random = cx.random();
        let mut vx = if ((random >> 8) as u8 as i8) < 0 {
            -4
        } else {
            -2
        };
        let vy = if (random as i16) < 0 { -3 } else { -1 };
        if e.x < cx.player.0 {
            vx = -vx;
        }
        let (mapping, gravity, damage) = self.bomb;
        let y = e.y - 0x28;
        let mut bomb = Projectile::new(e.x, y, 0, true, mapping, damage)
            .with_motion(Motion {
                vx: vx << 16,
                vy: vy << 16,
                gravity,
                ..Motion::default()
            })
            .unset_frame();
        bomb.passes_terrain = true;
        bomb.owner = Some(cx.current);
        bomb.water = Some(true);
        bomb.flip_period = Some(1);
        cx.shoot(bomb);
    }
    /// List-4 routine `$30` (`Data_02FC4A`): four shells fanning out from
    /// the launcher at (-64, +16), all accelerating left.
    pub(super) fn spread(&self, cx: &mut Ctx, e: &Enemy) {
        cx.effect_sound(self.sounds.spread);
        for shell in &self.spread {
            let (x, y) = (e.x - 64 + shell.offset.0, e.y + 16 + shell.offset.1);
            let mut shot = Projectile::new(x, y, 0, true, self.spread_mapping, 3)
                .with_motion(Motion {
                    vx: shell.speed.0 << 16,
                    vy: shell.speed.1 << 16,
                    ax: shell.accel.0,
                    gravity: shell.accel.1,
                    ..Motion::default()
                })
                .unset_frame();
            shot.passes_terrain = true;
            cx.shoot(shot);
        }
    }
    pub(super) fn release_spread_launcher(&self, cx: &mut Ctx, e: &Enemy) {
        let at = cx
            .effects
            .iter_mut()
            .find(|fx| fx.kind == EffectKind::Boss4Launcher && fx.alive)
            .map(|fx| {
                fx.alive = false;
                (fx.x, fx.y)
            })
            .unwrap_or((e.x - 64, e.y + 16));
        let mut shot = Projectile::new(at.0, at.1, 0, true, self.spread_mapping, 3)
            .with_motion(Motion {
                ax: -0x5000,
                ..Motion::default()
            })
            .unset_frame();
        shot.owner = Some(cx.current);
        shot.passes_terrain = true;
        shot.trail = Some((8, self.spread_mapping));
        cx.shoot(shot);
    }
    /// `$2EC76` allocates the list-4 `$28` launcher when the attack starts.
    /// Its charge frames come from `Data_02FB70` and it stays briefly after
    /// spawning the four balls.
    pub(super) fn start_spread_charge(&self, cx: &mut Ctx, e: &Enemy) {
        let mut launcher = Effect::new(
            EffectKind::Boss4Launcher,
            e.x - 64,
            e.y + 16,
            self.charge_frames[13],
        );
        launcher.list = SpriteList::EnemyShots;
        launcher.counter = 0x70;
        cx.effect(launcher);
    }
    pub(super) fn spread_charge_spark(&self, cx: &mut Ctx, e: &Enemy) {
        let random = *cx.random;
        let mut spark = Effect::new(EffectKind::Boss4Charge, 0, 0, 0x368);
        spark.back = true;
        spark.origin = (e.x - 64, e.y + 16);
        spark.angle = random >> 4;
        spark.phase = (random & 15) as u8;
        spark.counter = 0x1f;
        cx.effect(spark);
    }
    /// The launcher follows the hull only until it fires; the released
    /// sprite stays where it launched and sheds its own trail in the ROM.
    pub(super) fn update_spread_charge(&self, cx: &mut Ctx, e: &Enemy) {
        for fx in cx.effects.iter_mut() {
            if fx.kind == EffectKind::Boss4Charge {
                fx.origin = (e.x - 64, e.y + 16);
            } else if fx.kind == EffectKind::Boss4Launcher && fx.alive {
                if fx.counter > 0 && e.routine == 0x80 && e.vars[T] > 0x91 {
                    fx.x = e.x - 64;
                    fx.y = e.y + 16;
                    fx.counter = e.vars[T] - 0x91;
                    let left = fx.counter as usize;
                    fx.mapping = self.charge_frames[2 * (left / 16) + (left & 1)];
                } else if fx.counter > 0 && !(e.routine == 0x80 && e.vars[T] == 0x91) {
                    fx.counter -= 1;
                    fx.alive = fx.counter > 0;
                }
            }
        }
    }
    /// `$2EB12`: a machine-gun tracer from the lower gun toward the water.
    pub(super) fn strafe(&self, cx: &mut Ctx, e: &Enemy) {
        cx.effect_sound(self.sounds.gun);
        let (x, y) = (e.home_x - 0x58, e.y + 0x24);
        let mut tracer = Effect::new(EffectKind::Pieces, x, y, self.tracer);
        tracer.base = 0x8000;
        tracer.high_priority = true;
        tracer.back = true;
        tracer.counter = 6;
        tracer.motion = Motion {
            vx: -11 << 16,
            vy: 3 << 16,
            ..Motion::default()
        };
        cx.effect(tracer);
        let mut hazard = Projectile::new(e.home_x - 320, WATER, 0, true, 0xd5, 1);
        hazard.box_load = crate::combat::ShotBox::Fixed(Hitbox {
            x: 0,
            y: -16,
            half_width: 16,
            half_height: 16,
        });
        hazard.hidden = true;
        hazard.passes_terrain = true;
        hazard.life = 16;
        hazard.stays = true;
        cx.shoot(hazard);
    }
}
