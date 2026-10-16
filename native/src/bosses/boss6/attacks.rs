//! Shadow and lion projectile emission and steering.
use super::*;

impl Boss6 {
    /// `$1F820`: 8 px/tick along direction `d` (16 per turn, counter-
    /// clockwise from the right, up negative).
    fn needle_velocity(&self, d: u8) -> (i32, i32) {
        let (x, y) = unit_vector(&self.circle, (d & 15) * 8);
        (x << 3, y << 3)
    }
    /// List-4 `$0E` (`$1E5D4`): four parallel needles from `(x, y)` along
    /// direction `d`, 24 px out from `y - $2E`.
    pub(super) fn needles(&self, cx: &mut Ctx, x: i32, y: i32, d: u8) {
        let (vx, vy) = self.needle_velocity(d);
        let d = usize::from(d & 15);
        let flips = self.needle_flips[d];
        let hflip = flips & 2 != 0;
        let vflip = flips & 1 != 0;
        let (ox, oy) = ((vx >> 16) * 3, (vy >> 16) * 3);
        for (index, (dx, dy)) in self.needle_offsets.into_iter().enumerate().rev() {
            let mut needle = Projectile::new(
                x + dx + ox,
                y - 0x2e + dy + oy,
                0,
                true,
                self.needle_frames[d],
                self.needle_damage,
            )
            .with_motion(Motion {
                vx,
                vy,
                ..Motion::default()
            });
            needle.base = 0x2000 | if vflip { 0x1000 } else { 0 };
            needle.faces_left = needle.leftward() != hflip;
            // `$1E5D4` loads its box only in routine 0 (`$1E5F2`), before
            // `$1E824` sets the first frame: mapping 0's, a point, for life.
            needle.box_load = ShotBox::Fixed(Hitbox::NONE);
            needle.owner = Some(cx.current);
            // `$1E2C4` / `$32114`: only d1 = 0 gets flag bit 1;
            // `$1E754` requests SHURIKINS when that needle hits terrain.
            needle.wall_sound = (index == 0).then_some(self.needle_impact_sound);
            needle.life = 0xb4;
            if !cx.shoot_capped(needle, SHOT_CAP) {
                return;
            }
        }
    }
    /// `$1E2C4`: `Func_01F5DE` from `(x, y - $18)` to the player's chest
    /// (`y - $10`), `andi.w #$7F` / `lsr.w #3` to 16 directions.
    pub(super) fn aim_direction(&self, player: (i32, i32), x: i32, y: i32) -> u8 {
        let angle = aim_angle(&self.slopes, (x, y - 0x18), (player.0, player.1 - 0x10));
        (angle & 0x7f) >> 3
    }
    /// List-4 `$13` (`$1EBEE`): three fans from the lion's hand.
    pub(super) fn fans(&self, cx: &mut Ctx, e: &Enemy) {
        let dir = if e.facing_left { 1 } else { -1 };
        for index in 0..3u16 {
            let mut fan = Projectile::new(
                e.x + dir * 32,
                e.y - 56,
                0,
                true,
                self.fan_frames[0],
                self.fan_damage,
            )
            .with_motion(Motion {
                vx: -dir * (2 << 16),
                gravity: FAN_RISE_ACCEL,
                ..Motion::default()
            })
            // `$1EBEE` never loads a box (the record's last one stays):
            // native keeps the frame drawn.
            .box_after();
            fan.base = 0x4000;
            fan.owner = Some(cx.current);
            fan.life = FAN_LIFE + index;
            // `$1EC88` runs after setup in the allocation frame.
            if Self::fan_reaches_top(&fan, e.y) {
                Self::fan_drift(&mut fan, e.y, index);
            }
            if !cx.shoot_capped(fan, SHOT_CAP) {
                break;
            }
            // `$1EC82`: each allocated fan requests its own effect.
            cx.play(self.fan_sound);
        }
    }
    /// `$1EC94..$1ECB4`: predict the next integrated Y, then compare
    /// unsigned words. At upper doors the negative target wraps, so the
    /// fan clamps immediately rather than spending extra frames rising.
    fn fan_reaches_top(fan: &Projectile, lion_y: i32) -> bool {
        let next = ((i64::from(fan.y) << 16)
            + i64::from(fan.motion.fy)
            + i64::from(fan.motion.vy)
            + i64::from(fan.motion.gravity))
            >> 16;
        (lion_y - FAN_CLIMB) as u16 >= next as u16
    }
    fn fan_drift(fan: &mut Projectile, lion_y: i32, index: u16) {
        fan.y = lion_y - FAN_CLIMB;
        fan.motion = Motion::default();
        fan.life = FAN_DRIFT_LIFE + 3 * fan.age.min(1000) + index;
    }
    /// List-4 `$1D` (`$1F42A`): three spears below door columns 0..2,
    /// erupting in the order the lion travels.
    pub(super) fn spears(&self, cx: &mut Ctx, e: &Enemy) {
        let delays: [u16; 3] = if e.facing_left {
            [20, 70, 120]
        } else {
            [120, 70, 20]
        };
        for (column, delay) in delays.into_iter().enumerate() {
            let x = cx.camera.0 + self.doors[column].0;
            let mut spear = Projectile::new(
                x,
                cx.camera.1 + SPEAR_START,
                0,
                true,
                self.spear,
                self.spear_damage,
            );
            spear.base = 0x2000;
            spear.passes_terrain = true;
            spear.owner = Some(cx.current);
            spear.life = SPEAR_LIFE + delay;
            cx.shoot_capped(spear, SHOT_CAP);
        }
    }
    /// The handlers of the fans and spears, run while the lion lives.
    pub(super) fn steer(&self, cx: &mut Ctx, e: &Enemy) {
        let me = cx.current;
        let (player, camera, lion_y) = (cx.player, cx.camera, e.y);
        let mut sounds = Vec::new();
        for shot in cx.projectiles.iter_mut() {
            if shot.owner != Some(me) {
                continue;
            }
            // `$1E722..$1E74A`: needles pass the ledge band, even when
            // its material is solid. Their other impacts use the shot pass.
            if self.needle_frames.contains(&shot.mapping)
                && (0x68..0x80).contains(&(shot.y - camera.1))
            {
                shot.walled = false;
            }
            if (FAN_LIFE..FAN_LIFE + 3).contains(&shot.life) {
                // Rising (accelerating up) until 160 px above the lion.
                shot.mapping = self.fan_frames[usize::from(shot.age / 2) % 6];
                if Self::fan_reaches_top(shot, lion_y) {
                    let index = shot.life - FAN_LIFE;
                    Self::fan_drift(shot, lion_y, index);
                }
            } else if (FAN_DRIFT_LIFE..FAN_DRIFT_LIFE + 3003).contains(&shot.life) {
                shot.mapping = self.fan_frames[usize::from(shot.age / 2) % 6];
                let code = shot.life - FAN_DRIFT_LIFE;
                let (start, index) = (code / 3, usize::from(code % 3));
                let drift_end = start + FAN_DRIFT_TICKS;
                if shot.age == start + 1 {
                    shot.motion = Motion {
                        vx: (index as i32 - 1) << 16,
                        fx: 1,
                        ..Motion::default()
                    };
                }
                if shot.age == drift_end {
                    shot.motion = Motion {
                        fx: 1,
                        ..Motion::default()
                    };
                }
                if shot.age == drift_end + FAN_WAITS[index] + 1 {
                    let (vx, vy) = aim((shot.x, shot.y), (player.0, player.1 - 16));
                    shot.motion = Motion {
                        vx: vx * FAN_SPEED,
                        vy: vy * FAN_SPEED,
                        ..Motion::default()
                    };
                    sounds.push(self.aim_sound);
                    // `$1ED3C` then `$1ED5E`: throw and enemy-star cues,
                    // in that order, as the waiting fan starts homing.
                    sounds.push(self.fan_sound);
                }
            } else if shot.life >= SPEAR_LIFE && shot.mapping == self.spear {
                let delay = shot.life - SPEAR_LIFE;
                if shot.age < delay {
                    continue;
                }
                let top = camera.1 + SPEAR_TOP;
                let risen = i32::from(shot.age - delay) * SPEAR_RISE;
                if shot.motion.vy == 0 {
                    // fx marks the still spear as colliding.
                    shot.motion.fx = 1;
                    shot.y = (camera.1 + SPEAR_START - risen).max(top);
                    let held = shot.age - delay;
                    let rise_ticks = ((SPEAR_START - SPEAR_TOP) / SPEAR_RISE + 1) as u16;
                    if held >= rise_ticks + SPEAR_HOLD {
                        shot.motion.vy = SPEAR_SINK;
                    }
                }
            }
        }
        for sound in sounds {
            cx.play(sound);
        }
    }
}
