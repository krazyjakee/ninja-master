//! The item box (list-6 type 8) and the list-7 items it and `Enemy40`
//! leave for the player.
use super::{
    Context, LIST7_CAPACITY, Object, ObjectEvents, Record, StageObjects, UNIT, debris::push_debris,
    hazards::Bomb, placed,
};
use crate::combat::{AttackObject, Combat, Hitbox, Motion, PlayerAttack};

/// Pickup contents (`+$30`, the item box's `param & 7`).
pub(super) const SHURIKEN_5: u8 = 0;
pub(super) const SHURIKEN_20: u8 = 1;
pub(super) const POWER: u8 = 2;
pub(super) const ONE_UP: u8 = 3;
pub(super) const NINJUTSU: u8 = 4;
pub(super) const HEALTH_4: u8 = 5;
pub(super) const FULL_HEALTH: u8 = 6;
pub(super) const TIME_BOMB: u8 = 7;

/// A list-7 pickup (`$35E60`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pickup {
    /// `+$30`: the contents.
    pub kind: u8,
    pub x: i32,
    pub y: i32,
    pub mapping: u16,
    pub(super) motion: Motion,
    /// `+$2E`: the item mapping while it blinks.
    saved: u16,
    /// `obj_routine` / 4.
    pub(super) routine: u8,
    /// `+$2A`, `+$2C`, `+$34`, `+$28`.
    counter: i32,
    blink: i32,
    step: i32,
    pub(super) timer: u16,
    hitbox: Hitbox,
    alive: bool,
    pub(super) high_priority: bool,
}

/// An item scheduled by another object (`Enemy40`'s `$361D2`): it starts
/// at routine 4 (no debris), moving at `vx` (16.16 px/tick).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drop {
    pub kind: u8,
    pub x: i32,
    pub y: i32,
    pub vx: i32,
    pub(crate) high_priority: bool,
}

impl StageObjects {
    /// `$29704` after `$270F2`.
    pub(super) fn item_box(&self, index: usize, record: &Record, combat: &Combat) -> Object {
        let hidden = record.param & 0x100 != 0;
        // $2972E..$29736: bit 8 hides the box (mapping $F1, no collision,
        // material 0).
        let mapping = self.tables.box_mapping + u16::from(hidden);
        let material = if hidden { 0 } else { self.tables.box_material };
        Object {
            priority: record.param & 0x8000 != 0,
            collidable: !hidden,
            hitbox: self.tables.box_stand,
            param: record.param & 0x7fff,
            ..Object::new(index, record, mapping, material, combat)
        }
    }

    /// Items that other objects schedule (`$361D2`), from list 5.
    pub fn drop_pickups(&mut self, drops: impl IntoIterator<Item = Drop>) {
        for drop in drops {
            if self.shurikens_disabled && drop.kind <= SHURIKEN_20 {
                continue;
            }
            if self.pickups.len() >= LIST7_CAPACITY {
                break;
            }
            let mut pickup = self.new_pickup(drop.kind, drop.x, drop.y, drop.high_priority);
            pickup.routine = 1;
            pickup.motion.vx = drop.vx;
            self.pickups.push(pickup);
        }
    }

    /// Extend a scheduled pickup's approach into the extra columns only.
    /// The real list-7 object still owns collection and the original view.
    pub(crate) fn pickup_approach(
        &self,
        drops: Vec<Drop>,
        camera_x: i32,
    ) -> impl Iterator<Item = (u16, i32, i32)> + '_ {
        drops
            .into_iter()
            .filter(|drop| !self.shurikens_disabled || drop.kind > SHURIKEN_20)
            .take(LIST7_CAPACITY.saturating_sub(self.pickups.len()))
            // Keep even the widest item's artwork outside the 320px view
            // when allocation is delayed (for example, after a camera jump).
            .filter(move |drop| drop.x - camera_x >= crate::WIDTH as i32 + 16)
            .map(|drop| {
                (
                    self.tables.pickup_mappings[usize::from(drop.kind & 7)],
                    drop.x,
                    drop.y,
                )
            })
    }

    /// `$2973C`. Returns false when the box is freed.
    pub(super) fn update_box(
        &mut self,
        object: &mut Object,
        cx: &mut Context,
        attacks: &[PlayerAttack],
        spent: &mut Vec<AttackObject>,
        events: &mut ObjectEvents,
    ) -> bool {
        let tables = &self.tables;
        // $2973C: off screen at the last sprite pass, `$27160` frees it
        // and clears its `$FF3808` bit so the record can spawn again.
        if object.offscreen {
            return self.free_record(object);
        }
        // $29744..$2977C: the player stepped into the standing slab.
        if std::mem::take(&mut object.touched) && object.param >> 8 == 0 {
            let below = object.y - cx.player.position().1;
            let jumping = cx.player.animator.state & !7 == tables.snap_state;
            if below >= tables.snap_above || (below >= tables.snap_below && jumping) {
                cx.player.y = (object.y - tables.snap_height) * UNIT;
            }
        }
        // $29788..$2979C: player attacks against the box body.
        let body = object.placed(tables.box_attack);
        let Some(attack) = attacks.iter().find(|attack| {
            !spent.contains(&attack.object)
                && attack.high_priority == object.priority
                && attack.hitbox.collides(body)
        }) else {
            return true;
        };
        if attack.object.shuriken() {
            // The shuriken's own update removes it with its hit voice.
            spent.push(attack.object);
            events.sounds.extend(cx.combat.spend_attack(attack.object));
        } else {
            // $297AE..$297BE: the sword is not spent; the box plays voice 14.
            events.sounds.push(tables.box_voice);
        }
        self.break_box(object);
        false
    }

    /// `$297C2..$2984A`: the box rises 8 px and leaves its contents.
    fn break_box(&mut self, object: &Object) {
        let (x, y) = (object.x, object.y - 8);
        let kind = (object.param & 7) as u8;
        if kind == TIME_BOMB {
            // $297DA: a list-4 type-30 object at the box's origin, then
            // four fragments.
            self.bombs.push(Bomb::new(x, y + 8, object.priority));
            push_debris(&mut self.debris, x, y);
        } else if self.pickups.len() < LIST7_CAPACITY
            && (!self.shurikens_disabled || kind > SHURIKEN_20)
        {
            // $35DB0: the item appears where the box was, at routine 0.
            let pickup = self.new_pickup(kind, x, y, object.priority);
            self.pickups.push(pickup);
        }
    }

    fn new_pickup(&self, kind: u8, x: i32, y: i32, high_priority: bool) -> Pickup {
        Pickup {
            kind,
            x,
            y,
            mapping: self.tables.pickup_mappings[usize::from(kind & 7)],
            motion: Motion::default(),
            saved: 0,
            routine: 0,
            counter: 0,
            blink: 0,
            step: 0,
            timer: 0,
            hitbox: Hitbox::NONE,
            alive: true,
            high_priority,
        }
    }

    /// `$35DFC`: every list-7 object through `$35E60`.
    pub(super) fn update_pickups(&mut self, cx: &mut Context, events: &mut ObjectEvents) {
        let body = cx
            .combat
            .player_body(cx.player.position(), cx.player.facing_left);
        let mut pickups = std::mem::take(&mut self.pickups);
        for pickup in &mut pickups {
            pickup.timer = pickup.timer.wrapping_add(1);
            self.update_pickup(pickup, body, cx.combat, events);
        }
        pickups.retain(|pickup| pickup.alive);
        self.pickups = pickups;
    }

    /// One `$35E60` call. `code` follows the handler's fall-throughs:
    /// each routine ends with `addq.w #4,obj_routine` and runs straight
    /// into the next routine's code, whatever `obj_routine` now holds.
    fn update_pickup(
        &mut self,
        pickup: &mut Pickup,
        body: Hitbox,
        combat: &mut Combat,
        events: &mut ObjectEvents,
    ) {
        // `$360E0`: the player touched it; routine 16 collects it.
        let player_high_priority = combat.player_high_priority();
        let collide = |pickup: &mut Pickup| {
            if pickup.high_priority == player_high_priority
                && placed(pickup.hitbox, pickup.x, pickup.y).intersects(body)
            {
                pickup.routine = 4;
            }
        };
        let mut code = pickup.routine;
        loop {
            match code {
                // $35E6C: the fragments, once.
                0 => {
                    push_debris(&mut self.debris, pickup.x, pickup.y);
                    pickup.routine += 1;
                    return;
                }
                // $35E9C: hitbox from the mapping, 240 ticks to wait.
                1 => {
                    pickup.hitbox = combat.mapping_bounds(pickup.mapping);
                    pickup.counter = self.tables.pickup_wait;
                    pickup.saved = pickup.mapping;
                    pickup.routine += 1;
                    code = 2;
                }
                // $35EB2.
                2 => {
                    collide(pickup);
                    pickup.counter -= 1;
                    if pickup.counter >= 0 {
                        return;
                    }
                    pickup.counter = self.tables.blink_period;
                    pickup.blink = 0;
                    pickup.routine += 1;
                    code = 3;
                }
                // $35ED6: blinking; `$3611E` frees it after three steps.
                3 => {
                    collide(pickup);
                    pickup.mapping = 0;
                    if (i32::from(pickup.timer) >> pickup.blink) & 1 == 0 {
                        pickup.mapping = pickup.saved;
                        pickup.counter -= 1;
                        if pickup.counter < 0 {
                            pickup.counter = self.tables.blink_period;
                            pickup.blink += 1;
                            if pickup.blink == self.tables.blink_steps {
                                pickup.alive = false; // $360FA
                            }
                        }
                    }
                    return;
                }
                // $35EF0: collected.
                4 => {
                    let kind = pickup.kind & 7;
                    self.collect(kind, combat, events);
                    let id = self.tables.pickup_sounds[usize::from(kind)];
                    events.sounds.extend(self.tables.effect(id));
                    events.collected.push(kind);
                    events.collected_at.push((kind, pickup.x, pickup.y));
                    pickup.mapping = pickup.saved;
                    pickup.step = self.tables.rise_ticks;
                    pickup.motion.vy = self.tables.rise_speed;
                    pickup.routine += 1;
                    code = 5;
                }
                // $35F14: rising.
                5 => {
                    pickup.step -= 1;
                    if pickup.step >= 0 {
                        return;
                    }
                    pickup.step = 0;
                    pickup.motion.vy = 0;
                    pickup.routine += 1;
                    code = 6;
                }
                // $35F2A: the sparkle, then `$360FA` frees it.
                _ => {
                    match usize::try_from(pickup.step)
                        .ok()
                        .and_then(|step| self.tables.sparkle.get(step))
                    {
                        Some(&mapping) => pickup.mapping = mapping,
                        None => pickup.alive = false,
                    }
                    pickup.step += 1;
                    return;
                }
            }
        }
    }

    /// `$35F48`: the item's effect, then its score (`$360B4`/`$360BA`).
    pub(super) fn collect(&self, kind: u8, combat: &mut Combat, events: &mut ObjectEvents) {
        let tables = &self.tables;
        let base = tables.pickup_scores[usize::from(kind & 7)];
        // $36036..$36044: a streak doubles the score, at most four times.
        let streak = |counter: u8| -> u8 {
            if (counter as i8) <= tables.bonus_limit as i8 {
                counter
            } else {
                tables.bonus_limit
            }
        };
        let mut points: Vec<u16> = Vec::new();
        let hud = &mut combat.hud;
        let weapons = &mut combat.weapons;
        match kind {
            SHURIKEN_5 | SHURIKEN_20 => {
                // $35F8E: capped at 799.
                let ammo = hud.ammo.saturating_add(tables.ammo[usize::from(kind)]);
                hud.ammo = ammo.min(tables.ammo_limit);
                points.push(base);
            }
            POWER => {
                if !weapons.powered {
                    // $3605E: Player_PowerUp.
                    weapons.power_bonus = 0;
                    weapons.powered = true;
                    points.push(base);
                } else {
                    // $36072: the stored counter is not capped, the shift is.
                    weapons.power_bonus = weapons.power_bonus.wrapping_add(1);
                    points.push(base << streak(weapons.power_bonus));
                }
            }
            ONE_UP => {
                // $12566: a life and the 1UP effect.
                hud.lives = hud.lives.wrapping_add(1);
                events.sounds.push(tables.one_up);
                points.push(base);
            }
            NINJUTSU => {
                let uses = weapons.ninjutsu_uses.wrapping_add(1);
                weapons.ninjutsu_uses = if (uses as i8) <= tables.ninjutsu_limit as i8 {
                    uses
                } else {
                    tables.ninjutsu_limit
                };
                points.push(base);
            }
            HEALTH_4 | FULL_HEALTH => {
                if hud.health == hud.max_health {
                    weapons.health_bonus = streak(weapons.health_bonus.wrapping_add(1));
                    points.push(base << weapons.health_bonus);
                    // $35FCA: the +4 item adds its own score as well.
                    if kind == HEALTH_4 {
                        points.push(base);
                    }
                } else {
                    weapons.health_bonus = 0;
                    hud.health = if kind == HEALTH_4 {
                        hud.health.saturating_add(tables.health).min(hud.max_health)
                    } else {
                        hud.max_health
                    };
                    points.push(base);
                }
            }
            _ => {}
        }
        // $96AE converts each binary word to BCD for $125BC.
        for value in points {
            hud.score = hud.score.saturating_add(u32::from(value));
        }
    }
}

#[cfg(test)]
mod roadmap_tests {
    use super::*;
    #[test]
    #[ignore = "requires a user-supplied cartridge"]
    fn disabled_removes_container_and_enemy_ammo_but_keeps_other_items() {
        let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
        let rom =
            crate::rom::regions::prepare(std::fs::read(directory.join("shinobi3_us.bin")).unwrap())
                .unwrap();
        let id = crate::stage::StageId::FOREST;
        let combat = Combat::load_stage(&rom, id).unwrap();
        let mut objects = StageObjects::load(&rom, id).unwrap();
        objects.shurikens_disabled = true;
        objects.drop_pickups((0..7).map(|kind| Drop {
            kind,
            x: 100,
            y: 496,
            vx: 0,
            high_priority: false,
        }));
        assert_eq!(
            objects
                .pickups
                .iter()
                .map(|pickup| pickup.kind)
                .collect::<Vec<_>>(),
            [2, 3, 4, 5, 6]
        );
        objects.pickups.clear();
        for kind in 0..8 {
            let record = Record {
                source: 0,
                kind: 8,
                x: 100,
                y: 496,
                param: kind,
                x_window: (0, 0),
                y_window: (0, 0),
            };
            let mut object = Object::new(0, &record, 0, 0, &combat);
            object.param = kind;
            objects.break_box(&object);
        }
        assert_eq!(
            objects
                .pickups
                .iter()
                .map(|pickup| pickup.kind)
                .collect::<Vec<_>>(),
            [2, 3, 4, 5, 6]
        );
        assert_eq!(objects.bombs.len(), 1);
    }
}
