//! The helper objects: shurikens, energy shots, the slot object (blade,
//! spinner, shadow wave, MIJIN aura) and the attachments rebuilt as
//! collision objects every tick.
use super::*;

/// A collision object rebuilt this tick at (x, y), drawn with `base`,
/// mirrored like a record whose `$800` flip is `flip`; false when list 4
/// had no room.
fn attach(
    cx: &mut Ctx,
    at: (i32, i32),
    (mapping, before): (u16, u16),
    base: u16,
    flip: bool,
    damage: u8,
) -> bool {
    let mut shot = Projectile::new(at.0, at.1, 0, true, mapping, damage)
        .with_art(base, false)
        .with_motion(Motion {
            fx: 1,
            vx: if flip { -1 } else { 0 },
            ..Motion::default()
        });
    shot.life = ATTACHED;
    shot.held_frame = Some((before, flip, false));
    shot.passes_terrain = true;
    shot.owner = Some(cx.current);
    cx.shoot_capped(shot, LIST4)
}

/// [`attach`] for a list-4 object whose handler loads its box before it
/// sets the frame (`$1E462`'s blade, `$1EE18`'s pieces): shows `frame`
/// and meets the player with the box of `before`, the frame it showed on
/// the tick before (0, a point, before its first frame or while hidden).
fn attach_behind(cx: &mut Ctx, at: (i32, i32), (frame, before): (u16, u16), damage: u8) {
    let mut shot = Projectile::new(at.0, at.1, 0, true, frame, damage)
        .with_art(0x2000, false)
        .with_motion(Motion {
            fx: 1,
            ..Motion::default()
        });
    shot.held_frame = Some((before, false, false));
    shot.life = ATTACHED;
    shot.passes_terrain = true;
    shot.owner = Some(cx.current);
    cx.shoot_capped(shot, LIST4);
}

impl Boss7 {
    /// Projectile `$10`: one of three shurikens thrown from the hand.
    pub(super) fn throw_star(&self, cx: &mut Ctx, e: &Enemy, i: usize) {
        let ((dx, dy), (vx, vy)) = self.stars[i];
        let x = e.x + if e.facing_left { -dx } else { dx };
        let vx = if e.facing_left { vx } else { -vx };
        self.star_at(
            cx,
            (x, e.y + dy),
            vx,
            vy,
            self.star_frames[i][usize::from(e.facing_left)],
        );
    }
    fn star_at(&self, cx: &mut Ctx, at: (i32, i32), vx: i32, vy: i32, frame: u16) {
        let mut star = Projectile::new(at.0, at.1, 0, true, frame, self.star_damage)
            .with_art(0x2000, vx < 0)
            .with_motion(Motion {
                vx,
                vy,
                ..Motion::default()
            });
        star.frames = Some([(frame, false); 4]);
        star.owner = Some(cx.current);
        cx.shoot_capped(star, LIST4);
    }
    /// Projectile `$11`: one of the eight shurikens of the jump fan.
    pub(super) fn fan_star(&self, cx: &mut Ctx, e: &Enemy, i: usize) {
        let ((dx, dy), (angle, shift)) = self.fan[i];
        let x = e.x + if e.facing_left { -dx } else { dx };
        let (mut vx, vy) = unit_vector(&self.half, angle);
        if cx.player_left(e.x) {
            vx = -vx;
        }
        let frame = self.fan_frames[i][usize::from(!cx.player_left(e.x))];
        self.star_at(cx, (x, e.y + dy), vx << shift, vy << shift, frame);
    }
    /// Projectile `$1B`: energy shot `index` at 8 px/tick; the rain version
    /// starts 56 px higher.
    pub(super) fn energy(&self, cx: &mut Ctx, at: (i32, i32), index: usize, rain: bool) {
        let Some(&(angle, frame)) = self.energy.get(index) else {
            return;
        };
        let (vx, vy) = unit_vector(&self.half, angle);
        let flip = angle > 0x60;
        let y = at.1 - if rain { self.energy_rise } else { 0 };
        let mut shot = Projectile::new(at.0, y, 0, true, frame, self.energy_damage)
            .with_art(0x2000, (vx < 0) != flip)
            .with_motion(Motion {
                vx: vx << 3,
                vy: vy << 3,
                ..Motion::default()
            });
        shot.frames = Some([
            (frame, flip),
            (frame + 5, flip),
            (frame, flip),
            (frame + 5, flip),
        ]);
        // `$1F1A6` loads its box before its routine sets a frame; the
        // spawners (`$1F052`, `$339C6`) leave `obj_frame` 0.
        shot = shot.unset_frame();
        shot.passes_terrain = true;
        shot.owner = Some(cx.current);
        cx.shoot_capped(shot, LIST4);
    }
    /// The helper object in the slot for one tick: the `$1E462` blade and
    /// its 16 `$1EE18` pieces, the `$1A` spinner, the `$1C` shadow wave or
    /// the MIJIN aura `$19`.
    pub(super) fn slot(&self, cx: &mut Ctx, e: &mut Enemy) {
        let kind = Self::slot_kind(e);
        if kind == 0 {
            return;
        }
        let flip = get(e.vars[SLOT], S_FLIP) != 0;
        let mut age = get(e.vars[SLOT], S_AGE);
        let at = Self::slot_at(e);
        let on = e.age & 2 != 0;
        let mut close = false;
        match kind {
            BLADE => {
                age += 1;
                let (_, _, blade, burst, damage, sound) = self.blade;
                let blade = self.script(blade);
                let burst = self.script(burst);
                let length: i32 = blade.iter().map(|s| i32::from(s.1)).sum();
                let burst_length: i32 = burst.iter().map(|s| i32::from(s.1)).sum();
                // The frame the blade shows on tick `a`: it flickers (mapping
                // 0 on its even ticks), then bursts.
                let shown = |a: i32| -> u16 {
                    let frame = if a < 1 {
                        None
                    } else if a <= length {
                        frame_at(blade, a - 1).filter(|_| a & 1 == 1)
                    } else {
                        frame_at(burst, a - length - 1)
                    };
                    frame.map_or(0, |(frame, _)| frame)
                };
                // `$1E462` and `$1EE18` load their boxes before they set the
                // frame, so each meets the player with the frame it showed
                // on the tick before (Genesis Plus GX, 7-3 tape: the blade
                // shows `$319` on 1266 with `obj_hitbox` `0,0,0,0` and
                // mapping 0 on 1267 with `$319`'s `0,0,8,8`).
                if age <= length {
                    attach_behind(cx, at, (shown(age), shown(age - 1)), damage);
                } else {
                    let t = age - length - 1;
                    if t == 0 {
                        cx.effect_sound(sound);
                    }
                    if frame_at(burst, t).is_some() {
                        attach_behind(cx, at, (shown(age), shown(age - 1)), damage);
                    }
                    // $1E596: one $1EE18 piece per tick around the blade,
                    // allocated with `obj_frame` 0.
                    for (i, &(dx, dy)) in self.pieces.iter().enumerate() {
                        let i = i as i32;
                        if t < i {
                            break;
                        }
                        if let Some((frame, _)) = frame_at(burst, t - i) {
                            let before = match t - i {
                                0 => 0,
                                n => frame_at(burst, n - 1).map_or(0, |(f, _)| f),
                            };
                            let at = (at.0 + dx, at.1 + dy);
                            attach_behind(cx, at, (frame, before), self.piece_damage);
                        }
                    }
                    close = t >= self.pieces.len() as i32 + burst_length;
                }
            }
            SPINNER => {
                let mut frame = 0;
                // Runs its script only on the frames it is shown.
                if on {
                    age += 1;
                    match frame_at(self.script(self.spinner_script), age - 1) {
                        Some((shown, left)) => {
                            frame = shown;
                            if shown == self.spinner_shot && left == 1 {
                                self.energy(cx, at, if flip { 8 } else { 0 }, false);
                                cx.effect_sound(self.spinner_sound);
                            }
                        }
                        None => {
                            e.vars[FLAGS] |= SIGNAL;
                            close = true;
                        }
                    }
                }
                if !close {
                    attach(
                        cx,
                        at,
                        (frame, e.vars[SLOT_FRAME] as u16),
                        0x2000,
                        flip,
                        self.aura_damage,
                    );
                    e.vars[SLOT_FRAME] = i32::from(frame);
                }
            }
            WAVE => {
                let (_, script, shift, accel, damage) = self.wave;
                let hurt = e.mapping == self.hurt_frame;
                let mut out = hurt;
                let mut frame = 0;
                if !hurt && on {
                    age += 1;
                    match frame_at(self.script(script), age - 1) {
                        Some((shown, _)) => {
                            frame = shown;
                        }
                        None => out = true,
                    }
                }
                if out {
                    // $1F332: signal the boss, drop 64 px and slide off.
                    e.vars[FLAGS] |= SIGNAL;
                    let side = if flip { 1 } else { -1 };
                    let x = at.0 + side * shift;
                    let y = at.1 + self.wave_drop;
                    let [a, b] = self.wave_frames;
                    let mut wave = Projectile::new(x, y, 0, true, a, damage)
                        .with_art(0x2000, flip)
                        .with_motion(Motion {
                            ax: side * accel,
                            ..Motion::default()
                        });
                    wave.frames = Some([(a, flip), (b, flip), (a, flip), (b, flip)]);
                    wave.held_frame = Some((e.vars[SLOT_FRAME] as u16, flip, false));
                    wave.passes_terrain = true;
                    wave.owner = Some(cx.current);
                    wave.stays = true;
                    if cx.shoot_capped(wave, LIST4) {
                        e.vars[SLOT] = put(e.vars[SLOT], S_KIND, WAVE_OUT);
                    } else {
                        close = true;
                    }
                } else {
                    attach(
                        cx,
                        at,
                        (frame, e.vars[SLOT_FRAME] as u16),
                        0x2000,
                        flip,
                        damage,
                    );
                    e.vars[SLOT_FRAME] = i32::from(frame);
                }
            }
            WAVE_OUT => {
                let me = cx.current;
                let [a, b] = self.wave_frames;
                let wave = cx.projectiles.iter_mut().find(|p| {
                    p.owner == Some(me) && p.life != ATTACHED && (p.mapping == a || p.mapping == b)
                });
                let gone = if let Some(wave) = wave {
                    if wave.touching {
                        let state = cx.player_state.unwrap_or(1);
                        if self.player_classes.get(state) == Some(&0)
                            && e.mapping != self.hurt_frame
                        {
                            let x = (wave.x + if flip { 32 } else { -32 })
                                .clamp(cx.camera.0 + 40, cx.camera.0 + 280);
                            cx.signals
                                .push(crate::combat::Signal::PushPlayer(x - cx.player.0));
                            false
                        } else {
                            wave.alive = false;
                            true
                        }
                    } else {
                        false
                    }
                } else {
                    true
                };
                if gone {
                    e.vars[FLAGS] |= SIGNAL;
                    close = true;
                }
            }
            _ => {
                // AURA ($1EECE): 140 ticks, harmless, blinking.
                age += 1;
                if on {
                    // A list-4 attack record.
                    cx.overlay_in(
                        crate::combat::SpriteList::EnemyShots,
                        at.0,
                        at.1,
                        self.aura_frame + (e.age & 1) as u16,
                        0x2000,
                        false,
                    );
                }
                if age >= self.aura_ticks {
                    e.vars[FLAGS] |= SIGNAL;
                    close = true;
                }
            }
        }
        if close {
            e.vars[SLOT] = 0;
        } else {
            e.vars[SLOT] = put(e.vars[SLOT], S_AGE, age.min(0xffff));
        }
    }
    /// `$3417C`: body contact is tested before the routine changes pose.
    /// It is independent of list 4, cannot be guarded, and does not spark.
    pub(super) fn body_contact(&self, cx: &mut Ctx, e: &Enemy) {
        let flags = e.vars[FLAGS];
        let shielded = flags & PHASE2 != 0
            && e.routine == 0x10
            && (e.vars[SUB] == 4
                || e.mapping == self.dive_frame() - 1
                || e.mapping == self.dive_frame());
        if flags & (DYING | GUARDLESS) != 0 || e.routine == 0x18 || shielded || !e.live() {
            return;
        }
        let body = crate::combat::mapping_hitbox(cx.mappings, e.mapping, e.x, e.y, e.facing_left);
        if cx.collide_player(body) {
            cx.contact_player(
                if e.mapping == self.dash_end + 1 || e.mapping == self.dash_end + 2 {
                    2
                } else {
                    1
                },
            );
        }
    }
    /// `$1EF56`: persistent `$1A` variants 1..3. The previous displayed
    /// frame supplies the box, even during the two hidden blink ticks.
    pub(super) fn attachments(&self, cx: &mut Ctx, e: &mut Enemy) {
        let flip = e.facing_left;
        let on = e.age & 2 != 0;
        let bit0 = e.age & 1 != 0;
        let cancelled = e.mapping == self.hurt_frame
            || cx
                .player_state
                .is_some_and(|state| state >= 0x41 && state != 0x42);
        for (variant, flag) in [(1, AURA1), (2, AURA2), (3, AURA3)] {
            let existing = cx.projectiles.iter().position(|p| {
                p.owner == Some(cx.current) && p.boss7_aura.map(|a| a.0) == Some(variant)
            });
            let invalid_pose = match variant {
                1 => e.mapping == self.first,
                2 => e.mapping != self.dive_frame(),
                _ => e.mapping != self.hold_frame && e.mapping != self.hold_frame + 1,
            };
            if cancelled || (on && invalid_pose) {
                e.vars[FLAGS] &= !flag;
            }
            if e.vars[FLAGS] & flag == 0 {
                if let Some(i) = existing {
                    cx.projectiles.remove(i);
                }
                continue;
            }
            if on && existing.is_some_and(|i| cx.projectiles[i].boss7_aura.unwrap().1) {
                if variant <= 2 {
                    self.voice(cx, self.aura_voices[usize::from(variant - 1)]);
                }
                if variant == 2 {
                    let i = existing.unwrap();
                    let at = (cx.projectiles[i].x, cx.projectiles[i].y);
                    cx.projectiles.remove(i);
                    // $36AF8: eight list-1 sparks, not the ordinary hit spark.
                    for &(dx, dy, angle, shift) in &self.dive_sparks {
                        let (vx, vy) = cx.shared.sine.polar(angle);
                        animate(
                            cx,
                            (at.0 + dx, at.1 + dy),
                            Motion {
                                vx: if cx.player_facing_left {
                                    -(vx << shift)
                                } else {
                                    vx << shift
                                },
                                vy: vy << shift,
                                ..Motion::default()
                            },
                            0,
                            1,
                            &self.dive_spark_script,
                            false,
                        );
                    }
                    e.vars[FLAGS] = (e.vars[FLAGS] & !AURA2) | SIGNAL;
                    continue;
                }
            }
            let (dx, dy) = self.aura_offsets[usize::from(variant)];
            let at = (e.x + if flip { -dx } else { dx }, e.y + dy);
            let base = if variant == 1 { 0x3000 } else { 0x2000 };
            let index = if let Some(i) = existing {
                i
            } else {
                let mut aura =
                    Projectile::new(at.0, at.1, 0, true, 0, self.aura_damage).with_art(base, flip);
                aura.owner = Some(cx.current);
                aura.boss7_aura = Some((variant, false));
                aura.passes_terrain = true;
                aura.stays = true;
                if !cx.shoot_capped(aura, LIST4) {
                    e.vars[FLAGS] &= !flag;
                    continue;
                }
                cx.projectiles.len() - 1
            };
            let aura = &mut cx.projectiles[index];
            let (frame, horizontal, vertical) = aura.drawn_frame();
            let mut bounds = cx
                .mappings
                .get(&frame)
                .map_or(crate::combat::Hitbox::NONE, |m| m.bounds)
                .placed(0, 0, horizontal, vertical);
            if variant == 1 {
                bounds.half_width += 16;
                bounds.half_height += 32;
            }
            aura.box_load = crate::combat::ShotBox::Fixed(bounds);
            if on {
                if variant != 3 {
                    (aura.x, aura.y) = at;
                }
                aura.boss7_aura.as_mut().unwrap().1 = false;
            }
            aura.mapping = if !on {
                0
            } else {
                match variant {
                    1 => self.aura_frames[usize::from(!bit0)],
                    2 => self.aura_frames[2 + usize::from(!bit0)],
                    _ => self.aura_frames[4],
                }
            };
        }
    }
    /// `$306`: the last frame of the dive script, on which variant 2 shows.
    fn dive_frame(&self) -> u16 {
        self.script(self.dive_script).last().map_or(0, |s| s.0)
    }
}
