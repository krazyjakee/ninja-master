//! Type `$FC`, `EnemyFC` (`$1B686`): the hopping mutant released by `$A4`.
use crate::{
    combat::{Enemy, Spawn},
    enemies::{Behavior, Ctx, set_fraction},
    game::Terrain,
    rom::{Result, long, signed, word, word_i},
};

/// `$2A`: wait; `$30`: hops left.
const HOP_WAIT: usize = 0;
const HOPS: usize = 1;
/// Native: a dead `$FC` this far below the view's top never lands (it was
/// killed over a gap); it counts as landed. The ROM's arena floor always
/// catches it, and `$196F8` would otherwise wait for it forever.
const FALL_OUT: i32 = 320;

/// `$FC`: the mutant a tank or capsule releases. It jumps out (4 px/tick
/// toward the player if the player is above), lands crouched for 8 ticks,
/// makes two hops toward the player (-3 px/tick up, 2.5 across, bouncing
/// off walls and ceilings), sits 80 ticks (carried by conveyors), then a
/// 4.5 px/tick leap starts the next round. Left behind 40 px past the
/// camera's left edge it is gone. Killed, it tumbles away and bursts on
/// landing, one kill off `$FF3D06`. Gone or burst, it leaves `$FF3D05`.
pub(crate) struct Mutant {
    death: u16,
    small_vy: i32,
    big_vy: i32,
    leap_vy: i32,
    speed: i32,
    gravity: i32,
    hops: i32,
    air: u16,
    crouch: u16,
    dead: u16,
    probe: i32,
    land_wait: i32,
    margin: i32,
    hop_vy: i32,
    sit: i32,
    death_vy: i32,
    death_gravity: i32,
    death_vx: i32,
    wall: (i32, i32),
    ceiling: i32,
}
impl Mutant {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            death: word(rom, 0x1b6d6)?,
            small_vy: long(rom, 0x1b6e0)?,
            big_vy: long(rom, 0x1b6ee)?,
            leap_vy: long(rom, 0x1b6f8)?,
            speed: long(rom, 0x1b700)?,
            gravity: word_i(rom, 0x1b70c)?,
            hops: word_i(rom, 0x1b712)?,
            // Data_01B65E, Data_01B65C, Data_01B660
            air: word(rom, 0x1b65e)?,
            crouch: word(rom, 0x1b65c)?,
            dead: word(rom, 0x1b660)?,
            probe: word_i(rom, 0x1b734)?,
            land_wait: word_i(rom, 0x1b74e)?,
            margin: signed(rom, 0x1b762)?,
            hop_vy: long(rom, 0x1b78c)?,
            sit: word_i(rom, 0x1b7a4)?,
            death_vy: long(rom, 0x1b836)?,
            death_gravity: word_i(rom, 0x1b83e)?,
            death_vx: long(rom, 0x1b844)?,
            // `subq.w #8,d1` (the probe height) is an opcode immediate.
            wall: (word_i(rom, 0x1b910)?, 8),
            ceiling: word_i(rom, 0x1b956)?,
        })
    }
    /// `Func_01B890`: 2.5 px/tick toward the player.
    fn toward(&self, cx: &Ctx, e: &mut Enemy) {
        e.motion.vx = if cx.player_left(e.x) {
            -self.speed
        } else {
            self.speed
        };
    }
    /// `Func_01B904`: bounce off walls, stop rising under a ceiling.
    fn walls(&self, e: &mut Enemy, terrain: &dyn Terrain) {
        let ahead = if (e.motion.vx >> 16) < 0 {
            e.x - self.wall.0
        } else {
            e.x + self.wall.0
        };
        if terrain.solid_at(ahead, e.y - self.wall.1) {
            e.motion.vx = -e.motion.vx;
        }
        if (e.motion.vy >> 16) < 0 && terrain.solid_at(e.x, e.y - self.ceiling) {
            e.motion.vy = 0;
        }
    }
    /// `$1B884`: gone past the left edge (`$FF3D05`-1).
    fn left_behind(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        if cx.camera.0 + self.margin < e.x {
            return false;
        }
        cx.mutants.gone();
        Ctx::free(e);
        true
    }
}
impl Behavior for Mutant {
    fn label(&self, _kind: u16) -> String {
        "EnemyFC $1B686".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        // Func_01B69E: flipped while the player is to the right.
        e.facing_left = !cx.player_left(e.x);
        cx.process_hit(e);
        match e.routine {
            0 => {
                // $1B6BA: jump out; at the player if the player is above.
                e.timer = 0;
                e.motion.stop();
                e.death_routine = self.death;
                e.motion.vy = self.small_vy;
                if cx.player.1 < e.y {
                    e.motion.vy = self.big_vy;
                    self.toward(cx, e);
                }
                e.motion.gravity = self.gravity;
                e.vars[HOPS] = self.hops;
                e.routine = 8;
            }
            4 => {
                // $1B6F6: the long leap.
                e.motion.vy = self.leap_vy;
                self.toward(cx, e);
                e.motion.gravity = self.gravity;
                e.vars[HOPS] = self.hops;
                e.routine = 8;
            }
            8 => {
                // $1B71C: airborne until the floor 16 px below.
                e.mapping = self.air;
                self.walls(e, terrain);
                if !terrain.solid_at(e.x, e.y + self.probe) {
                    return;
                }
                e.y &= !15;
                e.mapping = self.crouch;
                e.vars[HOP_WAIT] = self.land_wait;
                e.routine = 0x0c;
                e.timer = 0;
                e.motion.stop();
            }
            0x0c => {
                // $1B75C: crouch, then hop toward the player.
                if self.left_behind(cx, e) {
                    return;
                }
                e.vars[HOP_WAIT] -= 1;
                if e.vars[HOP_WAIT] >= 0 {
                    return;
                }
                e.routine = 8;
                e.motion.gravity = self.gravity;
                self.toward(cx, e);
                e.motion.vy = set_fraction(self.hop_vy, cx.random());
                e.vars[HOPS] -= 1;
                if e.vars[HOPS] >= 0 {
                    return;
                }
                e.vars[HOP_WAIT] = self.sit;
                e.routine = 0x10;
                e.timer = 0;
                e.motion.stop();
            }
            0x10 => {
                // $1B7B4: sit, carried by conveyors.
                if self.left_behind(cx, e) {
                    return;
                }
                e.mapping = self.crouch;
                match terrain.material_at(e.x, e.y + self.probe).0 & 15 {
                    3 => e.x += 1,
                    4 => e.x -= 1,
                    _ => {}
                }
                e.vars[HOP_WAIT] -= 1;
                if e.vars[HOP_WAIT] < 0 {
                    e.routine = 4;
                }
            }
            0x14 | 0x18 => {}
            r if r == self.death => {
                // $1B820: score (central); tumble away from the player.
                // `$1B826`: keep the brain art and flip it upside down.
                e.tile = Some(e.tile.unwrap_or_else(|| cx.tile_base(0xfc)) ^ 0x1000);
                e.mapping = self.dead;
                e.motion.vy = self.death_vy;
                e.motion.gravity = self.death_gravity;
                e.motion.vx = if cx.player_left(e.x) {
                    self.death_vx
                } else {
                    -self.death_vx
                };
                e.routine = self.death + 4;
            }
            _ => {
                // $1B85E: burst on landing; one kill ($FF3D06) and one
                // fewer $FC ($FF3D05).
                if !terrain.solid_at(e.x, e.y + self.probe) {
                    if e.y <= cx.camera.1 + FALL_OUT {
                        return;
                    }
                } else {
                    cx.small_blast(e.x, e.y);
                }
                cx.mutants.killed();
                Ctx::free(e);
            }
        }
    }
}
