//! Round 6 boss: `Boss6` (`$31BB0`), called through `Boss_Jumps`.
//! 6-3: the dark room of revolving wall panels and the kabuki lion.
//!
//! Phase 1 (shadows): the list-8 record only runs timers; two list-4 type
//! `$0C` children (`$1DE82`) show its pose. They rise from the floor at
//! doors 3 (the real one) and 5 (a decoy), whirl, stand 32 ticks, turn away
//! through their panels, and 64 ticks later step out of two other doors
//! (`Func_0326EC`: never the one the player stands at) as black
//! silhouettes, whirl and throw four needles each at the player. Hitting
//! the real shadow makes the decoy vanish; 13 points (`+7` = `$0C` at
//! `$1DF30`) destroy it. Phase 2: the room fades to black and back lit,
//! the lion rises at door 4 and then attacks from the door it appears at
//! (`Data_032098`): the hair whirl (deflects weapons, eight bursts of four
//! needles in a rotating spiral, `Data_032188`), three fans that rise,
//! drift and home in (list-4 `$13`), or, from the upper corner doors, hops
//! across the ledge while three spears erupt from the floor (list-4 `$1D`).
//! Sixteen points (`+$43` = 3 from `Data_0364DE`, four units per step)
//! defeat it; it then leaves by door 1 in smoke, explodes for 120 ticks and
//! requests the next substage 180 ticks later.
//!
//! [`Boss6`] follows the record's routine table (`$31BD2`); each routine
//! constant is the `obj_routine` byte offset and cites its code. Tables are
//! read from the ROM at load (`boss6_tables_match_rom` pins them); timers
//! whose code was only walked (not converted) are constants citing their
//! address.
//!
//! Native model: one record plays the real shadow and lion, with an
//! independent list-4 decoy that receives body and weapon collisions.
//! Panel maps, room fades, intro/transition controls, petals and hop fans
//! use the recovered presentation data. The framework uses a combined
//! 29-point health pool and modern effect allocation; the boss steers fans
//! and spears while alive under the original list-4 limit of 31.
use super::{Flasher, HealthPalette, aim_angle, blast_offsets, half_turn, slopes, unit_vector};
use crate::{
    combat::{
        Effect, EffectKind, Enemy, Hitbox, Motion, PaletteOp, PlaneOp, Projectile, Reaction,
        ShotBox, ShotDown, Spawn,
    },
    enemies::{Behavior, Ctx, HitResponse, Roster, aim},
    game::Terrain,
    rom::{Result, byte, signed, signed_byte, word, word_i, words},
    sound_events::Table,
};
use std::sync::Arc;

/// `Boss_Jumps` target of this round.
pub(crate) const HANDLER: usize = 0x31bb0;

pub(crate) fn fight(rom: &[u8]) -> Result<Arc<dyn Behavior>> {
    Ok(Arc::new(Boss6::load(rom)?))
}

/// List-5 types this boss spawns and owns (registered by [`register_parts`]).
/// Its children are list-4 records, modelled inside [`Boss6`].
pub(crate) const PART_KINDS: &[u16] = &[];

pub(crate) fn register_parts(_rom: &[u8], _roster: &mut Roster) -> Result<()> {
    Ok(())
}

// Routines (`obj_routine` offsets of `$31BD2`).
const INIT: u16 = 0x00; // $31C0A
const INTRO: u16 = 0x04; // $31CA0
const STAND: u16 = 0x08; // $31D5A
const EXIT: u16 = 0x0c; // $31DDA
const EMPTY: u16 = 0x10; // $31EAA
const APPEAR: u16 = 0x14; // $31F1C
const WHIRL: u16 = 0x18; // $3209E
const RISE: u16 = 0x1c; // $32190
const FANS: u16 = 0x20; // $3220C
const JUMP: u16 = 0x24; // $322A8
const HURT: u16 = 0x28; // $32412
const EXPLODE: u16 = 0x2c; // $3247A
const CLEAR: u16 = 0x30; // $324E0
const TRANSITION: u16 = 0x34; // $32554
/// Native: the real shadow blinks out (children rt $20, `$1E39E`).
const SHADOW_DOWN: u16 = 0x38;
/// Native: the real shadow's 20-tick hurt (`$1E322`) before it leaves.
const SHADOW_HURT: u16 = 0x3c;
/// Native `obj_hurt_routine` / `obj_death_routine` of the record.
const ON_HIT: u16 = 0x80;
const ON_DEFEAT: u16 = 0x84;

const DOOR: usize = 0;
const DECOY: usize = 1;
const INVULN: usize = 2;
const DECOY_FLIP: usize = 3;
const DEFEATED: usize = 4;
const LAST: usize = 5;
const T: usize = 6;
const SPARE: usize = 7;
/// Native: the `$37` flasher's `$2C`/`$2A` ([`Flasher`]).
const FLASHER: usize = 8;
const DECOY_LIVE: usize = 9;
const DECOY_HURT: usize = 10;
const DECOY_SHOT: u16 = 0xfffc;
const PETAL_TICKS: usize = 11;
const HOP_DECOR: usize = 12;

// Timers (frames) of code walked from the ROM.
/// rt 8: `btst #5,$2F` (32 ticks).
const STAND_TICKS: i32 = 32;
/// rt $C: the panel steps once every 8 ticks through six maps (`$328C6`).
const PANEL_TICKS: i32 = 48;
/// Shadow exit handshake (`$1E0DA..$1E188`), confirmed in Genesis Plus GX.
const SHADOW_EXIT_TICKS: i32 = 43;
/// rt $C: frame 0 and voice $10 at `$28` = $18, again 15 ticks later.
const VANISH_AT: i32 = 0x18;
const VOICE_AGAIN: i32 = 15;
/// rt $10: `btst #6,$2F` (64 ticks).
const EMPTY_TICKS: i32 = 64;
/// rt $14: the first panel steps immediately, then every 8 ticks;
/// counter 3 starts the pose on update 17 (`$31F68`, `$32898`).
const APPEAR_AT: i32 = 16;
/// Phase-1 whirl: needles at pose step $18 (`$1E214`).
const AIMED_AT: i32 = 0x18;
/// Phase-2 whirl: a burst every 5 ticks (`$3209E`).
const BURST_PERIOD: i32 = 5;
/// Fan throw: the fans leave at pose step $20 (`$3220C`).
const FANS_AT: i32 = 0x20 - 1;
/// Invulnerability after a hit: shadow $19 (`$1E048`), lion $14 (`$32F4E`).
const SHADOW_INVULN: i32 = 0x19;
const LION_INVULN: i32 = 0x14;
const SHADOW_HURT_TICKS: i32 = 20;
const SHADOW_BLINK_TICKS: i32 = 128;
const HURT_TICKS: i32 = 60;
/// rt $34: fade out (observed 52), hold $D2 with the music at 50, fade in.
const FADE_TICKS: i32 = 52;
const HOLD_TICKS: i32 = 210;
const MUSIC_AT: i32 = 50;
/// rt $34 end: the lion starts at door 4 plus $78 (`$3265E`).
const RISE_DEPTH: i32 = 0x78;
/// rt $1C: rises to screen Y $D0.
const RISE_TOP: i32 = 0xd0;
/// rt $2C: 120 ticks ($78), effect $2F every 15, a burst every other.
const EXPLODE_TICKS: i32 = 0x78;
/// rt $30: debris and the music fade at $28, the request at $B4.
const DEBRIS_AT: i32 = 0x28;
const REQUEST_AT: i32 = 0xb4;
/// rt $24: ay $6000, vy -6, vx 1.5; stops at screen X $40..$100.
const HOP_GRAVITY: i32 = 0x6000;
const HOP_VY: i32 = -0x60000;
const HOP_VX: i32 = 0x18000;
const LEDGE: (i32, i32) = (0x40, 0x100);
/// List-4 limit while `Boss6` runs (`Object_ListParams+2` = $1E).
const SHOT_CAP: usize = 31;
/// `Data_0364DE` rows (`+$43` from 3).
const LION_ROWS: usize = 4;

/// Spear (list-4 `$1D`, `$1F42A`): start Y ($130 in the ROM; natively
/// $120, the lowest Y the projectile bounds keep, still below the screen),
/// top Y, rise, hold, sink.
const SPEAR_START: i32 = 0x120;
const SPEAR_TOP: i32 = 0xd8;
const SPEAR_RISE: i32 = 10;
const SPEAR_HOLD: u16 = 30;
const SPEAR_SINK: i32 = 0x12000;
/// Encodes a spear's delay in its `life` (native bookkeeping).
const SPEAR_LIFE: u16 = 2000;
/// A drifting fan's `life`: `FAN_DRIFT_LIFE + 3 * start + index`.
const FAN_DRIFT_LIFE: u16 = 4000;
/// Fan (list-4 `$13`, `$1EBEE`): rise until 160 px above the lion, drift
/// 70 ticks, wait 60/1/30, then home at 4 px/tick.
const FAN_CLIMB: i32 = 160;
const FAN_DRIFT_TICKS: u16 = 70;
const FAN_WAITS: [u16; 3] = [60, 1, 30];
const FAN_LIFE: u16 = 1000;
const FAN_SPEED: i32 = 4;
const FAN_RISE_ACCEL: i32 = -0x2000;

/// `Boss6` and its children.
pub(crate) struct Boss6 {
    /// `Data_01E428`: door (x, y) on screen; `Data_01E440`: facing.
    doors: [(i32, i32); 6],
    facing: [i8; 6],
    /// `Data_032788`: five doors per player region.
    rows: [[u8; 5]; 6],
    intro: Vec<u16>,
    exit: Vec<u16>,
    appear: Vec<u16>,
    whirl: Vec<u16>,
    fan_pose: Vec<u16>,
    base: u16,
    /// `Data_032098`: the attack routine by door.
    attacks: [u16; 6],
    /// `Data_032188`: spiral directions (16 per turn).
    spiral: [u8; 8],
    /// Needles: `$1E878` frames (+ base), `$1E888` flips, `$1E69E` offsets.
    needle_frames: [u16; 16],
    needle_flips: [u8; 16],
    needle_offsets: [(i32, i32); 4],
    /// `Data_034A00`: 64 (cos, sin) pairs, Q12, over half a turn.
    circle: Vec<(i32, i32)>,
    /// `Data_01F68E`: `Func_01F5DE`'s slope-to-angle bytes.
    slopes: [u8; 64],
    fan_frames: [u16; 6],
    spear: u16,
    shadow_points: u16,
    lion_points: u16,
    /// `Data_0364DE`: line 1 of the lit lion by `+$43` (4 rows, redder as
    /// it falls), shown by the `$37` flasher.
    health_palette: HealthPalette,
    flasher: Flasher,
    /// `$32F9C`: the flasher's duration (`$2A`).
    flash_ticks: u16,
    rise_depth: i32,
    rise_vy: i32,
    burst_offsets: Vec<(i32, i32)>,
    needle_damage: u8,
    fan_damage: u8,
    spear_damage: u8,
    katana: u8,
    trap: u8,
    mijin: u8,
    lion_voice: u8,
    death_voice: u8,
    fan_sound: u8,
    aim_sound: u8,
    needle_impact_sound: u8,
    music: u8,
    intro_palettes: [[u16; 16]; 9],
    room_palette: [u16; 16],
}

fn bytes<const N: usize>(rom: &[u8], p: usize) -> Result<[u8; N]> {
    let mut out = [0; N];
    for (i, value) in out.iter_mut().enumerate() {
        *value = byte(rom, p + i)?;
    }
    Ok(out)
}

impl Boss6 {
    fn collision_box(cx: &Ctx, e: &Enemy) -> Option<Hitbox> {
        // $1DFE2/$320D6/$32412 move the center up; $322A8 also grows it.
        cx.mappings.get(&e.mapping).map(|mapping| Hitbox {
            y: mapping.bounds.y - 16,
            half_height: mapping.bounds.half_height + if e.routine == JUMP { 16 } else { 0 },
            ..mapping.bounds
        })
    }
    fn load(rom: &[u8]) -> Result<Self> {
        // $1E832: addi.w #$2C7 to every pose byte.
        let base = word(rom, 0x1e834)?;
        let pose = |p: usize, n: usize| -> Result<Vec<u16>> {
            (0..n)
                .map(|i| Ok(u16::from(byte(rom, p + i)?) + base))
                .collect()
        };
        let mut doors = [(0, 0); 6];
        for (i, door) in doors.iter_mut().enumerate() {
            *door = (word_i(rom, 0x1e428 + i * 4)?, word_i(rom, 0x1e42a + i * 4)?);
        }
        let mut rows = [[0; 5]; 6];
        for (i, row) in rows.iter_mut().enumerate() {
            *row = bytes(rom, 0x32788 + i * 5)?;
        }
        let mut needle_frames = [0; 16];
        for (i, frame) in needle_frames.iter_mut().enumerate() {
            *frame = u16::from(byte(rom, 0x1e878 + i)?) + base;
        }
        let offset = |i: usize| -> Result<(i32, i32)> {
            Ok((
                signed_byte(rom, 0x1e69e + i)?,
                signed_byte(rom, 0x1e6a2 + i)?,
            ))
        };
        let mut fan_frames = [0; 6];
        for (i, frame) in fan_frames.iter_mut().enumerate() {
            *frame = word(rom, 0x1f584 + i * 2)?;
        }
        let mut intro_palettes = [[0; 16]; 9];
        for (i, row) in intro_palettes.iter_mut().enumerate() {
            *row = words(rom, 0x32fe2 + i * 32)?;
        }
        Ok(Self {
            doors,
            facing: bytes::<6>(rom, 0x1e440)?.map(|b| b as i8),
            rows,
            intro: pose(0x327a6, 0x58)?,
            exit: pose(0x327f6, 0x18)?,
            appear: pose(0x32810, 0x24)?,
            whirl: pose(0x32834, 0x2c)?,
            fan_pose: pose(0x32860, 0x38)?,
            base,
            attacks: bytes::<6>(rom, 0x32098)?.map(u16::from),
            spiral: bytes(rom, 0x32188)?,
            needle_frames,
            needle_flips: bytes(rom, 0x1e888)?,
            needle_offsets: [offset(0)?, offset(1)?, offset(2)?, offset(3)?],
            circle: half_turn(rom)?,
            slopes: slopes(rom)?,
            fan_frames,
            spear: 0x377,
            shadow_points: u16::from(byte(rom, 0x1df33)?) + 1,
            lion_points: (word(rom, 0x364de)? + 1) * 4,
            health_palette: HealthPalette::flasher(rom, usize::from(word(rom, 0x32f96)?))?,
            flasher: Flasher::load(rom)?,
            flash_ticks: word(rom, 0x32f9c)?,
            rise_depth: word_i(rom, 0x1df60)?,
            rise_vy: signed(rom, 0x1df66)?,
            burst_offsets: blast_offsets(rom)?,
            needle_damage: byte(rom, 0x1fbf6 + 0x0e)?,
            fan_damage: byte(rom, 0x1fbf6 + 0x13)?,
            spear_damage: byte(rom, 0x1fbf6 + 0x1d)?,
            katana: Table::Effect.command_at(rom, 0x31cb9)?,
            trap: Table::Effect.command_at(rom, 0x31dfb)?,
            mijin: Table::Effect.command_at(rom, 0x32493)?,
            lion_voice: Table::Voice.command_at(rom, 0x31e7f)?,
            death_voice: Table::Voice.command_at(rom, 0x32073)?,
            fan_sound: Table::Effect.command_at(rom, 0x1ec81)?,
            aim_sound: Table::Effect.command_at(rom, 0x1ed3b)?,
            needle_impact_sound: Table::Effect.command_at(rom, 0x1e753)?,
            // Func_004A36: music $19.
            music: Table::Music.command_at(rom, 0x32609)?,
            intro_palettes,
            room_palette: words(rom, 0x32678)?,
        })
    }

    fn door_at(&self, cx: &Ctx, door: i32) -> (i32, i32) {
        let (x, y) = self.doors[door.clamp(0, 5) as usize];
        (cx.camera.0 + x, cx.camera.1 + y)
    }
    /// `Data_01E440`: +1 faces right (flip), -1 faces the player, 0 left.
    fn door_flip(&self, cx: &Ctx, door: i32, x: i32) -> bool {
        match self.facing[door.clamp(0, 5) as usize] {
            1 => true,
            -1 => !cx.player_left(x),
            _ => false,
        }
    }
    /// `Func_0326EC`: a random door of the player's region row, not the
    /// one just taken (`$3E`).
    fn choose(&self, cx: &mut Ctx, last: i32) -> i32 {
        let (sx, sy) = (cx.player.0 - cx.camera.0, cx.player.1 - cx.camera.1);
        let pair = if sx < 0x70 {
            2
        } else if sx >= 0xd0 {
            4
        } else {
            0
        };
        let row = &self.rows[pair + usize::from(sy >= 0x70)];
        let index = usize::from(cx.random() % 5);
        let pick = i32::from(row[index]);
        if pick == last {
            i32::from(row[(index + 1) % 5])
        } else {
            pick
        }
    }
    fn pose(table: &[u16], step: i32) -> Option<u16> {
        usize::try_from(step)
            .ok()
            .and_then(|i| table.get(i).copied())
    }
    fn begin(e: &mut Enemy, routine: u16) {
        e.routine = routine;
        e.vars[T] = 0;
    }

    /// $32898/$328C6: first map immediately, then every eight updates.
    /// Direction follows each record's tile flip; map zero closes the door.
    fn panel(&self, cx: &mut Ctx, e: &Enemy, t: i32, exit: bool) {
        if t >= 48 || t % 8 != 0 {
            return;
        }
        let step = t / 8;
        let mut draw = |door: i32, flip: bool| {
            let frame = if flip == exit {
                5 - step
            } else {
                (step + 1) % 6
            };
            cx.plane_op(PlaneOp::KabukiPanel {
                door: door as u8,
                frame: frame as u8,
            });
        };
        draw(e.vars[DOOR], e.facing_left);
        if !Self::phase2(e) && e.vars[DECOY] >= 0 {
            draw(e.vars[DECOY], e.vars[DECOY_FLIP] != 0);
        }
    }

    /// `$1F820`: 8 px/tick along direction `d` (16 per turn, counter-
    /// clockwise from the right, up negative).
    fn needle_velocity(&self, d: u8) -> (i32, i32) {
        let (x, y) = unit_vector(&self.circle, (d & 15) * 8);
        (x << 3, y << 3)
    }
    /// List-4 `$0E` (`$1E5D4`): four parallel needles from `(x, y)` along
    /// direction `d`, 24 px out from `y - $2E`.
    fn needles(&self, cx: &mut Ctx, x: i32, y: i32, d: u8) {
        let (vx, vy) = self.needle_velocity(d);
        let d = usize::from(d & 15);
        let flips = self.needle_flips[d];
        let hflip = flips & 2 != 0;
        let vflip = flips & 1 != 0;
        let (ox, oy) = ((vx >> 16) * 3, (vy >> 16) * 3);
        for (index, (dx, dy)) in self.needle_offsets.into_iter().enumerate().rev() {
            let mut needle = Projectile::new(
                x + dx + ox,
                y - 0x2e + dy + oy,
                0,
                true,
                self.needle_frames[d],
                self.needle_damage,
            )
            .with_motion(Motion {
                vx,
                vy,
                ..Motion::default()
            });
            needle.base = 0x2000 | if vflip { 0x1000 } else { 0 };
            needle.faces_left = needle.leftward() != hflip;
            // `$1E5D4` loads its box only in routine 0 (`$1E5F2`), before
            // `$1E824` sets the first frame: mapping 0's, a point, for life.
            needle.box_load = ShotBox::Fixed(Hitbox::NONE);
            needle.owner = Some(cx.current);
            // `$1E2C4` / `$32114`: only d1 = 0 gets flag bit 1;
            // `$1E754` requests SHURIKINS when that needle hits terrain.
            needle.wall_sound = (index == 0).then_some(self.needle_impact_sound);
            needle.life = 0xb4;
            if !cx.shoot_capped(needle, SHOT_CAP) {
                return;
            }
        }
    }
    /// `$1E2C4`: `Func_01F5DE` from `(x, y - $18)` to the player's chest
    /// (`y - $10`), `andi.w #$7F` / `lsr.w #3` to 16 directions.
    fn aim_direction(&self, player: (i32, i32), x: i32, y: i32) -> u8 {
        let angle = aim_angle(&self.slopes, (x, y - 0x18), (player.0, player.1 - 0x10));
        (angle & 0x7f) >> 3
    }
    /// List-4 `$13` (`$1EBEE`): three fans from the lion's hand.
    fn fans(&self, cx: &mut Ctx, e: &Enemy) {
        let dir = if e.facing_left { 1 } else { -1 };
        for index in 0..3u16 {
            let mut fan = Projectile::new(
                e.x + dir * 32,
                e.y - 56,
                0,
                true,
                self.fan_frames[0],
                self.fan_damage,
            )
            .with_motion(Motion {
                vx: -dir * (2 << 16),
                gravity: FAN_RISE_ACCEL,
                ..Motion::default()
            })
            // `$1EBEE` never loads a box (the record's last one stays):
            // native keeps the frame drawn.
            .box_after();
            fan.base = 0x4000;
            fan.owner = Some(cx.current);
            fan.life = FAN_LIFE + index;
            // `$1EC88` runs after setup in the allocation frame.
            if Self::fan_reaches_top(&fan, e.y) {
                Self::fan_drift(&mut fan, e.y, index);
            }
            if !cx.shoot_capped(fan, SHOT_CAP) {
                break;
            }
            // `$1EC82`: each allocated fan requests its own effect.
            cx.play(self.fan_sound);
        }
    }
    /// `$1EC94..$1ECB4`: predict the next integrated Y, then compare
    /// unsigned words. At upper doors the negative target wraps, so the
    /// fan clamps immediately rather than spending extra frames rising.
    fn fan_reaches_top(fan: &Projectile, lion_y: i32) -> bool {
        let next = ((i64::from(fan.y) << 16)
            + i64::from(fan.motion.fy)
            + i64::from(fan.motion.vy)
            + i64::from(fan.motion.gravity))
            >> 16;
        (lion_y - FAN_CLIMB) as u16 >= next as u16
    }
    fn fan_drift(fan: &mut Projectile, lion_y: i32, index: u16) {
        fan.y = lion_y - FAN_CLIMB;
        fan.motion = Motion::default();
        fan.life = FAN_DRIFT_LIFE + 3 * fan.age.min(1000) + index;
    }
    /// List-4 `$1D` (`$1F42A`): three spears below door columns 0..2,
    /// erupting in the order the lion travels.
    fn spears(&self, cx: &mut Ctx, e: &Enemy) {
        let delays: [u16; 3] = if e.facing_left {
            [20, 70, 120]
        } else {
            [120, 70, 20]
        };
        for (column, delay) in delays.into_iter().enumerate() {
            let x = cx.camera.0 + self.doors[column].0;
            let mut spear = Projectile::new(
                x,
                cx.camera.1 + SPEAR_START,
                0,
                true,
                self.spear,
                self.spear_damage,
            );
            spear.base = 0x2000;
            spear.passes_terrain = true;
            spear.owner = Some(cx.current);
            spear.life = SPEAR_LIFE + delay;
            cx.shoot_capped(spear, SHOT_CAP);
        }
    }
    /// The handlers of the fans and spears, run while the lion lives.
    fn steer(&self, cx: &mut Ctx, e: &Enemy) {
        let me = cx.current;
        let (player, camera, lion_y) = (cx.player, cx.camera, e.y);
        let mut sounds = Vec::new();
        for shot in cx.projectiles.iter_mut() {
            if shot.owner != Some(me) {
                continue;
            }
            // `$1E722..$1E74A`: needles pass the ledge band, even when
            // its material is solid. Their other impacts use the shot pass.
            if self.needle_frames.contains(&shot.mapping)
                && (0x68..0x80).contains(&(shot.y - camera.1))
            {
                shot.walled = false;
            }
            if (FAN_LIFE..FAN_LIFE + 3).contains(&shot.life) {
                // Rising (accelerating up) until 160 px above the lion.
                shot.mapping = self.fan_frames[usize::from(shot.age / 2) % 6];
                if Self::fan_reaches_top(shot, lion_y) {
                    let index = shot.life - FAN_LIFE;
                    Self::fan_drift(shot, lion_y, index);
                }
            } else if (FAN_DRIFT_LIFE..FAN_DRIFT_LIFE + 3003).contains(&shot.life) {
                shot.mapping = self.fan_frames[usize::from(shot.age / 2) % 6];
                let code = shot.life - FAN_DRIFT_LIFE;
                let (start, index) = (code / 3, usize::from(code % 3));
                let drift_end = start + FAN_DRIFT_TICKS;
                if shot.age == start + 1 {
                    shot.motion = Motion {
                        vx: (index as i32 - 1) << 16,
                        fx: 1,
                        ..Motion::default()
                    };
                }
                if shot.age == drift_end {
                    shot.motion = Motion {
                        fx: 1,
                        ..Motion::default()
                    };
                }
                if shot.age == drift_end + FAN_WAITS[index] + 1 {
                    let (vx, vy) = aim((shot.x, shot.y), (player.0, player.1 - 16));
                    shot.motion = Motion {
                        vx: vx * FAN_SPEED,
                        vy: vy * FAN_SPEED,
                        ..Motion::default()
                    };
                    sounds.push(self.aim_sound);
                    // `$1ED3C` then `$1ED5E`: throw and enemy-star cues,
                    // in that order, as the waiting fan starts homing.
                    sounds.push(self.fan_sound);
                }
            } else if shot.life >= SPEAR_LIFE && shot.mapping == self.spear {
                let delay = shot.life - SPEAR_LIFE;
                if shot.age < delay {
                    continue;
                }
                let top = camera.1 + SPEAR_TOP;
                let risen = i32::from(shot.age - delay) * SPEAR_RISE;
                if shot.motion.vy == 0 {
                    // fx marks the still spear as colliding.
                    shot.motion.fx = 1;
                    shot.y = (camera.1 + SPEAR_START - risen).max(top);
                    let held = shot.age - delay;
                    let rise_ticks = ((SPEAR_START - SPEAR_TOP) / SPEAR_RISE + 1) as u16;
                    if held >= rise_ticks + SPEAR_HOLD {
                        shot.motion.vy = SPEAR_SINK;
                    }
                }
            }
        }
        for sound in sounds {
            cx.play(sound);
        }
    }
    /// The decoy shadow, drawn this tick at `(x, y)`.
    fn decoy(cx: &mut Ctx, e: &mut Enemy, x: i32, y: i32, mapping: u16, flip: bool) {
        if mapping == 0 || e.vars[DECOY] < 0 {
            return;
        }
        let hurt = e.vars[DECOY_HURT];
        if hurt > 0 {
            e.vars[DECOY_HURT] -= 1;
            if hurt == 1 {
                e.vars[DECOY] = -1;
                return;
            }
        }
        let interactive = hurt == 0 && matches!(e.routine, STAND | WHIRL);
        let mut shadow = Projectile::new(x, y, 0, true, mapping, 1).with_art(0x2000, false);
        shadow.frames = Some([(mapping, flip); 4]);
        let bounds = Self::collision_box(cx, e)
            .unwrap_or(Hitbox::NONE)
            .placed(0, 0, flip, false);
        shadow.box_load = ShotBox::Fixed(bounds);
        shadow.passes_terrain = true;
        shadow.stays = true;
        shadow.life = DECOY_SHOT;
        shadow.owner = Some(cx.current);
        shadow.no_collision = !interactive;
        shadow.hidden = hurt > 0 && hurt & 1 != 0;
        if interactive {
            shadow.shot_down = Some(ShotDown {
                blast: None,
                first: 0,
                frames: 0,
                period: 0,
                sound: None,
                smoke: None,
            });
        }
        e.vars[DECOY_LIVE] = i32::from(interactive);
        cx.push_projectile_unchecked(shadow);
    }
    fn phase2(e: &Enemy) -> bool {
        e.phase >= 2
    }
    fn defeated(e: &Enemy) -> bool {
        e.vars[DEFEATED] != 0
    }
    /// `+$43`: 3, less one per four lion points taken, at least 0.
    fn health_row(&self, e: &Enemy) -> usize {
        if Self::phase2(e) {
            usize::from(e.health.saturating_sub(1) / 4)
        } else {
            LION_ROWS - 1
        }
    }
    /// Smoke puffs while a defeated lion comes out (list-1 `$09`).
    fn smoke(cx: &mut Ctx, e: &Enemy) {
        let dx = i32::from(cx.random() & 31) - 16;
        let dy = -i32::from(cx.random() & 63);
        let mut puff = Effect::new(EffectKind::Puff, e.x + dx, e.y + dy, 0);
        puff.motion.vy = -0x8000;
        cx.effect(puff);
    }
    /// A hit (`ON_HIT`) or the lethal hit (`ON_DEFEAT`) taken this tick.
    fn react(&self, e: &mut Enemy, reaction: Option<Reaction>) {
        match (reaction, e.routine) {
            (Some(Reaction::Hurt), ON_HIT) if !Self::phase2(e) => {
                e.vars[DECOY] = -1;
                if e.health <= self.lion_points {
                    // The real shadow's counter went negative.
                    Self::begin(e, SHADOW_DOWN);
                } else {
                    e.vars[INVULN] = SHADOW_INVULN;
                    Self::begin(e, SHADOW_HURT);
                }
            }
            (Some(Reaction::Hurt), ON_HIT) | (Some(Reaction::Death), ON_DEFEAT) => {
                if reaction == Some(Reaction::Death) {
                    e.vars[DEFEATED] = 1;
                }
                e.vars[INVULN] = LION_INVULN;
                if !Self::phase2(e) {
                    e.vars[DECOY] = -1;
                    Self::begin(e, SHADOW_DOWN);
                } else if e.saved_routine == STAND {
                    Self::begin(e, HURT);
                } else {
                    // $32438..$32456: hits during recovery refresh immunity
                    // and the palette flasher, but keep its 60-tick timer.
                    // FANS/JUMP likewise retain their attack progress.
                    e.routine = e.saved_routine;
                }
            }
            _ => {}
        }
    }
}

impl Behavior for Boss6 {
    fn label(&self, _kind: u16) -> String {
        format!("Boss6 ${HANDLER:05X}")
    }
    fn touches_player(&self) -> bool {
        false // Each routine supplies its own contact damage, not Boss_Records' zero.
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        let live_decoy = cx
            .projectiles
            .iter()
            .any(|p| p.owner == Some(cx.current) && p.life == DECOY_SHOT && p.alive);
        if e.vars[DECOY_LIVE] != 0 && !live_decoy && e.vars[DECOY] >= 0 {
            e.vars[DECOY_HURT] = SHADOW_HURT_TICKS;
        }
        e.vars[DECOY_LIVE] = 0;
        cx.projectiles
            .retain(|p| !(p.owner == Some(cx.current) && p.life == DECOY_SHOT));
        // $32F2C / $38EEE: a petal every 32 live-lion updates.
        if Self::phase2(e) && !Self::defeated(e) && e.routine != TRANSITION {
            e.vars[PETAL_TICKS] += 1;
            if e.vars[PETAL_TICKS] == 32 {
                e.vars[PETAL_TICKS] = 0;
                let speed = if cx.random() & 1 == 0 {
                    0x14000
                } else {
                    0xa000
                };
                let x = cx.camera.0 + i32::from(cx.random() % 320);
                let mut petal = Effect::new(EffectKind::Petal, x, cx.camera.1, 0x2eb);
                petal.base = 0x4000;
                petal.origin = (x, cx.camera.1);
                petal.motion.vy = speed;
                petal.angle = if cx.random() & 1 == 0 { 0 } else { 64 };
                cx.effect(petal);
            }
        }
        let row = self.health_row(e);
        self.flasher
            .step(cx, &mut e.vars[FLASHER], &self.health_palette, row);
        let reaction = cx.process_hit(e);
        if reaction.is_some_and(|r| r != Reaction::Deflect) {
            // $1E446 (real shadow) / $32F4E (lion): both allocate the
            // table-3 `$37` flasher, revealing the caught shadow in colour.
            Flasher::start(&mut e.vars[FLASHER], self.flash_ticks);
        }
        self.react(e, reaction);
        if e.vars[INVULN] > 0 {
            e.vars[INVULN] -= 1;
        }
        self.steer(cx, e);
        let t = e.vars[T];
        e.vars[T] += 1;
        // $1DFE2/$1E284 and $31D88/$3220C/$322A8/$32412 test the
        // current pose before moving or animating it, even during a lion
        // hop/hurt immunity period. The lion's whirl never tests the body.
        let contact = match (Self::phase2(e), e.routine) {
            (_, STAND) => t + 1 < STAND_TICKS,
            (false, WHIRL) | (true, FANS) => e.vars[INVULN] == 0,
            (true, JUMP | HURT) => true,
            _ => false,
        };
        if contact
            && let Some(body) = Self::collision_box(cx, e)
            && cx.collide_player(body.placed(e.x, e.y, e.facing_left, e.vertical_flip()))
        {
            cx.contact_player(1);
        }
        match e.routine {
            INIT => {
                // $31C0A: the real shadow at door 3, the decoy at door 5,
                // both 80 px down, rising.
                e.phase = 1;
                e.health = self.shadow_points + self.lion_points;
                e.hurt_routine = ON_HIT;
                e.death_routine = ON_DEFEAT;
                e.body = None;
                e.motion.stop();
                e.vars[DOOR] = 3;
                e.vars[DECOY] = 5;
                e.vars[DECOY_FLIP] = 0;
                e.vars[LAST] = -1;
                e.vars[DEFEATED] = 0;
                e.vars[DECOY_HURT] = 0;
                cx.palette_op(PaletteOp::Fill(1, 0));
                let (x, y) = self.door_at(cx, 3);
                e.x = x;
                e.y = y + self.rise_depth;
                e.facing_left = true;
                e.mapping = self.base;
                Self::begin(e, INTRO);
            }
            INTRO => {
                cx.player_event(crate::movement::PlayerEvent::Idle);
                if t >= 35 && (t - 35) % 8 == 7 {
                    let row = (8 - (t - 35) / 8).clamp(0, 8) as usize;
                    cx.palette_op(PaletteOp::Load(3, self.intro_palettes[row]));
                }
                // $31CA0 / children $1DF5E: rise 1 px/tick, then the whirl.
                let (_, top) = self.door_at(cx, e.vars[DOOR]);
                let rise = self.rise_depth;
                if t < rise {
                    // `rise_vy` is -1 px/tick.
                    e.y = top + rise + self.rise_vy * (t + 1);
                } else {
                    let step = t - rise;
                    if step == 0x1e {
                        cx.play(self.katana);
                    }
                    match Self::pose(&self.intro, step) {
                        Some(frame) => e.mapping = frame,
                        None => Self::begin(e, EXIT),
                    }
                }
            }
            STAND => {
                // $31D5A
                e.mapping = if Self::phase2(e) {
                    e.mapping
                } else {
                    self.base
                };
                if t + 1 >= STAND_TICKS {
                    if Self::phase2(e) {
                        e.facing_left = !e.facing_left;
                        e.mapping = self.exit[0];
                    }
                    Self::begin(e, EXIT);
                }
            }
            EXIT => {
                // $1E0BA/$31E46 fade the revealed colours back to shadow.
                if t < VANISH_AT {
                    cx.fade_palette(1, [0; 16]);
                }
                // $31DDA / $1E07C: turn away into the panel.
                if t == 0 {
                    if !Self::phase2(e) {
                        e.facing_left = !e.facing_left;
                        e.vars[DECOY_FLIP] ^= 1;
                    }
                    cx.play(self.trap);
                }
                self.panel(cx, e, t, true);
                e.mapping = if t < VANISH_AT {
                    Self::pose(&self.exit, t).unwrap_or(0)
                } else {
                    0
                };
                let voice_at = VANISH_AT - i32::from(!Self::phase2(e));
                if t == voice_at || t == voice_at + VOICE_AGAIN {
                    cx.play(self.lion_voice);
                }
                let ticks = if Self::phase2(e) {
                    PANEL_TICKS
                } else {
                    SHADOW_EXIT_TICKS
                };
                if t + 1 >= ticks {
                    Self::begin(e, EMPTY);
                }
            }
            EMPTY => {
                // $31EAA: the empty room, then both pick doors.
                e.mapping = 0;
                if t + 1 >= EMPTY_TICKS {
                    let door = if Self::defeated(e) {
                        1
                    } else {
                        self.choose(cx, e.vars[LAST])
                    };
                    e.vars[DOOR] = door;
                    e.vars[LAST] = door;
                    // $1E18A clears each shadow's hidden flag on a new
                    // door cycle, including the decoy hidden by a hit.
                    if !Self::phase2(e) {
                        let decoy = self.choose(cx, door);
                        let (x, _) = self.door_at(cx, decoy);
                        e.vars[DECOY] = decoy;
                        e.vars[DECOY_HURT] = 0;
                        e.vars[DECOY_FLIP] = i32::from(self.door_flip(cx, decoy, x));
                    }
                    let (x, y) = self.door_at(cx, door);
                    e.x = x;
                    e.y = y;
                    e.facing_left = self.door_flip(cx, door, x);
                    Self::begin(e, APPEAR);
                }
            }
            APPEAR => {
                self.panel(cx, e, t, false);
                if Self::phase2(e) && t >= APPEAR_AT {
                    self.health_palette.fade(cx, self.health_row(e));
                }
                // $31F1C: the panel turns; the pose from counter 3.
                if t == 0 {
                    cx.play(self.trap);
                    if !Self::phase2(e) {
                        // `$1E1E6`: both shadow records request TRAP too.
                        cx.play(self.trap);
                        cx.play(self.trap);
                    }
                }
                if Self::defeated(e) {
                    Self::smoke(cx, e);
                }
                e.mapping = if t < APPEAR_AT {
                    0
                } else {
                    Self::pose(&self.appear, t - APPEAR_AT).unwrap_or(self.base)
                };
                if t + 1 >= APPEAR_AT + self.appear.len() as i32 {
                    if !Self::phase2(e) {
                        Self::begin(e, WHIRL);
                    } else if Self::defeated(e) {
                        cx.play(self.death_voice);
                        e.vars[SPARE] = 0;
                        Self::begin(e, EXPLODE);
                    } else {
                        let door = e.vars[DOOR].clamp(0, 5) as usize;
                        Self::begin(e, self.attacks[door]);
                    }
                }
            }
            WHIRL => {
                // $3209E: the hair whirl; needles.
                if t == 0 {
                    cx.play(self.katana);
                }
                if let Some(frame) = Self::pose(&self.whirl, t) {
                    e.mapping = frame;
                }
                if !Self::phase2(e) {
                    if t == AIMED_AT {
                        let d = self.aim_direction(cx.player, e.x, e.y);
                        self.needles(cx, e.x, e.y, d);
                        if e.vars[DECOY] >= 0 {
                            let (x, y) = self.door_at(cx, e.vars[DECOY]);
                            let d = self.aim_direction(cx.player, x, y);
                            self.needles(cx, x, y, d);
                        }
                    }
                } else if t % BURST_PERIOD == 0 && (t / BURST_PERIOD) < self.spiral.len() as i32 {
                    let d = self.spiral[(t / BURST_PERIOD) as usize];
                    self.needles(cx, e.x, e.y, d);
                }
                if t + 1 >= self.whirl.len() as i32 {
                    e.mapping = self.base;
                    Self::begin(e, STAND);
                }
            }
            RISE => {
                // $32190: up 1 px/tick to screen Y $D0, then the intro pose.
                if e.vars[SPARE] == 0 {
                    e.y -= 1;
                    if e.y - cx.camera.1 <= RISE_TOP {
                        e.y = cx.camera.1 + RISE_TOP;
                        e.vars[SPARE] = 1;
                        e.vars[T] = 0;
                    }
                } else {
                    if t == 0x1e - 1 {
                        cx.play(self.katana);
                    }
                    match Self::pose(&self.intro, t) {
                        Some(frame) => e.mapping = frame,
                        None => {
                            e.mapping = self.exit[0];
                            e.facing_left = !e.facing_left;
                            Self::begin(e, EXIT);
                        }
                    }
                }
            }
            FANS => {
                // $3220C
                if let Some(frame) = Self::pose(&self.fan_pose, t) {
                    e.mapping = frame;
                }
                if t == FANS_AT {
                    self.fans(cx, e);
                }
                if t + 1 >= self.fan_pose.len() as i32 {
                    e.mapping = self.base;
                    Self::begin(e, STAND);
                }
            }
            JUMP => {
                // $322A8: hops along the ledge; spears on the first frame.
                if t == 0 {
                    e.motion.gravity = HOP_GRAVITY;
                    e.motion.vy = HOP_VY;
                    e.motion.vx = if e.vars[DOOR] == 0 { HOP_VX } else { -HOP_VX };
                    self.spears(cx, e);
                }
                let sx = e.x - cx.camera.0;
                if (e.motion.vx > 0 && sx >= LEDGE.1) || (e.motion.vx < 0 && sx < LEDGE.0) {
                    e.motion.vx = 0;
                }
                e.mapping = if e.y - cx.camera.1 < 0x48 {
                    self.base + 2
                } else {
                    self.base + 1
                };
                if e.y - cx.camera.1 < 0x48 {
                    e.vars[HOP_DECOR] += 1;
                    if e.vars[HOP_DECOR] == 16 {
                        e.vars[HOP_DECOR] = 0;
                        let left = cx.random() & 1 != 0;
                        let mut fan = Effect::new(EffectKind::Pieces, e.x, e.y - 24, 0x2db);
                        fan.base = 0x4000;
                        fan.motion = Motion {
                            vx: if left { -0x40000 } else { 0x40000 },
                            vy: -0x80000,
                            gravity: 0x3000,
                            ..Motion::default()
                        };
                        fan.counter = 80;
                        cx.effect(fan);
                    }
                }
                let (_, floor) = self.door_at(cx, e.vars[DOOR]);
                if t > 0 && e.motion.vy >= 0 && (terrain.solid_at(e.x, e.y) || e.y >= floor) {
                    e.y = if e.y >= floor { floor } else { e.y & !7 };
                    if e.motion.vx != 0 {
                        e.motion.vy = HOP_VY;
                    } else {
                        e.motion.stop();
                        e.vars[DOOR] = if e.vars[DOOR] == 0 { 2 } else { 0 };
                        e.facing_left = !e.facing_left;
                        e.mapping = self.base;
                        Self::begin(e, STAND);
                    }
                }
            }
            HURT => {
                // $32412
                if t + 1 >= HURT_TICKS {
                    e.facing_left = !e.facing_left;
                    e.mapping = self.exit[0];
                    Self::begin(e, EXIT);
                }
            }
            EXPLODE => {
                // $3247A: 120 ticks of explosions.
                cx.player_event(crate::movement::PlayerEvent::Freeze);
                if (t + 1) % 15 == 0 {
                    cx.play(self.mijin);
                }
                let (x, y) = cx.center(e);
                let (dx, dy) = self.burst_offsets[t as usize % self.burst_offsets.len()];
                for flip in [false, true] {
                    let mut puff = Effect::new(EffectKind::Puff, x + dx, y + dy, 0);
                    puff.motion.vy = -0x10000;
                    puff.flip = flip;
                    cx.effect(puff);
                }
                if t + 1 >= EXPLODE_TICKS {
                    e.mapping = 0;
                    Self::begin(e, CLEAR);
                }
            }
            CLEAR => {
                // $324E0: debris and the music fade, then Game_Requests bit 1.
                cx.player_event(crate::movement::PlayerEvent::Freeze);
                if t == DEBRIS_AT {
                    let first = cx.shared.puff_seq.first().copied().unwrap_or(0);
                    for i in 0..20 {
                        let dx = (i % 5) * 12 - 24;
                        let dy = (i / 5) * 10 - 48;
                        let mut blast = Effect::new(EffectKind::Puff, e.x + dx, e.y + dy, first);
                        blast.flip = i & 1 != 0;
                        cx.effect(blast);
                    }
                    cx.play(0xfc);
                }
                if t + 1 >= REQUEST_AT {
                    cx.next_substage();
                    Ctx::free(e);
                }
            }
            TRANSITION => {
                if !matches!(cx.player_state, Some(41 | 42 | 43 | 46)) {
                    cx.player_event(crate::movement::PlayerEvent::Idle);
                }
                if t < FADE_TICKS && (t + 1) % 4 == 0 {
                    cx.palette_op(PaletteOp::FadeTo(1, [0; 16]));
                    cx.palette_op(PaletteOp::FadeTo(3, [0; 16]));
                } else if t >= FADE_TICKS + HOLD_TICKS && (t + 1) % 4 == 0 {
                    cx.palette_op(PaletteOp::FadeTo(3, self.room_palette));
                }
                // $32554: dark, the music, lit; the lion at door 4.
                e.mapping = 0;
                if t == FADE_TICKS + MUSIC_AT {
                    cx.play(self.music);
                }
                if t + 1 >= FADE_TICKS * 2 + HOLD_TICKS {
                    // $32670: Func_032698(1), line 1 = `Data_036540`,
                    // the lit lion's full-health row.
                    self.health_palette.show(cx, LION_ROWS - 1);
                    e.phase = 2;
                    if !Self::defeated(e) {
                        e.health = self.lion_points;
                    }
                    e.vars[DOOR] = 4;
                    e.vars[LAST] = -1;
                    let (x, y) = self.door_at(cx, 4);
                    e.x = x;
                    e.y = y + RISE_DEPTH;
                    e.mapping = self.intro[0];
                    e.vars[SPARE] = 0;
                    Self::begin(e, RISE);
                }
            }
            SHADOW_DOWN => {
                // $1E39E: the real shadow blinks out.
                e.mapping = if t % 2 == 0 { self.base } else { 0 };
                if t + 1 >= SHADOW_BLINK_TICKS {
                    e.mapping = 0;
                    Self::begin(e, TRANSITION);
                }
            }
            SHADOW_HURT => {
                // $1E322: a 20-tick hurt, then it leaves early.
                if t + 1 >= SHADOW_HURT_TICKS {
                    Self::begin(e, EXIT);
                }
            }
            _ => Self::begin(e, STAND),
        }
        e.body = Self::collision_box(cx, e);
        // The decoy follows the shared pose at its own door.
        if !Self::phase2(e) && e.vars[DECOY] >= 0 && e.routine != SHADOW_DOWN {
            let (x, mut y) = self.door_at(cx, e.vars[DECOY]);
            if e.routine == INTRO && e.vars[T] <= self.rise_depth {
                y += self.rise_depth - e.vars[T];
            }
            let (mapping, flip) = (e.mapping, e.vars[DECOY_FLIP] != 0);
            Self::decoy(cx, e, x, y, mapping, flip);
        }
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        if enemy.mapping == 0 || enemy.vars[INVULN] > 0 {
            return HitResponse::Pass;
        }
        if !Self::phase2(enemy) {
            return if matches!(enemy.routine, STAND | WHIRL) {
                HitResponse::Accept
            } else {
                HitResponse::Pass
            };
        }
        match enemy.routine {
            WHIRL => HitResponse::Deflect,
            STAND | FANS | JUMP | HURT => HitResponse::Accept,
            _ => HitResponse::Pass,
        }
    }
}
