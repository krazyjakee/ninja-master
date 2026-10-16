//! The stage-object system that `Game_RunFrame` runs through `$26F42`
//! (issue #60), separate from the `$41EFA` enemy spawn lists.
//!
//! `$26F42` reads its own spawn table, `$27178`, indexed by
//! round + substage * 8 and then by part. Each 16-byte record is a list-6
//! object type, an X/Y camera window, a spawn position and a parameter
//! word. The `$FF3808` long marks the records that are alive, so a record
//! emulator evidence.
use crate::{
    combat::{AttackObject, Combat, Hitbox, Motion, SpriteList},
    game::Terrain,
    movement::Player,
    render::{Frame, Lines, Tile},
    rom::Result,
    stage::StageId,
};
use debris::{BarrelBurst, BarrelSmoke, Debris, DeviceParticle};
use hazards::{Bomb, Flame, FloorFire, flammable_metatiles};
use laser::{LaserTables, Room};
use pickups::Pickup;
use records::{MAX_RECORDS, Record, keeps_state, stage_records};
use tables::Tables;

mod debris;
mod devices;
mod hazards;
mod laser;
mod pickups;
mod records;
mod tables;
mod terrain;

pub use pickups::Drop;
pub use terrain::ObjectTerrain;

/// List-6 type 8, the item box (spawn `$29704`, update `$2973C`).
const ITEM_BOX: u16 = 8;
/// List-6 type 1, the vertical lift (`$28E7E` / `$28ED0`).
const LIFT: u16 = 1;
const MOVING_PLATFORM: u16 = 0;
const CRUMBLING_PLATFORM: u16 = 2;
const ROUTE_PLATFORM: u16 = 3;
const MOUNTED_OBJECT: u16 = 4;
const SWITCH_DEVICE: u16 = 5;
const FALLING_PLATFORM: u16 = 6;
const EXPLODING_PLATFORM: u16 = 9;
const FALLING_DEBRIS: u16 = 0xa;
const MOVING_TRAP: u16 = 0xb;
const CRUSHER: u16 = 0xc;
const BOUNCER: u16 = 0xd;
const LASER: u16 = 0xf;
const CLIMB_PLATFORM: u16 = 0x10;
const DISAPPEARING_PLATFORM: u16 = 0x11;
const MECH_HAND: u16 = 0x1c;

const UNIT: i32 = 256;
/// `Object_ListParams`: list 6 holds 24 records (dbf 23), list 7 eight.
const LIST6_CAPACITY: usize = 24;
const LIST7_CAPACITY: usize = 8;

/// A live list-6 object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    /// `+$2A`: the record's index, its bit in `$FF3808`.
    pub record: usize,
    pub kind: u16,
    pub x: i32,
    pub y: i32,
    /// `obj_frame`.
    pub mapping: u16,
    /// `obj_tile` bit 15, from record parameter bit 15 (`$29716`).
    pub priority: bool,
    /// `+$29`: the material `Player_ProbeTerrain` reports inside the hitbox.
    pub material: u8,
    /// `obj_flags` bit 13 clear: the player probe finds the object.
    pub collidable: bool,
    /// `+$10..$13`, relative to (x, y).
    pub hitbox: Hitbox,
    /// The record's movement bounds (`+$2C..+$32`).
    x_window: (i32, i32),
    y_window: (i32, i32),
    /// Object-list velocity and fractional position.
    motion: Motion,
    /// `+$34`: the record parameter without bit 15.
    pub param: u16,
    /// `obj_flags` bit 7 from the last sprite pass (`$17A4`).
    pub offscreen: bool,
    /// `obj_flags` bit 14: the player probe found it this frame.
    touched: bool,
    /// Type-1's last rider Y and lift sprite Y (`+$36/$38`).
    saved_player_y: i32,
    saved_screen_y: i32,
    /// Type-specific routine, countdowns and sequence cursor.
    state: u8,
    counter: i32,
    auxiliary: i32,
    step: usize,
    /// Type `$F`: the laser room's list-1/list-9 pieces.
    room: Option<Box<Room>>,
}
impl Object {
    /// Common `$270F2` fields for a visible stage device.
    fn new(index: usize, record: &Record, mapping: u16, material: u8, combat: &Combat) -> Self {
        Self {
            record: index,
            kind: record.kind,
            x: record.x,
            y: record.y,
            mapping,
            priority: false,
            material,
            collidable: true,
            hitbox: combat.mapping_bounds(mapping),
            x_window: record.x_window,
            y_window: record.y_window,
            motion: Motion::default(),
            param: record.param,
            offscreen: false,
            touched: false,
            saved_player_y: 0,
            saved_screen_y: 0,
            state: 0,
            counter: 0,
            auxiliary: 0,
            step: 0,
            room: None,
        }
    }
    fn placed(&self, hitbox: Hitbox) -> Hitbox {
        placed(hitbox, self.x, self.y)
    }
    /// `$E(a0)`: the VDP sprite Y, 128 + the screen Y.
    fn sprite_y(&self, camera: (i32, i32)) -> i32 {
        self.y - camera.1 + 0x80
    }
    /// A rider's player Y, `above` pixels over the platform's next 16.16 Y
    /// (`move.l $24(a0),d0 / add.l $1C(a0),d0 / move.l d0,$24(a3)`). The
    /// carry replaces the rider's Y with the platform's, fraction included,
    /// so the rider cannot drift half a pixel off a half-pixel platform.
    fn carried_y(&self, above: i32) -> i32 {
        let next =
            (i64::from(self.y) << 16) + i64::from(self.motion.fy) + i64::from(self.motion.vy);
        (next >> 8) as i32 - above * UNIT
    }
}

impl StageObjects {
    /// Boss arenas have no record spawns; reserve bits 30/31 for the two
    /// records allocated by `$307F2`, so the ordinary terrain probe sees them.
    pub(crate) fn mech_hands_touched(&self, touched: u32) -> bool {
        self.objects
            .iter()
            .any(|o| o.kind == MECH_HAND && touched & (1 << o.record) != 0)
    }

    pub(crate) fn place_mech_hands(&mut self, hands: Option<[(i32, i32); 2]>, combat: &Combat) {
        let Some(hands) = hands else {
            self.objects.retain(|o| o.kind != MECH_HAND);
            return;
        };
        for (i, (x, y)) in hands.into_iter().enumerate() {
            let index = 30 + i;
            if let Some(hand) = self
                .objects
                .iter_mut()
                .find(|o| o.kind == MECH_HAND && o.record == index)
            {
                // `$312D6` leaves A3 pointing at the hand controller.
                // `$31350` adjusts that record's X, never Joe's X.
                (hand.x, hand.y) = (x, y);
            } else {
                let record = Record {
                    source: 0,
                    kind: MECH_HAND,
                    x_window: (0, 0),
                    y_window: (0, 0),
                    x,
                    y,
                    param: 0,
                };
                let mut hand = Object::new(index, &record, 0x2c5 + i as u16, 7, combat);
                hand.priority = true;
                self.objects.push(hand);
            }
        }
    }
}

/// `hitbox` relative to an object at (x, y).
fn placed(hitbox: Hitbox, x: i32, y: i32) -> Hitbox {
    Hitbox {
        x: x + hitbox.x,
        y: y + hitbox.y,
        ..hitbox
    }
}

/// What one update requested.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ObjectEvents {
    /// Sound commands, in request order.
    pub sounds: Vec<u8>,
    /// Contents of the items collected.
    pub collected: Vec<u8>,
    pub(crate) collected_at: Vec<(u8, i32, i32)>,
    /// Type 1 cleared `$FF0175` bit 0: release a held autoscroll.
    pub release_autoscroll: bool,
    /// A lift trapped the rider against solid terrain.
    pub crushed: bool,
    /// A 4-1 type-4 ramp touched the grounded surfer.
    pub ramp: bool,
    /// Layout writes for the game to apply (the 5-2 floor fire).
    pub terrain_edits: Vec<crate::stage::TerrainEdit>,
    /// A pressed type-5 switch ran this update, with its parameter: the
    /// game steps its palette fade and raises the request (`$29552`).
    pub switch: Option<u16>,
    /// Palette writes (the 3-2 laser's flash and fade, `$2A15A`).
    pub palette_ops: Vec<crate::combat::PaletteOp>,
}

/// The player and systems a stage-object update touches.
pub struct Context<'a> {
    pub combat: &'a mut Combat,
    pub player: &'a mut Player,
    /// `$FFE000`/`$FFE004` after this frame's camera update.
    pub camera: (i32, i32),
    /// Records the player probes found this frame ([`ObjectTerrain::touched`]).
    pub touched: u32,
    /// Stage collision queried by the type-1 crush probes.
    pub terrain: &'a dyn Terrain,
    /// `$FF5F03 & 3`, the 4-2 tile animation step (`TileAnimation::step`).
    pub animation_step: usize,
}

/// Lists 6 and 7 of one part, with the objects they leave behind.
#[derive(Clone)]
pub struct StageObjects {
    pub id: StageId,
    records: Vec<Record>,
    /// `$FF3808`: records whose object is alive or was broken.
    pub spawned: u32,
    pub objects: Vec<Object>,
    pub pickups: Vec<Pickup>,
    pub(crate) shurikens_disabled: bool,
    bombs: Vec<Bomb>,
    flames: Vec<Flame>,
    /// List-1 type 6, the 5-2 floor fire.
    fires: Vec<FloorFire>,
    /// `$FF4126`: metatiles a time bomb sets alight ([`flammable_metatiles`]).
    flammable: Vec<u8>,
    debris: Vec<Debris>,
    device_debris: Vec<DeviceParticle>,
    barrel_smoke: Vec<BarrelSmoke>,
    barrel_bursts: Vec<BarrelBurst>,
    /// `$FF39F7` bit 0: `$35D94` skips list 7.
    arena: bool,
    /// Frame parity: `$26F88` scans only when `Frame_Counter` bit 0 is clear.
    ticks: u64,
    tables: Tables,
    laser: LaserTables,
}
impl StageObjects {
    pub fn load(rom: &[u8], id: StageId) -> Result<Self> {
        Ok(Self {
            id,
            records: stage_records(rom, id)?,
            spawned: 0,
            objects: Vec::new(),
            pickups: Vec::new(),
            shurikens_disabled: false,
            bombs: Vec::new(),
            flames: Vec::new(),
            fires: Vec::new(),
            flammable: flammable_metatiles(rom, id)?,
            debris: Vec::new(),
            device_debris: Vec::new(),
            barrel_smoke: Vec::new(),
            barrel_bursts: Vec::new(),
            arena: crate::combat::boss_arena(rom, id)?,
            ticks: 0,
            tables: Tables::load(rom)?,
            laser: LaserTables::load(rom)?,
        })
    }
    /// A part change: `$26EE6` starts over unless the new part keeps the
    /// previous part's list ([`keeps_state`]). Returns whether it kept it.
    pub fn enter_part(&mut self, rom: &[u8], id: StageId) -> Result<bool> {
        if keeps_state(id) {
            self.id = id;
            self.records = stage_records(rom, id)?;
            return Ok(true);
        }
        *self = Self::load(rom, id)?;
        Ok(false)
    }
    /// A time bomb as a broken contents-7 box leaves it (`$297DA`).
    /// Live list-6 boxes considered by `Player_FindNearestEnemy` before the
    /// enemy and boss lists: type 8, visible, and with `+$34`'s high byte
    /// clear. The tuple is `(x, y, obj_tile bit 15)`.
    pub(crate) fn intact_item_boxes(&self) -> impl Iterator<Item = (i32, i32, bool)> + '_ {
        self.objects
            .iter()
            .filter(|object| object.kind == ITEM_BOX && object.param >> 8 == 0 && !object.offscreen)
            .map(|object| (object.x, object.y, object.priority))
    }

    /// One frame of `$26F42`, `$35D94`, the list-4 and list-1 objects they
    /// created, and the sprite pass that moves them.
    pub fn tick(&mut self, cx: &mut Context) -> ObjectEvents {
        let mut events = ObjectEvents::default();
        let attacks = cx.combat.player_attacks();
        let mut spent = Vec::new();
        // $1CC0A runs list 4 before $26F42.
        self.update_bombs(cx, &attacks, &mut spent, &mut events);
        self.update_flames();
        self.update_barrel_bursts();
        // $26F88: the scan runs on even frames.
        if self.ticks.is_multiple_of(2) {
            self.spawn_records(cx.camera, cx.player.position(), cx.combat, &mut events);
        }
        self.ticks += 1;
        for object in &mut self.objects {
            // A newly released lift retains bit 14 until its first carry.
            object.touched = (object.kind == LIFT && object.touched)
                || cx.touched & (1 << (object.record % MAX_RECORDS)) != 0;
        }
        self.update_objects(cx, &attacks, &mut spent, &mut events);
        if !self.arena {
            self.update_pickups(cx, &mut events);
        }
        self.update_debris();
        self.update_floor_fires(cx, &mut events);
        self.update_device_debris();
        self.update_barrel_smoke();
        cx.combat.remove_spent_attacks();
        // The sprite pass: Object_Integrate ($19D4), Object_UpdateVisibility ($17A4).
        for object in &mut self.objects {
            object.motion.step(&mut object.x, &mut object.y);
        }
        for pickup in &mut self.pickups {
            pickup.motion.step(&mut pickup.x, &mut pickup.y);
        }
        for debris in &mut self.debris {
            debris.motion.step(&mut debris.x, &mut debris.y);
        }
        for debris in &mut self.device_debris {
            debris.motion.step(&mut debris.x, &mut debris.y);
        }
        for smoke in &mut self.barrel_smoke {
            smoke.y -= 1;
        }
        for burst in &mut self.barrel_bursts {
            burst.motion.step(&mut burst.x, &mut burst.y);
        }
        for flame in &mut self.flames {
            flame.motion.step(&mut flame.x, &mut flame.y);
        }
        let body = cx
            .combat
            .player_body(cx.player.position(), cx.player.facing_left);
        // List-4 objects: the next damage check tests the first flame on
        // the body after the enemy shots (`Object_CollideL4`).
        let flame = self.flames.iter().find(|flame| {
            flame.high_priority == cx.combat.player_high_priority()
                && cx
                    .combat
                    .mapping_hitbox(flame.mapping, flame.x, flame.y, flame.flip)
                    .intersects(body)
        });
        let damage = self.tables.flame_damage;
        if let Some(flame) = flame
            && damage > 0
            && cx.combat.pending_hit.is_none()
        {
            cx.combat.pending_hit = Some(crate::combat::PlayerHit::shot(flame.x, damage));
        }
        // $29BA8 gives the barrel's list-4 bursts damage 1. The preceding
        // list-1 smoke is cosmetic; bursts use low plane priority.
        if cx.combat.pending_hit.is_none()
            && !cx.combat.player_high_priority()
            && let Some(burst) = self.barrel_bursts.iter().find(|burst| {
                burst.hitbox_mapping != 0
                    && cx
                        .combat
                        .mapping_hitbox(burst.hitbox_mapping, burst.x, burst.y, false)
                        .intersects(body)
            })
        {
            cx.combat.pending_hit = Some(crate::combat::PlayerHit::shot(burst.x, 1));
        }
        let camera = cx.camera;
        let view = cx.combat.screen_view(camera.0);
        for object in &mut self.objects {
            object.offscreen = crate::combat::outside_view_in((object.x, object.y), camera, view);
        }
        events
    }

    /// `$26FD2..$2703A`: spawn every record whose window holds the camera
    /// and whose `$FF3808` bit is clear.
    fn spawn_records(
        &mut self,
        camera: (i32, i32),
        player: (i32, i32),
        combat: &Combat,
        events: &mut ObjectEvents,
    ) {
        let view = combat.screen_view(camera.0);
        for (index, record) in self.records.iter().enumerate() {
            let bit = record_bit(index);
            // Mounted hazards move eight pixels on every update after
            // allocation. Starting them early changes their arrival relative
            // to the camera-based warning schedule, rather than just visibility.
            let active = if record.kind == MOUNTED_OBJECT {
                record.active_at(camera)
            } else {
                record.active_in_view(camera, view)
            };
            if self.spawned & bit != 0 || !active {
                continue;
            }
            // $16E0 fails when list 6 is full; the record waits.
            if self.objects.len() >= LIST6_CAPACITY {
                break;
            }
            let object = match record.kind {
                MOVING_PLATFORM => Some(Object::new(index, record, 0xff, 7, combat)),
                // $28E84..$28E8E: this part's lift, and only while the first
                // record's (the lift the rider boarded) `$FF3808` bit is clear.
                LIFT if usize::from(record.param >> 8) == self.id.part_index()
                    && self.spawned & record_bit(0) == 0 =>
                {
                    Some(Object::lift(index, record, combat))
                }
                CRUMBLING_PLATFORM => {
                    let mappings = [0xf6, 0xf6, 0xfe, 0xff, 0x101, 0x102, 0x104, 0x108];
                    let mapping = mappings[usize::from(record.param >> 8).min(7)];
                    let mut object = Object::new(index, record, mapping, 7, combat);
                    object.counter = i32::from(record.param & 0xff);
                    Some(object)
                }
                ROUTE_PLATFORM => {
                    let mut object = Object::new(index, record, 0x101, 7, combat);
                    object.state = 1;
                    object.counter = 0x40;
                    Some(object)
                }
                MOUNTED_OBJECT => {
                    let variant = record.param >> 8;
                    let mut object = Object::new(
                        index,
                        record,
                        if variant == 0 { 0xf2 } else { 0xf3 },
                        0,
                        combat,
                    );
                    object.priority = variant == 0;
                    object.counter = 0x200;
                    Some(object)
                }
                SWITCH_DEVICE => {
                    let mut object = Object::new(index, record, 0, 0, combat);
                    // $293D2 `move.l #$FC0804,$10(a0)`: bytes $10..$13 are
                    // 0, -4, 8, 4, a 16x8 sensor just above the floor.
                    object.hitbox = Hitbox {
                        x: 0,
                        y: -4,
                        half_width: 8,
                        half_height: 4,
                    };
                    Some(object)
                }
                FALLING_PLATFORM => Some(Object::new(index, record, 0x100, 1, combat)),
                ITEM_BOX => Some(self.item_box(index, record, combat)),
                EXPLODING_PLATFORM => {
                    let mut object = Object::new(index, record, 0xf7, 2, combat);
                    object.counter = i32::from(record.param);
                    Some(object)
                }
                FALLING_DEBRIS => {
                    let mut object = Object::new(index, record, 0, 1, combat);
                    object.hitbox = combat.mapping_bounds(0xf8);
                    object.counter = i32::from(record.param);
                    Some(object)
                }
                MOVING_TRAP => {
                    let mapping = if record.param >> 8 == 0 { 0x10b } else { 0x103 };
                    let mut object = Object::new(index, record, mapping, 1, combat);
                    object.motion.vx = i32::from(record.param as u8 as i8) << 11;
                    Some(object)
                }
                CRUSHER => Some(Object::new(index, record, 0x105, 1, combat)),
                BOUNCER => {
                    // $29DF4: high-byte bit 1 picks `Data_029E52`'s mapping
                    // and rider offset (`+$38`); bit 0 reverses the speed,
                    // the low byte << 12 (`+$2C`). `+$34` keeps the top.
                    let variant = record.param >> 8;
                    let (mapping, rider_offset) = if variant & 2 == 0 {
                        (0x105, 0x50)
                    } else {
                        (0x10a, 0)
                    };
                    let mut object = Object::new(index, record, mapping, 1, combat);
                    let speed = i32::from(record.param & 0xff) << 12;
                    object.counter = if variant & 1 != 0 { -speed } else { speed };
                    object.auxiliary = rider_offset;
                    object.saved_screen_y = object.y_window.0;
                    Some(object)
                }
                LASER if inside((record.x_window, record.y_window), player) => {
                    // Func_0270F2's mapping $EF + $17, hidden (`$2A040`,
                    // obj_flags bit 11); `$2A056` requests effect $23.
                    let mut object = Object::new(index, record, 0x106, 0, combat);
                    object.collidable = false;
                    object.room = Some(Box::new(Room::new(&self.laser)));
                    events
                        .sounds
                        .extend(self.tables.effect(self.laser.entry_sound));
                    Some(object)
                }
                CLIMB_PLATFORM => {
                    let mapping = 0x108 + (record.param >> 8 & 1);
                    let mut object = Object::new(index, record, mapping, 7, combat);
                    // $2A25A `ori.w #$8000,8(a0)`: on 6-1's raised layer.
                    object.priority = true;
                    Some(object)
                }
                DISAPPEARING_PLATFORM => {
                    let mut object = Object::new(index, record, 0x125, 1, combat);
                    object.counter = i32::from(record.param);
                    object.auxiliary = 0x200;
                    Some(object)
                }
                // The remaining lifts, traps and ramps are added below.
                _ => None,
            };
            if let Some(object) = object {
                self.spawned |= bit;
                self.objects.push(object);
            }
        }
    }

    /// `$27044..$2705E`: each object's `$270AA` handler.
    fn update_objects(
        &mut self,
        cx: &mut Context,
        attacks: &[crate::combat::PlayerAttack],
        spent: &mut Vec<AttackObject>,
        events: &mut ObjectEvents,
    ) {
        let mut objects = std::mem::take(&mut self.objects);
        objects.retain_mut(|object| match object.kind {
            MOVING_PLATFORM => self.update_moving_platform(object, cx, events),
            LIFT => self.update_lift(object, cx, events),
            CRUMBLING_PLATFORM => self.update_crumbling_platform(object, cx),
            ROUTE_PLATFORM => self.update_route_platform(object, cx, events),
            MOUNTED_OBJECT => self.update_mounted_object(object, cx, events),
            SWITCH_DEVICE => self.update_switch_device(object, cx, events),
            FALLING_PLATFORM => self.update_falling_platform(object, cx, events),
            ITEM_BOX => self.update_box(object, cx, attacks, spent, events),
            EXPLODING_PLATFORM => self.update_exploding_platform(object, cx, events),
            FALLING_DEBRIS => self.update_falling_debris(object, cx, events),
            MOVING_TRAP => self.update_moving_trap(object, cx, events),
            CRUSHER => self.update_crusher(object, cx, events),
            BOUNCER => self.update_bouncer(object, cx, events),
            LASER => self.update_laser(object, cx, events),
            CLIMB_PLATFORM => self.update_climb_platform(object, cx),
            DISAPPEARING_PLATFORM => self.update_disappearing_platform(object, cx),
            _ => true,
        });
        self.objects = objects;
    }

    /// This module's records of sprite list `list` (see
    /// [`crate::combat::SpriteList`]), painted last linked first: records
    /// join the tail of their list, so the older record wins overlaps.
    pub fn draw_list(
        &self,
        frame: &mut Frame,
        combat: &Combat,
        list: SpriteList,
        palettes: &Lines<'_>,
        tiles: &[Tile],
        camera: (i32, i32),
    ) {
        let reversed = |sprites: Vec<Sprite>| sprites.into_iter().rev();
        match list {
            // List 9 closes the sprite table: the laser rooms' masks, then
            // the monster behind every other object (low priority, `$268BE`).
            SpriteList::Back => {
                for room in self.rooms() {
                    let sprites = reversed(room.back_sprites(&self.laser, camera));
                    draw_sprites(frame, combat, palettes, tiles, camera, sprites);
                }
            }
            SpriteList::Pickups => {
                let pickups = self
                    .pickups
                    .iter()
                    .rev()
                    .map(|p| Sprite::new(p.mapping, p.x, p.y).prioritised(p.high_priority));
                draw_sprites(frame, combat, palettes, tiles, camera, pickups);
            }
            // The laser controller is hidden (`$2A040`).
            SpriteList::StageObjects => {
                let objects = self
                    .objects
                    .iter()
                    .rev()
                    .filter(|o| o.kind != LASER)
                    .map(|o| Sprite::new(o.mapping, o.x, o.y).prioritised(o.priority));
                draw_sprites(frame, combat, palettes, tiles, camera, objects);
            }
            // Bombs and flames (list 4).
            SpriteList::EnemyShots => {
                let bombs = self
                    .bombs
                    .iter()
                    .rev()
                    .map(|b| Sprite::new(b.mapping, b.x, b.y));
                let flames = self
                    .flames
                    .iter()
                    .rev()
                    .map(|f| Sprite::new(f.mapping, f.x, f.y).flipped(f.flip));
                draw_sprites(frame, combat, palettes, tiles, camera, bombs.chain(flames));
                let bursts = self
                    .barrel_bursts
                    .iter()
                    .rev()
                    .map(|b| Sprite::new(b.mapping, b.x, b.y));
                draw_sprites(frame, combat, palettes, tiles, camera, bursts);
            }
            // Fragments, floor fire and the laser rooms' pieces (list 1).
            SpriteList::Effects => {
                // $298AC allocates a list-1 $2B companion (routine $A0,
                // $26EA6) with mapping $F8 while the list-6 platform is
                // hidden. First contact frees it and reveals the platform.
                let platforms = self
                    .objects
                    .iter()
                    .rev()
                    .filter(|o| o.kind == FALLING_DEBRIS && o.state == 0)
                    .map(|o| Sprite::new(0xf8, o.x, o.y));
                draw_sprites(frame, combat, palettes, tiles, camera, platforms);
                let debris = self
                    .debris
                    .iter()
                    .rev()
                    .map(|d| Sprite::new(d.mapping, d.x, d.y));
                let fires = self
                    .fires
                    .iter()
                    .rev()
                    .map(|f| Sprite::new(f.mapping, f.x, f.y));
                let device = self
                    .device_debris
                    .iter()
                    .rev()
                    .map(|d| Sprite::new(d.mapping, d.x, d.y));
                let sprites = debris.chain(fires).chain(device);
                draw_sprites(frame, combat, palettes, tiles, camera, sprites);
                let smoke = self
                    .barrel_smoke
                    .iter()
                    .rev()
                    .filter(|s| s.visible())
                    .map(|s| Sprite::new(s.mapping, s.x, s.y).prioritised(true));
                draw_sprites(frame, combat, palettes, tiles, camera, smoke);
                for room in self.rooms() {
                    let sprites = reversed(room.front_sprites(&self.laser, camera));
                    draw_sprites(frame, combat, palettes, tiles, camera, sprites);
                }
            }
            SpriteList::Player
            | SpriteList::PlayerAttacks
            | SpriteList::Enemies
            | SpriteList::Bosses => {}
        }
    }
    /// Approach sprites before a mounted hazard's scheduled allocation.
    /// Extrapolate backwards from that allocation's first sprite position,
    /// including the every-other-update spawn scan. This never creates a
    /// collider or advances the obstacle's lifetime or warning schedule.
    pub(crate) fn mounted_approach(
        &self,
        camera_fixed: i32,
        camera_y: i32,
        step: i32,
    ) -> impl Iterator<Item = (u16, i32, i32)> + '_ {
        self.records
            .iter()
            .enumerate()
            .filter_map(move |(index, record)| {
                if step <= 0
                    || record.kind != MOUNTED_OBJECT
                    || self.spawned & record_bit(index) != 0
                    || self.objects.len() >= LIST6_CAPACITY
                    || camera_y < record.y_window.0 - 320
                    || camera_y > record.y_window.1 + 65
                    || (camera_fixed >> 8) > record.x_window.1 + 65
                {
                    return None;
                }
                let remaining = (i64::from(record.x_window.0 - 384) << 8) - i64::from(camera_fixed);
                let mut ahead = (remaining.max(0) as u64).div_ceil(step as u64).max(1);
                if !(self.ticks.saturating_sub(1) + ahead).is_multiple_of(2) {
                    ahead += 1;
                }
                let mapping = if record.param >> 8 == 0 { 0xf2 } else { 0xf3 };
                Some((mapping, record.x + (ahead as i32 - 1) * 8, record.y))
            })
    }

    fn rooms(&self) -> impl Iterator<Item = &Room> {
        self.objects.iter().filter_map(|o| o.room.as_deref())
    }
    pub fn report(&self) -> String {
        let mut text = format!("Stage objects ($27178): {} records\n", self.records.len());
        for (index, r) in self.records.iter().enumerate() {
            text.push_str(&format!(
                "  #{index:2} ${:05x}: type {:2} window x {}..{} y {}..{} at ({},{}) param ${:04x}\n",
                r.source,
                r.kind,
                r.x_window.0,
                r.x_window.1,
                r.y_window.0,
                r.y_window.1,
                r.x,
                r.y,
                r.param
            ));
        }
        text
    }
}

/// A record's bit in `$FF3808`.
fn record_bit(record: usize) -> u32 {
    1 << (record % MAX_RECORDS)
}

impl StageObjects {
    /// `Func_027160`: the object frees itself and clears its record's
    /// `$FF3808` bit so the record can spawn again. False, for `retain`.
    fn free_record(&mut self, object: &Object) -> bool {
        self.spawned &= !record_bit(object.record);
        false
    }
}

/// A record's X and Y window (`+$2C..+$32`) contains `(x, y)`.
pub(super) fn inside(window: ((i32, i32), (i32, i32)), (x, y): (i32, i32)) -> bool {
    let ((x0, x1), (y0, y1)) = window;
    (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
}

/// One sprite of a stage object: its mapping at a world position,
/// mirrored or not, with attribute base `tile` (0: the mapping's own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sprite {
    pub(super) mapping: u16,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) flip: bool,
    pub(super) tile: u16,
    /// `obj_tile` bit 15: the sprite stays over the high plane cells.
    pub(super) high: bool,
}
impl Sprite {
    pub(super) fn new(mapping: u16, x: i32, y: i32) -> Self {
        Self {
            mapping,
            x,
            y,
            flip: false,
            tile: 0,
            high: false,
        }
    }
    pub(super) fn prioritised(self, high: bool) -> Self {
        Self { high, ..self }
    }
    pub(super) fn flipped(self, flip: bool) -> Self {
        Self { flip, ..self }
    }
    pub(super) fn tiled(self, tile: u16) -> Self {
        Self { tile, ..self }
    }
}

/// Each nonblank sprite at its screen position, with its priority.
fn draw_sprites(
    frame: &mut Frame,
    combat: &Combat,
    palettes: &Lines<'_>,
    tiles: &[Tile],
    camera: (i32, i32),
    sprites: impl IntoIterator<Item = Sprite>,
) {
    for s in sprites {
        if s.mapping != 0 {
            let position = (s.x - camera.0, s.y - camera.1);
            frame.mark_high_sprites(s.high);
            combat.draw_object(frame, palettes, tiles, s.mapping, s.tile, position, s.flip);
        }
    }
    frame.mark_high_sprites(false);
}
