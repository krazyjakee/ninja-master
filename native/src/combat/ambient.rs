//! Ambient particles and water splashes (#63).
//!
//! `Func_00915A` runs every play frame between the camera and the bosses.
//! It adds one to `$FF4120` and jumps through the `$9130` table by the part
//! record's word 14, which `$11754` copies to `$FF411E`; `Func_009144`
//! clears `$FF4120..$FF4125` at every part load.
//!
//! | Word 14 | Routine | Parts | Object |
//! | --- | --- | --- | --- |
//! | 0 | `$9158` (`rts`) | all others | none |
//! | 4 | `$916C` | 1-1 parts 0 and 1, every part of 7-1 | list-1 type `$32` (`$9210`): a leaf in 1-1, a dark flake in 7-1 (tile `$106` of each stage's art) |
//! | 8 | `$9244` | 1-2 | list-1 type `$2E` (`$92BE`): a water drip |
//! | `$C` | `$9378` | none | list-1 type `$34`; no part record selects it, so it is not ported |
//! | `$10` | `$940C` | 6-1 | list-9 type `$34` (`$93F0`): a pebble rising up the shaft |
//!
//! All three use mapping `$392`, one 8x8 tile centred on the position; the
//! art is the attribute word. They draw from `Func_0096EE`
//! (`crate::enemies::Ctx::random`, the generator the enemies share); the
//! original also turns it while waiting for the next frame
//! (`Main_WaitFrame`, `$512`), so its sequence cannot be matched.
//!
//! The water splash is list-1 type `$24` (`Func_022D2E`): the player
//! allocates it on entering, leaving and wading through material 15
//! (`crate::movement::player::water`), a player shuriken on entering it.
//! Its routine `$10` (`$22D4E`) is the wake splash Boss4 leaves at the
//! waterline and the surfer behind its board ([`wake`], [`surf_wake`]): it drifts left with the water and shows the
//! `plr_ctrl` = `$3C` frames of `Data_022DD8`. Routine 8 (`$22E34`) is the
//! plume of water the sinking gunboat throws up ([`plume`]).
//!
//! The spawners see this update's camera: on foot `Combat::update_objects`
//! runs after the player and the camera, as `Func_00915A` does.
//!
//! Native differences, all invisible: the fraction words of a new record (`+$22`, `+$26`) and the leaf's
//! `+$28` counter start at 0 rather than at what the record's previous
//! user left; and the list limits count
//! only these objects and the dust of list 9.
use super::{Effect, EffectKind};
use crate::{
    enemies::Shared,
    game::Terrain,
    movement::Material,
    rom::{Result, long, word},
};

/// `Object_ListParams+5` and `+6`: lists 1 and 9 hold 30 records each.
const LIST_RECORDS: usize = 30;
/// `Object_UpdateVisibility` (`$17A4`): the screen centre is `$A0` right of
/// and `$70` below the camera; 256 pixels away from it (either axis) is off
/// screen. Native Stage_VScrollOffset is 0.
const CENTRE: (i32, i32) = (0xa0, 0x70);
const REACH: i32 = 0x100;

/// Values of the spawners and the object handlers, read from the ROM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rules {
    /// `$916C`: `$FF4120 & mask == 0` spawns (every 64 frames, twice).
    leaf_period: u16,
    /// Spawn point: camera + (x, y), plus `random & y_mask` down.
    leaf_offset: (i32, i32),
    leaf_y_mask: u16,
    /// X velocity: base (16.16) minus a random fraction.
    leaf_vx: i32,
    /// Vertical acceleration: base minus `random & mask`.
    leaf_accel_mask: u16,
    leaf_accel: i16,
    mapping: u16,
    leaf_base: u16,
    /// `$9244`: after the timer runs out, `(random & mask) + base` frames.
    drip_period: (u16, u16),
    /// X: camera + add + `random & mask`; Y: 16-pixel camera row - 8.
    drip_x: (u16, i32),
    /// `Data_00923A..$9242`: falling, dropping, landed, spray.
    drip_bases: [u16; 4],
    drip_gravity: i32,
    /// `$932C` and `$9344`: ticks of the landed and spray frames, minus one.
    drip_timers: [i32; 2],
    /// `Data_009358`: nonzero where a drip stops, by material.
    drip_stops: [u8; 16],
    /// `$940C`: period `(random & mask) + base`.
    pebble_period: (u16, u16),
    pebble_x: (u16, i32),
    /// Y: camera + this (just below the screen).
    pebble_y: i32,
    /// High byte of the vertical acceleration (`$FC00`).
    pebble_accel: u16,
    /// `Data_0093FC`: four (mapping, attribute) pairs.
    pebbles: [(u16, u16); 4],
    /// `Data_022DD8`: `$22D2E`'s (mapping, attribute) for each 8-tick step,
    /// in the four `plr_ctrl` groups (0, `$C`, `$18`, `$24`). Mapping is the
    /// table word + `$EF`.
    splashes: [[(u16, u16); 3]; 4],
    /// `$22D9A`: ticks the splash shows.
    splash_life: i32,
    /// `$22D4E`: the wake's four `Data_022DD8` frames at `plr_ctrl` `$3C`
    /// (`$22D66`), and whether the mapping's pieces carry the priority bit
    /// (`$119..$11C` are `$C1F4..$C1FD`: priority, palette 2).
    wake: [(u16, u16, bool); 4],
    /// `$22D52`: the wake's X speed is half the negated water speed
    /// (`Func_022D22`) less this.
    wake_drag: i32,
    /// `$22E5A`: the plume's `Data_022E7E` frames (+`$C9`), shown for four
    /// ticks each from the last, and whether their pieces are high
    /// priority (`$11D..$122`, all `$C1xx`/`$C9xx`).
    plume: [(u16, bool); 8],
    /// `$22E4E`: the plume's ticks (`plr_event`).
    plume_life: i32,
    /// The surfer's wake ([`surf_wake`]).
    surf_wake: SurfWake,
}
/// `Player_StepAnimation` (`$21772..$217AC`): a grounded surfer (4-1,
/// 4-3) allocates a list-1 wake splash on every update whose `plr_ticks`
/// has no bit of `period` set, `offset` from its position, with `+$28` =
/// `life`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SurfWake {
    /// `$2177C`: `andi.w #3`.
    period: u64,
    /// `$21796` (`addi.w #$10`) and `$217A0` (`addq.w #8`).
    offset: (i32, i32),
    /// `$217A8`: `move.w #$20,$28(a6)`.
    life: i32,
}
/// The data field of an `addq`/`subq` opcode word (0 stands for 8).
fn addq(opcode: u16) -> i32 {
    match (opcode >> 9) & 7 {
        0 => 8,
        n => i32::from(n),
    }
}
impl Rules {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        let i = |at: usize| -> Result<i32> { Ok(i32::from(word(rom, at)? as i16)) };
        let mut drip_stops = [0; 16];
        drip_stops.copy_from_slice(rom.get(0x9358..0x9368).ok_or("missing drip table")?);
        let mut pebbles = [(0, 0); 4];
        for (n, pebble) in pebbles.iter_mut().enumerate() {
            *pebble = (word(rom, 0x93fc + n * 4)?, word(rom, 0x93fe + n * 4)?);
        }
        let mut splashes = [[(0, 0); 3]; 4];
        for (group, frames) in splashes.iter_mut().enumerate() {
            for (n, frame) in frames.iter_mut().enumerate() {
                let at = 0x22dd8 + group * 12 + n * 4;
                *frame = (word(rom, at)? + word(rom, 0x22dc8)?, word(rom, at + 2)?);
            }
        }
        let mut wake = [(0, 0, false); 4];
        let group = usize::from(word(rom, 0x22d66)?);
        for (n, frame) in wake.iter_mut().enumerate() {
            let at = 0x22dd8 + group + n * 4;
            let (mapping, attr) = (word(rom, at)? + word(rom, 0x22dc8)?, word(rom, at + 2)?);
            let (_, pieces) = crate::rom::sprite_mapping(rom, mapping)?;
            let high = pieces.iter().all(|piece| piece.attr & 0x8000 != 0);
            *frame = (mapping, attr, high);
        }
        let mut plume = [(0, false); 8];
        for (n, frame) in plume.iter_mut().enumerate() {
            let mapping = word(rom, 0x22e7e + n * 2)? + word(rom, 0x22e76)?;
            let (_, pieces) = crate::rom::sprite_mapping(rom, mapping)?;
            *frame = (mapping, pieces.iter().all(|piece| piece.attr & 0x8000 != 0));
        }
        Ok(Self {
            leaf_period: word(rom, 0x9174)?,
            leaf_offset: (i(0x9194)?, i(0x91a0)?),
            leaf_y_mask: word(rom, 0x91aa)?,
            leaf_vx: long(rom, 0x91be)?,
            leaf_accel_mask: word(rom, 0x91ce)?,
            leaf_accel: word(rom, 0x91d4)? as i16,
            mapping: word(rom, 0x91dc)?,
            leaf_base: word(rom, 0x91e2)?,
            drip_period: (word(rom, 0x9254)?, word(rom, 0x9258)?),
            drip_x: (word(rom, 0x9278)?, i(0x927c)?),
            drip_bases: [
                word(rom, 0x923a)? | word(rom, 0x92a2)?,
                word(rom, 0x923c)?,
                word(rom, 0x9240)?,
                word(rom, 0x9242)?,
            ],
            drip_gravity: i(0x92e8)?,
            drip_timers: [i(0x932c)?, i(0x9344)?],
            drip_stops,
            pebble_period: (word(rom, 0x941c)?, word(rom, 0x9420)?),
            pebble_x: (word(rom, 0x9440)?, i(0x9444)?),
            pebble_y: i(0x9454)?,
            pebble_accel: word(rom, 0x9472)?,
            pebbles,
            splashes,
            splash_life: i(0x22d9c)?,
            wake,
            wake_drag: long(rom, 0x22d52)?,
            plume,
            plume_life: i(0x22e4e)?,
            surf_wake: SurfWake {
                period: u64::from(word(rom, 0x2177c)?),
                offset: (i(0x21796)?, addq(word(rom, 0x217a0)?)),
                life: i(0x217a8)?,
            },
        })
    }
}

impl Rules {
    // Every mapping the particles and the splash can select.
}

/// The `$9130` routine of the part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Routine {
    None,
    Leaves,
    Drips,
    Pebbles,
}

/// `Func_00915A`'s state: `$FF4120` (frames), `$FF4122` (countdown) and
/// `$FF4124` (leaves spawned).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ambient {
    routine: Routine,
    frames: u16,
    countdown: i16,
    spawned: u16,
}
impl Ambient {
    /// The routine part-record word 14 selects; `Func_009144` state.
    pub(crate) fn new(selector: u16) -> Self {
        Self {
            routine: match selector {
                4 => Routine::Leaves,
                8 => Routine::Drips,
                0x10 => Routine::Pebbles,
                _ => Routine::None,
            },
            frames: 0,
            countdown: 0,
            spawned: 0,
        }
    }
    /// One `Func_00915A` call. `random` is `Func_0096EE`.
    pub(crate) fn tick(
        &mut self,
        rules: &Rules,
        camera: (i32, i32),
        mut random: impl FnMut() -> u16,
        effects: &mut Vec<Effect>,
    ) {
        self.frames = self.frames.wrapping_add(1);
        match self.routine {
            Routine::None => {}
            Routine::Leaves => {
                if self.frames & rules.leaf_period != 0 {
                    return;
                }
                self.spawned = self.spawned.wrapping_add(1);
                if !room(effects, List::One) {
                    return;
                }
                let x = camera.0 + rules.leaf_offset.0;
                let y = camera.1 + rules.leaf_offset.1 + i32::from(random() & rules.leaf_y_mask);
                let mut leaf = Effect::new(EffectKind::AmbientLeaf, x, y, rules.mapping);
                leaf.base = rules.leaf_base;
                // $91B2..$91D6: vx = base - random / 65536; the Y
                // acceleration word is base - (random & $FFF).
                let vx = rules.leaf_vx.wrapping_sub(i32::from(random()));
                let accel = rules
                    .leaf_accel
                    .wrapping_sub((random() & rules.leaf_accel_mask) as i16);
                // $91E6..$920A: every other leaf halves its X velocity and
                // takes the acceleration, halved, as X acceleration too.
                let shift = u32::from(self.spawned & 1);
                leaf.motion.vx = vx >> shift;
                leaf.motion.gravity = i32::from(accel);
                leaf.motion.ax = i32::from(accel >> shift);
                effects.push(leaf);
            }
            Routine::Drips => {
                self.countdown -= 1;
                if self.countdown >= 0 {
                    return;
                }
                let (mask, base) = rules.drip_period;
                self.countdown = ((random() & mask) + base) as i16;
                if !room(effects, List::One) {
                    return;
                }
                let (mask, add) = rules.drip_x;
                let x = i32::from(random() & mask) + add + camera.0;
                let y = (camera.1 & !15) - 8;
                let mut drip = Effect::new(EffectKind::Drip, x, y, rules.mapping);
                drip.base = rules.drip_bases[0];
                effects.push(drip);
            }
            Routine::Pebbles => {
                self.countdown -= 1;
                if self.countdown >= 0 {
                    return;
                }
                let (mask, base) = rules.pebble_period;
                self.countdown = ((random() & mask) + base) as i16;
                if !room(effects, List::Nine) {
                    return;
                }
                let (mask, add) = rules.pebble_x;
                let x = i32::from(random() & mask) + add + camera.0;
                let y = camera.1 + rules.pebble_y;
                // $945A..$947A: vy is -(r & 3) + r / 65536; the acceleration's
                // low byte is that integer part's.
                let r = random();
                let rise = (r & 3).wrapping_neg();
                let (mapping, base) = rules.pebbles[usize::from(random() & 3)];
                let mut pebble = Effect::new(EffectKind::Pebble, x, y, mapping);
                pebble.base = base;
                // Object_AllocL9 ($93F0): drawn behind every other list.
                pebble.back = true;
                pebble.motion.vy = (i32::from(rise as i16) << 16) | i32::from(r);
                pebble.motion.gravity =
                    i32::from((rules.pebble_accel & 0xff00 | rise & 0xff) as i16);
                effects.push(pebble);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum List {
    One,
    Nine,
}
/// Preserve RNG gating for the ambient spawners: they share the enemy random
/// stream and skip coordinate/velocity draws when their original list is full.
/// This is a gameplay timing rule, not a capacity limit on native visuals.
fn room(effects: &[Effect], list: List) -> bool {
    let count = effects
        .iter()
        .filter(|effect| match list {
            List::One => {
                matches!(
                    effect.kind,
                    EffectKind::AmbientLeaf
                        | EffectKind::Drip
                        | EffectKind::Splash
                        | EffectKind::HorseWreck
                ) || (matches!(effect.kind, EffectKind::Wake | EffectKind::Plume) && !effect.back)
            }
            List::Nine => {
                matches!(
                    effect.kind,
                    EffectKind::Pebble
                        | EffectKind::Dust
                        | EffectKind::GuardPart
                        | EffectKind::ThrownRider
                        | EffectKind::SurfSlashBoard
                ) || (effect.kind == EffectKind::Wake && effect.back)
            }
        })
        .count();
    count < LIST_RECORDS
}

/// `Object_UpdateVisibility`: `obj_flags` bit 7.
pub(crate) fn off_screen((x, y): (i32, i32), camera: (i32, i32)) -> bool {
    (camera.0 + CENTRE.0 - x).abs() >= REACH || (camera.1 + CENTRE.1 - y).abs() >= REACH
}

/// A list-1 type-`$24` splash in frame group `group` (see
/// `crate::movement::SplashKind::group`). The player
/// (`Player_CollideVertical`, `Player_CollideCeiling`,
/// `Player_CollideWalls`) and the shuriken (`$24E24`) only allocate it; its
/// first update is the list-1 pass ([`update`]) of the same frame.
/// `$22D86` snaps Y to its 16-pixel row.
pub(crate) fn splash(rules: &Rules, (x, y): (i32, i32), group: usize) -> Option<Effect> {
    let mut effect = Effect::new(EffectKind::Splash, x, y & !15, 0);
    effect.phase = u8::try_from(group).ok().filter(|&g| g < 4)?;
    effect.counter = rules.splash_life;
    Some(effect)
}
/// `Func_022DA4`: count down; each 8-tick step shows the group's entry
/// `(counter & $18) >> 3`, so the table is read backwards.
fn splash_step(rules: &Rules, effect: &mut Effect) {
    effect.counter -= 1;
    if effect.counter < 0 {
        effect.alive = false;
        return;
    }
    let step = ((effect.counter & 0x18) >> 3) as usize;
    let (mapping, attr) = rules.splashes[usize::from(effect.phase)][step];
    effect.mapping = mapping;
    effect.base = attr & !0x800;
    effect.flip = attr & 0x800 != 0;
}

/// A type-`$24` record allocated with routine `$10` and `life` in `+$28`
/// (Boss4's `Func_02EE38`), in list 9 when `back`. The list-1/9 pass
/// ([`update`]) gives it its first update in the frame that allocates it:
/// `$22D4E` sets its X speed from `water`, the `$FF0190` water speed
/// (16.16), then `Func_022DA4` and `$19D4` run.
pub(crate) fn wake(rules: &Rules, (x, y): (i32, i32), life: i32, water: i32, back: bool) -> Effect {
    let mut effect = Effect::new(EffectKind::Wake, x, y, 0);
    effect.counter = life;
    effect.back = back;
    effect.motion.vx = wake_speed(rules, water);
    effect
}
/// `$22D4E`: a wake splash's X speed, `Func_022D22` (`move.l ($FF0190),d0`
/// / `neg.l d0` / `asr.l #1,d0`) less [`Rules::wake_drag`].
fn wake_speed(rules: &Rules, water: i32) -> i32 {
    (water.wrapping_neg() >> 1).wrapping_sub(rules.wake_drag)
}
/// The surfer's wake of an update that ends with `plr_ticks` = `ticks` and
/// the rider at `(x, y)` on the water (`$21772..$217AC`), or `None` on the
/// other updates. The caller checks list 1 for room.
pub(crate) fn surf_wake(
    rules: &Rules,
    (x, y): (i32, i32),
    ticks: u64,
    water: i32,
) -> Option<Effect> {
    let SurfWake {
        period,
        offset,
        life,
    } = rules.surf_wake;
    // Player_StepAnimation only allocates. The mounted player now runs
    // before list 1, which gives this record its first countdown and move.
    (ticks & period == 0).then(|| wake(rules, (x + offset.0, y + offset.1), life, water, false))
}
/// `$22D6E`: `Func_022DA4` over the `$3C` group, keeping the tile word
/// it had (`andi.w #$7FF` / `or.w`).
fn wake_step(rules: &Rules, effect: &mut Effect) {
    effect.counter -= 1;
    if effect.counter < 0 {
        effect.alive = false;
        return;
    }
    let step = ((effect.counter & 0x18) >> 3) as usize;
    let (mapping, attr, high) = rules.wake[step];
    let tile = (attr & 0x7ff) | effect.base | if effect.flip { 0x800 } else { 0 };
    effect.mapping = mapping;
    effect.base = tile & !0x800;
    effect.flip = tile & 0x800 != 0;
    // `Object_DrawSprite` (`$156E`): the tile word's top bits flip the
    // piece's (`eor.w d5,d0`).
    effect.high_priority = high != (tile & 0x8000 != 0);
}

/// A type-`$24` record allocated in list 1 with routine 8 (Boss4's sinking
/// gunboat, `$2E33C`). Its first update, in the frame that allocates it
/// ([`update`]): `$22E34` sets half the negated `water` speed (16.16) as
/// its X speed and `plr_event` to [`Rules::plume_life`], then `$22E5A` and
/// `$19D4` run.
pub(crate) fn plume(rules: &Rules, (x, y): (i32, i32), water: i32) -> Effect {
    let mut effect = Effect::new(EffectKind::Plume, x, y, 0);
    effect.counter = rules.plume_life;
    effect.motion.vx = water.wrapping_neg() >> 1;
    effect
}
/// A routine-8 plume that an object of a later list allocates in list 1
/// (the rider thrown off in a surf death, `$26D3A`/`$26D64`), as that frame leaves it:
/// the motion pass moves every record after the handlers ran, so the new
/// one moves once with the recycled record's velocity, in 4-1 a spent
/// wake splash's (`wake_speed`: Genesis Plus GX, 5 px left). `$22E34` and
/// its first update come in the next frame; its X speed and life are set
/// here, which is the same. `tile_priority` is the tile word's bit 15 (the
/// rider's), which flips its pieces' priority.
pub(crate) fn pending_plume(
    rules: &Rules,
    (x, y): (i32, i32),
    water: i32,
    tile_priority: bool,
) -> Effect {
    let mut effect = Effect::new(EffectKind::Plume, x, y, 0);
    effect.motion.vx = wake_speed(rules, water);
    effect.motion.step(&mut effect.x, &mut effect.y);
    effect.motion.vx = water.wrapping_neg() >> 1;
    effect.counter = rules.plume_life;
    effect.tile_priority = tile_priority;
    effect
}
/// `$22E5A`: count down; frame `(counter >> 2) & 7` of `Data_022E7E`.
fn plume_step(rules: &Rules, effect: &mut Effect) {
    effect.counter -= 1;
    if effect.counter < 0 {
        effect.alive = false;
        return;
    }
    let (mapping, high) = rules.plume[((effect.counter >> 2) & 7) as usize];
    effect.mapping = mapping;
    effect.high_priority = high != effect.tile_priority;
}

/// Whether `update` handles `kind`: these objects leave through
/// `Object_UpdateVisibility` or their own counters, never the generic
/// camera test of the other effects.
pub(crate) fn owns(kind: EffectKind) -> bool {
    matches!(
        kind,
        EffectKind::AmbientLeaf
            | EffectKind::Drip
            | EffectKind::Pebble
            | EffectKind::Splash
            | EffectKind::Wake
            | EffectKind::Plume
    )
}

/// One update of an object `owns` accepts: its handler, `$19D4`, then
/// visibility. `effect.age` has already counted this update (`+$28`).
pub(crate) fn update(
    shared: &Shared,
    effect: &mut Effect,
    camera: (i32, i32),
    terrain: &dyn Terrain,
) {
    // The handler consumes last frame's flag; $17A4 writes the next one
    // only in Objects_BuildSprites. $92C8 keeps a drip inside rock alive.
    let tested = matches!(effect.kind, EffectKind::AmbientLeaf | EffectKind::Pebble)
        || (effect.kind == EffectKind::Drip && effect.phase != 0);
    if tested && effect.flags & 0x80 != 0 {
        effect.alive = false;
        return;
    }
    update_visible(shared, effect, camera, terrain);
    effect.flags = (effect.flags & !0x80)
        | if off_screen((effect.x, effect.y), camera) {
            0x80
        } else {
            0
        };
}

fn update_visible(
    shared: &Shared,
    effect: &mut Effect,
    _camera: (i32, i32),
    terrain: &dyn Terrain,
) {
    let rules = &shared.ambient;
    match effect.kind {
        EffectKind::AmbientLeaf => {
            // $9210: +$28 >> 2 turns the leaf: bit 0 mirrors it, bit 1 flips it.
            let turn = (effect.age >> 2) & 3;
            effect.flip = turn & 1 != 0;
            effect.base = (effect.base & !0x1800) | if turn & 2 != 0 { 0x1000 } else { 0 };
        }
        EffectKind::Drip => drip(rules, effect, terrain),
        EffectKind::Splash => {
            splash_step(rules, effect);
            return;
        }
        EffectKind::Wake | EffectKind::Plume => {
            // Hidden off screen but kept until its counter runs out.
            if effect.kind == EffectKind::Wake {
                wake_step(rules, effect);
            } else {
                plume_step(rules, effect);
            }
            if effect.alive {
                effect.motion.step(&mut effect.x, &mut effect.y);
            }
            return;
        }
        _ => {}
    }
    if !effect.alive {
        return;
    }
    effect.motion.step(&mut effect.x, &mut effect.y);
}

/// `Func_0092BE`, by `+$04` / 4 in `phase`.
fn drip(rules: &Rules, effect: &mut Effect, terrain: &dyn Terrain) {
    // Func_009368: `$3854`'s material through `Data_009358`.
    let stops = |x: i32, y: i32| {
        let Material(code) = terrain.material_at(x, y);
        rules.drip_stops[usize::from(code & 15)] != 0
    };
    if effect.phase == 0 {
        // $92C8: 8 px per update through the rock it starts in, until the
        // pixel below is open.
        effect.y += 8;
        if stops(effect.x, effect.y + 1) {
            return;
        }
        effect.base = rules.drip_bases[1];
        effect.motion.gravity = rules.drip_gravity;
        effect.phase = 1;
    }
    if effect.phase == 1 {
        // $92F0: fall until the drip itself is in a stopping material.
        if !stops(effect.x, effect.y) {
            return;
        }
        // $930A: land 4 px above the 16-pixel row.
        effect.y = (effect.y & !15) - 4;
        effect.base = rules.drip_bases[2];
        effect.motion.gravity = 0;
        effect.motion.vy = 0;
        effect.counter = rules.drip_timers[0];
        effect.phase = 2;
    }
    if effect.phase == 2 {
        effect.counter -= 1;
        if effect.counter >= 0 {
            return;
        }
        effect.base = rules.drip_bases[3];
        effect.counter = rules.drip_timers[1];
        effect.phase = 3;
    }
    effect.counter -= 1;
    if effect.counter < 0 {
        effect.alive = false;
    }
}
