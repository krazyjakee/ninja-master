//! Type `$1C`, `Enemy1C` (`$15308`): the 2-1 end guard, with its kite,
//! afterimage and hurt image, and type `$94`, `Enemy94` (`$15B60`): the
//! spears it throws.
use super::riders::{CARRIER, Carrier, CarrierStep, quick};
use crate::{
    combat::{Blast, Effect, EffectKind, Enemy, Motion, PaletteOp, Projectile, Signal, Spawn},
    enemies::{Behavior, Ctx, HitResponse, set_whole, shot_damage, stop},
    game::Terrain,
    movement::PlayerEvent,
    rom::{Result, byte, signed, word, word_i, words},
    sound_events::Table,
};

/// Scratch words of the guard: `$2A` timer (also the run frame), `$2C`
/// count, `$2E` spears left in a leap, `$34` (dead: its spears burst),
/// `obj_flags` bit 0 (its kite holds), and the hit window of `Func_015748`.
const TIMER: usize = 0;
const COUNT: usize = 1;
const SPEARS: usize = 2;
const DEAD: usize = 3;
const HELD: usize = 4;
const WINDOW: usize = 5;
/// The afterimage record (`Func_015766`), kept in the guard's record: its
/// `$40` counter (0: none), the flip of its `obj_tile`, and four samples
/// (X/Y packed, then frames) of `$28..$3F`.
const TRAIL: usize = 6;
const TRAIL_FLIP: usize = 7;
const TRAIL_XY: usize = 8;
const TRAIL_FRAME: usize = 12;
/// The kite's launcher, by `Combat::enemies` index.
const OWNER: usize = 16;
/// What a `$1C` record is: the guard, or its list-9 kite.
const ROLE: usize = 19;
const KITE: i32 = 1;

/// `$1C`: starts the HASSOU! music and stops the autoscroll, rides in on
/// a kite (`Func_015894`), drops from it at its hover point and runs off
/// to the right. It then fights from routine `$18`: it runs in from the
/// right, leaps and throws three spears straight up from its apex, lands
/// and runs off left; it waits, runs back in from the left and, within
/// 192 px of the player, leaps and dives feet first with a list-4 `$39`
/// kick. A kick that lands (`$15AD8`) bounces it off. Each hit leaves a
/// fleeing hurt image (`$15858`) and every update a four-sample
/// afterimage (`Func_015766`) on palette line 3. The death freezes the
/// player, sets `$34` for the spears, releases the kite, bursts five
/// times, explodes, plays STAGE CLEAR and requests the next substage.
pub(crate) struct Guard {
    carrier: Carrier,
    colors: [u16; 7],
    music: u8,
    kite_ticks: i32,
    drop: (i32, i32, i32, i32, u16),
    lane: i32,
    exit: (i32, i32),
    ride: u16,
    enter_ticks: i32,
    wait: (u16, i32),
    run_vx: i32,
    back_vx: i32,
    leap: (i32, i32, i32, u16, i32, i32),
    ground: i32,
    rest: (u16, i32),
    reach: i32,
    kick_leap: (i32, i32, i32, u16, i32),
    kick_voice: u8,
    kick_pose: u16,
    dive: (i32, i32, i32, i32, u16),
    dive_sound: u8,
    return_run: (i32, i32),
    rest_back: (u16, i32),
    bounce: (u16, i32, i32, i32, i32),
    ghost: (u16, i32, i32, i32, i32),
    run_frames: (i32, u16),
    air_frames: [u16; 4],
    window: (u16, i32),
    death_vx: i32,
    death_pose: u16,
    bursts: i32,
    burst_sound: u8,
    blast: Blast,
    vanish_ticks: i32,
    vanish_sound: u8,
    clear_ticks: i32,
    clear_music: u8,
    kick: u16,
    kick_offset: (i32, i32),
    kick_damage: u8,
    kick_floor: i32,
    kick_hit_voice: u8,
    spear: SpearLaunch,
}
/// `Func_015B02`: a list-5 `$94` from 112 px above the guard.
struct SpearLaunch {
    rise: i32,
    tile: u16,
    mapping: u16,
    vy: i32,
    gravity: i32,
    sound: u8,
}
impl Guard {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            carrier: Carrier::load(rom)?,
            colors: words(rom, 0x42df4)?,
            music: Table::Music.command_at(rom, 0x1535b)?,
            kite_ticks: word_i(rom, 0x15366)?,
            drop: (
                quick(word(rom, 0x15384)?),
                signed(rom, 0x15394)?,
                signed(rom, 0x1539a)?,
                signed(rom, 0x153a0)?,
                word(rom, 0x153a6)?,
            ),
            lane: word_i(rom, 0x153b6)?,
            exit: (signed(rom, 0x153ca)?, signed(rom, 0x153d0)?),
            ride: word(rom, 0x153e4)?,
            enter_ticks: word_i(rom, 0x153fa)?,
            wait: (word(rom, 0x15440)?, word_i(rom, 0x15444)?),
            run_vx: signed(rom, 0x1544c)?,
            // $15508: move.w #-3,d0 / neg.w d0.
            back_vx: -signed(rom, 0x1550a)?,
            leap: (
                signed(rom, 0x15468)?,
                signed(rom, 0x1546e)?,
                signed(rom, 0x15474)?,
                word(rom, 0x1547a)?,
                word_i(rom, 0x15480)?,
                word_i(rom, 0x15486)?,
            ),
            ground: word_i(rom, 0x154b8)?,
            rest: (word(rom, 0x154ee)?, word_i(rom, 0x154f2)?),
            reach: word_i(rom, 0x15528)?,
            kick_leap: (
                signed(rom, 0x1552e)?,
                signed(rom, 0x15534)?,
                signed(rom, 0x1553a)?,
                word(rom, 0x15540)?,
                word_i(rom, 0x15546)?,
            ),
            kick_voice: Table::Voice.command_at(rom, 0x1554f)?,
            kick_pose: word(rom, 0x1555e)?,
            dive: (
                signed(rom, 0x15570)?,
                signed(rom, 0x15576)?,
                signed(rom, 0x1557c)?,
                signed(rom, 0x15582)?,
                word(rom, 0x15588)?,
            ),
            dive_sound: byte(rom, 0x15597)?,
            return_run: (signed(rom, 0x155b4)?, signed(rom, 0x155ba)?),
            rest_back: (word(rom, 0x155dc)?, word_i(rom, 0x155e0)?),
            bounce: (
                word(rom, 0x15600)?,
                signed(rom, 0x15606)?,
                signed(rom, 0x1560c)?,
                signed(rom, 0x15612)?,
                signed(rom, 0x15618)?,
            ),
            ghost: (
                word(rom, 0x1585a)?,
                word_i(rom, 0x15860)?,
                signed(rom, 0x15866)?,
                signed(rom, 0x1586c)?,
                signed(rom, 0x15872)?,
            ),
            run_frames: (word_i(rom, 0x15718)?, word(rom, 0x15724)?),
            air_frames: {
                let base = word(rom, 0x15740)?;
                let steps = [
                    byte(rom, 0x1572c)?,
                    byte(rom, 0x1572d)?,
                    byte(rom, 0x1572e)?,
                    byte(rom, 0x1572f)?,
                ];
                steps.map(|step| base + u16::from(step))
            },
            window: (word(rom, 0x1574e)?, word_i(rom, 0x15758)?),
            death_vx: signed(rom, 0x156a4)?,
            death_pose: word(rom, 0x156aa)?,
            bursts: word_i(rom, 0x156b0)?,
            burst_sound: byte(rom, 0x156cb)?,
            // $156DE: moveq #0,d5 / moveq #5,d7 / moveq #1,d4 into $36DB6.
            blast: Blast {
                y_shift: u32::from(byte(rom, 0x156e3)?),
                x_shift: u32::from(byte(rom, 0x156e1)?),
                sound: Table::Effect.command_at(rom, 0x36d97)?,
            },
            vanish_ticks: word_i(rom, 0x15700)?,
            vanish_sound: byte(rom, 0x15709)?,
            clear_ticks: word_i(rom, 0x13b2e)?,
            clear_music: Table::Music.command_at(rom, 0x13b37)?,
            kick: word(rom, 0x15a60)?,
            kick_offset: (word_i(rom, 0x15a74)?, word_i(rom, 0x15a78)?),
            kick_damage: shot_damage(rom, 0x39)?,
            kick_floor: word_i(rom, 0x15ab8)?,
            kick_hit_voice: Table::Voice.command_at(rom, 0x15ae5)?,
            spear: SpearLaunch {
                rise: signed(rom, 0x15b1e)?,
                tile: word(rom, 0x15b26)?,
                mapping: word(rom, 0x15b2c)?,
                vy: signed(rom, 0x15b32)?,
                gravity: signed(rom, 0x15b38)?,
                sound: byte(rom, 0x15b41)?,
            },
        })
    }
    /// `Func_015710`: the six run frames, one per update.
    fn run(&self, e: &mut Enemy) {
        let (count, first) = self.run_frames;
        e.vars[TIMER] += 1;
        if e.vars[TIMER] >= count {
            e.vars[TIMER] = 0;
        }
        e.mapping = first + e.vars[TIMER] as u16;
    }
    /// `Func_015730`: the leap frames, eight updates each.
    fn air(&self, e: &mut Enemy) {
        e.mapping = self.air_frames[usize::from((e.timer >> 3) & 3)];
    }
    /// `($FF001C & mask) + base`, reading the generator without advancing it.
    fn random(cx: &Ctx, (mask, base): (u16, i32)) -> i32 {
        i32::from(*cx.random & mask) + base
    }
    /// The vertical velocity's integer word has the gravity's sign: past
    /// the apex (`eor.w` / `bmi`).
    fn falling(e: &Enemy) -> bool {
        ((e.motion.vy >> 16) < 0) == (e.motion.gravity < 0)
    }
    /// Landed on the `$C0` line: stopped there.
    fn landed(&self, e: &mut Enemy) -> bool {
        if e.y <= self.ground {
            return false;
        }
        e.y = self.ground;
        stop(e);
        true
    }
    /// `Func_015894` from the guard: a `$1C` kite record holding it.
    fn launch_kite(&self, cx: &mut Ctx, e: &mut Enemy) {
        let (me, camera) = (cx.current as i32, cx.camera);
        let entry_x = cx.entry_x(
            self.carrier.offset.0,
            &[self.carrier.frame, self.carrier.frame + 1],
            false,
        );
        let Some(kite) = cx.spawn(0x1c, e.x, e.y) else {
            return;
        };
        kite.vars[ROLE] = KITE;
        kite.vars[OWNER] = me;
        kite.tile = Some(0);
        self.carrier.launch(camera, kite, self.kite_ticks);
        kite.x = entry_x;
        e.vars[HELD] = 1;
    }
    /// The guard's kite as it was before this pass (`$FF3A0C`/`$FF3A0E`
    /// and the `$2A` it clears), if it still flies.
    fn kite(cx: &Ctx) -> Option<(i32, i32, i32)> {
        let me = cx.current as i32;
        cx.others
            .iter()
            .find(|o| o.live && o.vars[ROLE] == KITE && o.vars[OWNER] == me)
            .map(|o| (o.x, o.y, o.vars[CARRIER]))
    }
    /// `Func_015B02`.
    fn spear(&self, cx: &mut Ctx, e: &Enemy) {
        let spear = &self.spear;
        let Some(child) = cx.spawn(0x94, e.x, e.y + spear.rise) else {
            return;
        };
        child.tile = Some(spear.tile);
        child.mapping = spear.mapping;
        child.motion.vy = spear.vy << 16;
        child.motion.gravity = spear.gravity;
        cx.effect_sound(spear.sound);
    }
    /// `Func_015A4E`: the list-4 `$39` kick, moving with the guard.
    fn kick(&self, cx: &mut Ctx, e: &Enemy) {
        let (dx, dy) = self.kick_offset;
        let mut kick = Projectile::new(e.x + dx, e.y + dy, 0, true, self.kick, self.kick_damage)
            .with_motion(Motion {
                fx: 0,
                fy: 0,
                ..e.motion
            });
        kick.owner = Some(cx.current);
        kick.passes_terrain = true;
        cx.shoot(kick);
    }
    /// `$15AB0`: the kick is freed once below `$B0`.
    fn drop_kick(&self, cx: &mut Ctx) {
        let me = cx.current;
        for shot in cx.projectiles.iter_mut() {
            if shot.owner == Some(me) && shot.mapping == self.kick && shot.y > self.kick_floor {
                shot.alive = false;
            }
        }
    }
    /// `$15858`: the hurt pose flies off up-left, its palette line
    /// toggling, for 60 updates.
    fn ghost(&self, cx: &mut Ctx, e: &Enemy) {
        let (mapping, ticks, vy, gravity, vx) = self.ghost;
        let tile = e.tile.unwrap_or(0);
        let mut ghost = Effect::new(EffectKind::Ghost, e.x, e.y, mapping);
        ghost.flip = e.facing_left;
        // Its first update ($15858 into $1587A, the effect pass of this
        // frame) toggles and counts.
        ghost.base = tile;
        ghost.counter = ticks;
        ghost.motion = Motion {
            vx: vx << 16,
            vy: vy << 16,
            gravity,
            ..Motion::default()
        };
        ghost.back = true;
        cx.effect(ghost);
    }
    /// `$157DE`, the afterimage record: it shows sample `$40 & 3`, then on
    /// even updates shifts in the guard's position and frame and takes its
    /// `obj_tile` on palette line 3 (`ori.w #$6000`).
    fn trail(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[TRAIL] == 0 {
            return;
        }
        e.vars[TRAIL] += 1;
        let n = (e.vars[TRAIL] - 1) as usize;
        let sample = n & 3;
        let (packed, mapping) = (e.vars[TRAIL_XY + sample], e.vars[TRAIL_FRAME + sample]);
        if mapping != 0 {
            let mut piece = Effect::new(
                EffectKind::Pieces,
                packed >> 16,
                i32::from(packed as i16),
                mapping as u16,
            );
            piece.base = (e.tile.unwrap_or(0) | 0x6000) & !0x800;
            piece.flip = e.vars[TRAIL_FLIP] != 0;
            piece.back = true;
            // Drawn this frame only: kept by this frame's effect pass.
            piece.counter = 1;
            cx.effect(piece);
        }
        if n & 1 == 0 {
            e.vars.copy_within(TRAIL_XY + 1..TRAIL_XY + 4, TRAIL_XY);
            e.vars
                .copy_within(TRAIL_FRAME + 1..TRAIL_FRAME + 4, TRAIL_FRAME);
            e.vars[TRAIL_XY + 3] = (e.x << 16) | (e.y & 0xffff);
            e.vars[TRAIL_FRAME + 3] = i32::from(e.mapping);
            e.vars[TRAIL_FLIP] = i32::from(e.facing_left);
        }
    }
    fn update_kite(&self, cx: &mut Ctx, e: &mut Enemy) {
        let held = cx.parent(e).is_some_and(|p| p.live && p.vars[HELD] != 0);
        if self.carrier.step(cx, e, held) == CarrierStep::Gone {
            Ctx::free(e);
        }
    }
    /// One dispatch through `Enemy1C_Routines` (`$152AC`).
    fn step(&self, cx: &mut Ctx, e: &mut Enemy) {
        loop {
            match e.routine {
                0 => {
                    // $1531A: the autoscroll stops (Func_01DD9C leaves d0 =
                    // 0 for $FF0180/$FF0184), line 1 colours 9..15.
                    stop(e);
                    cx.signal(Signal::StopAutoscroll);
                    for (i, &color) in self.colors.iter().enumerate() {
                        cx.palette_op(PaletteOp::Color(1, 9 + i as u8, color));
                    }
                    e.tile = Some(cx.tile_base(0x08));
                    e.routine = 4;
                    cx.play(self.music);
                    return;
                }
                4 => {
                    // $15364
                    self.launch_kite(cx, e);
                    e.vars[TIMER] = 1;
                    e.routine = 8;
                }
                8 => {
                    // $15378: until the kite's hover ends, then drop from it.
                    let Some((x, y, routine)) = Self::kite(cx) else {
                        return;
                    };
                    if routine < 0x1c {
                        return;
                    }
                    let (dx, vx, vy, gravity, mapping) = self.drop;
                    e.x = x - dx;
                    e.y = y;
                    e.motion.vx = set_whole(e.motion.vx, vx);
                    e.motion.vy = set_whole(e.motion.vy, vy);
                    e.motion.gravity = gravity;
                    e.mapping = mapping;
                    e.facing_left = true;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $153B4: onto the far lane, then off to the right.
                    if e.y <= self.lane {
                        return;
                    }
                    e.y = self.lane;
                    stop(e);
                    e.motion.vx = set_whole(e.motion.vx, self.exit.0);
                    e.motion.ax = self.exit.1;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $153D8
                    e.mapping = self.ride + ((e.timer >> 2) & 1);
                    if !e.offscreen {
                        return;
                    }
                    stop(e);
                    e.vars[TIMER] = self.enter_ticks;
                    e.vars[DEAD] = 0;
                    e.death_routine = 0x4c;
                    e.hurt_routine = 0x48;
                    e.facing_left = false;
                    e.routine = 0x14;
                    // Func_015766: the afterimage starts.
                    e.vars[TRAIL] = 1;
                    return;
                }
                0x14 => {
                    // $1541E
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] < 0 {
                        e.routine = 0x18;
                    }
                    return;
                }
                0x18 => {
                    // $1542A: run in from the right.
                    e.motion.vx &= !0xffff;
                    e.motion.vy &= !0xffff;
                    e.motion.gravity = 0;
                    e.vars[COUNT] = Self::random(cx, self.wait);
                    e.motion.vx = set_whole(e.motion.vx, self.run_vx);
                    e.vars[TIMER] = 0;
                    e.routine = 0x1c;
                }
                0x1c => {
                    // $1545A: then leap.
                    self.run(e);
                    e.vars[COUNT] -= 1;
                    if e.vars[COUNT] >= 0 {
                        return;
                    }
                    let (vx, vy, gravity, mapping, spears, count) = self.leap;
                    e.motion.vx = set_whole(e.motion.vx, vx);
                    e.motion.vy = set_whole(e.motion.vy, vy);
                    e.motion.gravity = gravity;
                    e.mapping = mapping;
                    e.vars[SPEARS] = spears;
                    e.vars[COUNT] = count;
                    e.routine = 0x20;
                    return;
                }
                0x20 => {
                    // $15490: spears every 16 ticks from the apex.
                    e.vars[COUNT] -= 1;
                    if e.vars[COUNT] < 0 {
                        if e.timer & 0xf == 0 && e.vars[SPEARS] >= 0 {
                            self.spear(cx, e);
                            e.vars[SPEARS] -= 1;
                        }
                        self.air(e);
                    }
                    if !self.landed(e) {
                        return;
                    }
                    e.motion.vx = set_whole(e.motion.vx, self.run_vx);
                    e.routine = 0x24;
                }
                0x24 => {
                    // $154D4: off the left edge, then wait.
                    self.run(e);
                    if !e.offscreen {
                        return;
                    }
                    stop(e);
                    e.vars[COUNT] = Self::random(cx, self.rest);
                    e.routine = 0x28;
                }
                0x28 => {
                    // $154FC: back in from the left.
                    e.vars[COUNT] -= 1;
                    if e.vars[COUNT] >= 0 {
                        return;
                    }
                    e.facing_left = !e.facing_left;
                    e.motion.vx = set_whole(e.motion.vx, self.back_vx);
                    e.routine = 0x2c;
                    return;
                }
                0x2c => {
                    // $15518: within 192 px, leap at the player.
                    self.run(e);
                    if cx.player_dx(e.x) >= self.reach {
                        return;
                    }
                    let (vx, vy, gravity, mapping, count) = self.kick_leap;
                    e.motion.vx = set_whole(e.motion.vx, vx);
                    e.motion.vy = set_whole(e.motion.vy, vy);
                    e.motion.gravity = gravity;
                    e.mapping = mapping;
                    e.vars[COUNT] = count;
                    e.routine = 0x30;
                    cx.play(self.kick_voice);
                    return;
                }
                0x30 => {
                    // $15556: past the apex, dive feet first.
                    e.vars[COUNT] -= 1;
                    if e.vars[COUNT] < 0 {
                        e.mapping = self.kick_pose;
                    }
                    if !Self::falling(e) {
                        return;
                    }
                    let (vx, vy, ax, gravity, mapping) = self.dive;
                    e.motion.vx = set_whole(e.motion.vx, vx);
                    e.motion.vy = set_whole(e.motion.vy, vy);
                    e.motion.ax = ax;
                    e.motion.gravity = gravity;
                    e.mapping = mapping;
                    e.routine = 0x34;
                    self.kick(cx, e);
                    cx.effect_sound(self.dive_sound);
                    return;
                }
                0x34 => {
                    // $1559E
                    if !self.landed(e) {
                        return;
                    }
                    e.motion.vx = set_whole(e.motion.vx, self.return_run.0);
                    e.motion.ax = self.return_run.1;
                    e.routine = 0x38;
                }
                0x38 => {
                    // $155C2: off the right edge, wait, and again.
                    self.run(e);
                    if !e.offscreen {
                        return;
                    }
                    stop(e);
                    e.vars[COUNT] = Self::random(cx, self.rest_back);
                    e.routine = 0x3c;
                }
                0x3c => {
                    // $155EA
                    e.vars[COUNT] -= 1;
                    if e.vars[COUNT] >= 0 {
                        return;
                    }
                    e.facing_left = !e.facing_left;
                    e.routine = 0x18;
                    return;
                }
                0x40 => {
                    // $155FE: the kick landed; bounce off.
                    let (mapping, vx, vy, ax, gravity) = self.bounce;
                    e.mapping = mapping;
                    e.motion.vx = set_whole(e.motion.vx, vx);
                    e.motion.vy = set_whole(e.motion.vy, vy);
                    e.motion.ax = ax;
                    e.motion.gravity = gravity;
                    e.routine = 0x44;
                }
                0x44 => {
                    // $15620
                    if !Self::falling(e) {
                        return;
                    }
                    self.air(e);
                    if !self.landed(e) {
                        return;
                    }
                    e.motion.vx = set_whole(e.motion.vx, self.return_run.0);
                    e.motion.ax = self.return_run.1;
                    e.routine = 0x38;
                    return;
                }
                0x48 => {
                    // $1565A: a hit; the hurt image, and on.
                    e.routine = e.saved_routine;
                    self.ghost(cx, e);
                    return;
                }
                0x4c => {
                    // $1568A: the player is frozen, the kite and the spears
                    // are released.
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[HELD] = 0;
                    e.vars[DEAD] = 1;
                    stop(e);
                    e.motion.vx = set_whole(e.motion.vx, self.death_vx);
                    e.mapping = self.death_pose;
                    e.vars[TIMER] = self.bursts;
                    e.routine = 0x50;
                }
                0x50 => {
                    // $156BA: fire every eighth tick, five times, then the
                    // explosion (score central).
                    cx.player_event(PlayerEvent::Freeze);
                    if e.timer & 7 != 0 {
                        return;
                    }
                    cx.effect_sound(self.burst_sound);
                    cx.fire_puffs(e.x, e.y);
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    let blast = self.blast;
                    cx.explode(e.x, e.y, blast);
                    stop(e);
                    e.mapping = 0;
                    e.tile = Some(0);
                    e.y = 0;
                    e.vars[TIMER] = self.vanish_ticks;
                    e.routine = 0x54;
                    cx.effect_sound(self.vanish_sound);
                    return;
                }
                0x54 => {
                    // $13B20: STAGE CLEAR after 61 ticks.
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    e.vars[TIMER] = self.clear_ticks;
                    e.routine = 0x58;
                    cx.play(self.clear_music);
                    return;
                }
                _ => {
                    // $13B3E: Game_Requests bit 1 after 181 more ticks.
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[TIMER] -= 1;
                    if e.vars[TIMER] >= 0 {
                        return;
                    }
                    cx.next_substage();
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
}
impl Behavior for Guard {
    fn label(&self, _kind: u16) -> String {
        "Enemy1C $15308 (+ kite $157D2)".into()
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        // Func_015748: Func_0128B2 from routine $18 while X <= camera +
        // $140; the body never touches the player.
        let fighting = (0x18..0x48).contains(&enemy.routine);
        if enemy.vars[ROLE] == 0 && fighting && enemy.vars[WINDOW] != 0 {
            HitResponse::WeaponsOnly
        } else {
            HitResponse::Pass
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        if e.vars[ROLE] == KITE {
            self.update_kite(cx, e);
            return;
        }
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        if cx.landed_shot == Some(cx.current) && e.routine < 0x48 {
            // $15AD8: its kick landed without a block or the shield.
            cx.play(self.kick_hit_voice);
            e.routine = 0x40;
        }
        self.drop_kick(cx);
        self.step(cx, e);
        self.trail(cx, e);
        let (min_routine, right) = self.window;
        e.vars[WINDOW] = i32::from(e.routine >= min_routine && e.x <= cx.camera.0 + right);
    }
}

// ---------------------------------------------------------------------------
// Type $94, Enemy94 ($15B60).

/// `$94`: a spear thrown up from the leaping guard, point down (tile
/// `$1000`). It falls into the ground 48 px above the `$C0` line, then
/// slides left with the ground (`-$FF0190`) until it leaves the screen.
/// One weapon hit, or the guard's death (its `$34`), breaks it into two
/// blinking halves with a spark; that frees it without a score.
pub(crate) struct Spear {
    ground_speed: i32,
    stuck: i32,
    ground: i32,
    death_routine: u16,
    halves: [u16; 2],
    velocities: [(i32, i32); 2],
    gravity: i32,
    life: i32,
    spark_dy: i32,
}
impl Spear {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let velocity = |at: usize| -> Result<(i32, i32)> {
            Ok((signed(rom, at)? << 4, signed(rom, at + 2)? << 4))
        };
        Ok(Self {
            // $1ED4, 2-1's scroll routine: move.w #6,($FF0190).
            ground_speed: word_i(rom, 0x1ed6)?,
            stuck: word_i(rom, 0x15ba0)?,
            ground: word_i(rom, 0x15ba4)?,
            death_routine: word(rom, 0x15b88)?,
            halves: [word(rom, 0x15b4c)?, word(rom, 0x15b4e)?],
            velocities: [velocity(0x15c2e)?, velocity(0x15c32)?],
            gravity: signed(rom, 0x15c72)?,
            life: word_i(rom, 0x15c78)?,
            spark_dy: word_i(rom, 0x15c26)?,
        })
    }
    /// `Func_015BE0`: two list-1 halves, then `Func_0368FA`'s spark.
    fn burst(&self, cx: &mut Ctx, e: &Enemy) {
        let left = cx.player_left(e.x);
        for half in [1, 0] {
            let (vx, vy) = self.velocities[half];
            let mut piece = Effect::new(EffectKind::Pieces, e.x, e.y, self.halves[half]);
            piece.base = e.tile.unwrap_or(0);
            piece.motion = Motion {
                vx: if left { -vx } else { vx },
                vy,
                gravity: self.gravity,
                ..Motion::default()
            };
            piece.counter = self.life;
            piece.blink = true;
            cx.effect(piece);
        }
        let first = cx.shared.hit_seq[0];
        cx.effect(Effect::new(
            EffectKind::HitSpark,
            e.x,
            e.y + self.spark_dy,
            first,
        ));
    }
}
impl Behavior for Spear {
    fn label(&self, _kind: u16) -> String {
        "Enemy94 $15B60".into()
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        // Func_0128E4 in routines 4 and 8.
        if matches!(enemy.routine, 4 | 8) {
            HitResponse::NoDeathSpark
        } else {
            HitResponse::Pass
        }
    }
    fn scores(&self) -> bool {
        false
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        // $15B60: the guard's $34 breaks every spear.
        if cx.parent(e).is_some_and(|p| p.vars[DEAD] != 0) {
            e.routine = self.death_routine;
        }
        e.timer = e.timer.wrapping_add(1);
        loop {
            match e.routine {
                0 => {
                    // $15B86
                    e.death_routine = self.death_routine;
                    e.routine = 4;
                }
                4 => {
                    // $15B90: falling until it sticks.
                    if cx.hit_taken(e) {
                        return;
                    }
                    if e.y + self.stuck < self.ground {
                        return;
                    }
                    stop(e);
                    e.motion.vx = set_whole(e.motion.vx, -self.ground_speed);
                    e.routine = 8;
                }
                8 => {
                    // $15BBE
                    if cx.hit_taken(e) {
                        return;
                    }
                    if e.offscreen {
                        Ctx::free(e);
                    }
                    return;
                }
                _ => {
                    // $15BD4
                    self.burst(cx, e);
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
}
