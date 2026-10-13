//! List 4: the item-box time bomb and the flames it and type 9 leave, and
//! the list-1 fire a bomb starts along a flammable 5-2 floor.
use super::{Context, ObjectEvents, StageObjects};
use crate::{
    combat::{AttackObject, Motion, PlayerAttack, in_sound_window},
    game::Terrain,
    rom::{Result, byte},
    stage::{StageId, TerrainEdit},
};

/// `Func_0374CC` (stage load): `$FF4126` is the 8-byte zero-terminated list
/// `$374A6 + 8 * Data_0374BE[round * 2 + substage]`; entry 0 is empty. Only
/// 5-2 (index 9, list 2 = `$374B6`) has one: its plank floor metatiles
/// `$2C..$30`. The index overlaps between rounds, as the ROM's does, and
/// 7-3 (index 14) reads past the table into code; no box spawns there.
pub(super) fn flammable_metatiles(rom: &[u8], id: StageId) -> Result<Vec<u8>> {
    let index = id.round_index() * 2 + id.substage_index();
    let list = usize::from(byte(rom, 0x374be + index)?);
    if list == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for i in 0..8 {
        // Func_037500 stops at the first zero byte.
        match byte(rom, 0x374a6 + list * 8 + i)? {
            0 => break,
            metatile => out.push(metatile),
        }
    }
    Ok(out)
}

/// List-1 type 6 (`Func_037532`): one step of the fire a time bomb starts
/// on a flammable floor. It waits, burns (`Data_037516`), then clears the
/// metatile under it and hands over to a new fire one metatile further on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FloorFire {
    pub(super) x: i32,
    pub(super) y: i32,
    /// `+$2A`: +16 or -16 px per step.
    step: i32,
    /// `+$28`: the wait, then the mapping cursor.
    counter: i32,
    routine: u8,
    pub(super) mapping: u16,
    alive: bool,
}
impl FloorFire {
    fn new(x: i32, y: i32, step: i32, delay: i32) -> Self {
        Self {
            x,
            y,
            step,
            counter: delay,
            routine: 0,
            // Func_01DDB2 leaves d0 = 0: nothing shows while it waits.
            mapping: 0,
            alive: true,
        }
    }
}

/// The list-4 type-30 time bomb an item box leaves (`$297DA`, `$1D52E`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Bomb {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) mapping: u16,
    routine: u8,
    /// `+$2A`, `+$2E`, `+$2C`.
    counter: i32,
    period: i32,
    digit: i32,
    alive: bool,
    high_priority: bool,
}
impl Bomb {
    pub(super) fn new(x: i32, y: i32, high_priority: bool) -> Self {
        Self {
            x,
            y,
            mapping: 0,
            routine: 1,
            counter: 30,
            period: 0,
            digit: 5,
            alive: true,
            high_priority,
        }
    }
}

/// A list-4 type-3 flame (`$1D144`). Its X swings to the mirrored side
/// every other tick, so three flames cover both sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Flame {
    base_x: i32,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) motion: Motion,
    /// `+$36` and the `+$32` accumulator.
    speed: i32,
    offset: i32,
    pub(super) mapping: u16,
    step: usize,
    timer: u16,
    pub(super) flip: bool,
    alive: bool,
    pub(super) high_priority: bool,
}

impl StageObjects {
    /// `$1D0EA`: the shared three-flame explosion used by type 9 and the
    /// item-box time bomb.
    pub(super) fn ignite(
        &mut self,
        x: i32,
        y: i32,
        high_priority: bool,
        camera: (i32, i32),
        events: &mut ObjectEvents,
    ) {
        for index in (0..3).rev() {
            let (cos, sin) = self.tables.sine.polar(self.tables.flame_angles[index]);
            self.flames.push(Flame {
                base_x: x,
                x,
                y: y + self.tables.flame_drop,
                motion: Motion {
                    vx: cos << 1,
                    vy: sin << 2,
                    gravity: self.tables.flame_gravity,
                    ..Motion::default()
                },
                speed: cos << 1,
                offset: 0,
                mapping: 0,
                step: 0,
                // Native: the record's leftover timer is not modelled; the
                // first mapping shows at once.
                timer: 3,
                flip: false,
                alive: true,
                high_priority,
            });
        }
        if in_sound_window((x, y), camera) {
            events.sounds.push(self.tables.bomb_sound);
        }
    }

    /// `$1D52E`: the list-4 type-30 bomb, from routine 4.
    pub(super) fn update_bombs(
        &mut self,
        cx: &mut Context,
        attacks: &[PlayerAttack],
        spent: &mut Vec<AttackObject>,
        events: &mut ObjectEvents,
    ) {
        let body = cx
            .combat
            .player_body(cx.player.position(), cx.player.facing_left);
        let mut bombs = std::mem::take(&mut self.bombs);
        let mut fires = Vec::new();
        for bomb in &mut bombs {
            let mut ignite = false;
            let hitbox = cx
                .combat
                .mapping_hitbox(bomb.mapping, bomb.x, bomb.y, false);
            let tables = &self.tables;
            loop {
                match bomb.routine {
                    // $1D580.
                    1 => {
                        bomb.mapping = tables.bomb_first;
                        bomb.counter = tables.bomb_lead;
                        bomb.routine = 2;
                    }
                    // $1D590.
                    2 => {
                        bomb.counter -= 1;
                        if bomb.counter < 0 {
                            bomb.counter = tables.bomb_period;
                            bomb.period = tables.bomb_period;
                            bomb.digit = 5;
                            bomb.routine = 3;
                        }
                        break;
                    }
                    // $1D5AE: the countdown; touching it shortens each digit
                    // to four ticks and an attack sets it off at once.
                    3 => {
                        if bomb.high_priority == cx.combat.player_high_priority()
                            && hitbox.intersects(body)
                            && bomb.period != tables.bomb_touched_period
                        {
                            bomb.period = tables.bomb_touched_period;
                            bomb.counter = tables.bomb_touched_period;
                        }
                        let hit = attacks.iter().find(|a| {
                            !spent.contains(&a.object)
                                && a.high_priority == bomb.high_priority
                                && a.hitbox.collides(hitbox)
                        });
                        if let Some(attack) = hit {
                            if attack.object.shuriken() {
                                spent.push(attack.object);
                                events.sounds.extend(cx.combat.spend_attack(attack.object));
                            }
                            bomb.counter = 0;
                            bomb.routine = 4;
                            break;
                        }
                        bomb.mapping = tables.bomb_digits[bomb.digit.clamp(0, 5) as usize];
                        bomb.counter -= 1;
                        if bomb.counter >= 0 {
                            break;
                        }
                        bomb.counter = bomb.period;
                        bomb.digit -= 1;
                        if bomb.digit >= 0 {
                            break;
                        }
                        bomb.counter = tables.bomb_fuse;
                        bomb.routine = 4;
                        break;
                    }
                    // $1D61C: the last digit blinks.
                    4 => {
                        bomb.mapping = if bomb.counter & 1 != 0 {
                            0
                        } else {
                            tables.bomb_blink
                        };
                        bomb.counter -= 1;
                        if bomb.counter >= 0 {
                            break;
                        }
                        bomb.routine = 5;
                    }
                    // $1D638: three flames and the blast sound ($4B2E).
                    // With a `$FF4126` list (5-2) the bomb stays, still
                    // showing its last digit, for the terrain test.
                    5 => {
                        ignite = true;
                        if self.flammable.is_empty() {
                            bomb.alive = false;
                        } else {
                            bomb.routine = 6;
                        }
                        break;
                    }
                    // $1D652: the metatile 8 px below the bomb; a flammable
                    // one starts a fire each way, the right one a metatile
                    // further on ($1D676).
                    6 => {
                        let (x, y) = (bomb.x, bomb.y + tables.fire_drop);
                        if self.flammable_at(cx.terrain, &events.terrain_edits, x, y) {
                            fires.push(FloorFire::new(
                                x + tables.fire_step,
                                y,
                                tables.fire_step,
                                tables.fire_delay,
                            ));
                            bomb.routine = 7;
                        } else {
                            bomb.alive = false;
                        }
                        break;
                    }
                    // $1D696: the left fire, on the bomb's own metatile.
                    7 => {
                        let (x, y) = (bomb.x, bomb.y + tables.fire_drop);
                        fires.push(FloorFire::new(x, y, -tables.fire_step, tables.fire_delay));
                        bomb.routine = 8;
                        break;
                    }
                    // $1D64C.
                    _ => {
                        bomb.alive = false;
                        break;
                    }
                }
            }
            if ignite {
                self.ignite(bomb.x, bomb.y, bomb.high_priority, cx.camera, events);
            }
        }
        bombs.retain(|bomb| bomb.alive);
        self.bombs = bombs;
        self.fires.extend(fires);
    }

    /// `Func_037606` + `Func_037500`: the layout byte under (x, y) is in the
    /// `$FF4126` list. Writes made earlier this frame count.
    fn flammable_at(&self, terrain: &dyn Terrain, edits: &[TerrainEdit], x: i32, y: i32) -> bool {
        let cell = |e: &TerrainEdit| {
            (e.x.div_euclid(16), e.y.div_euclid(16)) == (x.div_euclid(16), y.div_euclid(16))
        };
        let metatile = match edits.iter().rev().find(|e| cell(e)) {
            Some(edit) => Some(edit.metatile),
            None => terrain.metatile_at(x, y),
        };
        metatile.is_some_and(|m| m != 0 && self.flammable.contains(&m))
    }

    /// List-1 type 6 (`Func_037532`). A fire allocated during the pass is
    /// updated in the same pass, as `obj_next` reaches the list tail.
    pub(super) fn update_floor_fires(&mut self, cx: &mut Context, events: &mut ObjectEvents) {
        let mut index = 0;
        while index < self.fires.len() {
            let mut fire = self.fires[index].clone();
            let tables = &self.tables;
            loop {
                match fire.routine {
                    // $3753A: the wait.
                    0 => {
                        fire.counter -= 1;
                        if fire.counter >= 0 {
                            break;
                        }
                        fire.counter = 0;
                        fire.routine = 1;
                    }
                    // $3754A: a mapping a tick, then the burn ($37560):
                    // metatile 0 into Layout_Buffer and four blank cells into
                    // plane A (Func_0375D4 -> Func_015134, only near the
                    // screen), then one metatile on.
                    1 => {
                        if let Some(&mapping) = tables.fire_sequence.get(fire.counter as usize) {
                            fire.mapping = mapping;
                            fire.counter += 1;
                        } else if self.flammable_at(
                            cx.terrain,
                            &events.terrain_edits,
                            fire.x,
                            fire.y,
                        ) {
                            events.terrain_edits.push(TerrainEdit {
                                x: fire.x,
                                y: fire.y,
                                metatile: 0,
                            });
                            fire.x += fire.step;
                            fire.routine = 2;
                        } else {
                            // $375CE: freed without a blast.
                            fire.alive = false;
                        }
                        break;
                    }
                    // $37590: the next fire, where this one now stands.
                    2 => {
                        self.fires.push(FloorFire::new(
                            fire.x,
                            fire.y,
                            fire.step,
                            tables.fire_next_delay,
                        ));
                        fire.routine = 3;
                        break;
                    }
                    // $375C8: Func_036FF2's small blast, then freed.
                    _ => {
                        let sounds = cx.combat.small_blast((fire.x, fire.y), cx.camera);
                        events.sounds.extend(sounds);
                        fire.alive = false;
                        break;
                    }
                }
            }
            self.fires[index] = fire;
            index += 1;
        }
        self.fires.retain(|fire| fire.alive);
    }

    /// `$1D144`: the flames rise and fall, mirrored every other tick.
    pub(super) fn update_flames(&mut self) {
        let sequence = &self.tables.flame_sequence;
        for flame in &mut self.flames {
            flame.timer = flame.timer.wrapping_add(1);
            flame.offset = flame.offset.wrapping_add(flame.speed);
            let mut dx = flame.offset >> 16;
            if flame.timer & 1 != 0 {
                dx = -dx;
            }
            flame.x = flame.base_x + dx;
            flame.flip = !flame.flip;
            if flame.timer & 3 == 0 {
                match sequence.get(flame.step) {
                    Some(&mapping) => flame.mapping = mapping,
                    None => flame.alive = false,
                }
                flame.step += 1;
            }
        }
        self.flames.retain(|flame| flame.alive);
    }
}
