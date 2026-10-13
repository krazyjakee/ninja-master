//! Type `$A8`, `EnemyA8` (`$19064`): the block-edge crawler.
use crate::{
    combat::{Enemy, EnemyState, Spawn},
    enemies::{Behavior, Ctx, HitResponse, set_whole},
    game::Terrain,
    rom::{Result, signed_byte, word, word_i},
};

/// `$30`: tile step (0, 4, 8); `$2C`: blink ticks; blink state.
const STEP: usize = 0;
const BLINK: usize = 1;
const HIDDEN: usize = 2;

/// `$A8`: a spark that crawls round the edges of solid blocks at 1 px/tick,
/// clockwise or counter-clockwise by spawn flag bit 0, animating by tile
/// offsets (`obj_tile` + 0/4/8). Contact shocks the player (damage code
/// `$2F` at `$19072`, the shocked state 47). Killed, it falls blinking for
/// 16 ticks.
pub(crate) struct Crawler {
    mapping: u16,
    tile_step: i32,
    tile_cycle: i32,
    death: u16,
    death_gravity: i32,
    blink_ticks: i32,
    /// Moving horizontally (routines 1/2): probes, then the corner checks.
    horizontal: [(i32, i32); 6],
    /// Moving vertically (routines 3/4).
    vertical: [(i32, i32); 6],
}
impl Crawler {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let probe = |p: usize| -> Result<(i32, i32)> {
            Ok((signed_byte(rom, p)?, signed_byte(rom, p + 2)?))
        };
        Ok(Self {
            mapping: word(rom, 0x190d6)?,
            tile_step: 4,
            tile_cycle: word_i(rom, 0x19082)?,
            death: word(rom, 0x1910c)?,
            death_gravity: word_i(rom, 0x19242)?,
            blink_ticks: word_i(rom, 0x19248)?,
            horizontal: [
                probe(0x19147)?,
                probe(0x19151)?,
                probe(0x1915b)?,
                probe(0x19165)?,
                probe(0x19171)?,
                probe(0x1918d)?,
            ],
            vertical: [
                probe(0x191cf)?,
                probe(0x191d9)?,
                probe(0x191e3)?,
                probe(0x191ed)?,
                probe(0x191f9)?,
                probe(0x19215)?,
            ],
        })
    }
    /// `Func_019268`: solid at (dx, dy) mirrored by the velocity signs.
    fn probe(e: &Enemy, terrain: &dyn Terrain, (dx, dy): (i32, i32)) -> bool {
        let dx = if (e.motion.vx >> 16) < 0 { -dx } else { dx };
        let dy = if (e.motion.vy >> 16) < 0 { -dy } else { dy };
        terrain.solid_at(e.x + dx, e.y + dy)
    }
    fn go(e: &mut Enemy, routine: u16) {
        e.routine = routine;
        let (vx, vy) = match routine {
            1 => (1, 0),
            2 => (-1, 0),
            3 => (0, 1),
            _ => (0, -1),
        };
        e.motion.vx = set_whole(e.motion.vx, vx);
        e.motion.vy = set_whole(e.motion.vy, vy);
    }
}
impl Behavior for Crawler {
    fn label(&self, _kind: u16) -> String {
        "EnemyA8 $19064".into()
    }
    fn activate(&self, _cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.state = EnemyState::Waiting;
        // $19114: routine 1 (right) or 2 (left) from the spawn flags.
        e.vars[MODE_A8] = i32::from(spawn.flags & 1);
    }
    fn shocks(&self, _kind: u16) -> bool {
        true
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        if (1..=4).contains(&e.routine) {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        // $1907A: obj_tile = base + 0/4/8.
        let step = e.vars[STEP] + self.tile_step;
        e.vars[STEP] = if step == self.tile_cycle { 0 } else { step };
        if let Some(&base) = cx.bases.get(&0xa8) {
            e.tile = Some(base.wrapping_add(e.vars[STEP] as u16));
        }
        let h = &self.horizontal;
        let v = &self.vertical;
        match e.routine {
            0 => {
                // $190BC: settle onto the 8-pixel grid.
                e.mapping = self.mapping;
                e.motion.stop();
                e.vars[STEP] = 0;
                e.y &= !7;
                if terrain.solid_at(e.x, e.y - 8) {
                    e.y -= 1;
                }
                e.y += 8;
                e.death_routine = self.death;
                e.routine = e.vars[MODE_A8] as u16 + 1;
            }
            1 | 2 => {
                // $19120 / $1912E: along a floor or ceiling.
                let routine = e.routine;
                Self::go(e, routine);
                if Self::probe(e, terrain, h[0]) {
                    if Self::probe(e, terrain, h[4]) {
                        Self::go(e, 3);
                    }
                } else if Self::probe(e, terrain, h[1]) {
                    if Self::probe(e, terrain, h[5]) {
                        Self::go(e, 4);
                    }
                } else if Self::probe(e, terrain, h[2]) {
                    Self::go(e, 4);
                } else if Self::probe(e, terrain, h[3]) {
                    Self::go(e, 3);
                }
            }
            3 | 4 => {
                // $191A8 / $191B6: along a wall.
                let routine = e.routine;
                Self::go(e, routine);
                if Self::probe(e, terrain, v[0]) {
                    if Self::probe(e, terrain, v[4]) {
                        Self::go(e, 1);
                    }
                } else if Self::probe(e, terrain, v[1]) {
                    if Self::probe(e, terrain, v[5]) {
                        Self::go(e, 2);
                    }
                } else if Self::probe(e, terrain, v[2]) {
                    Self::go(e, 2);
                } else if Self::probe(e, terrain, v[3]) {
                    Self::go(e, 1);
                }
            }
            5 => {
                // $19230: score (central); fall blinking.
                e.motion.stop();
                e.motion.gravity = self.death_gravity;
                e.vars[BLINK] = self.blink_ticks;
                e.vars[HIDDEN] = 0;
                e.routine = 6;
                self.blink(e);
            }
            _ => self.blink(e),
        }
    }
}
/// `$A8` scratch: 0 right, 1 left.
const MODE_A8: usize = 3;
impl Crawler {
    /// `$19256`: toggle the hidden bit; freed after 16 ticks.
    fn blink(&self, e: &mut Enemy) {
        e.vars[HIDDEN] ^= 1;
        e.mapping = if e.vars[HIDDEN] != 0 { 0 } else { self.mapping };
        e.vars[BLINK] -= 1;
        if e.vars[BLINK] == 0 {
            Ctx::free(e);
        }
    }
}
