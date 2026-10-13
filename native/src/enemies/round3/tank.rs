//! Type `$A4`, `EnemyA4` (`$18B0C`): glass tanks, and the `$29` capsules of
//! `$C0` (`Func_018DEC`).
use super::Glass;
use super::collapse::{COLLAPSE_ALIVE, COLLAPSE_REMAINING};
use crate::{
    combat::{Enemy, EnemyState, Spawn},
    enemies::{Behavior, Ctx, HitResponse, metatile_list},
    game::Terrain,
    rom::{Result, byte, pointer, signed, word, word_i},
};

/// `$2A` (spawn flags) bit 0: single tank; `$2B` bits 0..2: tanks broken;
/// `$30`/`$32`: last release point; capsule: landings (`$2E`), roll
/// direction (`$34`).
const MODE: usize = 0;
const BROKEN: usize = 1;
const POINT_X: usize = 2;
const POINT_Y: usize = 3;
const LANDINGS: usize = 4;
const ROLL: usize = 5;
/// `MODE` values.
const TANKS: i32 = 0;
const SINGLE: i32 = 1;
const CAPSULE: i32 = 2;
/// Native routines of a capsule (`Func_018DEC` routines 8 and `$C`, and
/// the break reached through a lethal hit).
const CAPSULE_FALL: u16 = 0x40;
const CAPSULE_ROLL: u16 = 0x44;
const CAPSULE_BREAK: u16 = 0x48;

/// `$A4`: three invisible glass tanks at +0/+48/+96 px (spawn flag bit 0:
/// one visible tank, `$1E6`, health 5, releasing a `$FC` every 64 ticks
/// while the player is near). A tank breaks when the player comes within
/// 64 x 80 px of it: shards, GLASS, and a `$FC` mutant at the tank. Natively
/// the same type also runs `$C0`'s falling capsules (list-1 `$29`):
/// `$1E6`/`$1E7` flicker, fall under gravity `$C00`, roll on conveyor
/// floors, and break on landing or on a weapon hit, releasing a `$FC` while
/// fewer than three are alive and `$C0` still wants kills.
pub(crate) struct Tank {
    /// `Data_018AE4`.
    points: [(i32, i32); 3],
    near: (i32, i32),
    tile_kind: u16,
    single_routine: u16,
    hurt: u16,
    death: u16,
    mapping: u16,
    period: u16,
    single_near: (i32, i32),
    capsule_mapping: u16,
    capsule_gravity: i32,
    capsule_tile: u16,
    roll_gravity: i32,
    mutant_limit: i32,
    glass: Glass,
    /// `Data_019014`: the broken-glass metatiles `$70..$75` that
    /// `Func_01903A` writes 2x3 above the release point + `$10`.
    broken: Vec<(u8, i32, i32)>,
    broken_dy: i32,
}
impl Tank {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let point = |i: usize| -> Result<(i32, i32)> {
            Ok((signed(rom, 0x18ae4 + i * 4)?, signed(rom, 0x18ae6 + i * 4)?))
        };
        Ok(Self {
            points: [point(0)?, point(1)?, point(2)?],
            near: (word_i(rom, 0x18c60)?, word_i(rom, 0x18c6e)?),
            tile_kind: word(rom, 0x18b24)?,
            single_routine: word(rom, 0x18b46)?,
            hurt: word(rom, 0x18b7a)?,
            death: word(rom, 0x18b80)?,
            mapping: word(rom, 0x18b8c)?,
            period: word(rom, 0x18ba0)?,
            single_near: (word_i(rom, 0x18bae)?, word_i(rom, 0x18bbc)?),
            capsule_mapping: word(rom, 0x18dca)?,
            capsule_gravity: word_i(rom, 0x18dbe)?,
            capsule_tile: word(rom, 0x18dc4)?,
            roll_gravity: word_i(rom, 0x18f1a)?,
            mutant_limit: i32::from(byte(rom, 0x18f35)?),
            glass: Glass::load(rom)?,
            broken: metatile_list(rom, pointer(rom, 0x19040)?)?,
            broken_dy: word_i(rom, 0x1904e)?,
        })
    }
    /// `Func_01B8A8`: a `$FC` at (x, y).
    fn release(cx: &mut Ctx, x: i32, y: i32) {
        cx.spawn(0xfc, x, y);
    }
    /// A `$C0` capsule falling from (x, y) (`Func_018DA0`).
    pub(crate) fn drop_capsule(&self, cx: &mut Ctx, x: i32, y: i32) {
        let (mapping, gravity, tile) = (
            self.capsule_mapping,
            self.capsule_gravity,
            self.capsule_tile,
        );
        if let Some(capsule) = cx.spawn(0xa4, x, y) {
            capsule.vars[MODE] = CAPSULE;
            capsule.vars[LANDINGS] = 0;
            capsule.mapping = mapping;
            capsule.tile = Some(tile);
            capsule.motion.gravity = gravity;
            capsule.hurt_routine = CAPSULE_FALL;
            capsule.death_routine = CAPSULE_BREAK;
            capsule.routine = CAPSULE_FALL;
        }
    }
    /// `$18F26`: GLASS and shards; a `$FC` while `$FF3D05` < 3 and
    /// `$FF3D06` >= 0 (read from the `$C0` parent); freed.
    fn break_capsule(&self, cx: &mut Ctx, e: &mut Enemy) {
        self.glass.smash(cx, e.x, e.y);
        let (alive, remaining) = cx.parent(e).map_or((0, 0), |parent| {
            let children = cx
                .children
                .iter()
                .filter(|child| child.kind == 0xfc)
                .count();
            (
                parent.vars[COLLAPSE_ALIVE] + children as i32,
                parent.vars[COLLAPSE_REMAINING],
            )
        });
        if alive < self.mutant_limit && remaining >= 0 {
            Self::release(cx, e.x, e.y);
        }
        Ctx::free(e);
    }
    fn capsule(&self, cx: &mut Ctx, e: &mut Enemy, terrain: &dyn Terrain) {
        if cx.process_hit(e).is_some() || e.routine == CAPSULE_BREAK {
            // Func_018F5E: any weapon contact breaks it.
            self.break_capsule(cx, e);
            return;
        }
        if cx.outside_view(e.x, e.y) {
            Ctx::free(e);
            return;
        }
        // Func_018E2A: flicker between the two capsule poses.
        e.mapping = self.capsule_mapping + (e.timer & 1);
        if e.routine == CAPSULE_ROLL {
            // $18EF0: roll one pixel per tick while the floor holds.
            e.x += e.vars[ROLL];
            if !terrain.solid_at(e.x, e.y) {
                e.motion.gravity = self.roll_gravity;
                e.routine = CAPSULE_FALL;
            }
            return;
        }
        // $18E76
        if !terrain.solid_at(e.x, e.y) {
            return;
        }
        let material = terrain.material_at(e.x, e.y).0;
        e.y &= !15;
        e.vars[LANDINGS] -= 1;
        e.motion.vy = 0;
        e.motion.gravity = 0;
        match material & 7 {
            3 => {
                e.vars[ROLL] = 1;
                e.routine = CAPSULE_ROLL;
            }
            4 => {
                e.vars[ROLL] = -1;
                e.routine = CAPSULE_ROLL;
            }
            _ => self.break_capsule(cx, e),
        }
    }
    /// `Func_018C2C`: break every intact tank the player is near.
    fn tanks(&self, cx: &mut Ctx, e: &mut Enemy) {
        for (i, &(px, py)) in self.points.iter().enumerate().rev() {
            if e.vars[BROKEN] & (1 << i) != 0 {
                continue;
            }
            let (x, y) = (e.x + px, e.y + py);
            if cx.player_dx(x) >= self.near.0 || cx.player_dy(y) >= self.near.1 {
                continue;
            }
            e.vars[BROKEN] |= 1 << i;
            e.vars[POINT_X] = x;
            e.vars[POINT_Y] = y;
            Self::release(cx, x, y);
            self.glass.shards(cx, x, y);
            self.break_glass(cx, x, y);
        }
    }
    /// `Func_01903A`: the broken-glass metatiles (plane A and layout,
    /// `Func_015054`) at the release point, and GLASS.
    fn break_glass(&self, cx: &mut Ctx, x: i32, y: i32) {
        cx.write_metatiles(x, y + self.broken_dy, &self.broken);
        cx.effect_sound(self.glass.sound);
    }
}
impl Behavior for Tank {
    fn label(&self, _kind: u16) -> String {
        "EnemyA4 $18B0C".into()
    }
    fn activate(&self, _cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.state = EnemyState::Waiting;
        // Object_SpawnFromRecord copies the spawn flags into $2A.
        e.vars[MODE] = if spawn.flags & 1 != 0 { SINGLE } else { TANKS };
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        match (e.vars[MODE], e.routine) {
            (CAPSULE, CAPSULE_FALL | CAPSULE_ROLL) | (SINGLE, 0x10) => HitResponse::Accept,
            _ => HitResponse::Pass,
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        if e.vars[MODE] == CAPSULE {
            self.capsule(cx, e, terrain);
            return;
        }
        loop {
            match e.routine {
                0 => {
                    // $18B18: invisible; one tank or three.
                    e.timer = 0;
                    e.motion.stop();
                    e.tile = cx.bases.get(&self.tile_kind).copied();
                    e.mapping = 0;
                    e.vars[BROKEN] = 0;
                    e.routine = if e.vars[MODE] == SINGLE {
                        self.single_routine
                    } else {
                        4
                    };
                    return;
                }
                4 | 8 => {
                    // $18B54 / $18B58
                    self.tanks(cx, e);
                    e.routine = 4;
                    if e.vars[BROKEN] & 7 == 7 {
                        Ctx::free(e);
                    }
                    return;
                }
                0x0c => {
                    // $18B78
                    e.hurt_routine = self.hurt;
                    e.death_routine = self.death;
                    e.mapping = self.mapping;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $18B94: a $FC every 64 ticks while the player is near.
                    if cx.process_hit(e).is_some() {
                        continue;
                    }
                    if e.timer & self.period == 0
                        && cx.player_dx(e.x) < self.single_near.0
                        && cx.player_dy(e.y) < self.single_near.1
                    {
                        e.vars[POINT_X] = e.x;
                        e.vars[POINT_Y] = e.y;
                        Self::release(cx, e.x, e.y);
                    }
                    return;
                }
                r if r == self.death => {
                    // $18BD4: Func_01903A at the last release point ($30,
                    // $32), score (central), Func_018CFA.
                    self.break_glass(cx, e.vars[POINT_X], e.vars[POINT_Y]);
                    self.glass.smash(cx, e.x, e.y);
                    Ctx::free(e);
                    return;
                }
                _ => {
                    // $18BEC: hurt resumes.
                    e.routine = e.saved_routine;
                    return;
                }
            }
        }
    }
}
