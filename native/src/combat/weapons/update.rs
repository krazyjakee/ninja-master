use super::*;

impl Combat {
    /// Sword objects and spread shurikens, after the enemies moved.
    pub(in crate::combat) fn update_player_weapons(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
        event: &mut CombatEvents,
    ) {
        let view = self.screen_view(camera.0);
        let shuriken_hit = self.rules.cues.get(SoundEvent::ShurikenHit);
        let mut walls = Vec::new();
        let mut splashes = Vec::new();
        let steering = (self.weapons.spin_frames, self.weapons.spread);
        for shot in &mut self.weapons.shots {
            shot.age += 1;
            // Func_024BE0, as for the thrown shuriken.
            if let Some(mut turn) = shot.turn.filter(|turn| turn.left > 0)
                && shot.age.is_multiple_of(2)
            {
                let (mapping, flips, vx, vy) = steer(&steering, &mut turn);
                shot.turn = Some(turn);
                shot.mapping = mapping;
                shot.flips = Some(flips);
                shot.velocity_x = vx;
                shot.velocity_y = vy;
            }
            // Its held hit did not land: the terrain test where it met the
            // object comes before the next step (`loc_024DD0`).
            if std::mem::take(&mut shot.untested) {
                Self::spread_terrain_at(terrain, shot, &mut walls, &mut splashes);
                if shot.age == u16::MAX {
                    continue;
                }
            }
            // `Object_Integrate` moves it a whole step, and only where the
            // step ends do the objects' `Object_CollideL3` (next frame) and
            // `Shuriken_Update`'s terrain test meet it.
            shot.x += shot.velocity_x;
            shot.y += shot.velocity_y;
            let (x, y) = (shot.x >> 16, shot.y >> 16);
            let hitbox = Self::spread_box(&self.mappings, shot);
            let target = self.enemies.iter_mut().find(|enemy| {
                Self::attack_targetable(&self.roster, &self.spawns, enemy)
                    && enemy.high_priority == shot.high_priority
                    && hitbox.collides(enemy_hitbox(&self.mappings, enemy))
                    && {
                        let kind = self.spawns[enemy.spawn].kind;
                        if self.roster.hit(kind, enemy) == HitResponse::Reflect {
                            // $12984 turns shurikens off the 4-1 guard's
                            // shield; $2B126 turns them off Boss2's field.
                            shot.turn = shot.turn.or(Some(if kind == 0x18 {
                                Turn::guard(shot.direction, self.ticks)
                            } else {
                                Turn::away(shot.direction, y, enemy.y)
                            }));
                            false
                        } else {
                            true
                        }
                    }
            });
            if let Some(enemy) = target {
                // +$06 = 1 at $24ECC, 3 powered.
                let strike = Strike {
                    origin: (x, y),
                    damage: shot.damage,
                    cue: shuriken_hit, // $24DE0
                    shuriken: true,
                    board: false,
                    bounce: false,
                    hold: None,
                    serial: 0,
                };
                shot.held = Self::strike_enemy(
                    &self.roster,
                    &self.spawns[enemy.spawn],
                    enemy,
                    strike,
                    (self.deferring, Some(&mut self.holds)),
                    event,
                );
                if shot.held.is_none() {
                    shot.age = u16::MAX;
                }
            }
            // Spent or waiting for its hit, which frees it first.
            if shot.age != u16::MAX && shot.held.is_none() {
                Self::spread_terrain_at(terrain, shot, &mut walls, &mut splashes);
            }
            if shot.age == u16::MAX {
                continue;
            }
            // +$3A = 255 at $24EDC; offscreen objects are removed. A held
            // shuriken waits for its hit.
            if shot.held.is_none()
                && (shot.age >= 255
                    || !(camera.0 + view.0 - 64..=camera.0 + view.1 + 64).contains(&x)
                    || !(camera.1 - 64..=camera.1 + 288).contains(&y))
            {
                shot.age = u16::MAX;
            }
        }
        self.weapons.shots.retain(|shot| shot.age != u16::MAX);
        for (x, y, left) in splashes {
            self.splash_shuriken((x, y), left, event);
        }
        for wall in walls {
            self.strike_wall(wall, event);
        }
        self.weapons.swing_box = None;

        let Some(mut swing) = self.weapons.swing.take() else {
            return;
        };
        let spec = swing.kind.spec();
        let age = swing.age;
        swing.age += 1;
        if age > 0
            && self
                .weapons
                .player_state
                .is_some_and(|state| !spec.states.contains(&state))
        {
            return; // $25162/$24C8E delete the object with the state.
        }
        let diving = swing.kind == Sword::AirStab && self.weapons.diving;
        // PlayerAttack_Run ($24A8E..$24AAA) counts +$3A down after the
        // routine and frees the record when it reaches zero, before
        // Object_LoadHitbox: the update `life - 1` after creation leaves
        // no box (1-1: a slash made on frame 167 is gone on 181). The dive
        // holds +$3A at 11 ($25342).
        if age + 1 >= spec.life && !diving {
            return;
        }
        // $24CEC/$25530: the swing sound plays on the mapping word with bit 15
        // set when +$3A & 3 is 3: effect 3 for objects 9/11/13/17 and the
        // mounted 3/5 ($24D02), effect $1A for the powered 10/12/14/18 and
        // 4/6, the DASH ATTACK voice for object 19 ($25544). The air stab's
        // words have no bit 15; its voice came with the attack.
        if age == spec.sound_at {
            let cue = match swing.kind {
                Sword::Standing
                | Sword::Crouching
                | Sword::Airborne
                | Sword::Hanging
                | Sword::Horse
                | Sword::Surf => Some(SoundEvent::Slash),
                Sword::StandingPowered
                | Sword::CrouchingPowered
                | Sword::AirbornePowered
                | Sword::HangingPowered
                | Sword::HorsePowered
                | Sword::SurfPowered => Some(SoundEvent::PoweredSlash),
                Sword::DashKick => Some(SoundEvent::DashAttack),
                Sword::AirStab | Sword::SurfStab => None,
            };
            if let Some(cue) = cue {
                event.sound_ids.push(self.rules.cues.get(cue));
            }
        }
        // Outside its active entries the object shows mapping 0, whose
        // empty box `Object_CollideList` still tests as a point.
        let hitbox = self.sword_box(swing.kind, age, diving, player, facing_left);
        self.weapons.swing_box = Some(hitbox);
        // $24C9A (objects 9/13 and every powered object, whose
        // routines all end in loc_024C86), $251F6 (crouching object
        // 11), $252F2 (air stab), $254E0 (dash kick).
        let cue = self.rules.cues.get(swing.kind.hit_sound());
        // `Object_CollideList` ($1950..$19D2) stops at the first overlap
        // and sets bit 14 on the sword, which `flags & $E800 == $8000`
        // then excludes for every later object this frame: one enemy,
        // the first in list order, per update.
        // Boss2's field clears the sword's bit 14 (`$2B126`): it is
        // neither spent nor heard, and meets the next object.
        let target = self.enemies.iter_mut().find(|enemy| {
            Self::attack_targetable(&self.roster, &self.spawns, enemy)
                && enemy.high_priority == self.weapons.high_priority
                && enemy.attack_serial != swing.serial
                && hitbox.collides(enemy_hitbox(&self.mappings, enemy))
                && self.roster.hit(self.spawns[enemy.spawn].kind, enemy) != HitResponse::Reflect
        });
        if let Some(enemy) = target {
            let strike = Strike {
                origin: self.weapons.sword_origin(swing.kind, player, facing_left),
                damage: spec.damage,
                cue,
                shuriken: false,
                board: swing.kind == Sword::SurfStab,
                bounce: matches!(swing.kind, Sword::AirStab | Sword::SurfStab),
                hold: None,
                serial: swing.serial,
            };
            swing.held = Self::strike_enemy(
                &self.roster,
                &self.spawns[enemy.spawn],
                enemy,
                strike,
                (self.deferring, Some(&mut self.holds)),
                event,
            );
            // Landed at once: bit 14 hides the sword from the list-6
            // and list-4 checks for the rest of the frame, and every
            // sword but the dash kick is spent. Deferred, the sword runs
            // on until `land_strikes` sees whether the hit lands.
            if swing.held.is_none() {
                self.weapons.swing_box = None;
                if swing.kind != Sword::DashKick {
                    return;
                }
            }
        }
        self.weapons.swing = Some(swing);
    }
    /// `Shuriken_Update`'s terrain test (`$24DF8..$24E66`) where a spread
    /// shuriken stands: a stopping material or a negative X ends it.
    pub(in crate::combat) fn spread_terrain_at(
        terrain: &impl Terrain,
        shot: &mut SpreadShot,
        walls: &mut Vec<WallHit>,
        splashes: &mut Vec<(i32, i32, bool)>,
    ) {
        let at = (shot.x >> 16, shot.y >> 16);
        match PlayerWeapons::shuriken_terrain(terrain, at, &mut shot.in_water) {
            ShurikenTerrain::Fly => {}
            ShurikenTerrain::Splash => splashes.push((at.0, at.1, shot.velocity_x < 0)),
            ShurikenTerrain::Leave => shot.age = u16::MAX,
            ShurikenTerrain::Wall => {
                walls.push(WallHit {
                    at,
                    velocity: (shot.velocity_x, shot.velocity_y),
                    mapping: shot.mapping,
                    high_priority: shot.high_priority,
                    order: shot.order,
                });
                shot.age = u16::MAX;
            }
        }
    }
    /// `$24E24..$24E66`: entering material 15 plays effect `$27` and
    /// allocates list-1 type `$24` (`Func_022D2E`) at the shuriken with
    /// `+$2A` = `$18` moving right, `$24` left: the splash of wading
    /// (`combat::ambient::splash`).
    pub(in crate::combat) fn splash_shuriken(
        &mut self,
        (x, y): (i32, i32),
        left: bool,
        event: &mut CombatEvents,
    ) {
        event
            .sound_ids
            .push(self.rules.cues.get(SoundEvent::ShurikenSplash));
        let wade = crate::movement::SplashKind::Wade { left };
        if let Some(splash) =
            crate::combat::ambient::splash(&self.rules.ambient, (x, y), wade.group())
        {
            self.effects.push(splash);
        }
    }
    /// `$24E0A..$24E22`: a shuriken on a stopping material. Powered
    /// (`plr_status` bit 7) it is removed. Otherwise `Frame_Counter` bit 0
    /// picks: clear sticks it for 32 ticks (`loc_024C6C`), set knocks it back
    /// (`Func_024AFE`). The combat tick count stands in for the counter.
    pub(in crate::combat) fn strike_wall(&mut self, wall: WallHit, event: &mut CombatEvents) {
        if self.weapons.powered {
            return;
        }
        let (vx, vy) = wall.velocity;
        let leftward = vx < 0;
        let mut spent = SpentShuriken {
            x: wall.at.0,
            y: wall.at.1,
            mapping: wall.mapping,
            flip: if leftward { 0x800 } else { 0 },
            motion: Motion::default(),
            life: 32,
            spin: None,
            high_priority: wall.high_priority,
            order: wall.order,
        };
        if self.ticks % 2 == 1 {
            // Func_024AFE: effect 8, a quarter of the speed back, 4 px/tick
            // up ($3A 32) or, if it was rising, no vertical speed ($3A 24),
            // under $FF0014 gravity; +$28 is the spread's (or throw's)
            // starting frame, mirrored when thrown left.
            event
                .sound_ids
                .push(self.rules.cues.get(SoundEvent::ShurikenBounce));
            let frame = usize::from(wall.mapping - 0xca);
            spent.spin = Some((
                if vx < 0 { -1 } else { 1 },
                if leftward { 14 - frame } else { frame },
            ));
            spent.motion.vx = -vx >> 2;
            spent.motion.vy = if vy < 0 { 0 } else { -4 << 16 };
            spent.motion.gravity = GRAVITY;
            if vy < 0 {
                spent.life = 24;
            }
        }
        // PlayerAttack_Run counts +$3A down in the same update.
        spent.life -= 1;
        self.weapons.spent.push(spent);
    }
    /// Stuck and knocked-back shurikens: `Func_024B5E` turns the spin every
    /// fourth tick, the object moves, then `PlayerAttack_Run` counts `+$3A`
    /// down and frees it at zero.
    pub(in crate::combat) fn update_spent_shurikens(&mut self) {
        let frames = self.weapons.spin_frames;
        for spent in &mut self.weapons.spent {
            if let Some((step, index)) = &mut spent.spin
                && spent.life & 3 == 0
            {
                let next = *index as i32 + *step;
                *index = if next < 0 {
                    27
                } else if next >= 28 {
                    0
                } else {
                    next as usize
                };
                let frame = frames[*index];
                spent.mapping = 0xca + (frame & 7);
                spent.flip = frame & 0x1800;
            }
            spent.motion.step(&mut spent.x, &mut spent.y);
            spent.life -= 1;
        }
        self.weapons.spent.retain(|spent| spent.life > 0);
    }
    /// The sword object's hitbox `age` updates after creation, if its mapping
    /// entry is active then. The object follows the player each update.
    pub(in crate::combat) fn swing_hitbox(
        &self,
        kind: Sword,
        age: u16,
        diving: bool,
        player: (i32, i32),
        facing_left: bool,
    ) -> Option<Hitbox> {
        let spec = kind.spec();
        if !spec.active.contains(&age) && !(diving && age >= *spec.active.start()) {
            return None;
        }
        let (x, y) = self.weapons.sword_origin(kind, player, facing_left);
        Some(
            self.mappings[&spec.mapping]
                .bounds
                .placed(x, y, facing_left, false),
        )
    }
    /// The box `Object_LoadHitbox` leaves the sword object `age` updates
    /// after creation: its active mapping's ([`Self::swing_hitbox`]), or
    /// else mapping 0's, which is empty (`0,0,0,0`) but still meets any box
    /// around the object's origin in `Object_CollideList`.
    pub(in crate::combat) fn sword_box(
        &self,
        kind: Sword,
        age: u16,
        diving: bool,
        player: (i32, i32),
        facing_left: bool,
    ) -> Hitbox {
        self.swing_hitbox(kind, age, diving, player, facing_left)
            .unwrap_or_else(|| {
                let (x, y) = self.weapons.sword_origin(kind, player, facing_left);
                self.mapping_hitbox(0, x, y, facing_left)
            })
    }
}
