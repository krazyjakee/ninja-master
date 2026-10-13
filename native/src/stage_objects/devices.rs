//! The list-6 platforms, lifts and traps (`$270AA` handlers other than
//! the item box).
use super::{
    Context, Object, ObjectEvents, Record, StageObjects, UNIT, debris::DeviceParticle, placed,
};
use crate::{
    combat::{Combat, Hitbox, Motion},
    movement::{Material, PlayerEvent, WallProbe},
};

impl Object {
    /// `$28E7E` after the common `$270F2` setup. The low parameter bit
    /// selects mapping `$106` (2-2) rather than `$F4` (7-1); `+$35` then
    /// becomes 1 for both, so every lift waits for its first rider.
    pub(super) fn lift(index: usize, record: &Record, combat: &Combat) -> Self {
        let mapping = if record.param & 1 != 0 { 0x106 } else { 0xf4 };
        Self {
            priority: record.param & 1 == 0,
            param: 1,
            ..Self::new(index, record, mapping, 1, combat)
        }
    }
}

impl StageObjects {
    /// `$28ED0`: release and raise the climb lift, carry its rider, and
    /// kill the rider when both 16-pixel-spaced probes above it are solid.
    pub(super) fn update_lift(
        &self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        object.motion.vy = 0;
        // $28EE4: a dying rider (`plr_flags` bit 3) stops the lift.
        let dying = crate::movement::dying(cx.player.animator.state);
        if !dying && object.param & 0x80 == 0 {
            if object.param & 1 != 0 {
                if object.touched {
                    object.param &= !1;
                    events.release_autoscroll = true;
                    events.sounds.extend(self.tables.effect(0x39));
                    object.saved_player_y = cx.player.position().1;
                    object.saved_screen_y = object.sprite_y(cx.camera);
                }
                // $28F2A returns with bit 14 set: the next player probe
                // skips the released lift, then its carry consumes it.
                return true;
            }
            if object.y <= object.y_window.0 {
                object.param |= 0x80;
                events.sounds.extend(self.tables.effect(0x25));
            } else {
                object.motion.vy = -0x8000;
                // $28F62: a part change moved the rider by a whole
                // coordinate wrap; keep the lift at its sprite Y.
                if (cx.player.position().1 - object.saved_player_y).abs() >= 0x100 {
                    object.y = cx.camera.1 + object.saved_screen_y - 0x80;
                }
                object.saved_player_y = cx.player.position().1;
                object.saved_screen_y = object.sprite_y(cx.camera);
            }
        }
        if std::mem::take(&mut object.touched) && cx.player.position().1 - object.y <= 0x10 {
            // The object-list integrator runs after the handler. Carry the
            // player to that same next fixed-point Y now.
            cx.player.y = object.carried_y(0);
            let probe_y = cx.player.position().1 - 0x40;
            let x = cx.player.position().0;
            if cx.terrain.material_at(x, probe_y).object_solid(false) {
                cx.player.duck_under_lift_ceiling();
                if !dying
                    && cx
                        .terrain
                        .material_at(x, probe_y + 0x10)
                        .object_solid(false)
                {
                    cx.combat.die();
                    cx.player.die();
                    events.crushed = true;
                }
            }
        }
        true
    }

    /// Type 0 (`$28D80`): a 0.75 px/tick platform between the record's X
    /// or Y limits. Parameter bit 0 holds it until first ridden; bit 7 is
    /// the current direction.
    pub(super) fn update_moving_platform(
        &self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        object.motion.vx = 0;
        object.motion.vy = 0;
        let touched = std::mem::take(&mut object.touched);
        if touched && object.param & 1 != 0 {
            object.param &= !1;
            events.sounds.extend(self.tables.effect(0x39));
        }
        if object.param & 1 != 0 {
            return true;
        }
        let vertical = object.param >> 8 != 0;
        let coordinate = if vertical { object.y } else { object.x };
        let limits = if vertical {
            object.y_window
        } else {
            object.x_window
        };
        if coordinate < limits.0 || coordinate >= limits.1 {
            object.param ^= 0x80;
        }
        let velocity = if object.param & 0x80 != 0 {
            -0xc000
        } else {
            0xc000
        };
        if vertical {
            object.motion.vy = velocity;
        } else {
            object.motion.vx = velocity;
        }
        if touched {
            let distance = cx.player.position().1 - object.y;
            let close = distance <= if vertical { 10 } else { 16 };
            if close {
                if !vertical {
                    cx.player.x += velocity / 256;
                }
                cx.player.y = object.carried_y(0);
            }
        }
        true
    }

    /// Type 2 (`$2903A`): a shaking platform whose low parameter byte is
    /// its fuse. It then falls under `$1000` gravity for 96 updates.
    pub(super) fn update_crumbling_platform(&self, object: &mut Object, cx: &mut Context) -> bool {
        let touched = std::mem::take(&mut object.touched);
        if object.state == 0 {
            // $2903A: an untouched platform waits indefinitely. The camera
            // spawns it before Joe arrives; its fuse starts on contact.
            if !touched || cx.player.position().1 - object.y > 16 {
                return true;
            }
            let state = cx.player.animator.state;
            if matches!(state, 1 | 8 | 0x1c) {
                object.state = 1;
                object.auxiliary = 0x60;
            }
        }
        if object.counter > 0 {
            object.counter -= 1;
            if object.counter & 2 == 0 {
                object.x ^= 1;
            }
        } else {
            object.motion.gravity = 0x1000;
            if object.auxiliary == 0 {
                object.auxiliary = 0x60;
            }
            object.auxiliary -= 1;
            if object.auxiliary == 0 {
                return false;
            }
        }
        if touched && cx.player.position().1 - object.y <= 16 {
            cx.player.y = object.carried_y(0);
        }
        true
    }

    /// Type 3 (`$290F8`): four ROM-authored routes made of signed duration
    /// pairs. Each nonzero component moves exactly one pixel per update.
    pub(super) fn update_route_platform(
        &mut self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        match object.state {
            1 => {
                object.mapping ^= 0x101;
                object.counter -= 1;
                if object.counter == 0 {
                    events.sounds.extend(self.tables.effect(0x39));
                    object.state = 2;
                    object.step = 0;
                }
            }
            2 => {
                object.counter = (object.counter - 1).max(0);
                object.auxiliary = (object.auxiliary - 1).max(0);
                if object.counter == 0 && object.auxiliary == 0 {
                    // $290CE reads the record's low parameter byte before
                    // packing its route-table offset into the high byte.
                    let route = usize::from(object.param & 3);
                    let &(dx, dy) = self.tables.route_paths[route]
                        .get(object.step)
                        .unwrap_or(&(0, 0));
                    object.step += 1;
                    if (dx, dy) == (0, 0) {
                        // $29152 leaves the last velocity for this frame's
                        // sprite integration; $291A0 stops it next update.
                        object.state = 3;
                        object.counter = 0x40;
                    } else {
                        object.counter = i32::from(dx).abs();
                        object.auxiliary = i32::from(dy).abs();
                        object.motion.vx = i32::from(dx.signum()) << 16;
                        object.motion.vy = i32::from(dy.signum()) << 16;
                    }
                }
            }
            _ => {
                object.motion.stop();
                object.mapping ^= 0x101;
                object.counter -= 1;
                if object.counter == 0 {
                    return self.free_record(object);
                }
            }
        }
        if std::mem::take(&mut object.touched) && cx.player.position().1 - object.y <= 16 {
            cx.player.x += object.motion.vx / 256;
            cx.player.y = object.carried_y(0);
        }
        true
    }

    /// Type 4 (`$292AC`): a mounted-stage object carried left by the fast
    /// water/road layer. Variant 0 is the 4-1 wave ramp; variant 1 is the
    /// three breakable 2-1 obstacles.
    pub(super) fn update_mounted_object(
        &mut self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        object.counter -= 1;
        if object.counter == 0 {
            return false;
        }
        object.motion.vx = -8 << 16;
        let x = object.x - 8;
        let (px, feet) = cx.player.position();
        if object.param >> 8 == 0 {
            if !crate::movement::dying(cx.player.animator.state)
                && (x - px).abs() < 16
                && (object.y - 32..=object.y).contains(&feet)
            {
                events.ramp = true;
            }
            return true;
        }
        if object.state != 0 {
            return true;
        }
        let body = cx
            .combat
            .player_body(cx.player.position(), cx.player.facing_left);
        if object.priority == cx.combat.player_high_priority()
            && placed(object.hitbox, x, object.y).intersects(body)
        {
            object.state = 1;
            object.mapping = 0;
            object.collidable = false;
            // `$2932C`: the horseback obstacle writes code `$B`; its
            // one-damage stun is consumed on the next player update.
            cx.combat.queue_player_event(PlayerEvent::Stun);
            events.sounds.extend(self.tables.effect(0x25));
            for (vx, vy) in [(1, -6), (1, -5), (1, -4), (1, -3), (1, -2), (1, -1)] {
                self.device_debris.push(DeviceParticle {
                    x,
                    y: object.y,
                    mapping: 0xf5,
                    motion: Motion {
                        vx: vx << 15,
                        vy: vy << 16,
                        gravity: 0x3400,
                        ..Motion::default()
                    },
                    timer: 32,
                });
            }
        }
        true
    }

    /// Type 6 (`$29646`): a platform 128 pixels below the rider which drops
    /// with `$400` gravity after a standing/crouching touch.
    pub(super) fn update_falling_platform(
        &self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        if object.y >= 0x280 {
            return false;
        }
        let touched = std::mem::take(&mut object.touched);
        let close = cx.player.position().1 + 0x80 - object.y <= 15;
        if object.state == 0 && touched && close && matches!(cx.player.animator.state, 1 | 8) {
            object.state = 1;
            events.sounds.extend(self.tables.effect(0x39));
        }
        if object.state != 0 {
            object.motion.gravity = 0x400;
        }
        if touched && close {
            cx.player.y = object.carried_y(0x80);
        }
        true
    }

    /// Type 5 (`$293F8`) with a zero parameter high byte (`$29530`): the
    /// invisible 6-2 door switches. Up newly pressed while the player probe
    /// finds the sensor (`$FC0804`: 16x8 pixels, 8 below the origin) starts
    /// it (`$2954C`; `$293E8` never sets `+$38`, so Up is always needed).
    /// From then on every update forces the idle input (`plr_event` 5) and
    /// 16 frames of immunity on the player (`$29552`) and reports its
    /// parameter: [`ObjectEvents::switch`] fades palette line 0 and then
    /// requests the part or the next substage. The nonzero-high-byte branch
    /// (`$29402`, a plane message) has no record in the table.
    pub(super) fn update_switch_device(
        &self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        if object.param >> 8 != 0 {
            return true; // $29402, not ported (no record).
        }
        if object.state == 0 {
            // $29536: bit 14 is tested and cleared only while waiting.
            if !(std::mem::take(&mut object.touched) && cx.player.up_pressed) {
                return true;
            }
            object.state = 1;
        }
        cx.combat.queue_player_event(PlayerEvent::Idle);
        cx.combat.invulnerability = 16;
        events.switch = Some(object.param);
        true
    }

    /// Type 9 (`$29868`): a platform whose parameter is a post-touch fuse.
    pub(super) fn update_exploding_platform(
        &mut self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        let touched = std::mem::take(&mut object.touched);
        if object.state == 0 {
            if !touched {
                return true;
            }
            object.state = 1;
        }
        if object.counter > 0 {
            object.counter -= 1;
            if touched && cx.player.position().1 - object.y <= 15 {
                cx.player.y = object.y * UNIT;
            }
            return true;
        }
        self.ignite(object.x, object.y, object.priority, cx.camera, events);
        false
    }

    /// Type `$A` (`$298F6`): reveal when ridden, shed smoke for the
    /// parameter duration, burst into six pieces with BOOM D, then fall.
    pub(super) fn update_falling_debris(
        &mut self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        let touched = std::mem::take(&mut object.touched);
        if object.state == 0 {
            if !touched {
                return true;
            }
            object.state = 1;
            object.mapping = 0xf8;
        }
        if object.state == 1 {
            if object.counter > 0 {
                object.counter -= 1;
                // $29938 reads the idle-loop RNG ($512). Step it natively so
                // an otherwise idle scene still emits distinct puffs.
                let jitter = i32::from(cx.combat.next_random() as i8) >> 2;
                self.barrel_smoke
                    .push(super::BarrelSmoke::new(object.x + jitter, object.y + 0x10));
                if touched && cx.player.position().1 - object.y <= 15 {
                    cx.player.y = object.y * UNIT;
                }
                return true;
            }
            events.sounds.extend(self.tables.effect(0x38));
            for params in self.tables.barrel_bursts {
                self.barrel_bursts
                    .push(super::BarrelBurst::new(object.x, object.y, params));
            }
            object.motion.vy = -0x40000;
            object.motion.gravity = 0x3400;
            object.state = 2;
            object.auxiliary = 0x20;
        }
        // $299DE also decrements on the tick that launches the cap.
        object.auxiliary -= 1;
        object.auxiliary != 0
    }

    /// Type `$B` (`$29A64`): the very slow 7-2 moving wall. Attacks are
    /// blocked by it; reaching either X bound stops it with JUMP B.
    pub(super) fn update_moving_trap(
        &self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        // $29A80 uses last frame's sprite Y to keep the tall wall centred
        // on the camera, in 32-pixel steps. It does not stay at spawn Y.
        let screen_y = object.saved_screen_y - 0xf0;
        if screen_y <= -0x10 {
            object.y += 0x20;
        } else if screen_y >= 0x10 {
            object.y -= 0x20;
        }
        object.y = object.y.clamp(object.y_window.0, object.y_window.1);
        object.saved_screen_y = object.sprite_y(cx.camera);
        let projected = ((i64::from(object.x) << 16)
            + i64::from(object.motion.fx)
            + i64::from(object.motion.vx))
            >> 16;
        if object.motion.vx < 0 && projected < i64::from(object.x_window.0)
            || object.motion.vx > 0 && projected >= i64::from(object.x_window.1)
        {
            object.motion.vx = 0;
            events.sounds.extend(self.tables.effect(0x25));
        }
        let body = cx
            .combat
            .player_body(cx.player.position(), cx.player.facing_left);
        if object.priority == cx.combat.player_high_priority() {
            let contact = |half_width| {
                object
                    .placed(Hitbox {
                        half_width,
                        ..object.hitbox
                    })
                    .intersects(body)
            };
            // $29AEC: only the inner 8-pixel band crushes immediately.
            let mut crushed = contact(8);
            if !crushed && contact(0x18) {
                let wall_kick = cx.player.animator.state == crate::movement::state::WALL_CLING;
                // Static wall scans cannot see this list-6 wall. Refresh
                // the contact when a previous kick flies back into it, so
                // the next C press can cling again. Flight away stays free.
                let returning =
                    wall_kick && cx.player.animator.cursor > 0x100 && cx.player.velocity_x > 0;
                if !wall_kick || returning {
                    cx.player.against_wall = true;
                    cx.player.wall_probe = Some(WallProbe {
                        side: 1,
                        material: Material(1),
                        cling: true,
                    });
                }
                if contact(0x12) {
                    // $29B3C snaps the X word 33 pixels left of the wall,
                    // retaining its fraction. A solid pixel 14 pixels left
                    // of that and 16 above the feet is a genuine crush.
                    let x = object.x - 0x21;
                    cx.player.x = x * UNIT + cx.player.x.rem_euclid(UNIT);
                    crushed =
                        cx.terrain
                            .object_solid_at(x - 0xe, cx.player.position().1 - 0x10, false);
                }
            }
            if crushed && !crate::movement::dying(cx.player.animator.state) {
                object.motion.vx = 0;
                cx.combat.die();
                cx.player.die();
                events.crushed = true;
            }
        }
        object.touched = false;
        true
    }

    /// Type `$C` (`$29C6A`): paired 6-2 crushers. They make a fast six-pixel
    /// stroke, a slow one-pixel return, and pause 64 updates at each end.
    pub(super) fn update_crusher(
        &self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        if object.counter > 0 {
            object.counter -= 1;
            object.motion.vy = 0;
        } else {
            let bottom = object.param & 1 != 0;
            let fast = object.state == 0;
            let pixels = match (bottom, fast) {
                (false, true) => 6,
                (false, false) => -1,
                (true, true) => -6,
                (true, false) => 1,
            };
            object.motion.vy = pixels << 16;
            let bound = if pixels < 0 {
                object.y_window.0
            } else {
                object.y_window.1
            };
            let reached = if pixels < 0 {
                object.y <= bound
            } else {
                object.y >= bound
            };
            if reached {
                object.y = bound;
                object.motion.vy = 0;
                object.state ^= 1;
                object.counter = 0x40;
                if fast {
                    events.sounds.extend(self.tables.effect(0x25));
                }
            }
        }
        let touched = std::mem::take(&mut object.touched);
        if touched && cx.player.position().1 + 0x50 - object.y <= 15 {
            cx.player.y = object.carried_y(0x50);
        }
        let body = cx
            .combat
            .player_body(cx.player.position(), cx.player.facing_left);
        if object.priority == cx.combat.player_high_priority()
            && object.motion.vy.abs() > 0x10000
            && object.placed(object.hitbox).intersects(body)
        {
            cx.combat.pending_damage = cx.combat.pending_damage.max(2);
        }
        true
    }

    /// Type `$D` (`$29E5A`): the 4-2 bouncers move down while ridden and
    /// return upward when released, carrying the rider `+$38` above them.
    /// Variant 2 bobs at rest with the water (`$29EC0..$29EDA`).
    pub(super) fn update_bouncer(
        &self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        let touched = std::mem::take(&mut object.touched);
        let speed = object.counter;
        object.motion.vy = if touched { speed } else { -speed };
        if object.motion.vy < 0 && object.y <= object.y_window.0
            || object.motion.vy > 0 && object.y >= object.y_window.1
        {
            object.motion.vy = 0;
        }
        if !touched {
            object.state = 0;
            if object.param >> 8 == 2 && object.motion.vy == 0 {
                let top = object.saved_screen_y + [0, 1, 2, 1][cx.animation_step & 3];
                object.y_window.0 = top;
                object.y = top;
            }
            return true;
        }
        if object.state == 0 && object.param >> 8 == 2 {
            object.state = 1;
            events.sounds.extend(self.tables.effect(0x27));
        }
        let above = object.auxiliary;
        if cx.player.position().1 + above - object.y <= 15 {
            cx.player.y = object.carried_y(above);
        }
        true
    }

    /// Type `$F` (`$2A006` / `$2A116`): a 3-2 laser room, see
    /// [`super::laser`]. The controller itself is hidden and never touches
    /// the player; it hurts once per lock (damage 1, `$2A186`) and frees
    /// itself (`Func_027160`, clearing its `$FF3808` bit) once the player
    /// has left and the monster has sunk away.
    pub(super) fn update_laser(
        &mut self,
        object: &mut Object,
        cx: &mut Context,
        events: &mut ObjectEvents,
    ) -> bool {
        let Some(room) = object.room.as_mut() else {
            return false;
        };
        let out = room.update(&super::laser::Inputs {
            tables: &self.laser,
            sine: &self.tables.sine,
            player: cx.player.position(),
            camera: cx.camera,
            window: (object.x_window, object.y_window),
            // `tick` has counted this frame already.
            frame: self.ticks.wrapping_sub(1),
            random: cx.combat.random_word(),
        });
        for id in out.sounds {
            events.sounds.extend(self.tables.effect(id));
        }
        events.palette_ops.extend(out.palette_ops);
        if out.damage > 0 {
            // `obj_flags` bit 14 with `obj_health` 1: the next damage check
            // takes it like a body contact.
            cx.combat.pending_damage = cx.combat.pending_damage.max(out.damage);
        }
        if out.done {
            return self.free_record(object);
        }
        true
    }

    /// Type `$10` (`$2A26C`): a constant-speed platform in the 6-1 climb.
    pub(super) fn update_climb_platform(&self, object: &mut Object, cx: &mut Context) -> bool {
        if object.offscreen {
            return false;
        }
        object.motion.vy = i32::from(object.param as i16) << 12;
        if std::mem::take(&mut object.touched) && cx.player.position().1 - object.y <= 16 {
            cx.player.y = object.carried_y(0);
        }
        true
    }

    /// Type `$11` (`$2A2CC`): first ridden, shake for the parameter count,
    /// then become non-solid while mappings `$125..$123` disappear.
    pub(super) fn update_disappearing_platform(
        &self,
        object: &mut Object,
        cx: &mut Context,
    ) -> bool {
        if object.state != 0 && object.counter > 0 {
            object.counter -= 1;
            if object.counter & 3 == 0 {
                object.x ^= 2;
            }
        }
        if object.state != 0 && object.counter == 0 {
            object.collidable = false;
            if object.auxiliary > 0 {
                object.auxiliary = (object.auxiliary - 0x10).max(0);
                object.mapping = 0x123 + ((object.auxiliary >> 8) as u16);
            }
        }
        if std::mem::take(&mut object.touched) && cx.player.position().1 - object.y <= 15 {
            object.state = 1;
            cx.player.y = object.y * UNIT;
        }
        true
    }
}
