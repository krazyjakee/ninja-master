use super::*;

/// A list-3 player attack object (`Object_List3`) as the list-6 and list-4
/// collision helpers (`$194A`) see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttackObject {
    /// A thrown shuriken (list-3 type 0), by `Combat::projectiles` index.
    Shot(usize),
    /// A somersault spread shuriken (type 1), by `PlayerWeapons::shots` index.
    Spread(usize),
    /// The sword object (types 9 and up).
    Sword,
}
impl AttackObject {
    /// `cmpi.w #4,obj_type` / `bls` (`$297A6`): types 0 and 1 are shurikens.
    pub(crate) fn shuriken(self) -> bool {
        !matches!(self, Self::Sword)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlayerAttack {
    pub(crate) object: AttackObject,
    pub(crate) hitbox: Hitbox,
    pub(crate) high_priority: bool,
}

impl Combat {
    /// A shot's box where it stands (`Object_LoadHitbox`): a player
    /// shuriken's, flipped by its drawn `obj_tile` bits, or the box an enemy
    /// shot's last update loaded ([`crate::combat::ShotBox`]).
    pub(in crate::combat) fn shot_box(
        mappings: &BTreeMap<u16, Mapping>,
        shot: &Projectile,
    ) -> Hitbox {
        if let (true, crate::combat::ShotBox::Fixed(bounds)) = (shot.enemy, shot.box_load) {
            return bounds.placed(shot.x, shot.y, false, false);
        }
        let (mapping, horizontal, vertical) = match shot.box_frame {
            Some(frame) if shot.enemy => frame,
            _ => shot.drawn_frame(),
        };
        // Mapping 0 and any empty record load `0,0,0,0`: a point.
        mappings
            .get(&mapping)
            .map_or(Hitbox::NONE, |m| m.bounds)
            .placed(shot.x, shot.y, horizontal, vertical)
    }
    /// A spread shuriken's box where it stands (`Object_LoadHitbox`).
    pub(super) fn spread_box(mappings: &BTreeMap<u16, Mapping>, shot: &SpreadShot) -> Hitbox {
        let (horizontal, vertical) = shot.tile_flips();
        mappings.get(&shot.mapping).map_or(Hitbox::NONE, |m| {
            m.bounds
                .placed(shot.x >> 16, shot.y >> 16, horizontal, vertical)
        })
    }
    /// The player's live attack objects and their hitboxes, in list order.
    pub(crate) fn player_attacks(&self) -> Vec<PlayerAttack> {
        let mut attacks = Vec::new();
        for (index, shot) in self.projectiles.iter().enumerate() {
            if shot.enemy || !shot.alive {
                continue;
            }
            attacks.push(PlayerAttack {
                object: AttackObject::Shot(index),
                hitbox: Self::shot_box(&self.mappings, shot),
                high_priority: shot.high_priority,
            });
        }
        for (index, shot) in self.weapons.shots.iter().enumerate() {
            attacks.push(PlayerAttack {
                object: AttackObject::Spread(index),
                hitbox: Self::spread_box(&self.mappings, shot),
                high_priority: shot.high_priority,
            });
        }
        if let Some(hitbox) = self.weapons.swing_box {
            attacks.push(PlayerAttack {
                object: AttackObject::Sword,
                hitbox,
                high_priority: self.weapons.high_priority,
            });
        }
        attacks
    }
    pub(in crate::combat) fn attack_damage(&self, attack: AttackObject) -> u8 {
        match attack {
            AttackObject::Shot(i) => self.projectiles[i].damage,
            AttackObject::Spread(i) => self.weapons.shots[i].damage,
            AttackObject::Sword => self.weapons.swing.map_or(1, |s| s.kind.spec().damage),
        }
    }
    /// A shuriken that collided with a stage object: `Shuriken_Update`
    /// removes it and plays its hit voice (`$24DE0`). Returns the command.
    /// The sword is not spent; the object decides what it plays.
    pub(crate) fn spend_attack(&mut self, object: AttackObject) -> Option<u8> {
        match object {
            AttackObject::Shot(index) => self.projectiles.get_mut(index)?.alive = false,
            AttackObject::Spread(index) => self.weapons.shots.get_mut(index)?.age = u16::MAX,
            AttackObject::Sword => return None,
        }
        Some(self.rules.cues.get(SoundEvent::ShurikenHit))
    }
    /// A list-4 object's `Object_CollideL3` also marks a sword as hit:
    /// `$252E4` spends the air stab and sets the player's rebound flag.
    pub(in crate::combat) fn spend_projectile_attack(
        &mut self,
        object: AttackObject,
        event: &mut CombatEvents,
    ) {
        if object != AttackObject::Sword {
            event.sound_ids.extend(self.spend_attack(object));
            return;
        }
        let Some(swing) = self.weapons.swing else {
            return;
        };
        event
            .sound_ids
            .push(self.rules.cues.get(swing.kind.hit_sound()));
        event.bounce |= matches!(swing.kind, Sword::AirStab | Sword::SurfStab);
        self.weapons.swing_box = None;
        if swing.kind != Sword::DashKick {
            self.weapons.swing = None;
        }
    }
    /// The [`crate::combat::Strike::hold`]s of the attacks waiting for their hit.
    pub(in crate::combat) fn held_attacks(&self) -> Vec<u32> {
        let shots = self
            .projectiles
            .iter()
            .filter(|shot| !shot.enemy && shot.alive);
        let spread = self
            .weapons
            .shots
            .iter()
            .filter(|shot| shot.age != u16::MAX);
        shots
            .filter_map(|shot| shot.held)
            .chain(spread.filter_map(|shot| shot.held))
            .chain(self.weapons.held_sword)
            .chain(self.weapons.swing.and_then(|swing| swing.held))
            .collect()
    }
    /// Free the held attacks whose hit `landed`, and release the others,
    /// which run on: a shuriken's terrain test at the step it met the
    /// object on comes first ([`Projectile::untested`]). A landed hit frees the sword too (`$24C92`, `$251EE`,
    /// `$252E4`), except the dash kick, which only plays its sound
    /// (`$254D8`) and stays with bit 14 hiding it from the list-6 and
    /// list-4 checks until its update. A replaced sword ends either way.
    pub(in crate::combat) fn free_held_attacks(&mut self, landed: &[u32]) {
        for shot in self.projectiles.iter_mut().filter(|shot| !shot.enemy) {
            if let Some(hold) = shot.held.take() {
                if landed.contains(&hold) {
                    shot.alive = false;
                } else {
                    shot.untested = shot.alive;
                }
            }
        }
        for shot in &mut self.weapons.shots {
            if let Some(hold) = shot.held.take() {
                if landed.contains(&hold) {
                    shot.age = u16::MAX;
                } else {
                    shot.untested = true;
                }
            }
        }
        self.weapons.drop_spent_shots();
        self.weapons.held_sword = None;
        if let Some(swing) = &mut self.weapons.swing
            && swing.held.take().is_some_and(|hold| landed.contains(&hold))
        {
            self.weapons.swing_box = None;
            if swing.kind != Sword::DashKick {
                self.weapons.swing = None;
            }
        }
    }
    /// Drop spent attacks after stage objects (after `spend_attack`, which
    /// keeps shuriken indices).
    pub(crate) fn remove_spent_attacks(&mut self) {
        // Stage objects run before the next player update. Ordinary expired
        // shots are spent here; the Hydra hand's synthetic list-4 contact
        // must survive until Player_CheckDamage tests it.
        for shot in &mut self.projectiles {
            if !shot.alive && !shot.keep_contact {
                shot.touching = false;
            }
        }
        self.retain_projectiles(true);
        self.weapons.shots.retain(|shot| shot.age != u16::MAX);
    }
    /// Hitbox of mapping `id` placed at (x, y) (`$18F4` then the offsets).
    pub(crate) fn mapping_hitbox(&self, id: u16, x: i32, y: i32, left: bool) -> Hitbox {
        mapping_hitbox(&self.mappings, id, x, y, left)
    }
    /// Test hook: a shot's box ([`Self::shot_box`]).
    /// The mapping's own hitbox record, relative to the object.
    pub(crate) fn mapping_bounds(&self, id: u16) -> Hitbox {
        self.mappings.get(&id).map_or(Hitbox::NONE, |m| m.bounds)
    }
    /// ORs `$8000` into the player's `obj_tile` (`$22C4C`, `Boss5`): the
    /// player and its weapons join the high sprite-priority layer.
    pub(crate) fn raise_player_priority(&mut self) {
        self.player_priority_raised = true;
        self.weapons.high_priority = true;
    }
    /// Test hook: the player's `obj_tile` bit 15 as a part would set it.
    pub(crate) fn player_high_priority(&self) -> bool {
        self.weapons.high_priority
    }
    /// The player's full body hitbox from the frame shown on the previous
    /// update (`Object_LoadHitbox` via `Player_FrameTable`, `$23ADE`).
    pub(crate) fn player_body(&self, player: (i32, i32), facing_left: bool) -> Hitbox {
        let mapping = self
            .player_mappings
            .get(self.player_frame)
            .copied()
            .unwrap_or(0);
        mapping_hitbox(&self.mappings, mapping, player.0, player.1, facing_left)
    }
}
