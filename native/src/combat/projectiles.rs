use super::*;

impl Combat {
    /// `Shuriken_Update`'s terrain test (`$24DF8..$24E66`) where a thrown
    /// shuriken stands, with its own material table rather than `$3854`'s
    /// D2: a stopping material or a negative X ends it.
    pub(super) fn shuriken_terrain_at(
        terrain: &impl Terrain,
        shot: &mut Projectile,
        walls: &mut Vec<WallHit>,
        splashes: &mut Vec<(i32, i32, bool)>,
    ) {
        let at = (shot.x, shot.y);
        match PlayerWeapons::shuriken_terrain(terrain, at, &mut shot.in_water) {
            ShurikenTerrain::Fly => {}
            ShurikenTerrain::Splash => splashes.push((shot.x, shot.y, shot.leftward())),
            ShurikenTerrain::Leave => shot.alive = false,
            ShurikenTerrain::Wall => {
                walls.push(WallHit {
                    at,
                    velocity: ((shot.velocity_x << 16) + shot.motion.vx, shot.motion.vy),
                    mapping: shot.mapping,
                    high_priority: shot.high_priority,
                    order: shot.order,
                });
                shot.alive = false;
            }
        }
    }
    /// $18BC pushes a freed record on the free-list head without clearing
    /// +$28. Keep that word for the next native list-4 allocation.
    pub(super) fn retain_projectiles(&mut self, touching: bool) {
        self.projectiles.retain(|shot| {
            let keep = shot.alive || (touching && shot.touching);
            if !keep && shot.scratch_slot {
                self.projectile_scratch.push(shot.animation_counter);
            }
            keep
        });
    }
    /// One update of the enemy shots (list 4) or of the thrown shurikens.
    pub(super) fn update_projectiles(
        &mut self,
        enemy_shots: bool,
        body: Hitbox,
        camera: (i32, i32),
        terrain: &impl Terrain,
        event: &mut CombatEvents,
    ) {
        let view = self.screen_view(camera.0);
        let shuriken_hit = self.rules.cues.get(SoundEvent::ShurikenHit);
        let mut walls = Vec::new();
        let mut splashes = Vec::new();
        for shot in &mut self.projectiles {
            if shot.enemy != enemy_shots {
                continue;
            }
            shot.touching = false;
            // Removed by an enemy this tick (deflected, or its owner died).
            if !shot.alive {
                continue;
            }
            shot.age = shot.age.saturating_add(1);
            if let Some(segments) = &shot.sparkle_trail
                && shot.age.is_multiple_of(2)
            {
                // $1B646 calls $1AE84 before movement. The shot drives
                // these list-9 records even after its battery is destroyed.
                let mut start = 0;
                for &(mapping, ticks) in segments {
                    let mut sparkle = Effect::new(EffectKind::Pieces, shot.x, shot.y, mapping);
                    sparkle.base = 0x8000;
                    sparkle.high_priority = true;
                    sparkle.back = true;
                    sparkle.delay = start + 1;
                    sparkle.counter = i32::from(ticks);
                    self.effects.push(sparkle);
                    start += ticks;
                }
            }
            if shot
                .flip_period
                .is_some_and(|n| n > 0 && shot.age.is_multiple_of(n))
            {
                shot.base ^= 0x1000;
            }
            if let Some((period, mapping)) = shot.trail.filter(|(period, _)| *period > 0)
                && shot.age.is_multiple_of(period)
            {
                let kind = if mapping == 0 {
                    EffectKind::Puff
                } else {
                    EffectKind::Pieces
                };
                let mut trail = Effect::new(kind, shot.x, shot.y, mapping);
                trail.base = shot.base;
                trail.back = true;
                trail.flip = shot.leftward() != shot.faces_left;
                trail.blink = mapping != 0;
                trail.counter = 8;
                self.effects.push(trail);
            }
            shot.animation_counter = shot.animation_counter.wrapping_add(1);
            // $1CDE4 reads the visibility flag left by the preceding sprite
            // pass, before motion. Allocation clears it even off screen.
            if shot.enemy && !shot.left_edge_only && shot.offscreen {
                shot.alive = false;
                continue;
            }
            // `Object_LoadHitbox` at the top of most list-4 updates reads the
            // frame and flips the last update left drawn (enemy handlers that
            // animate a shot of theirs do so after it, in the ROM's handler).
            let held = if shot.enemy {
                shot.held_frame.take().unwrap_or_else(|| shot.drawn_frame())
            } else {
                shot.drawn_frame()
            };
            // Func_024BE0: a shuriken Boss2's field turned steers on every
            // other update and flies on its 16.16 velocity.
            if let Some(mut turn) = shot.turn.filter(|turn| turn.left > 0)
                && shot.age.is_multiple_of(2)
            {
                let (mapping, flips, vx, vy) = self.weapons.steer(&mut turn);
                shot.turn = Some(turn);
                shot.mapping = mapping;
                shot.velocity_x = 0;
                shot.motion.vx = vx;
                shot.motion.vy = vy;
                shot.base = (shot.base & !0x1000) | (flips & 0x1000);
                shot.faces_left = shot.leftward() != (flips & 0x800 != 0);
            }
            // $1D204: only the ninja star (projectile 4) cycles its mappings.
            if shot.enemy && (0x35b..=0x35e).contains(&shot.mapping) {
                shot.mapping = 0x35b + ((shot.animation_counter >> 1) & 3);
            }
            if let Some(frames) = shot.frames {
                let (mapping, flip) = frames[usize::from((shot.age >> 2) & 3)];
                shot.mapping = mapping;
                // Drawn flipped exactly when the frame says so.
                shot.faces_left = shot.leftward() != flip;
            }
            if let Some(mut sway) = shot.sway.take() {
                let stays = sway.update(shot);
                shot.sway = Some(sway);
                if !stays {
                    shot.alive = false;
                    continue;
                }
            }
            if !shot.enemy && std::mem::take(&mut shot.untested) {
                // Its hit held and did not land: `Shuriken_Update` found
                // no collision bit (`loc_024DD0`) and tests the terrain
                // where the step ended before the shuriken moves on.
                Self::shuriken_terrain_at(terrain, shot, &mut walls, &mut splashes);
                if !shot.alive {
                    continue;
                }
            }
            let moving = shot.motion != Motion::default();
            if !shot.enemy {
                // `Object_Integrate` in `Objects_BuildSprites` moves the
                // shuriken a whole step, and only where the step ends does
                // anything meet it: the objects' `Object_CollideL3` on the
                // next frame, then `Shuriken_Update`'s terrain test.
                shot.x += shot.velocity_x;
                if moving {
                    shot.motion.step(&mut shot.x, &mut shot.y);
                }
                let hitbox = Self::shot_box(&self.mappings, shot);
                for enemy in &mut self.enemies {
                    if !Self::attack_targetable(&self.roster, &self.spawns, enemy)
                        || shot.high_priority != enemy.high_priority
                        || !hitbox.collides(enemy_hitbox(&self.mappings, enemy))
                    {
                        continue;
                    }
                    let kind = self.spawns[enemy.spawn].kind;
                    if self.roster.hit(kind, enemy) == HitResponse::Reflect {
                        // `Shuriken_Init` left +$28 0 or $E. The guard's
                        // shield and Boss2's field select different turns.
                        let direction = if shot.leftward() { 0xe } else { 0 };
                        shot.turn = shot.turn.or(Some(if kind == 0x18 {
                            Turn::guard(direction, self.ticks)
                        } else {
                            Turn::away(direction, shot.y, enemy.y)
                        }));
                        continue;
                    }
                    let strike = Strike {
                        origin: (shot.x, shot.y),
                        damage: shot.damage,
                        cue: shuriken_hit, // $24DE0
                        shuriken: true,
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
                    shot.alive = shot.held.is_some();
                    break;
                }
                // Spent or waiting for its hit, which frees it first.
                if shot.alive && shot.held.is_none() {
                    Self::shuriken_terrain_at(terrain, shot, &mut walls, &mut splashes);
                }
            } else if std::mem::take(&mut shot.walled) {
                if shot.bounce_dart() {
                    shot.motion.step(&mut shot.x, &mut shot.y);
                    continue;
                }
                if let Some(sound) = shot.wall_sound.take() {
                    event.sound_ids.push(sound);
                }
                // `$1CDE4`: the handler meets the wall where the last step
                // left the shot (drawn there on that frame) and frees it
                // with `$3690C`'s spark. List 1 runs after list 4, so the
                // spark has its first update (hidden, `$35F`) on this frame
                // (the effect pass) and is first drawn on the next.
                if let Some(blast) = shot.contact_blast {
                    enemies::explode(
                        &self.rules,
                        &mut self.random,
                        &mut self.effects,
                        &mut event.sound_ids,
                        (shot.x, shot.y),
                        blast,
                        shot.high_priority,
                    );
                } else {
                    let mut spark = Effect::new(
                        EffectKind::WallSpark,
                        shot.x,
                        shot.y,
                        self.rules.dust_seq[0],
                    );
                    spark.blink = true;
                    spark.high_priority = shot.high_priority;
                    spark.base = if shot.high_priority { 0x8000 } else { 0 };
                    self.effects.push(spark);
                }
                shot.alive = false;
            } else {
                // `Object_Integrate` in `Objects_BuildSprites` moves the shot
                // a whole step, and only where it ends is it tested: by the
                // player's `Player_CheckDamage` (`Object_CollideL4`, strict,
                // no size check) on the next frame, then by its own next
                // update's terrain test there. A shot that crosses the body
                // inside a step misses it.
                shot.x += shot.velocity_x;
                if moving {
                    shot.motion.step(&mut shot.x, &mut shot.y);
                }
                // The box this update loaded, met where the step ends.
                shot.box_frame = Some(match shot.box_load {
                    ShotBox::Before => held,
                    ShotBox::After | ShotBox::Fixed(_) => shot.drawn_frame(),
                });
                shot.held_frame = Some(shot.drawn_frame());
                // The player's damage check decides whether it hits.
                shot.touching = shot.high_priority == self.weapons.high_priority
                    && !shot.no_collision
                    && Self::shot_box(&self.mappings, shot).collides(body);
                let material = terrain.material_at(shot.x, shot.y);
                // Enemy shots pass platforms; a wall leaves `$3690C`.
                let terrain_hit = material.object_solid(shot.high_priority)
                    && material != movement::Material::PLATFORM;
                shot.walled = !shot.passes_terrain && terrain_hit;
            }
            // A held shuriken waits for its hit.
            if shot.held.is_some() {
                continue;
            }
            // Enemy shots use Object_UpdateVisibility: ±256 px from the
            // screen centre on either axis. Specialized handlers may also
            // install an explicit lifetime.
            let outside = if shot.left_edge_only {
                shot.x < camera.0 + view.0
            } else if shot.enemy {
                shot.offscreen = outside_view_in((shot.x, shot.y), camera, view);
                false
            } else {
                shot.x < camera.0 + view.0 - 64 || shot.x > camera.0 + view.1 + 64
            };
            if shot.life != u16::MAX && shot.age >= shot.life || outside {
                shot.alive = false;
            }
            if let Some(home) = shot.home {
                // Back within 8 pixels of home after turning ($1A934).
                let returning = (shot.motion.vx < 0) == (shot.motion.ax < 0);
                if returning && (home - shot.x).abs() < 8 {
                    shot.alive = false;
                }
            }
            if !shot.enemy && moving && (shot.y < camera.1 - 64 || shot.y > camera.1 + 288) {
                shot.alive = false;
            }
        }
        for (x, y, left) in splashes {
            self.splash_shuriken((x, y), left, event);
        }
        for wall in walls {
            self.strike_wall(wall, event);
        }
    }
}
