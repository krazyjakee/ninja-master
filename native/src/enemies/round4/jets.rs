//! Type `$6C`, `Enemy6C` (`$173B2`), the squadron spawner, and the jet riders it
//! releases: `$60` (`Enemy60`, `$17BB6`) and `$70` (`Enemy70`, `$17474`).
use super::{Mines, count_down, nudge_y, settle, splash};
use crate::{
    combat::{Effect, EffectKind, Enemy, EnemyState, Motion, Projectile, Spawn, SpriteList},
    enemies::{Behavior, Ctx, HitResponse, shot_damage, stop},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `$2A`: remaining count (spawn flags & $F); `$2C`: 0 for `$70`, 2 for `$60`.
const SQ_COUNT: usize = 0;
const SQ_MODE: usize = 1;

/// `$6C`: every 65 ticks allocates the next rider (`$70`, or `$60` with
/// spawn flag bit 4) at camera + (-32, `Data_01733C`), moving right at
/// 1 px/tick with the spawner's flipped `$70` graphics and its health
/// counter; freed when the count (flags & $F) runs out.
pub(crate) struct Squadron {
    period: u16,
    kinds: [u16; 2],
    /// `Data_01733C` words by index (indices past its four entries read
    /// on into the following bytes, as the ROM does).
    heights: [i32; 16],
    x_offset: i32,
    mapping: u16,
    speed: i32,
    mines: Mines,
    /// Every frame a released rider draws at its X ([`RiderParts::frames`]).
    frames: [u16; 7],
}
impl Squadron {
    pub(super) fn load(rom: &[u8], mines: Mines) -> Result<Self> {
        let mut heights = [0; 16];
        for (i, height) in heights.iter_mut().enumerate() {
            *height = signed(rom, 0x1733c + i * 2)?;
        }
        Ok(Self {
            period: word(rom, 0x173f8)?,
            // Data_017344: Object_AllocL5 indices, * 4 for the type offset.
            kinds: [word(rom, 0x17344)? << 2, word(rom, 0x17346)? << 2],
            heights,
            x_offset: signed(rom, 0x17378)?,
            mapping: word(rom, 0x1738c)?,
            speed: signed(rom, 0x17380)? << 16,
            mines,
            frames: RiderParts::load(rom)?.frames,
        })
    }
    /// `Func_017348`. `Frame_Counter & 1` (the `$60` heights) is the
    /// native object age's parity.
    fn release(&self, cx: &mut Ctx, e: &Enemy, spawn: &Spawn) {
        let mode = e.vars[SQ_MODE];
        let index = if mode == 0 {
            e.vars[SQ_COUNT]
        } else {
            (e.age & 1) as i32 + 2
        };
        let height = self.heights[(index & 15) as usize];
        let kind = self.kinds[usize::from(mode != 0)];
        // The $6C's own `Object_SetTileBase($70)` tile, copied to the rider.
        let tile = cx.tile_base(0x70);
        // Behind the left edge, clear of a widened view (issue #32).
        let x = cx.entry_x(self.x_offset, &self.frames, true);
        let y = cx.camera.1 + height;
        let count = e.vars[SQ_COUNT];
        let Some(rider) = cx.spawn(kind, x, y) else {
            return;
        };
        rider.motion.vx = self.speed;
        rider.tile = Some(tile);
        rider.facing_left = true;
        rider.mapping = self.mapping;
        // move.w 6(a0),6(a6): the spawner's damage and health counter.
        rider.health = u16::from(spawn.health_counter) + 1;
        rider.vars[J_BOB] = count;
    }
}
impl Behavior for Squadron {
    fn label(&self, _kind: u16) -> String {
        "Enemy6C $173B2".into()
    }
    fn hit(&self, _enemy: &Enemy, _kind: u16) -> HitResponse {
        HitResponse::Pass
    }
    fn activate(&self, _cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.state = EnemyState::Waiting;
        // Object_SpawnFromRecord: obj_spawn_flags = flags | $8000 ($2A).
        e.vars[SQ_COUNT] = i32::from(spawn.flags | 0x8000);
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, _terrain: &dyn Terrain) {
        settle(cx, &self.mines);
        loop {
            match e.routine {
                0 => {
                    // $173C0: flipped $70 graphics; $2C = (flags & $10) >> 3.
                    stop(e);
                    e.facing_left = true;
                    e.vars[SQ_MODE] = (e.vars[SQ_COUNT] & 0x10) >> 3;
                    e.routine = 4;
                }
                4 => {
                    // $173F6
                    e.timer = self.period;
                    e.vars[SQ_COUNT] &= 0xf;
                    if e.vars[SQ_COUNT] == 0 {
                        Ctx::free(e);
                        return;
                    }
                    e.routine = 8;
                }
                _ => {
                    // $17408: $28 counts below zero, then the next rider.
                    if e.timer > 0 {
                        e.timer -= 1;
                        return;
                    }
                    e.vars[SQ_COUNT] -= 1;
                    e.routine = 4;
                    self.release(cx, e, spawn);
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Types $60 (Enemy60, $17BB6) and $70 (Enemy70, $17474): jet riders.

/// `$2A` timer, `$2C` bob angle, `$2E` passes left, `$30` bomb/shot count,
/// `$32` wake part alive; the thruster part's mapping (0: none) and the
/// wake's own Y before routine `$0C`; the thruster part's own `+$28` and
/// position.
const J_TIMER: usize = 0;
const J_BOB: usize = 1;
const J_PASSES: usize = 2;
const J_SHOTS: usize = 3;
const J_WAKE: usize = 4;
const J_THRUSTER: usize = 5;
const J_WAKE_Y: usize = 6;
const J_THRUSTER_TICKS: usize = 7;
const J_THRUSTER_X: usize = 8;
const J_THRUSTER_Y: usize = 9;
/// The wake copies obj_tile when allocated; it does not follow palette changes.
const J_WAKE_TILE: usize = 10;

/// Rider constants shared by `$60` and `$70` (`$175E6..$17B86`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct RiderParts {
    bob_step: u16,
    wake: u16,
    water: i32,
    thrusters: [u16; 2],
    /// `$17A50`: the thruster mapping that sprays, every tick with no bit
    /// of `spray_mask` set, a list-9 wake splash with `+$28` = `spray_life`
    /// `spray_dy` below it.
    spray: u16,
    spray_mask: i32,
    spray_life: i32,
    spray_dy: i32,
    exit: i32,
    wreck: u16,
    wreck_launch: (i32, i32, i32),
    rider: u16,
    rider_launch: (i32, i32, i32),
    rider_skim: i32,
    debris_angles: [u16; 4],
    debris_first: u16,
    debris_gravity: i32,
    sound: u8,
    /// The rider's poses (`$6C`'s, the `$70` sweep, the `$60` gun pose)
    /// and the wake and thruster parts drawn at its X: what must clear a
    /// widened view's edge before it is parked, turned or freed there.
    frames: [u16; 7],
}
impl RiderParts {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            bob_step: word(rom, 0x1768e)?,
            wake: word(rom, 0x17ad6)?,
            water: word_i(rom, 0x1763a)?,
            thrusters: [word(rom, 0x1787e)?, word(rom, 0x17884)?],
            spray: word(rom, 0x17a56)?,
            spray_mask: word_i(rom, 0x17a60)?,
            spray_life: i32::from(byte(rom, 0x17a65)?),
            spray_dy: signed(rom, 0x17a74)?,
            exit: word_i(rom, 0x17ac4)?,
            wreck: word(rom, 0x17616)?,
            wreck_launch: (
                signed(rom, 0x17604)? << 16,
                signed(rom, 0x1760a)? << 16,
                word_i(rom, 0x17610)?,
            ),
            rider: word(rom, 0x179c4)?,
            rider_launch: (
                signed(rom, 0x179b2)? << 16,
                signed(rom, 0x179b8)? << 16,
                word_i(rom, 0x179be)?,
            ),
            rider_skim: signed(rom, 0x179e0)? << 16,
            debris_angles: [
                word(rom, 0x179f6)?,
                word(rom, 0x179f8)?,
                word(rom, 0x179fa)?,
                word(rom, 0x179fc)?,
            ],
            debris_first: word(rom, 0x17a26)?,
            debris_gravity: word_i(rom, 0x17a1c)?,
            sound: byte(rom, 0x174b3)?,
            frames: [
                word(rom, 0x1738c)?,
                word(rom, 0x17512)?,
                word(rom, 0x17c7c)?,
                word(rom, 0x17ad6)?,
                word(rom, 0x17ad6)? - 1,
                word(rom, 0x1787e)?,
                word(rom, 0x17884)?,
            ],
        })
    }
}

/// `$70` (bomber) and `$60` (gunner). `$70` rides in behind the spawner
/// for 240 ticks, accelerates off to the right, then twice sweeps left at
/// camera + $170 ... player + 16 at 1 px/tick bobbing (`Func_01768C`) and
/// throwing a list-4 `$2C` bomb every 181 ticks, turning back right after
/// 30 ticks; the third sweep continues off the left edge. `$60` enters
/// fast from the left (4 px/tick, braking at $900), drifts back off the
/// left edge, then twice crosses right at 1.5 px/tick firing 9-round
/// bursts (list-4 `$30`, 10/8 px/tick down-right, only while the player is
/// right of it) and returns at 2.25 px/tick. Both die in a sinking wreck
/// that throws its rider, smokes and bursts into four pieces.
pub(crate) struct Jet {
    gunner: bool,
    parts: RiderParts,
    mines: Mines,
    // Enemy70.
    enter_ticks: i32,
    enter_accel: i32,
    wait: (i32, i32),
    heights: (u16, i32),
    sweep: u16,
    sweep_vx: i32,
    bomb_period: i32,
    first_bomb: i32,
    reach: i32,
    turn_ticks: i32,
    return_accel: i32,
    far_exit: i32,
    far_wait: (i32, i32),
    bomb_offset: (i32, i32),
    bomb_launch: (i32, i32, i32),
    bomb_sound: u8,
    // Enemy60.
    dash_accel: i32,
    dash_vx: i32,
    drift_vx: i32,
    hide: (i32, i32),
    gun_heights: (u16, i32),
    gun_pose: u16,
    burst: i32,
    cross_vx: i32,
    cross_exit: i32,
    back_vx: i32,
    back_exit: i32,
    reload: i32,
    bullet: u16,
    bullet_speed: (i32, i32),
    bullet_offset: (i32, i32),
    bullet_damage: u8,
    gun_sound: u8,
    casing: u16,
}
impl Jet {
    pub(super) fn load(rom: &[u8], gunner: bool, mines: Mines) -> Result<Self> {
        Ok(Self {
            gunner,
            parts: RiderParts::load(rom)?,
            mines,
            enter_ticks: word_i(rom, 0x17494)?,
            enter_accel: word_i(rom, 0x174a4)?,
            wait: (word_i(rom, 0x174d6)?, word_i(rom, 0x174da)?),
            heights: (word(rom, 0x17500)?, word_i(rom, 0x17504)?),
            sweep: word(rom, 0x17512)?,
            sweep_vx: long(rom, 0x17518)?,
            bomb_period: word_i(rom, 0x176fa)?,
            first_bomb: word_i(rom, 0x17520)?,
            reach: signed(rom, 0x17542)?,
            turn_ticks: word_i(rom, 0x17560)?,
            return_accel: word_i(rom, 0x1758e)?,
            far_exit: word_i(rom, 0x175ba)?,
            far_wait: (word_i(rom, 0x175d0)?, word_i(rom, 0x175d4)?),
            bomb_offset: (signed(rom, 0x1771c)?, signed(rom, 0x17720)?),
            bomb_launch: (
                signed(rom, 0x1772c)? << 16,
                signed(rom, 0x17732)? << 16,
                word_i(rom, 0x17738)?,
            ),
            bomb_sound: byte(rom, 0x1774f)?,
            dash_accel: signed(rom, 0x17bdc)?,
            dash_vx: long(rom, 0x17be2)?,
            drift_vx: long(rom, 0x17c0c)?,
            hide: (word_i(rom, 0x17c2a)?, word_i(rom, 0x17c2e)?),
            gun_heights: (word(rom, 0x17c54)?, word_i(rom, 0x17c58)?),
            gun_pose: word(rom, 0x17c7c)?,
            burst: word_i(rom, 0x17c60)?,
            cross_vx: long(rom, 0x17c88)?,
            cross_exit: word_i(rom, 0x17cb8)?,
            back_vx: long(rom, 0x17cce)?,
            back_exit: signed(rom, 0x17cec)?,
            reload: word_i(rom, 0x17d90)?,
            bullet: word(rom, 0x17d4c)?,
            bullet_speed: (signed(rom, 0x17d52)?, signed(rom, 0x17d58)?),
            bullet_offset: (signed(rom, 0x17d38)?, signed(rom, 0x17d44)?),
            bullet_damage: shot_damage(rom, 0x30)?,
            gun_sound: byte(rom, 0x17d63)?,
            casing: word(rom, 0x17930)?,
        })
    }
    /// Routine offsets of the shared tail (`loc_0175E4`): hurt, death,
    /// sinking, final burst; `$60`'s table is one slot shorter.
    fn hurt_routine(&self) -> u16 {
        if self.gunner { 0x18 } else { 0x1c }
    }
    fn death_routine(&self) -> u16 {
        self.hurt_routine() + 4
    }
    /// Camera + `offset`, the cartridge's off-screen X for parking, turning
    /// and freeing the rider, moved beyond a widened view ([`Ctx::entry_x`]).
    fn edge(&self, cx: &Ctx, offset: i32, e: &Enemy) -> i32 {
        cx.entry_x(offset, &self.parts.frames, e.facing_left)
    }
    /// `Func_01768C`: bob about the flight line.
    fn bob(&self, cx: &Ctx, e: &mut Enemy) {
        e.vars[J_BOB] = (e.vars[J_BOB] + i32::from(self.parts.bob_step)) & 0xffff;
        let d = cx.shared.sine.polar(e.vars[J_BOB] as u16).0 >> 1;
        nudge_y(e, d);
    }
    /// `Func_0176F2`: a bomb every `bomb_period + 1` ticks.
    fn bomb(&self, cx: &mut Ctx, e: &mut Enemy) {
        if count_down(&mut e.vars[J_SHOTS]) {
            return;
        }
        e.vars[J_SHOTS] = self.bomb_period;
        let (vx, vy, gravity) = self.bomb_launch;
        let motion = Motion {
            vx,
            vy,
            gravity,
            ..Motion::default()
        };
        let (x, y) = (e.x + self.bomb_offset.0, e.y + self.bomb_offset.1);
        if self.mines.throw(cx, x, y, motion) {
            cx.effect_sound(self.bomb_sound);
        }
    }
    /// `Func_017D02`: a round every eight ticks while the player is right
    /// of the gunner; false once the burst is spent (`$30` negative).
    fn gun(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        if cx.player.0 < e.x || e.vars[J_SHOTS] < 0 {
            return e.vars[J_SHOTS] >= 0;
        }
        if e.timer & 7 != 0 {
            return true;
        }
        let (x, y) = (e.x + self.bullet_offset.0, e.y + self.bullet_offset.1);
        let mut round = Projectile::new(
            x,
            y,
            self.bullet_speed.0,
            true,
            self.bullet,
            self.bullet_damage,
        )
        .with_motion(Motion {
            vy: self.bullet_speed.1 << 16,
            ..Motion::default()
        });
        round.passes_terrain = true;
        // $17DAC: freed where it reaches the water.
        let fall = (self.parts.water - y).max(1);
        round.life = ((fall + self.bullet_speed.1 - 1) / self.bullet_speed.1.max(1)) as u16;
        if !cx.shoot(round) {
            return e.vars[J_SHOTS] >= 0;
        }
        cx.effect_sound(self.gun_sound);
        if e.vars[J_SHOTS] & 1 == 0 {
            // Func_017910: a type-$23 casing, 40 blinking ticks.
            let mut casing = Effect::new(EffectKind::Pieces, x, y, self.casing);
            casing.blink = true;
            casing.counter = 40;
            casing.motion = Motion {
                vx: (1 << 16) | i32::from(cx.random() & 0x7fff),
                vy: (-3 << 16) | i32::from(cx.random() & 0x7fff),
                gravity: 0x1800,
                ..Motion::default()
            };
            cx.effect(casing);
        }
        e.vars[J_SHOTS] -= 1;
        if e.vars[J_SHOTS] < 0 {
            e.vars[J_TIMER] = self.reload;
        }
        e.vars[J_SHOTS] >= 0
    }
    /// `obj_tile`: the one `$6C` handed over, else the type's graphics base.
    fn tile(&self, cx: &Ctx, e: &Enemy) -> u16 {
        e.tile
            .unwrap_or_else(|| cx.base(if self.gunner { 0x60 } else { 0x70 }))
    }
    /// `Func_0178B2`: the wake part, 16 px below the rider.
    fn wake(&self, cx: &Ctx, e: &mut Enemy) {
        e.vars[J_WAKE] = 1;
        e.vars[J_WAKE_Y] = e.y + 16;
        e.vars[J_WAKE_TILE] = i32::from(self.tile(cx, e));
    }
    /// `$17A50`, the thruster part's update, before it follows the rider:
    /// `before` is the part the rider had when its update began. A part
    /// allocated this update (`Func_0178D4`) starts at the rider and runs
    /// at once; its `+$28` is the recycled record's, which is -1 when that
    /// was a spent splash (Genesis Plus GX, 4-1 idle from the r4-1 state:
    /// all 13 `$1D6` and 9 of the 11 `$1D5` thrusters), so it sprays on
    /// its first update and every fourth after, and shows on odd ticks.
    /// The spray is where the part was left on its last update, the
    /// rider's position then; the part that goes with this update (`$34`
    /// cleared, `$17BF4`) still sprays.
    fn spray(&self, cx: &mut Ctx, e: &mut Enemy, before: i32) {
        let now = e.vars[J_THRUSTER];
        if now != 0 && now != before {
            e.vars[J_THRUSTER_TICKS] = -1;
            e.vars[J_THRUSTER_X] = e.x;
            e.vars[J_THRUSTER_Y] = e.y;
        }
        let part = if now != 0 { now } else { before };
        if part == 0 {
            return;
        }
        // Func_03670C: addq.w #1,$28(a0).
        e.vars[J_THRUSTER_TICKS] = (e.vars[J_THRUSTER_TICKS] + 1) & 0xffff;
        if part as u16 == self.parts.spray && e.vars[J_THRUSTER_TICKS] & self.parts.spray_mask == 0
        {
            let at = (
                e.vars[J_THRUSTER_X],
                e.vars[J_THRUSTER_Y] + self.parts.spray_dy,
            );
            splash(cx, at, self.parts.spray_life, true);
        }
    }
    /// `$17A50`/`$17AD0`: this frame's wake and thruster sprites. The
    /// thruster shows on its odd ticks (`btst #0,$29(a0)`).
    fn draw_parts(&self, cx: &mut Ctx, e: &mut Enemy) {
        let base = e.vars[J_WAKE_TILE] as u16;
        if e.vars[J_WAKE] != 0 {
            let (mapping, y) = if e.routine >= 0x0c {
                (self.parts.wake - 1, self.parts.water)
            } else {
                (self.parts.wake, e.vars[J_WAKE_Y])
            };
            cx.overlay_in(SpriteList::Back, e.x, y, mapping, base, e.facing_left);
        }
        let thruster = e.vars[J_THRUSTER] as u16;
        if thruster != 0 {
            if e.routine >= 0x20 || e.x > self.edge(cx, self.parts.exit, e) {
                e.vars[J_THRUSTER] = 0;
            } else {
                // $17A9C: the part follows the rider.
                e.vars[J_THRUSTER_X] = e.x;
                e.vars[J_THRUSTER_Y] = e.y;
                if e.vars[J_THRUSTER_TICKS] & 1 != 0 {
                    cx.overlay_in(SpriteList::Back, e.x, e.y, thruster, 0, false);
                }
            }
        }
    }
    /// `$179CC`: the thrown rider (`$1CF`) skims left once it reaches the
    /// water. Idempotent over every rider piece.
    fn skim(&self, cx: &mut Ctx) {
        for piece in cx.effects.iter_mut() {
            if piece.kind == EffectKind::Pieces
                && piece.mapping == self.parts.rider
                && piece.motion.gravity != 0
                && piece.y >= self.parts.water
            {
                piece.motion = Motion {
                    vx: self.parts.rider_skim,
                    ..Motion::default()
                };
            }
        }
    }
    fn body(&self, cx: &mut Ctx, e: &mut Enemy) {
        loop {
            let routine = e.routine;
            if !self.gunner {
                match routine {
                    0 => {
                        // $17480
                        e.death_routine = self.death_routine();
                        e.hurt_routine = self.hurt_routine();
                        self.wake(cx, e);
                        e.vars[J_TIMER] = self.enter_ticks;
                        e.routine = 4;
                        continue;
                    }
                    4 => {
                        // $1749C: accelerate away to the right.
                        if count_down(&mut e.vars[J_TIMER]) {
                            return;
                        }
                        e.motion.ax = self.enter_accel;
                        e.vars[J_THRUSTER] = i32::from(self.parts.thrusters[0]);
                        e.routine = 8;
                        cx.effect_sound(self.parts.sound);
                        return;
                    }
                    8 => {
                        // $174BA
                        if self.edge(cx, self.parts.exit, e) >= e.x {
                            return;
                        }
                        stop(e);
                        e.vars[J_TIMER] = (i32::from(cx.random()) & self.wait.0) + self.wait.1;
                        e.vars[J_PASSES] = 0;
                        e.routine = 0x0c;
                        return;
                    }
                    0x0c => {
                        // $174EC: wait off the right edge, then sweep left.
                        e.x = self.edge(cx, self.parts.exit, e);
                        if count_down(&mut e.vars[J_TIMER]) {
                            return;
                        }
                        e.y = i32::from(cx.random() & self.heights.0) + self.heights.1;
                        // ori.w #$8000,8(a0): the sweeps are on the
                        // surfer's high sprite-priority layer.
                        e.tile = Some(self.tile(cx, e) | 0x8000);
                        e.mapping = self.sweep;
                        e.motion.vx = self.sweep_vx;
                        e.vars[J_SHOTS] = self.first_bomb;
                        e.routine = 0x10;
                        continue;
                    }
                    0x10 => {
                        // $17528
                        if cx.hit_taken(e) {
                            return;
                        }
                        self.bob(cx, e);
                        self.bomb(cx, e);
                        if cx.player.0 + self.reach < e.x {
                            return;
                        }
                        if e.vars[J_PASSES] >= 0 {
                            e.vars[J_TIMER] = self.turn_ticks;
                            stop(e);
                            e.routine = 0x14;
                        } else if e.offscreen {
                            e.routine = 0x2c;
                        }
                        return;
                    }
                    0x14 => {
                        // $17570: pause, then back to the right.
                        if cx.hit_taken(e) {
                            return;
                        }
                        self.bob(cx, e);
                        if count_down(&mut e.vars[J_TIMER]) {
                            return;
                        }
                        e.vars[J_THRUSTER] = i32::from(self.parts.thrusters[1]);
                        e.motion.ax = self.return_accel;
                        e.routine = 0x18;
                        cx.effect_sound(self.parts.sound);
                        return;
                    }
                    0x18 => {
                        // $1759E
                        if cx.hit_taken(e) {
                            return;
                        }
                        self.bob(cx, e);
                        self.bomb(cx, e);
                        if self.edge(cx, self.far_exit, e) >= e.x {
                            return;
                        }
                        stop(e);
                        e.vars[J_TIMER] =
                            (i32::from(cx.random()) & self.far_wait.0) + self.far_wait.1;
                        e.vars[J_PASSES] -= 1;
                        e.routine = 0x0c;
                        return;
                    }
                    0x2c => {
                        // $17664
                        e.vars[J_WAKE] = 0;
                        e.routine = 0x30;
                        return;
                    }
                    0x30 => {
                        Ctx::free(e);
                        return;
                    }
                    _ => {}
                }
            } else {
                match routine {
                    0 => {
                        // $17BC2: brake in from the left edge.
                        e.death_routine = self.death_routine();
                        e.hurt_routine = self.hurt_routine();
                        self.wake(cx, e);
                        e.vars[J_THRUSTER] = i32::from(self.parts.thrusters[0]);
                        e.motion.ax = self.dash_accel;
                        e.motion.vx = self.dash_vx;
                        e.routine = 4;
                        cx.effect_sound(self.parts.sound);
                        return;
                    }
                    4 => {
                        // $17BF4: until the braking has turned it around.
                        if ((e.motion.vx >> 16) < 0) != (e.motion.ax < 0) {
                            return;
                        }
                        e.vars[J_THRUSTER] = 0;
                        e.motion.ax = 0;
                        e.motion.vx = self.drift_vx;
                        e.routine = 8;
                        continue;
                    }
                    8 => {
                        // $17C16
                        if !e.offscreen {
                            return;
                        }
                        stop(e);
                        e.vars[J_TIMER] = (i32::from(cx.random()) & self.hide.0) + self.hide.1;
                        e.vars[J_PASSES] = 1;
                        e.routine = 0x0c;
                        return;
                    }
                    0x0c => {
                        // $17C40: wait off the left edge.
                        e.x = self.edge(cx, -48, e);
                        if count_down(&mut e.vars[J_TIMER]) {
                            return;
                        }
                        e.y = i32::from(cx.random() & self.gun_heights.0) + self.gun_heights.1;
                        e.vars[J_SHOTS] = self.burst;
                        e.vars[J_TIMER] = -1;
                        // andi.w #$1FFF / ori.w #$8000: palette line 0 on
                        // the surfer's high sprite-priority layer.
                        e.tile = Some((self.tile(cx, e) & 0x1fff) | 0x8000);
                        e.mapping = self.gun_pose;
                        e.routine = 0x10;
                        e.motion.vx = self.cross_vx;
                        continue;
                    }
                    0x10 => {
                        // $17C8E
                        if cx.hit_taken(e) {
                            return;
                        }
                        self.bob(cx, e);
                        if !self.gun(cx, e) && !count_down(&mut e.vars[J_TIMER]) {
                            e.vars[J_SHOTS] = self.burst;
                        }
                        if self.edge(cx, self.cross_exit, e) >= e.x {
                            return;
                        }
                        stop(e);
                        e.x = self.edge(cx, self.parts.exit, e);
                        e.motion.vx = self.back_vx;
                        e.routine = 0x14;
                        continue;
                    }
                    0x14 => {
                        // $17CD8
                        if cx.hit_taken(e) {
                            return;
                        }
                        self.bob(cx, e);
                        if self.edge(cx, self.back_exit, e) < e.x {
                            return;
                        }
                        e.vars[J_PASSES] -= 1;
                        if e.vars[J_PASSES] >= 0 {
                            e.routine = 0x10;
                            e.motion.vx = self.cross_vx;
                        } else {
                            e.routine = 0x28;
                        }
                        return;
                    }
                    0x28 => {
                        // $17664
                        e.vars[J_WAKE] = 0;
                        e.routine = 0x2c;
                        return;
                    }
                    0x2c => {
                        Ctx::free(e);
                        return;
                    }
                    _ => {}
                }
            }
            // The shared tail, from the hurt routine on.
            let tail = routine.wrapping_sub(self.hurt_routine());
            match tail {
                0 => {
                    // $1765C: a surviving hit resumes.
                    e.routine = e.saved_routine;
                    return;
                }
                4 => {
                    // $175E6: the rider is thrown, the wreck sinks.
                    let (vx, vy, gravity) = self.parts.rider_launch;
                    let tile = e.tile.unwrap_or(0x2300);
                    let mut rider = Effect::new(EffectKind::Pieces, e.x, e.y, self.parts.rider);
                    rider.base = (tile & 0x07ff) | 0x2000;
                    rider.flip = e.facing_left;
                    rider.counter = 240;
                    rider.motion = Motion {
                        vx,
                        vy,
                        gravity,
                        ..Motion::default()
                    };
                    cx.effect(rider);
                    if self.gunner {
                        // ori.w #$6000,8(a0): palette 3, priority kept.
                        e.tile = Some(tile | 0x6000);
                    }
                    stop(e);
                    let (vx, vy, gravity) = self.parts.wreck_launch;
                    e.motion.vx = vx;
                    e.motion.vy = vy;
                    e.motion.gravity = gravity;
                    e.mapping = self.parts.wreck;
                    e.routine += 4;
                    return;
                }
                8 => {
                    // $17624: smoke every other tick until the water.
                    if e.timer & 1 != 0 {
                        return;
                    }
                    let dx = i32::from(cx.random() & 0x3f) - 0x1f;
                    let dy = i32::from(cx.random() & 7) - 3;
                    let puff = Effect::new(EffectKind::Puff, e.x + dx, e.y + dy, 0);
                    cx.effect(puff);
                    if e.y < self.parts.water {
                        return;
                    }
                    e.vars[J_WAKE] = 0;
                    e.routine += 4;
                    return;
                }
                _ => {
                    // $1764A: Func_017872's four pieces and the $36D90 blast.
                    let tile = e.tile.unwrap_or(0x2300);
                    for (i, &angle) in self.parts.debris_angles.iter().enumerate() {
                        let (cos, sin) = cx.shared.sine.polar(angle);
                        let mapping = self.parts.debris_first + i as u16;
                        let mut piece = Effect::new(EffectKind::Pieces, e.x, e.y, mapping);
                        piece.base = tile;
                        piece.flip = e.facing_left;
                        piece.blink = true;
                        piece.counter = 120;
                        piece.motion = Motion {
                            vx: cos << 4,
                            vy: sin << 2,
                            gravity: self.parts.debris_gravity,
                            ..Motion::default()
                        };
                        cx.effect(piece);
                    }
                    let blast = cx.shared.blasts[1];
                    cx.explode(e.x, e.y, blast);
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
}
impl Behavior for Jet {
    fn label(&self, _kind: u16) -> String {
        if self.gunner {
            "Enemy60 $17BB6".into()
        } else {
            "Enemy70 $17474".into()
        }
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        // Func_0128B2 runs only in the passes.
        let passes: &[u16] = if self.gunner {
            &[0x10, 0x14]
        } else {
            &[0x10, 0x14, 0x18]
        };
        if passes.contains(&enemy.routine) {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        settle(cx, &self.mines);
        self.skim(cx);
        let thruster = e.vars[J_THRUSTER];
        self.body(cx, e);
        self.spray(cx, e, thruster);
        if e.live() {
            self.draw_parts(cx, e);
        }
    }
}
