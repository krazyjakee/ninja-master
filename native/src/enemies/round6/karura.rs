//! Type `$F8`, `EnemyF8` (`$34BEE`): Karura, the 6-1 mid-round boss.
use crate::{
    bosses::{Flasher, HealthPalette},
    combat::{Effect, EffectKind, Enemy, Motion, Reaction, Spawn},
    enemies::{Behavior, Ctx, HitResponse},
    game::Terrain,
    rom::{Result, byte, signed, word},
    sound_events::Table,
};

// Routines (`EnemyF8_Routines` `$34C12`).
const K_INIT: u16 = 0x00; // $34C3E
const K_SWIRL: u16 = 0x04; // $34CAC
const K_FORM: u16 = 0x08; // $34D40
const K_HOVER: u16 = 0x0c; // $34DA0
const K_WINDUP: u16 = 0x10; // $34E44
const K_SWOOP: u16 = 0x14; // $34F16
const K_KNOCK: u16 = 0x18; // $3502A
const K_DISSOLVE: u16 = 0x1c; // $351F6
const K_GATHER: u16 = 0x20; // $3522E
const K_DEATH: u16 = 0x24; // $352C8
const K_FALL: u16 = 0x28; // $35304
/// Native `obj_hurt_routine` / `obj_death_routine` markers.
const K_HIT: u16 = 0x80;
const K_KILL: u16 = 0x84;

/// Scratch: routine clock, cooldown (byte 6), health at the last stage hit,
/// flags (`$40` bits and byte-1 bits), the arc/path state and the step.
const KT: usize = 0;
const KCOOL: usize = 1;
const KMARK: usize = 2;
const KFLAGS: usize = 3;
/// Screen X anchor while gathering, forming or dissolving; path origin
/// during the swirl and return flight.
const KX: usize = 4;
const KY: usize = 5;
const KARC: usize = 6;
const KSTEP: usize = 7;
/// `$43`: remaining stage-hit counter, selecting Data_03645C's colour row.
const KPALETTE: usize = 8;
/// Native state of the list-1 `$37` palette flasher.
const KFLASHER: usize = 9;
/// `$12`: half-width of the falling body's particle-emission box.
const KTRAIL_WIDTH: usize = 10;
/// `KFLAGS` bits: byte-1 bit 1 (swirl phase), `$40` bit 0 (heading right),
/// byte-1 bit 3 (high arc next), `$40` bit 5 (slow first gather), `$40`
/// bit 6 (lethal), native: the arc heads right, the path's end Y.
const SWIRL_PHASE: i32 = 1;
const RIGHT: i32 = 2;
const HIGH: i32 = 4;
const SLOW: i32 = 8;
const LETHAL: i32 = 16;
const ARC_RIGHT: i32 = 32;

// Timings walked from the handler (ticks).
/// `$20`: 16 feathers, one every 7 ticks, then their fly-in (64 the first
/// time with `$40` bit 5, else 16).
const FEATHERS: i32 = 16;
const FEATHER_PERIOD: i32 = 7;
const FLY_IN: (i32, i32) = (64, 16);
/// `$04`: arcs of 32 steps, one every 2 ticks, plus an idle step; 18 arcs.
const ARC_STEPS: i32 = 0x21;
const ARCS: i32 = 0x12;
/// `$1C`: a feather leaves every 4 ticks, then 40 ticks.
const DISSOLVE_TICKS: i32 = 16 * 4 + 40;
/// `$08`: frame 0 for 48, `$27A` to 64, `$27B` for 30 more.
const FORM_TICKS: (i32, i32, i32) = (48, 64, 94);
/// `$0C`: X limits (`$36`), attack step and the player's minimum depth.
const HOVER_EDGES: (i32, i32) = (0x60, 0xd6);
const ATTACK_STEP: usize = 0x16;
const ATTACK_DEPTH: i32 = 0x28;
const FLUTTER_STEP: usize = 0x14;
/// `$10`: 32 ticks, rx $14, ry $3C, centre X $4C / $EA.
const WINDUP_TICKS: i32 = 32;
/// `$18`: knock-back 35 ticks (voice at `$44` = $19), return flight $72
/// steps (doubled on the final flight).
const KNOCK_TICKS: i32 = 0x23;
const KNOCK_VOICE: i32 = 0x23 - 0x19;
/// `$28`: $27A for 10 ticks, the music after 55, the request 256 ticks
/// after vanishing.
const FALL_POSE: i32 = 10;
const MUSIC_AFTER: i32 = 0x37;
const REQUEST_AFTER: i32 = 0x100;
/// Hit cooldown (byte 6), damage per stage hit (`$42` >= 6), stage hits.
const COOLDOWN: i32 = 0x14;
const STAGE_DAMAGE: u16 = 6;
const STAGE_HITS: u16 = 4;

/// `$F8`: Karura, a winged bird-man with a scythe (sound names KARURA).
/// Sixteen feathers gather (`$286`-`$28A`), then swirl around an invisible
/// body in 18 random quarter arcs; a hit during the swirl, or its end,
/// scatters them, they gather again and the bird forms (`$27A`/`$27B`). It
/// hovers (`$277`-`$279`) toward a screen edge and, with the player 40 px
/// or more below, winds up (`$27C`) and swoops through the player's X and
/// up to the other edge (`$27D`/`$27E`/`$28B`). Every 6 points of damage
/// (a "stage hit", four in all) knock it back (`$27F`) and it flies back
/// to the centre and swirls again; the fourth sends it on a final swaying
/// flight, it falls in a shower of feathers and the next substage follows.
///
/// Native simplifications: the arcs are quarter ellipses between random
/// end points of `Func_0354D6`'s ranges; the return flights are straight
/// eased paths. Particle collisions remain visual only. The list-1
/// `$37` hit flash alternates the full palette line 1 between white and
/// the remaining stage-hit colour row.
/// Damage counts framework points: 24 in all (`Data_03645C` =
/// 3 steps below 4), so a hit worth 8 does not waste its excess as the
/// original's `$42` reset does.
pub(crate) struct Karura {
    bob: Vec<u8>,
    hover: Vec<u8>,
    dive: Vec<u8>,
    dive_steps: Vec<u8>,
    return_steps: Vec<u8>,
    sway: Vec<i8>,
    /// `Data_034A00`: 65 (cos, sin) pairs, Q12, 0..180 degrees.
    circle: Vec<(i32, i32)>,
    feather_frames: Vec<u8>,
    feather_tiles: Vec<u8>,
    frame: u16,
    steps: u16,
    cry: u8,
    flutter: u8,
    katana: u8,
    hurt_voice: u8,
    death_voice: u8,
    clear_music: u8,
    /// `$34C8C` `ori.w #$A000,d0` into `obj_tile`: its priority bit.
    tile_or: u16,
    health_palette: HealthPalette,
    flasher: Flasher,
    flash_ticks: u16,
}
impl Karura {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let run =
            |p: usize, n: usize| -> Result<Vec<u8>> { (0..n).map(|i| byte(rom, p + i)).collect() };
        let mut circle = Vec::with_capacity(65);
        for i in 0..65 {
            circle.push((signed(rom, 0x34a00 + i * 4)?, signed(rom, 0x34a02 + i * 4)?));
        }
        Ok(Self {
            bob: run(0x35850, 38)?,
            hover: run(0x35876, 38)?,
            dive: run(0x3589c, 109)?,
            dive_steps: run(0x3590a, 108)?,
            return_steps: run(0x35976, 114)?,
            sway: run(0x359e8, 38)?.into_iter().map(|b| b as i8).collect(),
            circle,
            feather_frames: run(0x3827c, 128)?,
            feather_tiles: run(0x381fc, 128)?,
            frame: 0x277,
            // Data_03645C: the hit counter $43 (3 -> four stage hits).
            steps: word(rom, 0x3645c)? + 1,
            cry: Table::Effect.command(rom, 0x35)?,
            flutter: Table::Effect.command(rom, 0x14)?,
            katana: Table::Effect.command(rom, 0x0d)?,
            hurt_voice: Table::Voice.command(rom, 5)?,
            death_voice: Table::Voice.command(rom, 2)?,
            clear_music: Table::Music.command(rom, 0x16)?,
            tile_or: word(rom, 0x34c8e)?,
            health_palette: HealthPalette::flasher(rom, usize::from(word(rom, 0x3563c)?))?,
            flasher: Flasher::load(rom)?,
            flash_ticks: word(rom, 0x35642)?,
        })
    }
    /// `Func_0349C2` over a full circle (`$80`): (cos, sin) in Q12 with
    /// sin positive for angles 0..$40.
    fn polar(&self, angle: i32) -> (i32, i32) {
        let a = angle.rem_euclid(0x80);
        if a <= 0x40 {
            self.circle[a as usize]
        } else {
            let (c, s) = self.circle[(0x80 - a) as usize];
            (c, -s)
        }
    }
    fn body(e: &mut Enemy) {
        e.body = if e.mapping == 0 {
            Some(crate::combat::Hitbox {
                x: 0,
                y: -40,
                half_width: 8,
                half_height: 8,
            })
        } else {
            None
        };
    }
    /// A one-tick feather at angle `a` of the `$0E` orbit.
    fn feather(&self, cx: &mut Ctx, x: i32, y: i32, a: i32) {
        let a = a.rem_euclid(0x80) as usize;
        let frame = self.frame + u16::from(self.feather_frames[a]);
        // Func_038AC6: no flip, vertical, horizontal, both.
        let base = [0x2000, 0x3000, 0x2800, 0x3800][usize::from(self.feather_tiles[a] & 3)];
        cx.overlay(x, y, frame, base, false);
    }
    /// The orbit of `count` feathers (radius $20 -> $0C) around (x, y - 40).
    fn orbit(&self, cx: &mut Ctx, e: &Enemy, count: i32) {
        let spin = -6 * (e.age as i32 & 0x7f);
        let r = 0x20 - (e.age as i32 & 0x0f) * (0x20 - 0x0c) / 16;
        for i in 0..count.clamp(0, FEATHERS) {
            let a = spin + i * 8;
            let (c, s) = self.polar(a);
            self.feather(cx, e.x + ((r * c) >> 12), e.y - 40 - ((r * s) >> 12), a);
        }
    }
    /// `Func_0354D6`: the next quarter arc from the current position.
    fn plan(&self, cx: &mut Ctx, e: &mut Enemy) {
        let sx = e.x - cx.camera.0;
        let right = sx < 0x50 || (sx < 0xf0 && cx.random() & 1 != 0);
        let reach = if right { 0x140 - sx } else { sx }.max(1);
        let dx = i32::from(cx.random()) % reach;
        let high = e.vars[KFLAGS] & HIGH != 0;
        let spread = i32::from(cx.random()) % 0x50;
        let end_y = if high { 0x78 - spread } else { 0x78 + spread };
        e.vars[KFLAGS] ^= HIGH;
        e.vars[KFLAGS] = if right {
            e.vars[KFLAGS] | ARC_RIGHT
        } else {
            e.vars[KFLAGS] & !ARC_RIGHT
        };
        e.vars[KX] = sx;
        e.vars[KY] = e.y - cx.camera.1;
        e.vars[KARC] = ((if right { dx } else { -dx }) << 16) | ((end_y - e.vars[KY]) & 0xffff);
        e.vars[KSTEP] = 0;
    }
    /// One swirl tick: the angle steps every second tick (32 steps).
    fn arc(&self, cx: &Ctx, e: &mut Enemy) {
        let step = (e.vars[KSTEP] / 2).min(0x20);
        let (c, s) = self.polar(step);
        let dx = e.vars[KARC] >> 16;
        let dy = i32::from(e.vars[KARC] as i16);
        e.x = cx.camera.0 + e.vars[KX] + ((dx * (0x1000 - c)) >> 12);
        e.y = cx.camera.1 + e.vars[KY] + ((dy * s) >> 12);
    }
    fn begin(e: &mut Enemy, routine: u16) {
        e.routine = routine;
        e.vars[KT] = 0;
    }
    /// `loc_03566C`: toggle the phase; entering the bird phase turns round.
    fn regather(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.vars[KFLAGS] ^= SWIRL_PHASE;
        if e.vars[KFLAGS] & SWIRL_PHASE == 0 {
            e.vars[KFLAGS] ^= RIGHT;
            e.facing_left = !e.facing_left;
        }
        e.mapping = 0;
        // $35686: obj_2A is a saved screen X, reset to the centre.
        e.vars[KX] = 0xa0;
        e.y = e.vars[KMARK] >> 16;
        cx.play(self.cry);
        Self::begin(e, K_GATHER);
    }
    fn base_y(e: &Enemy) -> i32 {
        e.vars[KMARK] >> 16
    }
    fn mark(e: &Enemy) -> u16 {
        (e.vars[KMARK] & 0xffff) as u16
    }
    /// `Func_0356B8`: the stage hit's knock-back away from the player.
    fn knock(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.mapping = self.frame + 8;
        e.motion.vx = if cx.player_left(e.x) {
            2 << 16
        } else {
            -2 << 16
        };
        e.motion.vy = -4 << 16;
        e.motion.gravity = 0x5000;
        for _ in 0..2 {
            let a = i32::from(cx.random() & 0x7f);
            let mut piece = Effect::new(EffectKind::Pieces, e.x, e.y - 32, self.frame + 0x13);
            piece.base = 0x2000;
            piece.motion = Motion {
                vx: if a & 1 != 0 { 0x10000 } else { -0x10000 },
                vy: 4 << 16,
                gravity: 0x3000,
                ..Motion::default()
            };
            piece.counter = 90;
            cx.effect(piece);
        }
        Self::begin(e, K_KNOCK);
    }
    /// The feathers shed on forming, or on the fall (`$10`, `Func_0357C8`).
    fn shed(&self, cx: &mut Ctx, e: &Enemy, count: i32) {
        for i in 0..count {
            let index = 15 - i * (16 / count);
            let a = i32::from(cx.shared.karura.feather_angles[index as usize]);
            let (c, s) = self.polar(a);
            let speed = if a & 4 != 0 { 6 } else { 8 };
            let forming = e.routine == K_FORM;
            let x = e.x
                + if forming {
                    if a >= 0x20 { -40 } else { 40 }
                } else {
                    0
                };
            let y = e.y - if forming { 48 } else { 32 };
            let mut piece = Effect::new(EffectKind::KaruraFeather, x, y, 0);
            piece.base = 0x2000;
            piece.motion = Motion {
                // $38454/$38460 write integer velocity words.
                vx: ((4 * c) >> 12) << 16,
                vy: -((speed * s) >> 12) << 16,
                gravity: cx.shared.karura.feather_gravity,
                ..Motion::default()
            };
            cx.effect(piece);
        }
    }
    /// `$357AE`: seven list-1 type-7 sparks using the $379FC animation.
    fn death_sparks(&self, cx: &mut Ctx, e: &Enemy) {
        for angle in cx.shared.karura.spark_angles.into_iter().rev() {
            let (c, s) = self.polar(i32::from(angle));
            let mut spark = Effect::new(EffectKind::KaruraSpark, e.x, e.y - 32, 0);
            spark.motion = Motion {
                vx: (3 * c) << 4,
                vy: -((4 * s) << 4),
                gravity: cx.shared.karura.spark_gravity,
                ..Motion::default()
            };
            cx.effect(spark);
        }
    }
    /// `$35372/$37C1A`: one effect in the growing body box each falling tick.
    fn fall_trail(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.vars[KTRAIL_WIDTH] = (e.vars[KTRAIL_WIDTH] + 8) & 0xff;
        let width = e.vars[KTRAIL_WIDTH];
        let dx = ((i32::from(cx.random() & 0xff) * width * 2) >> 8) - width;
        let dy = ((i32::from(cx.random() & 0xff) * 64) >> 8) - 64;
        let mut trail = Effect::new(EffectKind::KaruraTrail, e.x + dx, e.y + dy, 0);
        trail.origin = (dx, dy);
        trail.phase = (cx.random() & 1) as u8;
        cx.effect(trail);
    }
    fn step(&self, cx: &mut Ctx, e: &mut Enemy, terrain: &dyn Terrain) {
        let t = e.vars[KT];
        e.vars[KT] += 1;
        match e.routine {
            K_INIT => {
                // $34C3E
                e.motion.stop();
                e.vars[KX] = 0xa0;
                e.x = cx.camera.0 + 0xa0;
                let base = cx.camera.1 + 0x70;
                e.y = base;
                e.health = STAGE_DAMAGE * self.steps.max(STAGE_HITS);
                e.vars[KMARK] = (base << 16) | i32::from(e.health);
                e.vars[KPALETTE] = i32::from(self.steps.saturating_sub(1));
                e.vars[KFLAGS] = SWIRL_PHASE | SLOW;
                e.hurt_routine = K_HIT;
                e.death_routine = K_KILL;
                e.mapping = 0;
                // Object_InitRecord starts obj_tile at zero. $34C84..$34C94
                // raises it and selects palette 1; these mappings already
                // contain absolute tile indices, so do not add the art base.
                e.tile = Some(self.tile_or);
                cx.play(self.cry);
                Self::begin(e, K_GATHER);
            }
            K_GATHER => {
                // $3522E: 16 feathers fly in and orbit the body point.
                // $3522E adds the saved obj_2A value, not its offset.
                e.x = cx.camera.0 + e.vars[KX];
                let fly = if e.vars[KFLAGS] & SLOW != 0 {
                    FLY_IN.0
                } else {
                    FLY_IN.1
                };
                self.orbit(cx, e, t / FEATHER_PERIOD);
                if t + 1 >= FEATHERS * FEATHER_PERIOD + fly {
                    e.vars[KFLAGS] &= !SLOW;
                    if e.vars[KFLAGS] & SWIRL_PHASE != 0 {
                        e.vars[KFLAGS] &= !HIGH;
                        e.vars[KSTEP] = 0;
                        e.phase = 0;
                        Self::begin(e, K_SWIRL);
                        self.plan(cx, e);
                    } else {
                        Self::begin(e, K_FORM);
                    }
                }
            }
            K_SWIRL => {
                // $34CAC: invisible, swirling in quarter arcs.
                e.mapping = 0;
                self.arc(cx, e);
                self.orbit(cx, e, FEATHERS);
                e.vars[KSTEP] += 1;
                if e.vars[KSTEP] >= ARC_STEPS * 2 {
                    e.phase += 1;
                    if i32::from(e.phase) >= ARCS {
                        self.scatter(cx, e);
                    } else {
                        self.plan(cx, e);
                    }
                }
            }
            K_DISSOLVE => {
                // $351F6: a feather leaves every 4 ticks.
                // $351F6 adds the screen X saved by $35752 in obj_28.
                e.x = cx.camera.0 + e.vars[KX];
                self.orbit(cx, e, FEATHERS - t / 4);
                if t + 1 >= DISSOLVE_TICKS {
                    self.regather(cx, e);
                }
            }
            K_FORM => {
                // $34D40: the bird forms; four feathers fall away.
                e.x = cx.camera.0 + e.vars[KX];
                let (dark, blink, done) = FORM_TICKS;
                e.mapping = if t < dark {
                    0
                } else if t < blink {
                    self.frame + 3
                } else {
                    self.frame + 4
                };
                if t < blink {
                    self.orbit(cx, e, FEATHERS);
                }
                if t == blink {
                    self.shed(cx, e, 4);
                }
                if t + 1 >= done {
                    e.vars[KX] = e.x - cx.camera.0;
                    e.vars[KSTEP] = 0;
                    Self::begin(e, K_HOVER);
                }
            }
            K_HOVER => {
                // $34DA0 (Func_035784): bob and flap toward an edge.
                let s = (e.vars[KSTEP] as usize) % self.bob.len();
                let right = e.vars[KFLAGS] & RIGHT != 0;
                let at_edge = if right {
                    e.vars[KX] > HOVER_EDGES.1
                } else {
                    e.vars[KX] < HOVER_EDGES.0
                };
                if !at_edge {
                    e.vars[KX] += if right { 1 } else { -1 };
                }
                e.x = cx.camera.0 + e.vars[KX];
                e.y = Self::base_y(e) + i32::from(self.bob[s]) / 2;
                e.mapping = self.frame + u16::from(self.hover[s]);
                if s == FLUTTER_STEP {
                    cx.play(self.flutter);
                }
                let dy = cx.player.1 - e.y;
                if at_edge && s == ATTACK_STEP && dy > ATTACK_DEPTH {
                    e.vars[KARC] = if right { 0 } else { 0x40 };
                    Self::begin(e, K_WINDUP);
                    return;
                }
                e.vars[KSTEP] = ((s + 1) % self.bob.len()) as i32;
            }
            K_WINDUP => {
                // $34E44: a small loop before the dive.
                let right = e.vars[KFLAGS] & RIGHT != 0;
                e.vars[KARC] += if right { -2 } else { 2 };
                let (c, s) = self.polar(e.vars[KARC]);
                let centre = if right { 0xea } else { 0x4c };
                e.x = cx.camera.0 + centre + ((0x14 * c) >> 12);
                e.y = Self::base_y(e) + 0x14 - ((0x3c * s) >> 12);
                // Half way round the loop ($20 of angle).
                if t == 16 {
                    e.mapping = self.frame + 2;
                }
                if t + 1 >= WINDUP_TICKS {
                    let sx = (cx.player.0 - cx.camera.0).clamp(0x4a, 0xec);
                    let below = cx.player.1 - e.y - ATTACK_DEPTH;
                    e.vars[KX] = sx;
                    e.vars[KY] = if below > 0 { below } else { 0x20 };
                    e.vars[KARC] = if right { 0 } else { 0x40 };
                    e.vars[KSTEP] = 0;
                    Self::begin(e, K_SWOOP);
                }
            }
            K_SWOOP => {
                // $34F16: through the player's X and up to the other edge.
                let step = e.vars[KSTEP] as usize;
                let right = e.vars[KFLAGS] & RIGHT != 0;
                if step >= self.dive.len() {
                    e.vars[KX] = e.x - cx.camera.0;
                    e.vars[KSTEP] = 0;
                    Self::begin(e, K_HOVER);
                    return;
                }
                let a = e.vars[KARC];
                let (c, s) = self.polar(a);
                let centre = e.vars[KX];
                let rx = if a >= 0x20 {
                    centre - 0x38
                } else {
                    0xfe - centre
                };
                let ry = e.vars[KY] + if step >= 0x20 { 0x14 } else { 0 };
                e.x = cx.camera.0 + centre + ((rx * c) >> 12);
                e.y = Self::base_y(e) + ((ry * s) >> 12) + if step < 0x20 { 0x14 } else { 0 };
                e.mapping = self.frame + u16::from(self.dive[step]);
                if step == 0x20 {
                    cx.play(self.katana);
                }
                if step == 0x37 || step == 0x57 {
                    cx.play(self.flutter);
                }
                let delta = i32::from(*self.dive_steps.get(step).unwrap_or(&0));
                e.vars[KARC] += if right { delta } else { -delta };
                e.vars[KSTEP] += 1;
            }
            K_KNOCK => {
                // $3502A: knocked back, then back to the centre (blinking),
                // or the final swaying flight.
                let lethal = e.vars[KFLAGS] & LETHAL != 0;
                if t < KNOCK_TICKS {
                    if t == KNOCK_VOICE {
                        cx.play(self.hurt_voice);
                    }
                    if t + 1 == KNOCK_TICKS {
                        e.motion.stop();
                        e.vars[KX] = e.x;
                        e.vars[KY] = e.y;
                    }
                    return;
                }
                let steps = self.return_steps.len() as i32 * if lethal { 2 } else { 1 };
                let k = t - KNOCK_TICKS;
                let (c, _) = self.polar(k * 0x40 / steps.max(1));
                let ease = (0x1000 - c) / 2;
                let (tx, ty) = (cx.camera.0 + 0xa0, Self::base_y(e));
                e.x = e.vars[KX] + (((tx - e.vars[KX]) * ease) >> 12);
                e.y = e.vars[KY] + (((ty - e.vars[KY]) * ease) >> 12);
                let s = (k as usize) % self.bob.len();
                e.mapping = self.frame + u16::from(self.hover[s]);
                if lethal {
                    e.x += i32::from(self.sway[(k as usize / 2) % self.sway.len()]);
                } else if k & 1 == 0 {
                    e.mapping = 0;
                }
                if s == FLUTTER_STEP {
                    cx.play(self.flutter);
                }
                if k + 1 >= steps {
                    if lethal {
                        Self::begin(e, K_DEATH);
                    } else {
                        self.regather(cx, e);
                    }
                }
            }
            K_DEATH => {
                // $352C8: score (central), debris, the music fades.
                e.mapping = self.frame + 3;
                cx.play(self.death_voice);
                self.death_sparks(cx, e);
                cx.play(0xfc);
                Self::begin(e, K_FALL);
            }
            K_FALL => {
                // $35304: vanishes in 16 feathers, the stage-clear music,
                // then Game_Requests bit 1.
                if t + 1 == MUSIC_AFTER {
                    cx.play(self.clear_music);
                }
                if t + 1 == FALL_POSE {
                    e.mapping = 0;
                    e.vars[KTRAIL_WIDTH] = 32;
                    self.shed(cx, e, 16);
                    e.motion.gravity = 0x1000;
                    return;
                }
                if e.motion.gravity != 0 {
                    self.fall_trail(cx, e);
                    e.motion.vx = e.motion.vx.clamp(-8 << 16, 8 << 16);
                    e.motion.vy = e.motion.vy.clamp(-8 << 16, 8 << 16);
                    if terrain.solid_at(e.x, e.y) {
                        e.motion.stop();
                    }
                }
                if t + 1 >= FALL_POSE + REQUEST_AFTER {
                    cx.next_substage();
                    Ctx::free(e);
                }
            }
            _ => Self::begin(e, K_HOVER),
        }
    }
    /// `Func_035752`: the swirl ends (or was hit); the feathers scatter.
    fn scatter(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.play(self.cry);
        e.mapping = 0;
        e.vars[KX] = e.x - cx.camera.0;
        Self::begin(e, K_DISSOLVE);
    }
}
impl Behavior for Karura {
    fn label(&self, _kind: u16) -> String {
        "EnemyF8 $34BEE (Karura)".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        if e.vars[KCOOL] > 0 {
            e.vars[KCOOL] -= 1;
        }
        let reaction = cx.process_hit(e);
        if reaction.is_some() && e.routine >= K_HIT {
            e.routine = e.saved_routine;
            e.vars[KCOOL] = COOLDOWN;
            if e.routine == K_SWIRL && reaction == Some(Reaction::Hurt) {
                // $35604: a swirl hit only scatters the feathers.
                e.health = Self::mark(e);
                self.scatter(cx, e);
            } else {
                Flasher::start(&mut e.vars[KFLASHER], self.flash_ticks);
                let lethal = reaction == Some(Reaction::Death);
                if lethal || Self::mark(e).saturating_sub(e.health) >= STAGE_DAMAGE {
                    // A stage hit ($42 >= 6): knock-back.
                    let base = e.vars[KMARK] & !0xffff;
                    e.vars[KMARK] = base | i32::from(e.health);
                    e.vars[KPALETTE] = (e.vars[KPALETTE] - 1).max(0);
                    if lethal {
                        e.vars[KFLAGS] |= LETHAL;
                    }
                    self.knock(cx, e);
                }
            }
        }
        self.step(cx, e, terrain);
        for effect in cx
            .effects
            .iter_mut()
            .filter(|effect| effect.kind == EffectKind::KaruraTrail && effect.phase == 1)
        {
            effect.x = e.x + effect.origin.0;
            effect.y = e.y + effect.origin.1;
        }
        // The list-1 flasher runs after list 5, including its allocating
        // frame; apply it after the hit has updated the stage counter.
        let row = e.vars[KPALETTE] as usize;
        self.flasher
            .step(cx, &mut e.vars[KFLASHER], &self.health_palette, row);
        Self::body(e);
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        if enemy.vars[KCOOL] > 0 || enemy.vars[KFLAGS] & LETHAL != 0 {
            return HitResponse::Pass;
        }
        match enemy.routine {
            K_SWIRL | K_HOVER | K_SWOOP => HitResponse::Accept,
            _ => HitResponse::Pass,
        }
    }
}
