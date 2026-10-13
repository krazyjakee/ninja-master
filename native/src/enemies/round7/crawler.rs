//! Type `$C4`, `EnemyC4` (`$19918`): the wall climber.
use crate::{
    combat::{Enemy, Motion, Spawn, advance},
    enemies::{Behavior, Ctx, set_whole},
    game::Terrain,
    rom::{Result, signed, signed_byte, word, word_i},
};

/// `$2C`: climb countdown; `$2E`: climbing direction saved over a leap;
/// death blink counter and the pose it blinks.
const C4_CLIMB: usize = 0;
const C4_DIR: usize = 1;
const C4_POSE: usize = 2;

/// `$C4` (no spawn record): clings to a wall (flip `$800` when the wall is
/// on its right) and crawls along it at 1 px/tick (`$1FA`/`$1FB`), turning
/// where the wall ends or something blocks it. After 64..191 ticks it leaps
/// off at 5 px/tick (vy -6, gravity `$3400`), bumps its head on ceilings and
/// grips the opposite wall, 8-px aligned. Contact damage 1, one point kills
/// it: it drops blinking for 16 ticks.
pub(crate) struct Crawler {
    frame: u16,
    climb_speed: i32,
    climb_mask: u16,
    climb_min: i32,
    probe: i32,
    inset: i32,
    side: i32,
    leap: i32,
    leap_vy: i32,
    gravity: i32,
    head: i32,
    grip: i32,
    death_gravity: i32,
    blink: i32,
}
impl Crawler {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            frame: word(rom, 0x199bc)?,
            climb_speed: signed(rom, 0x1996c)?,
            climb_mask: word(rom, 0x19978)?,
            climb_min: word_i(rom, 0x1997c)?,
            probe: word_i(rom, 0x19988)?,
            inset: signed_byte(rom, 0x199a1)?,
            side: signed_byte(rom, 0x199e9)?,
            leap: signed_byte(rom, 0x19a09)?,
            leap_vy: signed(rom, 0x19a20)?,
            gravity: word_i(rom, 0x19a26)?,
            head: word_i(rom, 0x19a56)?,
            grip: signed_byte(rom, 0x19a8f)?,
            death_gravity: word_i(rom, 0x19ae4)?,
            blink: word_i(rom, 0x19aea)?,
        })
    }
}
impl Behavior for Crawler {
    fn label(&self, _kind: u16) -> String {
        "EnemyC4 $19918".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.frame
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        match e.routine {
            0 => {
                // $19942: face the wall, step 12 px off it.
                e.motion = Motion::default();
                e.death_routine = 3;
                e.motion.vy = self.climb_speed << 16;
                e.vars[C4_CLIMB] = i32::from(cx.random() & self.climb_mask) + self.climb_min;
                e.facing_left = terrain.solid_at(e.x + self.probe, e.y);
                e.x += if e.facing_left {
                    -self.inset
                } else {
                    self.inset
                };
                e.mapping = self.frame;
                e.routine = 1;
            }
            1 => {
                // $199BA: crawl; turn at an obstacle or where the wall ends.
                e.mapping = self.frame + ((e.timer >> 3) & 1);
                let dir = e.motion.vy >> 16;
                let ahead = terrain.solid_at(e.x, e.y + dir);
                let side = if e.facing_left { self.side } else { -self.side };
                if ahead || !terrain.solid_at(e.x + side, e.y) {
                    e.motion.vy = set_whole(e.motion.vy, -dir);
                }
                e.vars[C4_CLIMB] -= 1;
                if e.vars[C4_CLIMB] != 0 {
                    return;
                }
                e.motion.vx = set_whole(
                    e.motion.vx,
                    if e.facing_left { -self.leap } else { self.leap },
                );
                e.vars[C4_DIR] = e.motion.vy >> 16;
                e.motion.vy = set_whole(e.motion.vy, self.leap_vy);
                e.motion.gravity = self.gravity;
                e.vars[C4_CLIMB] = i32::from(cx.random() & self.climb_mask) + self.climb_min;
                e.routine = 2;
            }
            2 => {
                // $19A44: leap; grip the wall it reaches.
                let (vx, vy) = (e.motion.vx >> 16, e.motion.vy >> 16);
                if terrain.solid_at(e.x + vx, e.y + vy - self.head) {
                    let back = -e.motion.vy;
                    advance(&mut e.y, &mut e.motion.fy, back);
                }
                if terrain.solid_at(e.x + vx, e.y + (e.motion.vy >> 16)) {
                    let full =
                        (i64::from(e.x) << 16) + i64::from(e.motion.fx) + i64::from(e.motion.vx);
                    let offset = if vx < 0 { self.grip } else { -(self.grip - 8) };
                    let snapped = (full & !0x7ffff) + (i64::from(offset) << 16);
                    e.x = (snapped >> 16) as i32;
                    e.motion.fx = snapped as u16;
                    e.facing_left = vx > 0;
                    e.motion = Motion {
                        fx: e.motion.fx,
                        fy: e.motion.fy,
                        vy: e.vars[C4_DIR] << 16,
                        ..Motion::default()
                    };
                    e.routine = 1;
                }
            }
            3 => {
                // $19AD2: score (central), drop and blink.
                e.motion.vx = 0;
                e.motion.vy = 0;
                e.motion.gravity = self.death_gravity;
                e.vars[C4_CLIMB] = self.blink;
                e.vars[C4_POSE] = i32::from(e.mapping);
                e.routine = 4;
                self.blink_step(e);
            }
            _ => self.blink_step(e),
        }
    }
}
impl Crawler {
    /// `$19AF8`: obj_flags bit 11 (hidden) toggles every tick.
    fn blink_step(&self, e: &mut Enemy) {
        e.mapping = if e.mapping == 0 {
            e.vars[C4_POSE] as u16
        } else {
            0
        };
        e.vars[C4_CLIMB] -= 1;
        if e.vars[C4_CLIMB] == 0 {
            Ctx::free(e);
        }
    }
}
