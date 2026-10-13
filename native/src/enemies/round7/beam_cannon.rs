//! Type `$E8`, `EnemyE8` (`$1ACA6`), with its beam (projectile `$2E`, `$1B1AC`).
use super::{ATTACHED, animate, bursts, runs};
use crate::{
    combat::{Effect, EffectKind, Enemy, Hitbox, Motion, Projectile, ShotBox, Spawn},
    enemies::{Behavior, Ctx, HitResponse, aim, shot_damage},
    game::Terrain,
    rom::{Result, byte, sequence, word, word_i},
};

/// `$30`: countdown; `$2A`: opening frame / death timer; `$2C`: rest;
/// `$2E`: beam duration; hurt countdown; beam state, frame index and timer.
const E8_COUNT: usize = 0;
const E8_STEP: usize = 1;
const E8_REST: usize = 2;
const E8_BEAM_TICKS: usize = 3;
const E8_HURT: usize = 4;
const E8_BEAM: usize = 5;
const E8_BEAM_INDEX: usize = 6;
const E8_BEAM_TIMER: usize = 7;

/// `$1B1AC`: `move.w #$007F,obj_hitbox_x` and `move.w #$207F,obj_hitbox_w`
/// on every update of the beam, whatever frame it shows.
const BEAM_BOX: Hitbox = Hitbox {
    x: 0,
    y: 0x7f,
    half_width: 0x20,
    half_height: 0x7f,
};

/// `$E8`: a ceiling cannon drawn from stage tiles (`obj_tile` `$8000`).
/// Rests `(flags & $F0) + $3C` ticks, charges 61 (CHARGE effect `$34`,
/// opening `$1AE..$1B4` on even ticks, particles drawn up into it every 8
/// ticks), holds 49 and fires its beam (`$1B1AC`, effect `$2E`): 32 px
/// below, 254 px long, growing `$1A8/$1A9/$1AA` then `$19E`, flickering,
/// damage 3, for the 112-tick rest after firing (and while hurt), then
/// retracting. Six points kill it (health counter 5); a hit whitens its
/// palette while it fades back, invulnerable. The death burns 61 ticks
/// with falling puffs and ends in nine double bursts.
pub(crate) struct BeamCannon {
    tile: u16,
    idle: u16,
    open: u16,
    open_last: u16,
    open_frames: i32,
    rest_add: i32,
    beam_ticks: i32,
    charge_ticks: i32,
    charge_sound: u8,
    aim_ticks: i32,
    beam_sound: u8,
    beam_offset: i32,
    beam_damage: u8,
    grow: Vec<u16>,
    on: [u16; 2],
    retract: Vec<u16>,
    flash: i32,
    burn_ticks: i32,
    burn_sound: u8,
    burn_tile: u16,
    burn_angle: i32,
    pause_ticks: i32,
    burst_count: i32,
    burst_sound: u8,
    particle: Vec<u16>,
    particle_tile: u16,
    particle_below: i32,
    particle_spread: i32,
    particle_stop: i32,
}
impl BeamCannon {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        // Data_01B136 + Data_01B142 (growth runs on into the steady pair),
        // Data_01B148 (retraction).
        let mut grow = Vec::new();
        for i in 0..16 {
            let value = word(rom, 0x1b136 + i * 2)?;
            if value & 0x8000 != 0 {
                break;
            }
            grow.push(value);
        }
        Ok(Self {
            tile: word(rom, 0x1acc8)?,
            idle: word(rom, 0x1acce)?,
            open: word(rom, 0x1ad42)?,
            open_last: word(rom, 0x1ad4e)?,
            open_frames: word_i(rom, 0x1ad3c)?,
            rest_add: word_i(rom, 0x1acf0)?,
            beam_ticks: word_i(rom, 0x1acf4)?,
            charge_ticks: word_i(rom, 0x1ad14)?,
            charge_sound: byte(rom, 0x1ad23)?,
            aim_ticks: word_i(rom, 0x1ad6e)?,
            beam_sound: byte(rom, 0x1b197)?,
            beam_offset: word_i(rom, 0x1b17e)?,
            beam_damage: shot_damage(rom, usize::from(byte(rom, 0x1b15b)?))?,
            grow,
            on: [word(rom, 0x1b142)?, word(rom, 0x1b144)?],
            retract: {
                let mut out = Vec::new();
                for i in 0..16 {
                    let value = word(rom, 0x1b148 + i * 2)?;
                    if value & 0x8000 != 0 {
                        break;
                    }
                    out.push(value);
                }
                out
            },
            // $1AE52 fades palette line 1 back from white one nibble step per
            // tick: at most 14 ticks.
            flash: 14,
            burn_ticks: word_i(rom, 0x1adbe)?,
            burn_sound: byte(rom, 0x1af7f)?,
            burn_tile: word(rom, 0x1af9e)?,
            burn_angle: word_i(rom, 0x1afbe)?,
            pause_ticks: word_i(rom, 0x1add4)?,
            burst_count: word_i(rom, 0x1adea)?,
            burst_sound: byte(rom, 0x1ae01)?,
            particle: sequence(rom, 0x1b0b6)?,
            particle_tile: word(rom, 0x1af30)?,
            particle_below: word_i(rom, 0x1aede)?,
            particle_spread: word_i(rom, 0x1aef4)?,
            particle_stop: word_i(rom, 0x1af3a)?,
        })
    }
    /// `Func_01AEC0` (d7 = 0): a charge particle 96..159 px below, drawn up
    /// into the cannon at three times the `$9562` direction; frames every
    /// fourth tick from its fourth, freed above cannon Y + 16.
    fn charge(&self, cx: &mut Ctx, e: &Enemy) {
        let y = e.y + self.particle_below + i32::from(cx.random() & 63);
        let x = e.x + i32::from(cx.random() & 31) - self.particle_spread;
        let (dx, dy) = aim((x, y), (e.x, e.y));
        let motion = Motion {
            vx: dx * 3,
            vy: dy * 3,
            ..Motion::default()
        };
        let stop = e.y + self.particle_stop;
        let (mut sx, mut sy, mut m) = (x, y, motion);
        let mut life: u16 = 0;
        while life < 64 && sy >= stop {
            m.step(&mut sx, &mut sy);
            life += 1;
        }
        let mut left = life.saturating_sub(3);
        let mut segments = Vec::new();
        for (mapping, ticks) in runs(&self.particle, 4) {
            let ticks = ticks.min(left);
            if ticks == 0 {
                break;
            }
            segments.push((mapping, ticks));
            left -= ticks;
        }
        animate(cx, (x, y), motion, self.particle_tile, 3, &segments, false);
    }
    /// `Func_01AF74`: effect `$12` every 8 ticks; on even ticks one puff
    /// falling at 4 px/tick within ±31/1024 of straight down (the cannon
    /// hangs from the ceiling, so its tile flips them).
    fn burn(&self, cx: &mut Ctx, e: &Enemy) {
        if e.timer & 7 == 0 {
            cx.effect_sound(self.burn_sound);
        }
        if e.timer & 1 != 0 {
            return;
        }
        let x = e.x + i32::from(cx.random() & 63) - 31;
        let y = e.y + i32::from(cx.random() & 31);
        let angle = (i32::from(cx.random() & 63) - 31 + self.burn_angle) as u16 & 0x3ff;
        let (c, s) = cx.shared.sine.polar(angle);
        let mut puff = Effect::new(EffectKind::Puff, x, y, 0);
        puff.base = self.burn_tile;
        // $1AFDC: these falling puffs advance every fourth update,
        // retaining $9000's vertical flip and high plane priority.
        puff.period = 4;
        puff.high_priority = true;
        puff.motion = Motion {
            vx: c << 2,
            vy: s << 2,
            ..Motion::default()
        };
        cx.effect(puff);
    }
    /// `$1B1AC` for one tick: grow, flicker while the cannon rests after
    /// firing (or is hurt), retract. The projectile is rebuilt every tick
    /// so that its fixed frame still collides (native). The handler writes
    /// its own box at the top of every update (`$1B1AC..$1B1B8`: centre
    /// `0,127`, half size `32,127`), so it meets the player at full width
    /// from the first growth frame, and on the blank (mapping 0) frames of
    /// the flicker too.
    fn beam(&self, cx: &mut Ctx, e: &mut Enemy) {
        let me = cx.current;
        cx.projectiles
            .retain(|p| !(p.owner == Some(me) && p.life == ATTACHED));
        if e.vars[E8_BEAM] == 0 {
            return;
        }
        e.vars[E8_BEAM_TIMER] += 1;
        let mapping = loop {
            match e.vars[E8_BEAM] {
                1 => match self.grow.get(e.vars[E8_BEAM_INDEX] as usize) {
                    Some(&m) => {
                        e.vars[E8_BEAM_INDEX] += 1;
                        break m;
                    }
                    None => e.vars[E8_BEAM] = 2,
                },
                2 => {
                    // $1B1D6: kept while the cannon is in routines $10..$18.
                    if e.live() && (0x10..0x1c).contains(&e.routine) {
                        break self.on[(e.vars[E8_BEAM_TIMER] & 1) as usize];
                    }
                    e.vars[E8_BEAM_INDEX] = 0;
                    e.vars[E8_BEAM] = 3;
                }
                _ => match self.retract.get(e.vars[E8_BEAM_INDEX] as usize) {
                    Some(&m) => {
                        e.vars[E8_BEAM_INDEX] += 1;
                        break m;
                    }
                    None => {
                        // $1B238: freed at the end of the table.
                        e.vars[E8_BEAM] = 0;
                        return;
                    }
                },
            }
        };
        let mut beam = Projectile::new(
            e.x,
            e.y + self.beam_offset,
            0,
            true,
            mapping,
            self.beam_damage,
        )
        .with_art(self.tile, false)
        .with_motion(Motion {
            fx: 1,
            ..Motion::default()
        });
        beam.life = ATTACHED;
        beam.passes_terrain = true;
        beam.box_load = ShotBox::Fixed(BEAM_BOX);
        beam.owner = Some(me);
        cx.shoot(beam);
    }
}
impl Behavior for BeamCannon {
    fn label(&self, _kind: u16) -> String {
        "EnemyE8 $1ACA6".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.idle
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        // Func_01AE74: Func_0128B2 only below routine $14.
        if (4..0x14).contains(&e.routine) {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, _terrain: &dyn Terrain) {
        if e.offscreen && e.routine != 0 {
            // $1ACA6: nothing runs off screen; the beam object still does.
            self.beam(cx, e);
            return;
        }
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $1ACC0
                    e.motion.stop();
                    e.timer = 0;
                    e.tile = Some(self.tile);
                    e.mapping = self.idle;
                    e.death_routine = 0x1c;
                    e.hurt_routine = 0x14;
                    let rest = i32::from(spawn.flags & 0xf0) + self.rest_add;
                    e.vars[E8_REST] = rest;
                    e.vars[E8_BEAM_TICKS] = self.beam_ticks;
                    e.vars[E8_COUNT] = rest;
                    e.routine = 4;
                    break;
                }
                4 => {
                    // $1AD08: rest, then charge.
                    e.vars[E8_COUNT] -= 1;
                    if e.vars[E8_COUNT] >= 0 {
                        break;
                    }
                    e.vars[E8_COUNT] = self.charge_ticks;
                    e.vars[E8_STEP] = 0;
                    e.routine = 8;
                    cx.effect_sound(self.charge_sound);
                    break;
                }
                8 => {
                    // $1AD2A: open on even ticks, particles every 8.
                    let mut frame = self.idle;
                    if e.timer & 1 == 0 {
                        if e.vars[E8_STEP] != self.open_frames {
                            frame = self.open + e.vars[E8_STEP] as u16;
                            e.vars[E8_STEP] += 1;
                        } else {
                            frame = self.open_last;
                        }
                    }
                    e.mapping = frame;
                    e.vars[E8_COUNT] -= 1;
                    if e.vars[E8_COUNT] >= 0 {
                        if e.timer & 7 == 0 {
                            self.charge(cx, e);
                        }
                        break;
                    }
                    e.vars[E8_COUNT] = self.aim_ticks;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $1AD76: hold, then the beam (`Func_01B15A`).
                    e.mapping = if e.timer & 1 == 1 {
                        self.idle
                    } else {
                        self.open_last
                    };
                    e.vars[E8_COUNT] -= 1;
                    if e.vars[E8_COUNT] >= 0 {
                        break;
                    }
                    e.mapping = self.idle;
                    e.vars[E8_BEAM] = 1;
                    e.vars[E8_BEAM_INDEX] = 0;
                    e.vars[E8_BEAM_TIMER] = 0;
                    cx.effect_sound(self.beam_sound);
                    e.vars[E8_COUNT] = e.vars[E8_BEAM_TICKS];
                    e.routine = 0x10;
                    break;
                }
                0x10 => {
                    // $1ADA8
                    e.vars[E8_COUNT] -= 1;
                    if e.vars[E8_COUNT] >= 0 {
                        break;
                    }
                    e.vars[E8_COUNT] = e.vars[E8_REST];
                    e.routine = 4;
                    break;
                }
                0x14 => {
                    // $1AE28: palette line 1 white, invulnerable ($36 = -1).
                    e.flash = self.flash as u16;
                    e.vars[E8_HURT] = self.flash;
                    e.routine = 0x18;
                    break;
                }
                0x18 => {
                    // $1AE52: until the fade back is complete.
                    e.vars[E8_HURT] -= 1;
                    if e.vars[E8_HURT] > 0 {
                        break;
                    }
                    e.routine = e.saved_routine;
                    break;
                }
                0x1c => {
                    // $1ADBC: score (central).
                    e.vars[E8_STEP] = self.burn_ticks;
                    e.routine = 0x20;
                }
                0x20 => {
                    // $1ADC6
                    self.burn(cx, e);
                    e.vars[E8_STEP] -= 1;
                    if e.vars[E8_STEP] >= 0 {
                        break;
                    }
                    e.vars[E8_STEP] = self.pause_ticks;
                    e.routine = 0x24;
                    break;
                }
                0x24 => {
                    // $1ADE2
                    e.vars[E8_STEP] -= 1;
                    if e.vars[E8_STEP] >= 0 {
                        break;
                    }
                    e.vars[E8_STEP] = self.burst_count;
                    e.routine = 0x28;
                }
                _ => {
                    // $1ADF2: a double burst every 4 ticks, then freed.
                    if e.timer & 3 == 0 {
                        cx.effect_sound(self.burst_sound);
                        bursts(cx, e, (0, 0));
                        e.vars[E8_STEP] -= 1;
                        if e.vars[E8_STEP] < 0 {
                            Ctx::free(e);
                        }
                    }
                    break;
                }
            }
        }
        self.beam(cx, e);
    }
}
