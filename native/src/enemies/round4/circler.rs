//! Type `$38`, `Enemy38` (`$166D6`): the circling star thrower.
use super::{count_down, round_sources};
use crate::{
    combat::{Blast, Enemy, EnemyState, Motion, Projectile, Spawn},
    enemies::{Behavior, Ctx, Offsets, aim, stop},
    game::Terrain,
    rom::{Result, byte, long, sequence, word, word_i},
    sound_events::Table,
};
use std::collections::BTreeSet;

/// `$2A` animation index, `$2C`/`$2E` post, `$32` angle, `$30` count.
const C_ANIM: usize = 0;
const C_HOME_X: usize = 1;
const C_HOME_Y: usize = 2;
const C_ANGLE: usize = 3;
const C_COUNT: usize = 4;
const C_FLAGS: usize = 5;

/// `$38`: circles its post at 2.1 px/tick, 256 ticks clockwise then 256
/// back, from angle $300. On the upper half of the circle (angle bit 9)
/// it cycles `$157..$159`; otherwise every 128 ticks it holds `$15B` for
/// seven and throws a star aimed at three times the `Func_009562`
/// direction. One hit kills: the shared arc, or in round 6 a fall to
/// camera + $A0 and the `$36DA4` blast. Spawn flag bit 0 (unused by the
/// stage lists) first waits hidden ($14F) and drops ($150).
pub(crate) struct Circler {
    hidden: u16,
    drop: u16,
    walk: Vec<u16>,
    throw_pose: u16,
    dead: u16,
    range: i32,
    start: u16,
    lap: i32,
    star: Offsets,
    star_sound: u8,
    launch: (i32, i32, i32),
    low_blast: Blast,
    round6: BTreeSet<usize>,
}
impl Circler {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            hidden: word(rom, 0x16726)?,
            drop: word(rom, 0x1674c)?,
            walk: sequence(rom, 0x166a0)?,
            throw_pose: word(rom, 0x16874)?,
            dead: word(rom, 0x166b0)?,
            range: word_i(rom, 0x16738)?,
            start: word(rom, 0x16788)?,
            lap: word_i(rom, 0x1678e)?,
            star: Offsets::read(rom, 0x1d1ce)?,
            star_sound: byte(rom, 0x168cb)?,
            // $14D5C: move.l #$FFFD8000,$1C / move.w #$2200,$16 / $14000.
            launch: (
                long(rom, 0x14d5a)?,
                word_i(rom, 0x14d62)?,
                long(rom, 0x14d68)?,
            ),
            // $36DA4: moveq #2,d4 / #0,d7 / #1,d6.
            low_blast: Blast {
                y_shift: u32::from(byte(rom, 0x36da5)?),
                x_shift: u32::from(byte(rom, 0x36da7)?),
                sound: Table::Effect.command_at(rom, 0x36dab)?,
            },
            round6: round_sources(rom, 6, 0x38)?,
        })
    }
    /// `Func_016820`.
    fn animate(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.mapping = self.walk[0];
        if e.vars[C_ANGLE] & 0x200 != 0 {
            let index = ((e.vars[C_ANIM] >> 3) & 3) as usize;
            if let Some(&mapping) = self.walk.get(index) {
                e.mapping = mapping;
                e.vars[C_ANIM] += 1;
            }
            return;
        }
        let t = e.timer & 0x7f;
        if t >= 7 {
            return;
        }
        e.mapping = self.throw_pose;
        if t == 6 {
            // $16880: list-4 ID 4, aimed.
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
                cx.visible_sound(e.x, e.y, self.star_sound);
            }
        }
    }
    fn circle(&self, cx: &mut Ctx, e: &mut Enemy, step: i32) -> bool {
        self.animate(cx, e);
        e.vars[C_ANGLE] = (e.vars[C_ANGLE] + step) & 0xffff;
        let (cos, sin) = cx.shared.sine.polar(e.vars[C_ANGLE] as u16);
        e.motion.vx = cos << 1;
        e.motion.vy = sin << 1;
        count_down(&mut e.vars[C_COUNT])
    }
    fn restart(&self, e: &mut Enemy) {
        e.vars[C_ANGLE] = i32::from(self.start);
        e.vars[C_COUNT] = self.lap;
        e.x = e.vars[C_HOME_X];
        e.y = e.vars[C_HOME_Y];
    }
}
impl Behavior for Circler {
    fn label(&self, _kind: u16) -> String {
        "Enemy38 $166D6".into()
    }
    fn activate(&self, _cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.state = EnemyState::Waiting;
        e.vars[C_FLAGS] = i32::from(spawn.flags | 0x8000);
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $166E8
                    stop(e);
                    e.death_routine = 0x18;
                    e.vars[C_HOME_X] = e.x;
                    e.vars[C_HOME_Y] = e.y;
                    if e.vars[C_FLAGS] & 1 == 0 {
                        e.routine = 0x0c;
                        e.vars[C_ANIM] = 0;
                        return;
                    }
                    e.mapping = self.hidden;
                    e.routine = 4;
                }
                4 => {
                    // $1672E: drop once the player is near.
                    if cx.player_dx(e.x) >= self.range {
                        return;
                    }
                    e.motion.gravity = 0x3000;
                    e.motion.vy = -2;
                    e.mapping = self.drop;
                    e.routine = 8;
                }
                8 => {
                    // $16754
                    if ((e.motion.vy >> 16) < 0) != (e.motion.gravity < 0) {
                        return;
                    }
                    stop(e);
                    e.vars[C_ANIM] = 0;
                    e.vars[C_HOME_X] = e.x;
                    e.vars[C_HOME_Y] = e.y;
                    e.routine = 0x0c;
                    return;
                }
                0x0c => {
                    // $1677C
                    stop(e);
                    self.restart(e);
                    e.routine = 0x10;
                }
                0x10 => {
                    // $167A2
                    if self.circle(cx, e, 4) {
                        return;
                    }
                    self.restart(e);
                    e.routine = 0x14;
                    return;
                }
                0x14 => {
                    // $167E6
                    if !self.circle(cx, e, -4) {
                        e.routine = 0x0c;
                    }
                    return;
                }
                0x18 => {
                    // $16814 + $14D46: the round-6 variant falls to the floor line.
                    e.mapping = self.dead;
                    let (vy, gravity, vx) = self.launch;
                    e.motion.vy = vy;
                    e.motion.gravity = gravity;
                    e.motion.vx = if cx.player_left(e.x) { vx } else { -vx };
                    e.routine = if self.round6.contains(&spawn.source) {
                        0x20
                    } else {
                        0x1c
                    };
                    return;
                }
                0x1c => {
                    // $14D72
                    if cx.landed_blast(e, terrain) {
                        Ctx::free(e);
                    }
                    return;
                }
                _ => {
                    // $14DA0
                    if cx.camera.1 + 0xa0 >= e.y {
                        return;
                    }
                    cx.explode(e.x, e.y, self.low_blast);
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
}
