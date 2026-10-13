//! Types `$08`, `Enemy08` (`$132BC`), the rider spawner with its list-9 carrier
//! (`$157D2`), and `$0C`, `Enemy0C` (`$13466`), the horsemen it drops.
use crate::{
    combat::{Effect, EffectKind, Enemy, EnemyState, Motion, Projectile, Spawn, advance},
    enemies::{Behavior, Ctx, HitResponse, Offsets, aim, set_fraction, set_whole},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `$2A`: rider count / flags (the carrier clears it after hovering);
/// `$2C`: carried riders left; `$28`: spawn countdown; carrier state.
const COUNT: usize = 0;
const LEFT: usize = 1;
const COUNTDOWN: usize = 2;
/// The kite's routine (`+$04` of its list-9 record), its `$2A` timer, its
/// `$30` sway angle and `$32` hover offset from the camera.
pub(super) const CARRIER: usize = 3;
const CARRIER_TIMER: usize = 4;
const ANGLE: usize = 5;
const OFFSET: usize = 6;
const HOLD: usize = 7;
/// Native routine: the spawner freed itself while its carrier leaves.
const SPAWNER_DONE: u16 = 0x20;

/// `$08`: invisible spawner of `$0C` riders. Spawn flags bit 4 selects the
/// carrier mode: a list-9 carrier (frames `$16C`/`$16D`) flies in from the
/// right, hovers, and drops `N + 1` riders at 60-tick intervals; otherwise
/// `N` riders enter from the left edge at random 17..80-tick gaps. The
/// native object is drawn as the carrier (it has no sprite of its own).
pub(crate) struct Spawner {
    far_frame: u16,
    drop_frame: u16,
    carrier: Carrier,
    edge: (i32, i32),
    drop_gravity: i32,
}
impl Spawner {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            far_frame: word(rom, 0x13364)?,
            drop_frame: word(rom, 0x13424)?,
            carrier: Carrier::load(rom)?,
            edge: (signed(rom, 0x13344)?, word_i(rom, 0x13350)?),
            drop_gravity: word_i(rom, 0x1340a)?,
        })
    }
    /// `Object_AllocL5(3)`: a `$0C` with the spawner's tile (base and flip).
    fn rider<'a>(cx: &'a mut Ctx, x: i32, y: i32) -> Option<&'a mut Enemy> {
        let tile = cx.tile_base(0x08);
        let rider = cx.spawn(0x0c, x, y)?;
        rider.tile = Some(tile);
        rider.facing_left = true;
        Some(rider)
    }
    /// The spawner's `Func_0129B8`: the carrier (if any) is released.
    fn finish(e: &mut Enemy) {
        e.vars[HOLD] = 0;
        if e.vars[CARRIER] != 0 {
            e.routine = SPAWNER_DONE;
        } else {
            Ctx::free(e);
        }
    }
    fn spawner(&self, cx: &mut Ctx, e: &mut Enemy) {
        loop {
            match e.routine {
                0 => {
                    // $132D0: carrier mode with spawn flag bit 4.
                    e.motion.stop();
                    e.routine = if e.vars[COUNT] & 0x10 != 0 { 0x0c } else { 4 };
                    return;
                }
                4 => {
                    // $13300: next rider after 16..79 ticks; none left frees.
                    e.vars[COUNTDOWN] = 0x10 + i32::from(cx.random() & 0x3f);
                    e.vars[COUNT] &= 0xf;
                    if e.vars[COUNT] == 0 {
                        Self::finish(e);
                        return;
                    }
                    e.routine = 8;
                }
                8 => {
                    // $1331E: a background rider enters at the left edge.
                    e.vars[COUNTDOWN] -= 1;
                    if e.vars[COUNTDOWN] >= 0 {
                        return;
                    }
                    e.vars[COUNT] -= 1;
                    e.routine = 4;
                    let (x, y) = (
                        cx.entry_x(self.edge.0, &[self.far_frame, self.drop_frame], true),
                        cx.camera.1 + self.edge.1,
                    );
                    let frame = self.far_frame;
                    if let Some(rider) = Self::rider(cx, x, y) {
                        rider.motion.vx = 1 << 16;
                        rider.mapping = frame;
                        rider.routine = 4;
                    }
                    return;
                }
                0x0c => {
                    // $13376: N + 1 riders from a carrier holding 64..191 ticks.
                    e.vars[LEFT] = e.vars[COUNT] & 0xf;
                    let ticks = 0x40 + i32::from(cx.random() & 0x7f);
                    self.carrier.launch(cx.camera, e, ticks);
                    e.x = cx.entry_x(
                        self.carrier.offset.0,
                        &[self.carrier.frame, self.carrier.frame + 1],
                        false,
                    );
                    e.vars[HOLD] = 1;
                    e.vars[COUNT] = 1;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $133A2: wait for the carrier to settle.
                    if e.vars[COUNT] == 0 {
                        e.routine = 0x14;
                    }
                    return;
                }
                0x14 => {
                    // $133AE: ($1C & $1F) + $1F = 59 ticks between riders.
                    e.vars[COUNTDOWN] = (0x1c & 0x1f) + 0x1f;
                    if e.vars[LEFT] < 0 {
                        e.routine = 0x1c;
                        continue;
                    }
                    e.routine = 0x18;
                }
                0x18 => {
                    // $133C8: a rider drops from the carrier.
                    e.vars[COUNTDOWN] -= 1;
                    if e.vars[COUNTDOWN] >= 0 {
                        return;
                    }
                    e.vars[LEFT] -= 1;
                    e.routine = 0x14;
                    // $FF001C: the generator's current value, not advanced.
                    let seed = *cx.random;
                    let (x, y) = (e.x - 8, e.y);
                    let (frame, gravity) = (self.drop_frame, self.drop_gravity);
                    if let Some(rider) = Self::rider(cx, x, y) {
                        rider.motion.vx = set_fraction(-1 << 16, seed);
                        rider.motion.vy = set_fraction(-1 << 16, seed);
                        rider.motion.gravity = gravity;
                        rider.mapping = frame;
                    }
                    return;
                }
                0x1c => {
                    // $13436
                    Self::finish(e);
                    return;
                }
                _ => return,
            }
        }
    }
    /// The list-9 carrier, in the spawner's record.
    fn carrier(&self, cx: &mut Ctx, e: &mut Enemy) {
        match self.carrier.step(cx, e, e.vars[HOLD] != 0) {
            // $15942: the hover ends and the spawner may drop riders.
            CarrierStep::Hovered => e.vars[COUNT] = 0,
            CarrierStep::Gone => {
                if e.routine == SPAWNER_DONE {
                    Ctx::free(e);
                }
            }
            CarrierStep::Flying => {}
        }
    }
}

/// The data field of an `addq`/`subq` opcode (0 means 8).
pub(super) fn quick(opcode: u16) -> i32 {
    match (opcode >> 9) & 7 {
        0 => 8,
        q => i32::from(q),
    }
}

/// What a carrier update did, for its launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CarrierStep {
    Flying,
    /// `$15942`: the hover ended (the launcher's `$2A` is cleared).
    Hovered,
    /// `$15980`: it left the screen and was freed.
    Gone,
}

/// The list-9 kite (`Func_015894`, routines `$14..$20` of `$157D2`) that
/// the `$08` spawner and the `$1C` guard launch. It flies in from the
/// camera plus ($150, $70), hovers on a sway, holds while its launcher's
/// `obj_flags` bit 0 is set, then flies off. Each update before it leaves
/// draws a wind streak (`Func_0159E0`). Native: it lives in a record of its
/// launcher's type, drawn in the list-9 pass ([`Enemy::back`]).
pub(super) struct Carrier {
    pub(super) frame: u16,
    velocity: (i32, i32),
    lift: i32,
    pub(super) offset: (i32, i32),
    hover_ticks: i32,
    streak: u16,
    streak_offset: (i32, i32),
    /// `$15A28` then `$15A36`: the first update of an even-tick streak
    /// adds both steps.
    streak_steps: [(i32, i32); 2],
}
impl Carrier {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            frame: word(rom, 0x158de)?,
            velocity: (long(rom, 0x158ac)?, long(rom, 0x158b4)?),
            lift: signed(rom, 0x158bc)?,
            offset: (word_i(rom, 0x158ce)?, word_i(rom, 0x158d2)?),
            hover_ticks: word_i(rom, 0x15908)?,
            streak: word(rom, 0x15a1c)?,
            streak_offset: (signed(rom, 0x15a0c)?, signed(rom, 0x15a10)?),
            streak_steps: [
                (word_i(rom, 0x15a2a)?, quick(word(rom, 0x15a2e)?)),
                (word_i(rom, 0x15a38)?, word_i(rom, 0x15a3e)?),
            ],
        })
    }
    /// `Func_015894` with d6 = `ticks`.
    pub(super) fn launch(&self, camera: (i32, i32), e: &mut Enemy, ticks: i32) {
        e.vars[CARRIER] = 0x14;
        e.vars[CARRIER_TIMER] = ticks;
        e.x = camera.0 + self.offset.0;
        e.y = camera.1 + self.offset.1;
        self.fly(e);
        e.counter = 0;
        e.mapping = self.frame;
        e.back = true;
    }
    fn fly(&self, e: &mut Enemy) {
        e.motion = Motion {
            vx: self.velocity.0,
            vy: self.velocity.1,
            gravity: self.lift,
            ..e.motion
        };
    }
    /// `Func_015992`: sway about the hover point (`$30` angle += 2). The
    /// result is also `$FF3A0C`/`$FF3A0E`, where the `$1C` guard drops.
    fn hover(cx: &Ctx, e: &mut Enemy) {
        e.vars[ANGLE] = (e.vars[ANGLE] + 2) & 0xffff;
        let d = cx.shared.sine.polar(e.vars[ANGLE] as u16).0 >> 3;
        let anchor = i64::from(cx.camera.0 + e.vars[OFFSET]) << 16;
        e.x = ((anchor + i64::from(d)) >> 16) as i32;
        advance(&mut e.y, &mut e.motion.fy, -d);
    }
    /// `Func_0159E0`: a list-9 `$16E` streak behind the kite. It moves on
    /// its first update (routine `$24` on even ticks adds both steps), then
    /// once more, and is freed on the third.
    fn streak(&self, cx: &mut Ctx, e: &Enemy) {
        let [first, step] = self.streak_steps;
        let (mut x, mut y) = (e.x + self.streak_offset.0, e.y + self.streak_offset.1);
        if e.counter & 1 == 0 {
            (x, y) = (x + first.0, y + first.1);
        }
        // The effect pass of this frame is its first update.
        let mut streak = Effect::new(EffectKind::Pieces, x, y, self.streak);
        streak.motion.vx = step.0 << 16;
        streak.motion.vy = step.1 << 16;
        streak.counter = 2;
        streak.back = true;
        cx.effect(streak);
    }
    /// One update of `$157D2` routines `$14..$20`; `held` is the
    /// launcher's `obj_flags` bit 0.
    pub(super) fn step(&self, cx: &mut Ctx, e: &mut Enemy, held: bool) -> CarrierStep {
        if e.vars[CARRIER] == 0 {
            return CarrierStep::Flying;
        }
        e.counter = e.counter.wrapping_add(1);
        // Func_0159CC: two wing frames, eight ticks each.
        e.mapping = self.frame + ((e.counter >> 3) & 1);
        loop {
            match e.vars[CARRIER] {
                0x14 => {
                    // $158EE: fly in, then hover for $80 ticks.
                    self.streak(cx, e);
                    e.vars[CARRIER_TIMER] -= 1;
                    if e.vars[CARRIER_TIMER] >= 0 {
                        return CarrierStep::Flying;
                    }
                    e.motion.stop();
                    e.vars[CARRIER_TIMER] = self.hover_ticks;
                    e.vars[ANGLE] = 0x100;
                    e.vars[OFFSET] = e.x - cx.camera.0;
                    e.vars[CARRIER] = 0x18;
                }
                0x18 => {
                    // $15922: hover; afterwards the launcher goes on.
                    self.streak(cx, e);
                    Self::hover(cx, e);
                    e.vars[CARRIER_TIMER] -= 1;
                    if e.vars[CARRIER_TIMER] >= 0 {
                        return CarrierStep::Flying;
                    }
                    e.vars[CARRIER] = 0x1c;
                    return CarrierStep::Hovered;
                }
                0x1c => {
                    // $15948: hover while held, then fly off.
                    Self::hover(cx, e);
                    self.streak(cx, e);
                    if held {
                        return CarrierStep::Flying;
                    }
                    self.fly(e);
                    e.vars[CARRIER] = 0x20;
                    return CarrierStep::Flying;
                }
                _ => {
                    // $15980: removed off screen.
                    if e.offscreen {
                        e.vars[CARRIER] = 0;
                        e.mapping = 0;
                        return CarrierStep::Gone;
                    }
                    return CarrierStep::Flying;
                }
            }
        }
    }
}
impl Behavior for Spawner {
    fn label(&self, _kind: u16) -> String {
        "Enemy08 $132BC (+ carrier $157D2)".into()
    }
    fn activate(&self, _cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.state = EnemyState::Waiting;
        e.vars[COUNT] = i32::from(spawn.flags);
        // The carrier's own tile is 0 (Object_InitRecord): absolute art.
        e.tile = Some(0);
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        self.spawner(cx, e);
        if e.live() {
            self.carrier(cx, e);
        }
    }
    fn hit(&self, _enemy: &Enemy, _kind: u16) -> HitResponse {
        HitResponse::Pass
    }
}

// ---------------------------------------------------------------------------
// Type $0C, Enemy0C ($13466).

/// `$2A` jump countdown, `$2C` retreat trigger (screen X), `$2E` gallop
/// frame, `$30` passes left, `obj_flags` bit 0 (star thrown this jump).
const JUMP: usize = 0;
const TRIGGER: usize = 1;
const GALLOP: usize = 2;
const PASSES: usize = 3;
const THROWN: usize = 4;

/// `$0C`: a horseman. One background pass (`$167`/`$168`) left to right,
/// then it re-enters from the right in the foreground, gallops left, jumps
/// after 32..95 ticks and throws one aimed star at the apex, lands and
/// retreats right; the next pass ends in a dash off to the left. Hittable
/// only in the foreground routines; one hit kills (`$36D9A` blast).
pub(crate) struct Rider {
    lane: i32,
    road: i32,
    far: u16,
    gallop: u16,
    jump: u16,
    rise: u16,
    fall: u16,
    dash: u16,
    dead: u16,
    entry: i32,
    exit: i32,
    jump_vy: i32,
    jump_gravity: i32,
    jump_vx: i32,
    dash_vx: i32,
    death: u16,
    star: Offsets,
    star_sound: u8,
}
impl Rider {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            lane: word_i(rom, 0x13474)?,
            road: word_i(rom, 0x1350a)?,
            far: word(rom, 0x13364)?,
            gallop: word(rom, 0x1368c)?,
            jump: word(rom, 0x13556)?,
            rise: word(rom, 0x136a6)?,
            fall: word(rom, 0x136b6)?,
            dash: word(rom, 0x135e0)?,
            dead: word(rom, 0x13618)?,
            entry: word_i(rom, 0x13502)?,
            exit: word_i(rom, 0x13576)?,
            jump_vy: signed(rom, 0x13544)?,
            jump_gravity: word_i(rom, 0x1354a)?,
            jump_vx: signed(rom, 0x13550)?,
            dash_vx: signed(rom, 0x135e6)?,
            death: word(rom, 0x13492)?,
            star: Offsets::read(rom, 0x1d1ce)?,
            star_sound: byte(rom, 0x13667)?,
        })
    }
    /// `Func_01366E`: six gallop poses, one every four ticks.
    fn gallop(&self, e: &mut Enemy) {
        if e.timer & 3 != 0 {
            return;
        }
        e.vars[GALLOP] += 1;
        if e.vars[GALLOP] >= 6 {
            e.vars[GALLOP] = 0;
        }
        e.mapping = self.gallop + e.vars[GALLOP] as u16;
    }
    /// `Func_013694`: rising slower than 1 px/tick, or falling after the throw.
    fn jump_pose(&self, e: &mut Enemy) {
        if e.motion.vy < 0 {
            if -e.motion.vy < 0x10000 {
                e.mapping = self.rise;
            }
        } else if e.vars[THROWN] != 0 {
            e.mapping = self.fall;
        }
    }
    /// `.loc_013534`: retreat right at 4 px/tick plus a random fraction.
    fn retreat(cx: &mut Ctx, e: &mut Enemy) {
        e.routine += 4;
        e.motion.vx = set_fraction(4 << 16, cx.random());
    }
    /// `Func_013638`: one star (projectile ID 4) aimed at the player at
    /// three times the `Func_009562` direction.
    fn throw(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[THROWN] != 0 {
            return;
        }
        let (vx, vy) = aim((e.x, e.y), cx.player);
        let (x, y) = self.star.place(e.x, e.y, e.facing_left, false);
        let damage = cx.star_damage;
        let star = Projectile::new(x, y, 0, true, 0x35b, damage)
            .with_motion(Motion {
                vx: vx * 3,
                vy: vy * 3,
                ..Motion::default()
            })
            .unset_frame();
        if cx.shoot(star) {
            e.vars[THROWN] = 1;
            cx.visible_sound(e.x, e.y, self.star_sound);
        }
    }
}
impl Behavior for Rider {
    fn label(&self, _kind: u16) -> String {
        "Enemy0C $13466".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.far
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        loop {
            match e.routine {
                0 => {
                    // $13472: a dropped rider falls to the background lane.
                    if e.y <= self.lane {
                        return;
                    }
                    e.y = self.lane;
                    e.timer = 0;
                    e.motion.stop();
                    e.motion.vx = 1 << 16;
                    e.routine = 4;
                }
                4 => {
                    // $13490: background pass until off screen.
                    e.death_routine = self.death;
                    e.motion.vx = set_fraction(e.motion.vx, cx.random());
                    e.mapping = self.far + ((e.timer >> 2) & 1);
                    if e.offscreen {
                        e.vars[PASSES] = 1;
                        e.routine = 8;
                    }
                    return;
                }
                8 => {
                    // $134C2: re-enter from the right on the road, below
                    // the high-priority plane cells (`obj_tile` bit 15 off).
                    e.high_priority = false;
                    e.vars[PASSES] -= 1;
                    e.motion.vx = set_fraction(-2 << 16, cx.random());
                    e.vars[JUMP] = 0x20 + i32::from(cx.random() & 0x3f);
                    e.vars[TRIGGER] = i32::from(cx.random() & 0xff);
                    e.vars[GALLOP] = 0;
                    e.x = cx.entry_x(self.entry, &[self.gallop], e.facing_left);
                    e.y = self.road;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $13512: gallop left; jump, or turn back at the trigger.
                    if cx.hit_taken(e) {
                        return;
                    }
                    self.gallop(e);
                    e.vars[JUMP] -= 1;
                    if e.vars[JUMP] < 0 {
                        e.motion.vy = set_whole(e.motion.vy, self.jump_vy);
                        e.motion.gravity = self.jump_gravity;
                        e.motion.vx = set_whole(e.motion.vx, self.jump_vx);
                        e.mapping = self.jump;
                        e.routine = 0x14;
                        return;
                    }
                    if cx.camera.0 + e.vars[TRIGGER] < e.x {
                        return;
                    }
                    Self::retreat(cx, e);
                    return;
                }
                0x10 => {
                    // $13562: retreat right; off screen, pass again.
                    if cx.hit_taken(e) {
                        return;
                    }
                    self.gallop(e);
                    if e.x <= cx.camera.0 + self.exit {
                        return;
                    }
                    if e.offscreen {
                        e.routine = 8;
                    }
                    return;
                }
                0x14 => {
                    // $1358C: jump; throw once falling; land on the road.
                    if cx.hit_taken(e) {
                        return;
                    }
                    self.jump_pose(e);
                    if ((e.motion.vy >> 16) < 0) != (e.motion.gravity < 0) {
                        return;
                    }
                    self.throw(cx, e);
                    if e.y <= self.road {
                        return;
                    }
                    e.vars[THROWN] = 0;
                    e.y = self.road;
                    e.motion.vy = 0;
                    e.motion.gravity = 0;
                    e.routine = 0x0c;
                    if e.vars[PASSES] >= 0 {
                        Self::retreat(cx, e);
                    } else {
                        e.routine = 0x18;
                        e.mapping = self.dash;
                        e.motion.vx = set_whole(e.motion.vx, self.dash_vx);
                    }
                    return;
                }
                0x18 => {
                    // $135EC: dash off, freed off screen.
                    if e.offscreen {
                        Ctx::free(e);
                    }
                    return;
                }
                0x1c => {
                    // $135F8: thrown back (score central), 17 ticks.
                    e.motion.vx = set_whole(e.motion.vx, self.dash_vx);
                    e.motion.vy = set_whole(e.motion.vy, -2);
                    e.motion.ax = 0;
                    e.motion.gravity = 0x2800;
                    e.vars[JUMP] = 16;
                    e.mapping = self.dead;
                    e.routine = 0x20;
                    return;
                }
                _ => {
                    // $13626: Func_036D9A blast, then free.
                    e.vars[JUMP] -= 1;
                    if e.vars[JUMP] >= 0 {
                        return;
                    }
                    let blast = cx.shared.wide_blast;
                    cx.explode(e.x, e.y, blast);
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        // Func_0128B2 is called only by the foreground routines.
        if matches!(enemy.routine, 0x0c | 0x10 | 0x14) {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
}
