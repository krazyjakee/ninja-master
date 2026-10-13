//! Round 3 boss: `Boss3_Hydra` (`$2BFF2`), called through `Boss_Jumps`
//! (issues #41, #52).
//!
//! The hydra is a plane, not a sprite: the head is plane B (`Data_02D2EE`,
//! 24x20 cells at `$E000`, base `$A300`), moved by per-line horizontal and
//! per-column vertical scroll from its position (`$40`/`$44`, 16.16) plus a
//! sine wobble (`Func_02D130`), and patched every frame (eyes, mouth, jaw,
//! wound). Plane A is the procedural corridor that the stage's plane routine
//! `Func_00319A` draws from the `$3EFF2` map, its mirrored rows and two
//! generated floor rows. `stage::BossPlanes` recovers the initial corridor
//! and head; this state machine drives them through [`PlaneOp`]s, which
//! 3-3; issue #96).
//!
//! The list-8 record runs the script at `Data_02C17A` (first `04 FF`, then
//! `Data_02C16E`: spit, wait, inhale + laser, wait, dive, wait, rise, wait):
//!
//! | Command | Routine | Behaviour |
//! | --- | --- | --- |
//! | `$00`/`$04` | `$1F8` | drift left 1 px/tick to X `$B8` |
//! | `$08` | `Func_02CA5E` | HYDRA SHOT: three spit balls (`Data_02CAFE`), mouth frames |
//! | `$10` | `$258` | dive: left 1.5 px/tick while sinking (accel `$800`) below the floor |
//! | `$14` | `$2D6` | floor splashes, then rise 1 px/tick at the player's X - 64 |
//! | `$20` | `$474` | wait `Data_02C480` ticks (by level; NORMAL 120) |
//! | `$24` | `$4FE` | the jaw part opens, inhales (`Data_02C8E0`), fires the laser |
//!
//! Only the weak spot (`$26B`, the record's own box) takes damage, in the
//! routines that call `Func_02CD94` and not while it flashes; health 15
//! (16 points). A hit flashes palette line 1 white, opens the jaw, shakes
//! the wobble, and sends the tongue (list-4 `$3C`) out of the floor; the
//! boss waits (routine `$18`, HYDRA DAMAGE) until it is back. Face and neck
//! (`$26C`/`$26D`) deflect weapons; they and `$272` hurt on contact. At
//! health below zero the head rises, HYDRA DEATH, sinks shaking below the
//! floor and requests the round clear (`$FF0029` bit 1).
//!
//! Native model: the boss record runs the whole encounter, including the
//! tongue, jaw (`$4C`..`$58`) and laser (list-4 `$6C`..`$78`) state
//! machines, and packs their state into its `Enemy` ([`State`]). Its four
//! list-5 `$78` parts are puppets that copy their place from the boss
//! snapshot; contact damage and the laser are one-tick enemy projectiles
//! ([`hurtbox`]), because `$78`'s stats carry no contact damage. Neither the
//! record nor the puppets touch the player themselves
//! (`Behavior::touches_player`), or `$78`'s zero would knock Joe back
//! without damage, even while the dying head sinks.
//!
//! Presentation (#96), all through `Ctx::plane_op`/`Ctx::palette_op`:
//! plane B H-scroll per 8-line band and V-scroll per 2-cell column from the
//! boss position and the `$FF3A00`/`$FF3A02` sine wobble (`Func_02D130`);
//! the eye patches (`Data_02CD1E`/`Data_02CD46`, `loc_02CE78`), spit vents
//! (`Data_02D066`, `Func_02D044`), open/shut jaw (`Data_02D8D6`/`Data_02D920`)
//! and wounds (`Data_02D770..794`); palette line 1's hit flash
//! (`Func_02CE4C`) and `$6E8` fade back to the health line (`Func_02CE0A`),
//! its colour 12..15 rotation (`Func_02D072`) and inhale warning
//! (`Func_02C968`); the laser's plane-A hole (`Func_02D23A`), white corridor
//! line fading back (`Func_02D1FC`), random backdrop colour, band H-scroll
//! (`loc_02D1C2`) and the corridor rows' restore (`Func_02D2CC`).
//! `.loc_02CE78` falls into `Func_02CEA8` a second time, so outside routines
//! `$18`/`$1C` the wobble and those counters run twice a frame.
//!
//! Each of the face, neck and chin also shoves an overlapping Joe out of
//! the head (`Func_037244`, `crate::combat`'s pusher), dying or not.
//! The inhale pulls Joe right 1 px/tick (`addq.w #1,$20(a3)`) through
//! `Signal::PushPlayer`. The three timer tables (`Data_02C480`,
//! `Data_02C8E0`, `Data_02CC94`) take the `Game_Options` level column
//! (`Ctx::level`, #96). The spit balls are list-5 `$78` parts (`ROLE_SPIT`)
//! so that `Object_CollideL3` has the weapons' real boxes: a weapon that
//! meets one is spent and frees it (`loc_02CB2E`), and their `obj_tile`
//! bit 15 is set for their first 32 updates. The floor splashes
//! (`$E000`/`$8000`) and inhale sparkles (`$8000`) are high-priority
//! sprites over the head and corridor's high cells; the jaw and tongue are
//! low-priority sprites under them.
//!
//! Not modelled: `Func_02D130` returns at once while the player's
//! `plr_flags` bit 4 (ninjutsu) is set, holding the head's scroll and
//! wobble phase. Native runs no combat in `Mode_Ninjutsu`, so the head holds
//! through the technique anyway; only the press frame (`Player_Update`
//! before `Boss_Update`) differs. The `btst #4,$29(a0)` tests in routines
//! `$04`, `$10`, `$14`, `$1C` and `$24` read the boss's own `$29`, the low
//! byte of the script cursor, whose bit 4 is never set, so they never
//! branch. The spit balls come from list 5's capacity rather than list 4's.

use super::HealthPalette;
use crate::{
    combat::{
        Effect, EffectKind, Enemy, EnemyState, Hitbox, Motion, PaletteOp, Plane, PlaneOp,
        Projectile, Reaction, Signal, Spawn, mapping_hitbox,
    },
    enemies::{Behavior, Ctx, HitResponse, Roster, set_whole},
    game::Terrain,
    raster::SineWave,
    rom::{Result, byte, long, pointer, signed, word, word_i, words},
    sound_events::Table,
};
use parts::{
    J_IDLE, Puppet, ROLE, ROLE_FACE, ROLE_JAW, ROLE_NECK, ROLE_SPIT, ROLE_TONGUE, SPIT_HIGH,
    SPIT_TICKS, T_IDLE,
};
use std::sync::Arc;

mod parts;

/// `Boss_Jumps` target of this round.
pub(crate) const HANDLER: usize = 0x2bff2;
/// The part type (`Object_HandlerTable[$78]` is the same handler).
pub(crate) const PART: u16 = 0x78;

/// Plane B's name table (`$60000003`), where the head is drawn.
const PLANE_B: u16 = 0xe000;
/// `Func_02CD6E`: the open-jaw head patch (`Data_02D8D6`).
const JAW_OPEN: u32 = 0x2d8d6;
/// `Func_02CD80`: the shut-jaw patch (`Data_02D920`), then a wound.
const JAW_SHUT: u32 = 0x2d920;
/// `loc_02D0B8`: the wound patch for health below 3, below 6, and above.
const WOUNDS: [(u8, u32); 3] = [(3, 0x2d794), (6, 0x2d782), (u8::MAX, 0x2d770)];
/// `Func_02D0B8`/`Func_02D23A`: VDP writes of the laser. Plane A rows
/// 53..56 (screen lines 144..175 at V-scroll $118), columns 32..63, become
/// cell `$6000`; `Func_02D2CC` redraws all 64 columns afterwards.
const LASER_ROWS: u16 = 0xda80;
const LASER_HOLE: u16 = 0xdac0;
const LASER_HOLE_CELL: u16 = 0x6000;
/// `loc_02D1C2`: the band's lines 144..175 and its H-scroll, which shows the
/// transparent columns 32..63, so the backdrop.
const LASER_LINE: u8 = 144;
const LASER_SHIFT: i16 = -0x100;
/// `Func_02C968`: the inhale warning colour of palette line 1 (`$2CE44`).
const WARNING: u16 = 0x0fa7;
/// `Func_02CE4C`: the hit flash colour.
const FLASH: u16 = 0x0fff;

/// Difficulty column of the `Data_02C480`-style tables: `RAM_Start` bits
/// 2..3 (`asr.w #1` / `andi.w #6`), 0 NORMAL, 1 EASY, 2 HARD, 3 EXPERT.
pub(super) fn level(cx: &Ctx) -> usize {
    usize::from(cx.level & 3)
}

pub(crate) fn fight(rom: &[u8]) -> Result<Arc<dyn Behavior>> {
    Ok(Arc::new(Hydra::load(rom)?))
}

/// List-5 types this boss spawns and owns (registered by [`register_parts`]).
pub(crate) const PART_KINDS: &[u16] = &[PART];

pub(crate) fn register_parts(rom: &[u8], roster: &mut Roster) -> Result<()> {
    roster.add(PART, Arc::new(Puppet::load(rom)?));
    Ok(())
}

/// A one-tick enemy projectile with `mapping`'s hitbox at (x, y), mirrored
/// when `left`: contact damage for objects whose type has none. It is
/// removed in the same tick's projectile pass, so it is never drawn.
/// `body` marks an `Object_CollidePlayer` part (unblockable body contact)
/// rather than a list-4 object. `keep_contact` carries the floor hand's
/// list-4 overlap through the stage-object pass to `Player_CheckDamage`.
pub(crate) fn hurtbox(
    cx: &mut Ctx,
    (x, y): (i32, i32),
    mapping: u16,
    damage: u8,
    left: bool,
    body: bool,
    keep_contact: bool,
) {
    let mut shot = Projectile::new(x, y, 0, true, mapping, damage);
    shot.life = 1;
    shot.passes_terrain = true;
    shot.body_contact = body;
    shot.keep_contact = keep_contact;
    // A non-default motion makes the pass test the box once; fx + vx cancel
    // so it does not move, and vx < 0 mirrors the box.
    shot.motion = Motion {
        fx: 1,
        vx: if left { -1 } else { 0 },
        ..Motion::default()
    };
    cx.push_projectile_unchecked(shot);
}
/// `plr_event $A`: drain one health point through the player-event channel,
/// bypassing ordinary immunity and the Ikazuchi shield.
pub(crate) fn shock(cx: &mut Ctx, damage: u8) {
    if damage != 0 {
        cx.player_event(crate::movement::PlayerEvent::GrabDamage);
    }
}

/// `Func_02C102` records: type, routine, tile, frame, damage/health, X, Y.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Record {
    tile: u16,
    frame: u16,
    damage: u8,
    health: u8,
    x: i32,
    y: i32,
}
impl Record {
    fn load(rom: &[u8], p: usize) -> Result<Self> {
        let stats = word(rom, p + 8)?;
        Ok(Self {
            tile: word(rom, p + 4)?,
            frame: word(rom, p + 6)?,
            damage: (stats >> 8) as u8,
            health: stats as u8,
            x: signed(rom, p + 10)?,
            y: signed(rom, p + 12)?,
        })
    }
}

/// `Data_02CAFE`: one spit ball (16.16 velocity, acceleration words, offset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spit {
    vx: i32,
    vy: i32,
    ax: i32,
    ay: i32,
    dx: i32,
    dy: i32,
}

/// ROM constants of the encounter.
pub(crate) struct Hydra {
    body: Record,
    face: Record,
    neck: Record,
    chin: Record,
    jaw: Record,
    tongue: Record,
    spit_record: Record,
    laser: Record,
    splash: Record,
    /// `Data_02C17A`: the four script byte lists.
    scripts: [Vec<u8>; 4],
    main_script: Vec<u8>,
    first_script: u16,
    start: (i32, i32),
    rest_x: i32,
    dive_dx: i32,
    dive_accel: i32,
    floor_y: i32,
    dive_end_x: i32,
    rise_offset: i32,
    rise_min_x: i32,
    top_y: i32,
    entry_sound: u8,
    hurt_ticks: i32,
    damage_voice: u8,
    death_drool: i32,
    death_voice: u8,
    jitter: [(i32, i32); 8],
    waits: [i32; 4],
    inhales: [i32; 4],
    drools: [i32; 4],
    spit_sound: u8,
    spit_y: i32,
    spits: [Spit; 3],
    hit_step: u16,
    rest_step: u16,
    inhale_sound: u8,
    inhale_sound_at: i32,
    drop_max: u8,
    recoil: u8,
    laser_sound: u8,
    laser_ticks: u8,
    laser_start: u8,
    tongue_x: i32,
    tongue_rise: i32,
    tongue_sweep: u8,
    tongue_step: i32,
    tongue_drool_dx: i32,
    tongue_frame: u16,
    /// `Data_02CC28` multipliers of the splash velocity.
    splash_scale: [i32; 8],
    splash_small: u16,
    /// Weak spot box (`$26B`) and the three contact boxes.
    weak: Hitbox,
    /// `Data_042062 + Data_02BFCA[health]`: palette line 1 by health, the
    /// target of the hit flash's fade.
    flesh: HealthPalette,
    /// `Data_02CD1E`/`Data_02CD46`: the two eye patch lists, alternating
    /// every 8 updates (`loc_02CE78`).
    eyes: [Vec<u32>; 2],
    /// `Data_02D066`: the spit vent patches (`Func_02D044`).
    vents: [u32; 3],
    /// `Data_02D006`: laser band lines by the laser's `$28`.
    laser_widths: [u8; 32],
    /// `Data_02D2AC`: palette line 2 when the laser fires (`Func_02D23A`).
    laser_white: [u16; 16],
    /// `Data_042422`: the corridor line the laser fades back to (`$2D1FC`).
    corridor: [u16; 16],
}

impl Hydra {
    fn load(rom: &[u8]) -> Result<Self> {
        let mut scripts: [Vec<u8>; 4] = Default::default();
        for (i, script) in scripts.iter_mut().enumerate() {
            let mut p = pointer(rom, 0x2c17a + i * 4)?;
            loop {
                let b = byte(rom, p)?;
                script.push(b);
                p += 1;
                if b & 0x80 != 0 || script.len() > 32 {
                    break;
                }
            }
        }
        let main_script = scripts[0].clone();
        let words4 = |p: usize| -> Result<[i32; 4]> { Ok(words::<4>(rom, p)?.map(i32::from)) };
        let mut jitter = [(0, 0); 8];
        for (i, j) in jitter.iter_mut().enumerate() {
            *j = (signed(rom, 0x2c43c + i * 2)?, signed(rom, 0x2c44c + i * 2)?);
        }
        let mut spits = [Spit {
            vx: 0,
            vy: 0,
            ax: 0,
            ay: 0,
            dx: 0,
            dy: 0,
        }; 3];
        for (i, s) in spits.iter_mut().enumerate() {
            let p = 0x2cafe + i * 16;
            *s = Spit {
                vx: long(rom, p)?,
                vy: long(rom, p + 4)?,
                ax: signed(rom, p + 8)?,
                ay: signed(rom, p + 10)?,
                dx: signed(rom, p + 12)?,
                dy: signed(rom, p + 14)?,
            };
        }
        let weak = crate::combat::Mapping::load(rom, word(rom, 0x2c13e)?)?.bounds;
        let patch_list = |mut p: usize| -> Result<Vec<u32>> {
            let mut list = Vec::new();
            while let Some(header) = Some(long(rom, p)? as u32).filter(|&h| h != 0) {
                list.push(header);
                p += 4;
                if list.len() > 16 {
                    return Err("unterminated Hydra patch list".into());
                }
            }
            Ok(list)
        };
        let mut laser_widths = [0; 32];
        for (i, width) in laser_widths.iter_mut().enumerate() {
            *width = byte(rom, 0x2d006 + i)?;
        }
        let flesh = HealthPalette::load(
            rom,
            1,
            (0..16).map(|health| Ok(0x42062 + usize::from(word(rom, 0x2bfca + health * 2)?))),
        )?;
        Ok(Self {
            body: Record::load(rom, 0x2c138)?,
            face: Record::load(rom, 0x2c522)?,
            neck: Record::load(rom, 0x2c530)?,
            chin: Record::load(rom, 0x2c53e)?,
            jaw: Record::load(rom, 0x2c878)?,
            tongue: Record::load(rom, 0x2c63c)?,
            spit_record: Record::load(rom, 0x2ca50)?,
            laser: Record::load(rom, 0x2cf4e)?,
            splash: Record::load(rom, 0x2cb64)?,
            scripts,
            main_script,
            first_script: word(rom, 0x2c150)?,
            start: (long(rom, 0x2c156)?, long(rom, 0x2c15e)?),
            rest_x: long(rom, 0x2c1fa)?,
            dive_dx: signed(rom, 0x2c248)?,
            dive_accel: long(rom, 0x2c276)?,
            floor_y: long(rom, 0x2c27e)?,
            dive_end_x: word_i(rom, 0x2c298)?,
            rise_offset: word_i(rom, 0x2c2b6)?,
            rise_min_x: i32::from(byte(rom, 0x2c2bb)?),
            top_y: long(rom, 0x2c302)?,
            entry_sound: byte(rom, 0x2c251)?,
            hurt_ticks: word_i(rom, 0x2c33e)?,
            damage_voice: Table::Voice.command_at(rom, 0x2c355)?,
            death_drool: word_i(rom, 0x2c396)?,
            death_voice: Table::Voice.command_at(rom, 0x2c3e9)?,
            jitter,
            waits: words4(0x2c480)?,
            inhales: words4(0x2c8e0)?,
            drools: words4(0x2cc94)?,
            spit_sound: byte(rom, 0x2ca5f)?,
            spit_y: word_i(rom, 0x2cac6)?,
            spits,
            hit_step: word(rom, 0x2cddc)?,
            rest_step: word(rom, 0x2d106)?,
            inhale_sound: byte(rom, 0x2c937)?,
            inhale_sound_at: word_i(rom, 0x2c930)?,
            drop_max: word(rom, 0x2c8d0)? as u8,
            recoil: word(rom, 0x2c9ac)? as u8,
            laser_sound: byte(rom, 0x2cf5d)?,
            laser_ticks: word(rom, 0x2cf9e)? as u8,
            laser_start: word(rom, 0x2cfae)? as u8,
            tongue_x: word_i(rom, 0x2c708)?,
            tongue_rise: long(rom, 0x2c854)?,
            tongue_sweep: word(rom, 0x2c6d0)? as u8,
            tongue_step: word_i(rom, 0x2c7a2)?,
            tongue_drool_dx: long(rom, 0x2c678)? >> 16,
            tongue_frame: word(rom, 0x2c71a)?,
            splash_scale: [4, 4, 4, 6, 6, 5, 5, 8],
            splash_small: word(rom, 0x2cd0e)?,
            weak,
            flesh,
            eyes: [
                patch_list(pointer(rom, 0x2ce7e)?)?,
                patch_list(pointer(rom, 0x2ce92)?)?,
            ],
            vents: [
                pointer(rom, 0x2d066)? as u32,
                pointer(rom, 0x2d06a)? as u32,
                pointer(rom, 0x2d06e)? as u32,
            ],
            laser_widths,
            laser_white: words(rom, pointer(rom, 0x2d27a)?)?,
            corridor: words(rom, pointer(rom, 0x2d224)?)?,
        })
    }
}

// ---------------------------------------------------------------------------

/// Boss scratch as stored in the record (`Snapshot` exposes `vars`, so what
/// the puppets read lives there).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    /// `$40`/`$44`: plane position (16.16).
    pub(crate) x: i32,
    pub(crate) y: i32,
    /// `$FF3A00`/`$FF3A02`: wobble phase and step.
    pub(crate) phase: u16,
    pub(crate) step: u16,
    /// `$28`: script cursor (bits 14..15 select the list).
    script: u16,
    /// `$2E`, `$3C`, `$30`, `$34`.
    timer: i32,
    accel: i32,
    drool: i32,
    voice: i32,
    /// `$38`: tongue requested; `$32`: inhale requested.
    tongue_request: bool,
    inhale_request: bool,
    /// `$2C`: hit flash counter (no hits while it runs).
    flash: u8,
    /// Death jitter index.
    jitter: u8,
    /// `$2A`: eye patch counter; `$3A`: vent patch countdown; `$2D`:
    /// palette rotation counter.
    eyes: u8,
    vent: u8,
    spin: u8,
    /// Tongue: routine, 16.16 position, `$28` and its drool counter.
    t_rt: u8,
    t_x: i32,
    t_y: i32,
    t_count: u8,
    t_drool: i16,
    /// Jaw part: routine, drop (`$28`), recoil (`$30`), inhale timer (`$2E`).
    j_rt: u8,
    j_drop: u8,
    j_recoil: u8,
    j_timer: i32,
    /// Laser: active, collide ticks (`$2A`), thickness index (`$28`).
    laser: bool,
    laser_ticks: u8,
    laser_step: i8,
}

impl State {
    fn load(e: &Enemy) -> Self {
        let v = &e.vars;
        let b = |value: i32, shift: u32| (value >> shift) as u8;
        let flags = v[7];
        Self {
            x: v[0],
            y: v[1],
            phase: v[2] as u16,
            step: (v[2] >> 16) as u16,
            t_x: v[3],
            t_y: v[4],
            t_rt: b(v[5], 0),
            t_count: b(v[5], 8),
            t_drool: (v[5] >> 16) as i16,
            j_rt: b(v[6], 0),
            j_drop: b(v[6], 8),
            j_recoil: b(v[6], 16),
            tongue_request: flags & 1 != 0,
            inhale_request: flags & 2 != 0,
            laser: flags & 4 != 0,
            flash: b(flags, 8),
            timer: e.motion.vx,
            accel: e.motion.vy,
            drool: e.motion.ax,
            j_timer: e.motion.gravity,
            script: e.home_x as u16,
            voice: (e.home_x >> 16) as u16 as i32,
            laser_ticks: b(e.launch_y, 0),
            laser_step: b(e.launch_y, 8) as i8,
            jitter: b(e.step_x, 0),
            eyes: b(e.step_x, 8),
            vent: b(e.step_x, 16),
            spin: b(e.step_x, 24),
        }
    }
    fn store(&self, e: &mut Enemy) {
        let flags = i32::from(self.tongue_request)
            | i32::from(self.inhale_request) << 1
            | i32::from(self.laser) << 2
            | i32::from(self.flash) << 8;
        e.vars[..8].copy_from_slice(&[
            self.x,
            self.y,
            i32::from(self.phase) | i32::from(self.step) << 16,
            self.t_x,
            self.t_y,
            i32::from(self.t_rt)
                | i32::from(self.t_count) << 8
                | i32::from(self.t_drool as u16) << 16,
            i32::from(self.j_rt) | i32::from(self.j_drop) << 8 | i32::from(self.j_recoil) << 16,
            flags,
        ]);
        e.motion.vx = self.timer;
        e.motion.vy = self.accel;
        e.motion.ax = self.drool;
        e.motion.gravity = self.j_timer;
        e.home_x = i32::from(self.script) | i32::from(self.voice as u16) << 16;
        e.launch_y = i32::from(self.laser_ticks) | i32::from(self.laser_step as u8) << 8;
        e.step_x = (u32::from(self.jitter)
            | u32::from(self.eyes) << 8
            | u32::from(self.vent) << 16
            | u32::from(self.spin) << 24) as i32;
    }
    /// Snapshot fields the puppets read.
    fn tongue_of(vars: &[i32]) -> (u8, i32, i32) {
        (vars[5] as u8, vars[3] >> 16, vars[4] >> 16)
    }
    fn drop_of(vars: &[i32]) -> u8 {
        (vars[6] >> 8) as u8
    }
}

// ---------------------------------------------------------------------------

impl Hydra {
    /// `.loc_02C086`/`loc_02C146`: parts, health, script, position.
    fn init(&self, cx: &mut Ctx, e: &mut Enemy, s: &mut State) {
        // The record takes Data_02C138's damage/health: 15 (16 points).
        e.health = u16::from(self.body.health) + 1;
        e.hurt_routine = 0x18;
        e.death_routine = 0x1c;
        e.mapping = 0;
        e.body = Some(self.weak);
        e.facing_left = false;
        *s = State {
            x: self.start.0,
            y: self.start.1,
            step: self.rest_step,
            script: self.first_script,
            t_rt: T_IDLE,
            j_rt: J_IDLE,
            laser_step: -1,
            ..State::default()
        };
        let (x, y) = (e.x, e.y);
        for role in [ROLE_FACE, ROLE_NECK, ROLE_JAW, ROLE_TONGUE] {
            if let Some(part) = cx.spawn(PART, x, y) {
                part.vars[ROLE] = role;
                part.routine = 4;
                part.tile = Some(0x2000);
            }
        }
    }
    /// `loc_02C18A`: the next script command.
    fn next(&self, cx: &mut Ctx, e: &mut Enemy, s: &mut State) {
        for _ in 0..16 {
            let list = &self.scripts[usize::from(s.script >> 14) & 3];
            let index = usize::from(s.script & 0x3fff);
            let command = match list.get(index) {
                Some(&b) if b & 0x80 == 0 => b,
                _ => {
                    s.script = 0;
                    match self.main_script.first() {
                        Some(&b) if b & 0x80 == 0 => b,
                        _ => return,
                    }
                }
            };
            s.script = s.script.wrapping_add(1);
            match command {
                0 | 4 => {
                    e.routine = 4;
                    if s.x > self.rest_x {
                        return;
                    }
                }
                8 => {
                    e.routine = 8;
                    self.spit(cx, e, s);
                }
                0x0c => {
                    e.routine = 0x0c;
                    s.tongue_request = true;
                    return;
                }
                0x10 => {
                    // .loc_02C23C
                    e.routine = 0x10;
                    s.drool = self.drools[level(cx)];
                    s.timer = self.dive_dx;
                    // `$2C23C` falls through the first dive update: its
                    // zero velocity leaves Y still and advances gravity.
                    s.accel = self.dive_accel;
                    self.drool(cx, &mut s.drool, e.x, 0x40);
                    cx.effect_sound(self.entry_sound);
                    return;
                }
                0x14 => {
                    // .loc_02C2A0: surface at the player's X - 64.
                    e.routine = 0x14;
                    s.drool = self.drools[level(cx)];
                    let mut x = cx.player.0 - self.rise_offset;
                    if x < 0 {
                        x = self.rise_min_x;
                    }
                    x = x.min(self.rest_x >> 16) - cx.camera.0;
                    s.x = set_whole(s.x, x);
                    cx.effect_sound(self.entry_sound);
                    // `$2C2A0` falls through `$2C2D6` on entry too.
                    self.drool(cx, &mut s.drool, e.x, 0x40);
                    return;
                }
                0x20 => {
                    e.routine = 0x20;
                    // `$2C45C` falls through `$2C474` and consumes the
                    // first wait tick in the command's entry frame.
                    s.timer = self.waits[level(cx)] - 1;
                    return;
                }
                0x24 => {
                    e.routine = 0x24;
                    s.inhale_request = true;
                    return;
                }
                _ => return,
            }
        }
    }
    /// `Func_02CA5E`: HYDRA SHOT, the vent patches (`Func_02D03E`), three
    /// balls (`Func_02CAA2`, `$28` = `$20`), run by [`Puppet`] as
    /// `ROLE_SPIT` parts.
    fn spit(&self, cx: &mut Ctx, e: &Enemy, s: &mut State) {
        cx.effect_sound(self.spit_sound);
        s.vent = 0x2f;
        let r = self.spit_record;
        for spit in &self.spits {
            let (x, y) = (e.x + spit.dx, e.y + self.spit_y + spit.dy);
            let Some(ball) = cx.spawn(PART, x, y) else {
                break;
            };
            ball.vars[ROLE] = ROLE_SPIT;
            ball.vars[SPIT_TICKS] = SPIT_HIGH;
            ball.routine = 4;
            ball.tile = Some(r.tile);
            ball.mapping = r.frame;
            ball.high_priority = true;
            ball.motion = Motion {
                vx: spit.vx,
                vy: spit.vy,
                ax: spit.ax,
                gravity: spit.ay,
                ..Motion::default()
            };
        }
    }
    /// `Func_02CB72`: every fourth call a flesh splash from the floor below
    /// (x + `dx`), up to 135 degrees either side of straight up.
    fn drool(&self, cx: &mut Ctx, counter: &mut i32, x: i32, dx: i32) {
        *counter -= 1;
        if *counter & 3 != 0 {
            return;
        }
        let r = cx.random();
        let angle = (r & 0xff) + 0x180;
        let (cos, sin) = cx.shared.sine.polar(angle);
        let k = self.splash_scale[usize::from((r & 0x1c) >> 2)];
        let jitter = (i32::from(r as i16) << 6) >> 16;
        let (mut vx, mut vy) = (sin * k, cos * k);
        let small = r & 1 != 0;
        let (mapping, base) = if small {
            vx >>= 2;
            vy >>= 2;
            (self.splash_small, 0x8000)
        } else {
            (self.splash.frame, self.splash.tile)
        };
        let mut piece = Effect::new(EffectKind::Pieces, x + dx + jitter, 0x1f0, mapping);
        piece.base = base;
        // Both tile words (`Data_02CB64`'s `$E000`, the small one's `$8000`)
        // are high priority: the splash shows over the corridor floor.
        piece.high_priority = base & 0x8000 != 0;
        piece.counter = if small { 20 } else { 255 };
        piece.motion = Motion {
            vx,
            vy,
            ax: i32::from(((-vx) >> 6) as i16),
            gravity: 0x4000,
            ..Motion::default()
        };
        cx.effect(piece);
    }
    /// `Func_02CD94` on a hit: line 1 white (`Func_02CE4C`), flash,
    /// tongue, open jaw (`Func_02CD6E`), wobble.
    fn struck(&self, cx: &mut Ctx, s: &mut State) {
        cx.fill_palette(1, FLASH);
        s.flash = 1;
        s.tongue_request = true;
        cx.plane_op(PlaneOp::WordMap(JAW_OPEN));
        s.step = self.hit_step;
    }
    /// `Func_02CE0A`: palette line 1 for `health`.
    pub(super) fn health_palette(&self, cx: &mut Ctx, health: u8) {
        self.flesh.show(cx, usize::from(health));
    }
    /// `Func_02CD80`: shut the jaw, then the wound for `health`
    /// (`loc_02D0B8`).
    pub(super) fn shut_jaw(&self, cx: &mut Ctx, health: u8) {
        cx.plane_op(PlaneOp::WordMap(JAW_SHUT));
        let wound = WOUNDS
            .iter()
            .find(|&&(below, _)| health < below)
            .map_or(WOUNDS[2].1, |&(_, patch)| patch);
        cx.plane_op(PlaneOp::WordMap(wound));
    }
    /// `Func_02D130`: plane B H-scroll per 8-line band (the head's X
    /// against the camera plus a 4 px sine, `$FF3A02` apart) and V-scroll
    /// per 2-cell column (its Y, a sine `$40` apart per column).
    fn scroll(&self, cx: &Ctx, s: &State) -> PlaneOp {
        let h = (s.x >> 16) - cx.camera.0;
        let v = cx.camera.1 - (s.y >> 16) - 0x110;
        let horizontal = SineWave::shifted(s.phase, s.step, 12);
        let vertical = SineWave::shifted(s.phase.wrapping_mul(2).wrapping_add(0x100), 0x40, 12);
        let bands = std::array::from_fn(|i| (h + horizontal.offset(i, &cx.shared.sine)) as i16);
        let columns = std::array::from_fn(|i| (v + vertical.offset(i, &cx.shared.sine)) as i16);
        PlaneOp::Scroll {
            plane: Plane::B,
            bands,
            columns,
        }
    }
    /// Fade steps until the flash reaches the health palette.
    fn fade_done(&self, health: u8, steps: u8) -> bool {
        let need = self
            .flesh
            .colors(usize::from(health))
            .iter()
            .map(|&c| 15 - ((c >> 1) & 7).min((c >> 5) & 7).min((c >> 9) & 7) * 2)
            .max()
            .unwrap_or(0);
        u16::from(steps) > need
    }
    /// `Func_02CEA8`: wobble, flash fade (`$6E8` every 8 updates, then the
    /// jaw shuts), palette rotation, vent patches, plane B scroll, position.
    fn common(&self, cx: &mut Ctx, e: &mut Enemy, s: &mut State, health: u8) {
        if s.step != self.rest_step {
            s.step = s.step.wrapping_sub(2);
        }
        if s.flash != 0 {
            s.flash = s.flash.wrapping_add(1);
            if s.flash & 7 == 0 {
                self.flesh.fade(cx, usize::from(health));
                if self.fade_done(health, s.flash >> 3) {
                    s.flash = 0;
                    self.shut_jaw(cx, health);
                    if e.routine == 0x20 {
                        s.timer = 1;
                    }
                }
            }
        }
        // Func_02D072: every 8th update out of the flash and the drift,
        // line 1 colours 12..15 rotate.
        if s.flash == 0 && e.routine != 4 {
            s.spin = s.spin.wrapping_add(1);
            if s.spin & 7 == 0 {
                cx.palette_op(PaletteOp::Rotate(1, 12, 4));
            }
        }
        // Func_02D044: the vent patch of the countdown's quarter.
        if s.vent != 0 {
            let patch = self.vents[usize::from(s.vent >> 4).min(2)];
            s.vent -= 1;
            cx.plane_op(PlaneOp::WordMap(patch));
        }
        cx.plane_op(self.scroll(cx, s));
        s.phase = s.phase.wrapping_add(8);
        e.x = s.x >> 16;
        e.y = (s.y >> 16) + 0x110;
    }
}

impl Behavior for Hydra {
    fn label(&self, _kind: u16) -> String {
        format!("Boss3_Hydra ${HANDLER:05X}")
    }
    fn integrates(&self) -> bool {
        false
    }
    /// `Func_02CD94` tests weapons only; the parts' boxes hurt Joe.
    fn touches_player(&self) -> bool {
        false
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        let s = State::load(e);
        if s.flash != 0 {
            return HitResponse::Pass;
        }
        match e.routine {
            4 | 0x0c | 0x10 | 0x20 => HitResponse::Accept,
            0x14 if s.drool < 0 => HitResponse::Accept,
            _ => HitResponse::Pass,
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        let mut s = State::load(e);
        if e.routine == 0 {
            self.init(cx, e, &mut s);
            e.state = EnemyState::Waiting;
            e.routine = 4;
            self.next(cx, e, &mut s);
        }
        match cx.process_hit(e) {
            Some(Reaction::Hurt) => {
                // loc_02C330: routine $18 (process_hit saved the old one).
                self.struck(cx, &mut s);
                s.voice = self.hurt_ticks;
            }
            Some(Reaction::Death) => {
                // loc_02C380
                self.struck(cx, &mut s);
                s.jitter = 0;
                s.drool = self.death_drool;
            }
            _ => {}
        }
        let health = e.health.saturating_sub(1).min(15) as u8;
        let dying = e.routine == 0x1c;
        match e.routine {
            4 => {
                // .loc_02C1F8
                if s.x <= self.rest_x {
                    self.next(cx, e, &mut s);
                } else {
                    s.x -= 1 << 16;
                }
            }
            0x0c => {
                if !s.tongue_request {
                    self.next(cx, e, &mut s);
                }
            }
            0x10 => {
                // .loc_02C258: dive below the floor.
                let x = e.x;
                self.drool(cx, &mut s.drool, x, 0x40);
                s.y += s.accel;
                s.accel += self.dive_accel;
                if s.y >= self.floor_y {
                    s.x = set_whole(s.x, self.dive_end_x);
                    self.next(cx, e, &mut s);
                } else {
                    s.x += s.timer << 4;
                }
            }
            0x14 => {
                // .loc_02C2D6: splashes, then rise at 1 px/tick.
                let x = e.x;
                let rising = s.drool < 0;
                self.drool(cx, &mut s.drool, x, 0x40);
                if rising {
                    let mut moved = false;
                    if s.y > self.top_y {
                        s.y -= 1 << 16;
                        moved = true;
                    }
                    if (s.x >> 16) < (self.rest_x >> 16) {
                        s.x += 1 << 16;
                        moved = true;
                    }
                    if !moved {
                        self.next(cx, e, &mut s);
                    }
                }
            }
            0x18 => {
                // loc_02C348: HYDRA DAMAGE after 90 ticks; resume when the
                // tongue is back.
                if s.voice > 0 {
                    s.voice -= 1;
                    if s.voice == 0 {
                        cx.play(self.damage_voice);
                    }
                }
                if !s.tongue_request {
                    e.routine = e.saved_routine;
                    if e.routine == 0x20 {
                        s.timer = 1;
                        self.health_palette(cx, health);
                    }
                }
            }
            0x1c => {
                // loc_02C39A: rise, HYDRA DEATH, sink shaking, round clear.
                cx.player_event(crate::movement::PlayerEvent::Freeze);
                let x = e.x;
                if s.drool >= 0 {
                    if s.y > self.top_y {
                        s.y -= 1 << 16;
                    }
                    self.drool(cx, &mut s.drool, x, 0x40);
                    if s.drool < 0 {
                        cx.effect_sound(self.entry_sound);
                        cx.play(self.death_voice);
                    }
                } else if s.y > self.floor_y {
                    // Func_02C4DE: round clear; Func_0045AA clears plane B.
                    cx.next_substage();
                    cx.plane_op(PlaneOp::Fill {
                        vram: PLANE_B,
                        width: 64,
                        height: 64,
                        cell: 0,
                    });
                    s.store(e);
                    Ctx::free(e);
                    return;
                } else {
                    s.y += 1 << 16;
                    let (jx, jy) = self.jitter[usize::from(s.jitter >> 1) & 7];
                    s.x += jx << 16;
                    s.y += jy << 16;
                    s.jitter = s.jitter.wrapping_add(2);
                    self.drool(cx, &mut s.drool, x, 0x40);
                }
            }
            0x20 => {
                s.timer -= 1;
                if s.timer <= 0 {
                    self.next(cx, e, &mut s);
                }
            }
            0x24 => {
                // loc_02C4F2: drift to $B8 while the jaw inhales.
                if s.x >= self.rest_x {
                    s.x -= 1 << 16;
                }
                if !s.inhale_request {
                    self.next(cx, e, &mut s);
                }
            }
            _ => {}
        }
        // Player_CheckDamage visits list 4 before the boss handler moves
        // the tongue. Keep that pre-update box for this frame's shot test.
        let tongue_before = (s.t_rt, s.t_x >> 16, s.t_y >> 16);
        self.tongue(cx, &mut s, dying);
        self.jaw(cx, &mut s, dying, health);
        self.laser_tick(cx, &mut s);
        self.common(cx, e, &mut s, health);
        // .loc_02CE78: the eye patches, except while the tongue of a hit is
        // out and in death (those routines branch to Func_02CEA8 alone).
        // The patch loop ends in `beq.s Func_02CEA8`, so every other routine
        // runs Func_02CEA8 twice a frame: the wobble phase moves 16, the
        // step decays 4, and the flash, rotation and vent counters run at
        // double rate (`$FF3A00` +$10 per frame in emulator traces).
        if !matches!(e.routine, 0x18 | 0x1c) {
            s.eyes = s.eyes.wrapping_add(1);
            for &patch in &self.eyes[usize::from(s.eyes & 8 != 0)] {
                cx.plane_op(PlaneOp::WordMap(patch));
            }
            self.common(cx, e, &mut s, health);
        }
        // Func_037244 after each part's contact: its box shoves Joe out,
        // whatever his collision flags (combat::shove), so also in death.
        for part in [self.face, self.neck, self.chin] {
            let at = (e.x + part.x, e.y + part.y);
            let part = mapping_hitbox(cx.mappings, part.frame, at.0, at.1, false);
            cx.signal(Signal::Shove(part));
        }
        // Contact: face, neck and chin boxes (Func_02C5AE, body contact)
        // and the list-4 tongue; the player is excluded from collision
        // while the boss dies.
        if !dying {
            let (x, y) = (e.x, e.y);
            for part in [self.face, self.neck, self.chin] {
                let at = (x + part.x, y + part.y);
                hurtbox(cx, at, part.frame, part.damage, false, true, false);
            }
            if tongue_before.0 != T_IDLE {
                let t = self.tongue;
                let at = (tongue_before.1, tongue_before.2);
                let left = t.tile & 0x800 != 0;
                hurtbox(cx, at, self.tongue_frame, t.damage, left, false, true);
            }
        }
        s.store(e);
    }
}
