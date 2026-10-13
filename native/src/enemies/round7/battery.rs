//! Type `$F0`, `EnemyF0` (`$1B4E6`), and its shot (projectile `$37`, `$1B646`).
use super::{Shot, runs};
use crate::{
    combat::{Enemy, Motion, Projectile, Spawn},
    enemies::{Behavior, Ctx, HitResponse, shot_damage},
    game::Terrain,
    rom::{Result, byte, sequence, signed, signed_byte, word, word_i},
};

/// `$32`: countdown; `$30`: shots left in the volley; `$31`: volleys left;
/// `$2C`: volley gap; `$2E`: rest; hurt countdown.
const F0_COUNT: usize = 0;
const F0_SHOT: usize = 1;
const F0_VOLLEYS: usize = 2;
const F0_GAP: usize = 3;
const F0_REST: usize = 4;
const F0_GUARD: usize = 5;

/// `$F0`: a wall battery drawn from stage tiles (`$E000`). Fires volleys of
/// four shots (`$1A6`, -4 px/tick on both axes, damage 2, effect `$3B`)
/// 11 ticks apart from the four `Data_01B5E0` muzzles; `(flags & $F) + 1`
/// volleys `((flags & $F0) + $10) * 2 + 1` ticks apart, then rests
/// `((flags & $F00) + $100) / 4 + 1`. Each shot leaves a sparkle trail. A hit
/// makes it invulnerable for 17 ticks; three points destroy it in the wide
/// `$36D9A` explosion 40 px right and 48 px below its origin.
pub(crate) struct Battery {
    tile: u16,
    frame: u16,
    reload: i32,
    muzzles: [(i32, i32); 4],
    shot: Shot,
    speed: (i32, i32),
    guard_ticks: i32,
    blast: (i32, i32),
    trail: Vec<(u16, u16)>,
}
impl Battery {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut muzzles = [(0, 0); 4];
        for (i, muzzle) in muzzles.iter_mut().enumerate() {
            *muzzle = (
                signed_byte(rom, 0x1b5e0 + i * 2)?,
                signed_byte(rom, 0x1b5e1 + i * 2)?,
            );
        }
        Ok(Self {
            tile: word(rom, 0x1b506)?,
            frame: word(rom, 0x1b50c)?,
            reload: word_i(rom, 0x1b56a)?,
            muzzles,
            shot: Shot {
                mapping: word(rom, 0x1b638)?,
                damage: shot_damage(rom, usize::from(byte(rom, 0x1b5e9)?))?,
                sound: byte(rom, 0x1b63d)?,
            },
            speed: (signed(rom, 0x1b626)?, signed(rom, 0x1b62c)?),
            guard_ticks: word_i(rom, 0x1b388)?,
            blast: (word_i(rom, 0x1b5ce)?, word_i(rom, 0x1b5d2)?),
            // Data_01B10A read as `$1B112` does: entry `$2A >> 1`.
            trail: runs(&sequence(rom, 0x1b10a)?, 2),
        })
    }
    /// `Func_01B5E8`: projectile `$37` from muzzle `index & 3`.
    fn fire(&self, cx: &mut Ctx, e: &Enemy, index: i32) {
        let (dx, dy) = self.muzzles[(index & 3) as usize];
        let (x, y) = (e.x + dx, e.y + dy);
        let mut bolt = Projectile::new(x, y, 0, true, self.shot.mapping, self.shot.damage)
            .with_art(self.tile, self.speed.0 < 0)
            .with_motion(Motion {
                vx: self.speed.0 << 16,
                vy: self.speed.1 << 16,
                ..Motion::default()
            });
        bolt.owner = Some(cx.current);
        bolt.sparkle_trail = Some(self.trail.clone());
        if cx.shoot(bolt) {
            cx.visible_sound(e.x, e.y, self.shot.sound);
        }
    }
}
impl Behavior for Battery {
    fn label(&self, _kind: u16) -> String {
        "EnemyF0 $1B4E6".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.frame
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        if (4..0x18).contains(&e.routine) {
            HitResponse::Accept
        } else {
            HitResponse::Pass
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, _terrain: &dyn Terrain) {
        if e.offscreen && e.routine != 0 {
            return;
        }
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        let flags = i32::from(spawn.flags | 0x8000);
        loop {
            match e.routine {
                0 => {
                    // $1B4FE
                    e.motion.stop();
                    e.timer = 0;
                    e.tile = Some(self.tile);
                    e.mapping = self.frame;
                    e.death_routine = 0x20;
                    e.hurt_routine = 0x18;
                    e.vars[F0_GAP] = ((flags & 0xf0) + 0x10) << 1;
                    e.vars[F0_REST] = ((flags & 0xf00) + 0x100) >> 2;
                    e.routine = 4;
                    return;
                }
                4 => {
                    // $1B546
                    e.vars[F0_VOLLEYS] = flags & 0xf;
                    e.vars[F0_SHOT] = 3;
                    e.routine = 8;
                }
                8 => {
                    // $1B55C
                    self.fire(cx, e, e.vars[F0_SHOT]);
                    e.vars[F0_COUNT] = self.reload;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $1B572
                    e.vars[F0_COUNT] -= 1;
                    if e.vars[F0_COUNT] >= 0 {
                        return;
                    }
                    e.vars[F0_SHOT] -= 1;
                    if e.vars[F0_SHOT] >= 0 {
                        e.routine = 8;
                        continue;
                    }
                    e.vars[F0_COUNT] = e.vars[F0_GAP];
                    e.routine = 0x10;
                    return;
                }
                0x10 => {
                    // $1B58A
                    e.vars[F0_COUNT] -= 1;
                    if e.vars[F0_COUNT] >= 0 {
                        return;
                    }
                    e.vars[F0_VOLLEYS] -= 1;
                    if e.vars[F0_VOLLEYS] < 0 {
                        e.vars[F0_COUNT] = e.vars[F0_REST];
                        e.routine = 0x14;
                    } else {
                        e.vars[F0_SHOT] = 3;
                        e.routine = 8;
                        continue;
                    }
                }
                0x14 => {
                    // $1B5A8
                    e.vars[F0_COUNT] -= 1;
                    if e.vars[F0_COUNT] >= 0 {
                        return;
                    }
                    e.routine = 4;
                    return;
                }
                0x18 => {
                    // $1B386 (shared with $EC)
                    e.vars[F0_GUARD] = self.guard_ticks;
                    e.routine = 0x1c;
                }
                0x1c => {
                    e.vars[F0_GUARD] -= 1;
                    if e.vars[F0_GUARD] >= 0 {
                        return;
                    }
                    e.routine = e.saved_routine;
                    return;
                }
                _ => {
                    // $1B5B6: score (central), wide blast, freed.
                    let blast = cx.shared.wide_blast;
                    cx.explode(e.x + self.blast.0, e.y + self.blast.1, blast);
                    Ctx::free(e);
                    return;
                }
            }
        }
    }
}
