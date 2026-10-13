//! Type `$C0`, `EnemyC0` (`$19656`): the end-of-3-1 capsule rain and floor collapse.
use super::tank::Tank;
use crate::{
    combat::{Effect, EffectKind, Enemy, Motion, Signal, Spawn},
    enemies::{Behavior, Ctx, HitResponse},
    game::Terrain,
    movement::PlayerEvent,
    rom::{Result, byte, long, signed, word, word_i},
    sound_events::Table,
};

/// `$2A`: timer; mirrors of `$FF3D06` (kills still wanted) and `$FF3D05`
/// (live `$FC`) for the capsules; `$2C`: the sinking floor's scroll word.
const COLLAPSE_TIMER: usize = 0;
pub(super) const COLLAPSE_REMAINING: usize = 1;
pub(super) const COLLAPSE_ALIVE: usize = 2;
const SINK: usize = 3;
const SINK_PHASE: usize = 4;
const SINK_COLUMNS: std::ops::Range<usize> = 5..9;

/// `$C0`: the scripted end of 3-1. After 60 ticks (the ROM walls the
/// screen in) a capsule falls every 60 ticks at a random column
/// (`Data_019626`, 16 words; half of the upper eight read the routine
/// table and land off screen) while `$FF3D06` >= 0; each capsule releases
/// a `$FC` and eight `$FC` kills end the rain. Once no `$FC` is left the
/// attacks are locked for 60 ticks, the floor sinks with falling debris
/// (the scroll word drops 3 every 16 ticks until below `$100`) and gives
/// way, and the next substage is requested when the player falls below
/// `$1FF`.
pub(crate) struct Collapse {
    remaining: i8,
    lock_ticks: i32,
    rain_ticks: i32,
    columns: Vec<i32>,
    column_mask: u16,
    drop_y: i32,
    wait_ticks: i32,
    stage: (i32, i32),
    sink_step: i32,
    sink_period: u16,
    sink_floor: i32,
    fall_lines: (i32, i32),
    /// `Func_0197FE`: the top-left pixel of the 4 x 2 metatiles it clears.
    floor: (i32, i32),
    /// `Func_03775E`'s `d6` and `d5` for the marker over the hole.
    marker: (u16, u16),
    music: (u8, u8),
    debris: Debris,
    tank: Tank,
}
/// `Func_018F8A`: list-1 `$33` debris every other tick while the floor
/// sinks: `x ± 31`, `vy` -3.5, gravity `$2000..$3FFF`, `Data_036878`
/// (native: its first mapping for the sequence's length).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Debris {
    x_mask: u16,
    x_bias: i32,
    vy: i32,
    gravity_mask: u16,
    gravity_base: i32,
    base: u16,
    mapping: u16,
    life: i32,
}
impl Debris {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let seq = crate::rom::sequence(rom, 0x36878)?;
        Ok(Self {
            x_mask: word(rom, 0x18fa2)?,
            x_bias: word_i(rom, 0x18fa6)?,
            vy: long(rom, 0x18fc0)?,
            gravity_mask: word(rom, 0x18fd4)?,
            gravity_base: word_i(rom, 0x18fd8)?,
            base: word(rom, 0x18fe6)?,
            mapping: *seq.first().ok_or("empty Data_036878")?,
            life: seq.len() as i32,
        })
    }
    fn spawn(&self, cx: &mut Ctx, x: i32, y: i32) {
        let dx = i32::from(cx.random() & self.x_mask) - self.x_bias;
        let gravity = i32::from(cx.random() & self.gravity_mask) + self.gravity_base;
        let mut piece = Effect::new(EffectKind::Pieces, x + dx, y, self.mapping);
        piece.base = self.base;
        piece.counter = self.life;
        piece.motion = Motion {
            vy: self.vy,
            gravity,
            ..Motion::default()
        };
        cx.effect(piece);
    }
}
impl Collapse {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut columns = Vec::with_capacity(16);
        for i in 0..16 {
            columns.push(signed(rom, 0x19626 + i * 2)?);
        }
        let subq = word(rom, 0x19788)?;
        Ok(Self {
            remaining: byte(rom, 0x19673)? as i8,
            lock_ticks: word_i(rom, 0x19682)?,
            rain_ticks: word_i(rom, 0x196b0)?,
            columns,
            column_mask: word(rom, 0x196bc)?,
            drop_y: signed(rom, 0x196d6)?,
            wait_ticks: word_i(rom, 0x1971a)?,
            stage: (word_i(rom, 0x19738)?, word_i(rom, 0x1973e)?),
            // `subq.w #3,$2C(a0)`: the count sits in opcode bits 9..11.
            sink_step: match (subq >> 9) & 7 {
                0 => 8,
                n => i32::from(n),
            },
            sink_period: word(rom, 0x19784)?,
            sink_floor: word_i(rom, 0x19792)?,
            fall_lines: (word_i(rom, 0x197ca)?, word_i(rom, 0x197de)?),
            floor: (word_i(rom, 0x19800)?, word_i(rom, 0x19804)?),
            marker: (word(rom, 0x1979e)?, word(rom, 0x197a2)?),
            music: (
                Table::Music.command_at(rom, 0x19761)?,
                Table::Music.command_at(rom, 0x197af)?,
            ),
            debris: Debris::load(rom)?,
            tank: Tank::load(rom)?,
        })
    }
    /// The counters as the capsules read them from their parent.
    fn mirror(cx: &Ctx, e: &mut Enemy) {
        e.vars[COLLAPSE_REMAINING] = i32::from(cx.mutants.wanted);
        e.vars[COLLAPSE_ALIVE] = i32::from(cx.mutants.alive);
    }
    fn step(&self, cx: &mut Ctx, e: &mut Enemy) {
        loop {
            match e.routine {
                0 => {
                    // $19662
                    e.timer = 0;
                    e.motion.stop();
                    e.mapping = 0;
                    cx.mutants.wanted = self.remaining;
                    e.vars[COLLAPSE_TIMER] = self.lock_ticks;
                    e.routine = 4;
                }
                4 => {
                    // $1968A: then Func_0198C0 walls the screen in (not modelled).
                    e.vars[COLLAPSE_TIMER] -= 1;
                    if e.vars[COLLAPSE_TIMER] >= 0 {
                        return;
                    }
                    e.vars[COLLAPSE_TIMER] = 0;
                    e.routine = 8;
                    return;
                }
                8 => {
                    // $196A8: a capsule every 60 ticks.
                    e.vars[COLLAPSE_TIMER] -= 1;
                    if e.vars[COLLAPSE_TIMER] < 0 {
                        e.vars[COLLAPSE_TIMER] = self.rain_ticks;
                        let index = usize::from((cx.random() & self.column_mask) >> 1);
                        let x = cx.camera.0 + self.columns[index % self.columns.len()];
                        let y = cx.camera.1 + self.drop_y;
                        if cx.mutants.wanted >= 0 {
                            self.tank.drop_capsule(cx, x, y);
                        }
                    }
                    if cx.mutants.wanted < 0 {
                        e.routine = 0x0c;
                    }
                    return;
                }
                0x0c => {
                    // $196F8: wait until no $FC is left. The ROM scans
                    // list 5 for type $FC; $FF3D05 counts the same objects.
                    if cx.mutants.alive > 0 {
                        return;
                    }
                    e.vars[COLLAPSE_TIMER] = self.wait_ticks;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $19722: event 6 locks attacks and mutes player SFX;
                    // movement and gravity remain available to reach the hole.
                    cx.player_event(PlayerEvent::Freeze);
                    e.vars[COLLAPSE_TIMER] -= 1;
                    if e.vars[COLLAPSE_TIMER] >= 0 {
                        return;
                    }
                    e.timer = 0;
                    e.x = self.stage.0;
                    e.y = self.stage.1;
                    // Func_01983C: the column scroll word ($FFF480) is plane
                    // A's vertical scroll, the camera Y.
                    e.vars[SINK] = cx.camera.1;
                    e.vars[SINK_PHASE] = 0;
                    e.vars[SINK_COLUMNS].fill(cx.camera.1);
                    Self::scroll(cx, e);
                    cx.play(self.music.0);
                    e.routine = 0x14;
                    return;
                }
                0x14 => {
                    // $19768: debris; the floor sinks 3 every 16 ticks.
                    cx.player_event(PlayerEvent::Freeze);
                    if e.timer & 1 == 0 {
                        self.debris.spawn(cx, e.x, e.y);
                    }
                    // $1988E: four two-cell columns (13..17). Each new
                    // cosine sample pushes the previous three rightward.
                    e.vars[SINK_PHASE] = (e.vars[SINK_PHASE] + 16) & 0xffff;
                    let (cos, _) = cx.shared.sine.polar(e.vars[SINK_PHASE] as u16);
                    e.vars.copy_within(5..8, 6);
                    e.vars[5] = e.vars[SINK] + (cos.wrapping_shl(2) >> 16);
                    Self::scroll(cx, e);
                    if e.timer & self.sink_period != 0 {
                        return;
                    }
                    e.vars[SINK] -= self.sink_step;
                    if e.vars[SINK] >= self.sink_floor {
                        return;
                    }
                    // Func_0197FE writes metatile 0 over the sunk floor
                    // (Func_015074 at columns +0..3, rows +0..1), then
                    // Func_03775E marks the hole.
                    for col in 0..4 {
                        for row in 0..2 {
                            cx.edit_terrain(self.floor.0 + col * 16, self.floor.1 + row * 16, 0);
                        }
                    }
                    cx.signal(Signal::Marker(self.marker.0, self.marker.1));
                    cx.play(self.music.1);
                    e.routine = 0x18;
                    return;
                }
                0x18 => {
                    // $197BE
                    cx.player_event(PlayerEvent::Freeze);
                    if cx.player.1 < self.fall_lines.0 {
                        return;
                    }
                    e.routine = 0x1c;
                }
                _ => {
                    // $197D2: Game_Requests bit 1.
                    cx.player_event(PlayerEvent::Freeze);
                    if cx.player.1 < self.fall_lines.1 {
                        return;
                    }
                    cx.next_substage();
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
    fn scroll(cx: &mut Ctx, e: &Enemy) {
        let mut columns = [cx.camera.1; crate::stage::COLUMNS];
        columns[13..17].copy_from_slice(&e.vars[SINK_COLUMNS]);
        cx.signal(Signal::PlaneAVScroll(columns));
    }
}
impl Behavior for Collapse {
    fn label(&self, _kind: u16) -> String {
        "EnemyC0 $19656".into()
    }
    fn hit(&self, _e: &Enemy, _kind: u16) -> HitResponse {
        HitResponse::Pass
    }
    fn integrates(&self) -> bool {
        false
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        self.step(cx, e);
        Self::mirror(cx, e);
    }
}
