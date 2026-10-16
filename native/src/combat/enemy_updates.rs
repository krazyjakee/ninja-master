use super::*;

impl Combat {
    /// Enemy state machines and effects, in object-list order.
    pub(super) fn update_enemies(
        &mut self,
        player: (i32, i32),
        facing_left: bool,
        camera: (i32, i32),
        terrain: &impl Terrain,
        event: &mut CombatEvents,
    ) {
        self.weapon_contacts.clear();
        let others: Vec<Snapshot> = self.enemies.iter().map(Snapshot::of).collect();
        let live = self
            .enemies
            .iter()
            .filter(|enemy| {
                enemy.live() && self.spawns[enemy.spawn].kind != crate::bosses::BOSS_KIND
            })
            .count();
        // List 3 as `Func_012BFE` scans it: thrown and spread shurikens and
        // the sword object, which follows the player.
        let mut attacks: Vec<(i32, i32, bool)> = self
            .projectiles
            .iter()
            .filter(|shot| !shot.enemy)
            .map(|shot| (shot.x, shot.y, shot.high_priority))
            .chain(
                self.weapons
                    .shots
                    .iter()
                    .map(|shot| (shot.x >> 16, shot.y >> 16, shot.high_priority)),
            )
            .collect();
        attacks.extend(
            self.weapons
                .sword_position(player, facing_left)
                .map(|(x, y)| (x, y, self.weapons.high_priority)),
        );
        // `Object_CollidePlayer`'s view of the player: where its update
        // left it before `Object_Integrate`, unless hidden or raised.
        let player_at = self
            .weapons
            .anchors
            .map_or(player, |anchors| anchors.unmoved);
        let player_body = (!self.player_hidden && !self.weapons.high_priority)
            .then(|| self.player_body(player_at, facing_left));
        let view = self.screen_view(camera.0);
        let mut cx = Ctx {
            weapon_contacts: &mut self.weapon_contacts,
            shared: &self.rules,
            mappings: &self.mappings,
            bases: &self.bases,
            effects: &mut self.effects,
            projectiles: &mut self.projectiles,
            projectile_scratch: &mut self.projectile_scratch,
            random: &mut self.random,
            sounds: &mut event.sound_ids,
            score: &mut self.hud.score,
            score_suppressed: self.score_suppressed,
            level: self.level,
            children: Vec::new(),
            others,
            live,
            attacks: &attacks,
            // `Object_DistanceToPlayer` ($126F2) reads the player's obj_x/
            // obj_y before `Object_Integrate` adds this update's step
            // (`Objects_BuildSprites`): where the last update left it.
            player: player_at,
            player_input: self.player_input,
            camera,
            view,
            camera_locked: self.camera_locked,
            mech_hand_contact: self.mech_hand_contact,
            star_damage: self.enemy_projectile_damage,
            throw_sound: self.sound_commands[1],
            current: 0,
            spawn_flags: 0,
            signals: Vec::new(),
            player_event: None,
            landed_shot: self.landed_shot.take(),
            mutants: &mut self.mutants,
            player_body,
            player_state: self.weapons.player_state,
            player_airborne: self.weapons.airborne,
            player_facing_left: facing_left,
            player_at,
            player_contact: None,
            surf_jump: None,
        };
        for (index, enemy) in self.enemies.iter_mut().enumerate() {
            let spawn = &self.spawns[enemy.spawn];
            cx.current = index;
            let dormant = enemy.state == EnemyState::Dormant;
            let allocated = cx.children.len();
            enemies::run(&self.roster, &mut cx, enemy, spawn, self.ticks, terrain);
            // Boss5 runs before list 5: its parts read this frame's position
            // and commands, not the previous frame's snapshot.
            if spawn.handler == 0x30654 {
                cx.others[index] = Snapshot::of(enemy);
            }
            // A spawn record's activation, then the records its update
            // allocated, join the tail of the list in that order.
            if dormant && enemy.state != EnemyState::Dormant {
                self.allocations += 1;
                enemy.order = self.allocations;
            }
            for child in &mut cx.children[allocated..] {
                self.allocations += 1;
                child.enemy.order = self.allocations;
            }
        }
        // `Object_AllocFromList` links a new record at the tail of list 5
        // and `Objects_RunEnemyList` ($126C2) reads each record's `obj_next`
        // after its handler returns, so what the pass allocated runs in the
        // same pass, after every older record (and so do its own children).
        // Their parents have already updated: they see them as they are now.
        while !cx.children.is_empty() {
            let adopted = adopt(
                &self.rules,
                &mut self.enemies,
                &mut self.spawns,
                std::mem::take(&mut cx.children),
            );
            cx.live += adopted.len();
            cx.others = self.enemies.iter().map(Snapshot::of).collect();
            for index in adopted {
                let enemy = &mut self.enemies[index];
                cx.current = index;
                let allocated = cx.children.len();
                enemies::run(
                    &self.roster,
                    &mut cx,
                    enemy,
                    &self.spawns[enemy.spawn],
                    self.ticks,
                    terrain,
                );
                for child in &mut cx.children[allocated..] {
                    self.allocations += 1;
                    child.enemy.order = self.allocations;
                }
            }
        }
        let Ctx {
            signals,
            player_event,
            player_contact,
            surf_jump,
            ..
        } = cx;
        self.object_contact = player_contact;
        event.surf_jump = surf_jump;
        for signal in signals {
            match signal {
                // The raise precedes this update's player collisions.
                Signal::RaisePlayerPriority => self.raise_player_priority(),
                // Func_0373E6: the part box against the player's.
                Signal::Shove(part) => {
                    if self.player_body(player, facing_left).collides(part) {
                        self.shove = Some(shove::Shove::new(part));
                    }
                }
                _ => event.signals.push(signal),
            }
        }
        event.player_event = player_event;
    }
    /// Enemy-side reaction to a hit ($127C0). The pose, knockback, sparks and
    /// death sequence start in the enemy's next update from `reaction`.
    pub(super) fn hurt_enemy(
        roster: &Roster,
        enemy: &mut Enemy,
        spawn: &Spawn,
        damage: u8,
        show_death_spark: bool,
    ) {
        // Handlers call $127C0 only in live states.
        if !Self::hittable(enemy) {
            return;
        }
        enemy.spark_handled = false;
        let response = roster.hit_by(spawn.kind, enemy, damage);
        match response {
            HitResponse::Pass
            | HitResponse::Ignore
            | HitResponse::BodyOnly
            | HitResponse::Reflect
            | HitResponse::Collides => return,
            HitResponse::Deflect => {
                enemy.reaction = Some(Reaction::Deflect);
                return;
            }
            HitResponse::Accept | HitResponse::WeaponsOnly | HitResponse::NoDeathSpark => {}
        }
        let taken = enemy.health.min(u16::from(damage));
        enemy.health -= taken;
        enemy.damage_taken = enemy.damage_taken.saturating_add(taken);
        if enemy.health == 0 {
            enemy.state = EnemyState::Defeated;
            enemy.dying = true;
            enemy.death_spark = show_death_spark && response.death_spark();
            enemy.pending_score = Self::score(spawn.score_bcd);
            enemy.reaction = Some(Reaction::Death);
        } else {
            enemy.state = EnemyState::Hurt;
            enemy.reaction = Some(Reaction::Hurt);
        }
    }
    /// A player attack met `enemy`: land the hit now, or on the next pass
    /// when this one runs after the player (see `update_objects`). A
    /// deferred hit by an attack that its hit frees returns the
    /// [`Strike::hold`] from `holds` that the attack keeps until then.
    pub(super) fn strike_enemy(
        roster: &Roster,
        spawn: &Spawn,
        enemy: &mut Enemy,
        mut strike: Strike,
        (defer, holds): (bool, Option<&mut u32>),
        event: &mut CombatEvents,
    ) -> Option<u32> {
        if defer {
            if let Some(holds) = holds {
                *holds = holds.wrapping_add(1);
                strike.hold = Some(*holds);
            }
            enemy.strike = Some(strike);
            strike.hold
        } else {
            Self::land_strike(roster, spawn, enemy, strike, event);
            None
        }
    }
    pub(super) fn land_strike(
        roster: &Roster,
        spawn: &Spawn,
        enemy: &mut Enemy,
        strike: Strike,
        event: &mut CombatEvents,
    ) {
        enemy.struck = true;
        if strike.serial != 0 {
            enemy.attack_serial = strike.serial;
        }
        let before = enemy.health;
        Self::hurt_enemy(roster, enemy, spawn, strike.damage, true);
        if enemy.health < before {
            event.hits.push(crate::profile::Hit {
                target: spawn.kind,
                defeated: enemy.health == 0,
                kind: if strike.shuriken {
                    crate::profile::HitKind::Shuriken
                } else if strike.board {
                    crate::profile::HitKind::Board
                } else if strike.bounce {
                    crate::profile::HitKind::DeathKick
                } else {
                    crate::profile::HitKind::Melee
                },
            });
        }
        match enemy.reaction {
            Some(Reaction::Death) if enemy.death_spark => {
                enemy.hit_spark = Some((strike.origin, true));
                enemy.spark_handled = true;
            }
            Some(Reaction::Hurt) => {
                enemy.hit_spark = Some((strike.origin, false));
                enemy.spark_handled = true;
            }
            _ => {}
        }
        if !strike.shuriken || enemy.reaction != Some(Reaction::Deflect) {
            event.sound_ids.push(strike.cue);
        }
        event.bounce |= strike.bounce;
    }
    /// The hits the last pass found, after this pass's object updates:
    /// each object still able to take one does, and skips its player test.
    /// The attack is freed with it, as its own update does in the ROM
    /// (`$24DD6` shurikens, `$24C92` swords, `$252E4` the air stab), after
    /// the frame it was drawn meeting the object. An attack whose hit does
    /// not land (the object no longer takes hits, or a new life cleared the
    /// attack) runs on: a shuriken flies on, a sword keeps its routine.
    pub(super) fn land_strikes(&mut self, event: &mut CombatEvents) {
        let live = self.held_attacks();
        let mut landed = Vec::new();
        for enemy in &mut self.enemies {
            let Some(strike) = enemy.strike.take() else {
                continue;
            };
            if strike.hold.is_some_and(|hold| !live.contains(&hold))
                || !Self::weapon_targetable(&self.roster, &self.spawns, enemy)
            {
                continue;
            }
            let spawn = &self.spawns[enemy.spawn];
            Self::land_strike(&self.roster, spawn, enemy, strike, event);
            landed.extend(strike.hold);
        }
        self.free_held_attacks(&landed);
    }
    /// A weapon target without a hit waiting from the last pass: one
    /// `Object_CollideL3` overlap per object update.
    pub(super) fn attack_targetable(roster: &Roster, spawns: &[Spawn], enemy: &Enemy) -> bool {
        enemy.strike.is_none() && Self::weapon_targetable(roster, spawns, enemy)
    }
    /// A live object that the player's weapons collide with now.
    pub(super) fn weapon_targetable(roster: &Roster, spawns: &[Spawn], enemy: &Enemy) -> bool {
        Self::hittable(enemy)
            && roster
                .hit(spawns[enemy.spawn].kind, enemy)
                .weapon_collides()
    }
    /// A live object whose body collides with the player's now.
    pub(super) fn body_targetable(roster: &Roster, spawns: &[Spawn], enemy: &Enemy) -> bool {
        let kind = spawns[enemy.spawn].kind;
        Self::hittable(enemy)
            && roster.touches_player(kind)
            && roster.hit(kind, enemy).body_collides()
    }
    pub(super) fn hittable(enemy: &Enemy) -> bool {
        !matches!(
            enemy.state,
            EnemyState::Dormant | EnemyState::Emerging | EnemyState::Defeated
        )
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;
    #[test]
    #[ignore = "requires a user-supplied cartridge"]
    fn only_landed_damage_counts_a_hit_and_lethal_damage_counts_once() {
        let dir = std::env::var_os("NINJA_MASTER_TEST_ROMS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
        let rom = crate::rom::regions::prepare(std::fs::read(dir.join("shinobi3_us.bin")).unwrap())
            .unwrap();
        let mut combat = Combat::load_stage(&rom, crate::stage::StageId::FOREST).unwrap();
        let index = combat
            .enemies
            .iter()
            .position(|e| combat.spawns[e.spawn].kind == 0x14)
            .unwrap();
        let enemy = &mut combat.enemies[index];
        let spawn = &combat.spawns[enemy.spawn];
        let strike = Strike {
            origin: (0, 0),
            damage: 1,
            cue: 0,
            shuriken: false,
            board: false,
            bounce: false,
            hold: None,
            serial: 1,
        };
        let mut events = CombatEvents::default();
        Combat::land_strike(&combat.roster, spawn, enemy, strike, &mut events);
        assert!(events.hits.is_empty()); // dormant
        enemy.state = EnemyState::Waiting;
        enemy.health = 2;
        Combat::land_strike(&combat.roster, spawn, enemy, strike, &mut events);
        assert_eq!(events.hits.len(), 1);
        assert!(!events.hits[0].defeated);
        Combat::land_strike(&combat.roster, spawn, enemy, strike, &mut events);
        assert_eq!(events.hits.len(), 1); // hurt immunity
        enemy.state = EnemyState::Waiting;
        Combat::land_strike(&combat.roster, spawn, enemy, strike, &mut events);
        assert_eq!(events.hits.len(), 2);
        assert!(events.hits[1].defeated);
        Combat::land_strike(&combat.roster, spawn, enemy, strike, &mut events);
        assert_eq!(events.hits.len(), 2);
    }
}
