//! Round 7 boss: `Boss7` (`$33102`), called through `Boss_Jumps` (entries 7
//! and 8: round 8 repeats it). 7-3: the final boss on the grid floor.
//!
//! The handler is a fighter driven by frame scripts (`Func_0346E0`,
//! `Data_03474E`: 24 lists of (frame - `$2F1`, ticks) bytes). Natively the
//! list-8 record keeps the ROM fields in `Enemy::vars` (see the constants
//! below) and [`Boss7::update`] follows the `obj_routine` table:
//!
//! | Routine | Address | Native |
//! | --- | --- | --- |
//! | 0 | `$33160` | set up: arena `[camera X + $18, + $120]`, drop from the top of the screen, health 13 |
//! | 4 | `$331D6` | fall, landing script, then the intro object (projectile `$17`, `$1F970`) |
//! | 8 | `$33212` | the intro: H-scroll wave (VDP `$0B` = 3), palette lines 2-3 fade to `Data_01FB44`, `Boss_LoadArt` round + 1 and VDP `$0B` = 7, fade back |
//! | `$C` | `$3323A` | stand 36 ticks (12 in the second form), parrying every weapon, then pick an attack |
//! | `$10` | `$33344` | attack (below); the only routine in which weapons hurt it |
//! | `$14` | `$33C50` | parry pose after a blocked weapon; a second block breaks into `$2FA/$2FB` |
//! | `$18` | `$33CD6` | MIJIN: white-out of palette lines 1-3, fade to `Data_036560` row 0, aura `$19` (140 ticks), second form |
//! | `$20` | `$33FC2` | knocked back (`Func_0345C4`); on landing MIJIN, death or stand |
//! | `$24` | `$34072` | final white-out with explosions all over the screen |
//! | `$2C` | `$34140` | 128 ticks, then the round clear (`Game_Requests` bit 1) |
//!
//! Attacks, first form: run in (8 px/tick) and cut, leaving the `$1E462`
//! blade that bursts into 16 pieces; four volleys of three shurikens
//! (projectile `$10`); a dash at 11 px/tick braking inside 200 px; a
//! retreating jump that fans eight shurikens down at the player (`$11`).
//! Second form adds a rising uppercut with an aura (`$1A` variant 1) when the
//! player jumps close, a spinning disc firing five horizontal shots (`$1A`
//! variant 0, `$1B`), the shadow wave (`$1C`), and alternately a hover that
//! rains 72 energy shots (`$1B`, `Data_0339F2`) or a diving leap (`$1A`
//! variant 2). Each hit costs 1 point natively the weapon's damage (the
//! cartridge: 1, or 3 for the player's powered or special attacks); four hits
//! within 28 ticks of each other knock it back; 14 points end the first form
//! (MIJIN), 41 more kill it. Score: the `Boss_Records` 3000, added once, at
//! the lethal hit.
//!
//! `+$43` is not a hit counter: it is the `Data_036560` palette row of line
//! 1 (word 0 = 1 before MIJIN, decremented to 0 after), read by the `$38CEE`
//! hit flash. Intro/MIJIN apply their art uploads, CRAM fades and plane
//! replacement through the shared presentation operations; VDP mode 7
//! enables the palette cycle and two-plane/column wobble.
//!
//! Player states drive the close-range shove, broken-guard recoil,
//! shadow-wave alignment, teleport and death freeze. Shurikens
//! use the direction-specific ROM frames at `$1EA02`/`$1EA8E`, whose empty
//! hitboxes collide as points. Body contact (1 point, 2 on `$309/$30A`)
//! runs before pose changes and cannot be blocked; hit flashes only change
//! the palette. The attached auras persist through their hidden blink ticks.
//! The MIJIN pillar follows `$37D1C`'s growing and releasing section chain.
//! Delayed dash poses remain short-lived native effects.
use super::{cap_speed, half_turn, unit_vector};
use crate::{
    combat::{Enemy, EnemyState, Motion, PaletteOp, Projectile, Reaction, Spawn},
    enemies::{
        Behavior, Ctx, HitResponse, Roster,
        round7::{ATTACHED, animate},
        shot_damage,
    },
    game::Terrain,
    rom::{Result, byte, long, signed, signed_byte, word, word_i},
    sound_events::Table,
};
use std::sync::Arc;

mod attacks;
mod hits;
mod load;
mod objects;
mod pillar;
mod sequences;

/// `Boss_Jumps` target of this round.
pub(crate) const HANDLER: usize = 0x33102;

pub(crate) fn fight(rom: &[u8]) -> Result<Arc<dyn Behavior>> {
    Ok(Arc::new(Boss7::load(rom)?))
}

/// List-5 types this boss spawns and owns: none (its parts are list 4/1/9).
pub(crate) const PART_KINDS: &[u16] = &[];

pub(crate) fn register_parts(_rom: &[u8], _roster: &mut Roster) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------

/// `Enemy::vars`: `+$28` countdown, `+$2A` sub-state, the script
/// (`+$2C`/`+$2E`/`+$30` packed), the helper object slot (position; kind
/// and age), flags, `+$07` health, `+$44` second sub-state.
const COUNT: usize = 0;
const SUB: usize = 1;
const ANIM: usize = 2;
const SLOT_POS: usize = 3;
const SLOT: usize = 4;
const FLAGS: usize = 5;
const HEALTH: usize = 6;
const STEP: usize = 7;
const SHOVE_TIMER: usize = 8;
const SLOT_FRAME: usize = 9;
// `Enemy::home_x` is `+$36` (arena left), `counter` `+$3C` (rain index),
// `phase` `+$32` (shots/explosions counted for sounds), `timer` the intro
// wave amplitude, `step_x` the packed MISC fields and `launch_y` the packed
// fade steps.

/// (shift, bits) of a packed field.
type Field = (u32, u32);
const A_SCRIPT: Field = (0, 8);
const A_CURSOR: Field = (8, 8);
const A_TICKS: Field = (16, 8);
const M_PARRIES: Field = (0, 4);
const M_COMBO: Field = (4, 4);
const M_COMBO_TIMER: Field = (8, 8);
const M_DEFLECT: Field = (16, 8);
const M_CYCLE: Field = (24, 2);
const S_AGE: Field = (0, 16);
const S_KIND: Field = (16, 4);
const S_FLIP: Field = (20, 1);
/// Nibble steps palette lines 1-3 have faded toward white (`Func_00075C`,
/// MIJIN and the final fade) and lines 2-3 toward `Data_01FB44` (intro).
const P_WHITE: Field = (8, 4);
const P_INTRO: Field = (12, 4);

fn get(value: i32, (shift, bits): Field) -> i32 {
    (value >> shift) & ((1 << bits) - 1)
}
fn put(value: i32, (shift, bits): Field, field: i32) -> i32 {
    let mask = ((1 << bits) - 1) << shift;
    (value & !mask) | ((field << shift) & mask)
}

// Flags (obj_flags, `+$40` and `+$06` bits of the cartridge).
/// obj_flags bit 1: second form (after MIJIN).
const PHASE2: i32 = 1 << 0;
/// obj_flags bit 3: guard broken by a second block.
const BROKEN: i32 = 1 << 1;
/// obj_flags bit 4: the landing script after a jump was played.
const LANDED: i32 = 1 << 2;
/// obj_flags bit 5: restart the script.
const RESTART: i32 = 1 << 3;
/// obj_flags bit 6: the script ended.
const DONE: i32 = 1 << 4;
/// obj_flags word bit 1: a helper object finished or hit the player.
const SIGNAL: i32 = 1 << 5;
/// `+$40` bit 0: running in / wind-up pending.
const RUN: i32 = 1 << 6;
/// `+$40` bit 1: MIJIN pending (no damage).
const MIJIN: i32 = 1 << 7;
/// `+$40` bit 2: dying (no collision).
const DYING: i32 = 1 << 8;
/// `+$40` bit 6: retreated before MIJIN.
const RETREAT: i32 = 1 << 9;
/// `+$06` bit 0: no hit processing (uppercut).
const GUARDLESS: i32 = 1 << 10;
/// `+$06` bit 1: MIJIN reached (second health).
const SECOND: i32 = 1 << 11;
/// `$1A` variants 1..3 attached.
const AURA1: i32 = 1 << 12;
const AURA2: i32 = 1 << 13;
const AURA3: i32 = 1 << 14;
// Helper object kinds of the slot.
const BLADE: i32 = 1;
const SPINNER: i32 = 2;
const WAVE: i32 = 3;
const WAVE_OUT: i32 = 4;
const AURA: i32 = 5;

/// `Func_034B04`: velocities limited to 8 px/tick.
const MAX_SPEED: i32 = 0x80000;
/// Native health pool of the first form: its points are counted in
/// `vars[HEALTH]`, so only the second form's lethal hit scores.
const POOL: u16 = 0x4000;
/// `Object_ListParams+2` = `$17` at `$331AC`: list 4 holds 24 records.
const LIST4: usize = 24;

// ---------------------------------------------------------------------------

/// Frame and remaining ticks (1 = last) of `script` at tick `t`.
fn frame_at(script: &[(u16, u16)], t: i32) -> Option<(u16, i32)> {
    let mut start = 0;
    for &(frame, ticks) in script {
        let end = start + i32::from(ticks);
        if t < end {
            return Some((frame, end - t));
        }
        start = end;
    }
    None
}

pub(crate) struct Boss7 {
    base: u16,
    player_classes: [u8; 74],
    first_palette: [u16; 16],
    second_palette: [u16; 16],
    scripts: Vec<Vec<(u16, u16)>>,
    /// `Data_034A00`: (cos, sin) 4.12 over half a turn (`Func_01F820`).
    half: Vec<(i32, i32)>,
    voices: Vec<u8>,
    first: u16,
    arena: i32,
    span: i32,
    fall_gravity: i32,
    intro_script: i32,
    health: i32,
    intro_sound: u8,
    /// `$34E2`: `Func_00346A`'s VDP register `$07` value (`$8720`).
    backdrop: u8,
    idle: i32,
    near: i32,
    slash_script: i32,
    throw_script: i32,
    spin_script: i32,
    wave_script: i32,
    run_speed: i32,
    run_min: i32,
    run_frame: u16,
    after_slash: i32,
    blade_frame: u16,
    step_frame: u16,
    step_start: i32,
    step_vx: i32,
    step_stop: i32,
    throw_frame: u16,
    throw_sound: u8,
    throw_count: i32,
    dash_after_throw: i32,
    jump_script: i32,
    windup_frame: u16,
    windup_voice: u8,
    dash_script: i32,
    dash_vx: i32,
    afterimage: u16,
    brake_range: i32,
    brake: i32,
    dash_end: u16,
    landing_script: i32,
    jump_frame: u16,
    jump_vx: i32,
    margin: i32,
    room: i32,
    far_side: i32,
    jump_vy: i32,
    gravity: i32,
    jump_sound: u8,
    jump2_frame: u16,
    jump2_vx: i32,
    jump2_vy: i32,
    fan_cursor: i32,
    fan_sound: u8,
    fan_count: i32,
    uppercut_frame: u16,
    uppercut_vy: i32,
    uppercut_gravity: i32,
    uppercut_range: i32,
    uppercut_script: i32,
    uppercut_vx: i32,
    hold_frame: u16,
    hold: i32,
    wave_sound: u8,
    wave_end_script: i32,
    wave_hit_sound: u8,
    hover_script: i32,
    leap_script: i32,
    hover: i32,
    hover_sound: u8,
    hover_vy: i32,
    death_step: i32,
    death_voice: u8,
    rain_interval: i32,
    rain_rise: i32,
    rain_sound: u8,
    rain: Vec<usize>,
    leap: [(i32, i32, i32, i32); 3],
    dive_script: i32,
    dive_vx: i32,
    dive_vy: i32,
    bounce_script: i32,
    bounce_vx: i32,
    bounce_vy: i32,
    retreat: i32,
    mijin_frame: u16,
    mijin_step: i32,
    mijin_sound: u8,
    mijin_after: u16,
    aura_sound: u8,
    hurt_frame: u16,
    second_health: i32,
    knock_wait: i32,
    knock_vx: i32,
    knock_vy: i32,
    death_script: i32,
    fade_step: i32,
    end_sound: u8,
    boom_sound: u8,
    combo_window: i32,
    combo_max: i32,
    deflect_cooldown: i32,
    deflect_sound: u8,
    broken_script: i32,
    broken_voice: u8,
    high_script: i32,
    low_range: i32,
    low_script: i32,
    flash: (u16, u16),
    guard: [u16; 2],
    star_damage: u8,
    star_frames: [[u16; 2]; 3],
    fan_frames: [[u16; 2]; 8],
    stars: [((i32, i32), (i32, i32)); 3],
    fan: [((i32, i32), (u8, u32)); 8],
    blade: (i32, i32, i32, i32, u8, u8),
    pieces: Vec<(i32, i32)>,
    piece_damage: u8,
    energy: [(u8, u16); 9],
    energy_damage: u8,
    energy_rise: i32,
    spinner_script: i32,
    spinner_shot: u16,
    spinner_sound: u8,
    wave: (i32, i32, i32, i32, u8),
    wave_frames: [u16; 2],
    wave_drop: i32,
    wave_hold: u16,
    run_voice: u8,
    spin_voice: u8,
    broken_frame: u16,
    broken_cry: u8,
    aura_frames: [u16; 5],
    aura_offsets: [(i32, i32); 4],
    aura_damage: u8,
    aura_voices: [u8; 2],
    aura_ticks: i32,
    aura_frame: u16,
    pillar_frame: u16,
    pillar_spacing: i32,
    pillar_velocity: i32,
    dive_sparks: Vec<(i32, i32, u16, u32)>,
    dive_spark_script: Vec<(u16, u16)>,
    boom_script: i32,
}

impl Boss7 {
    fn script(&self, offset: i32) -> &[(u16, u16)] {
        self.scripts
            .get((offset / 4) as usize)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    /// Select script `offset` from the next `animate` (`bset #5` + `$2C`).
    fn start(e: &mut Enemy, offset: i32) {
        e.vars[ANIM] = put(e.vars[ANIM], A_SCRIPT, offset);
        e.vars[FLAGS] |= RESTART;
    }
    fn ticks(e: &Enemy) -> i32 {
        get(e.vars[ANIM], A_TICKS)
    }
    fn cursor(e: &Enemy) -> i32 {
        get(e.vars[ANIM], A_CURSOR)
    }
    fn done(e: &Enemy) -> bool {
        e.vars[FLAGS] & DONE != 0
    }
    /// `Func_0346E0`: one tick of the current script.
    fn animate(&self, e: &mut Enemy) {
        let anim = e.vars[ANIM];
        let script = self.script(get(anim, A_SCRIPT));
        let mut cursor = get(anim, A_CURSOR);
        let mut ticks = get(anim, A_TICKS);
        if e.vars[FLAGS] & RESTART != 0 {
            e.vars[FLAGS] &= !(RESTART | DONE);
            cursor = 0;
        } else {
            if e.vars[FLAGS] & DONE != 0 {
                return;
            }
            ticks -= 1;
            if ticks > 0 {
                if let Some(&(frame, _)) = script.get(cursor as usize) {
                    e.mapping = frame;
                }
                e.vars[ANIM] = put(anim, A_TICKS, ticks);
                return;
            }
            cursor += 1;
        }
        match script.get(cursor as usize) {
            Some(&(frame, t)) => {
                ticks = i32::from(t);
                e.mapping = frame;
            }
            None => e.vars[FLAGS] |= DONE,
        }
        let anim = put(anim, A_CURSOR, cursor);
        e.vars[ANIM] = put(anim, A_TICKS, ticks);
    }
    /// `Func_0344C6`: flip (`$800`) set when the player is to the right.
    fn face(cx: &Ctx, e: &mut Enemy) {
        e.facing_left = !cx.player_left(e.x);
    }
    /// +1 toward the flipped side.
    fn sign(e: &Enemy) -> i32 {
        if e.facing_left { 1 } else { -1 }
    }
    /// Player status bit 5; a jump can start below the boss's feet.
    fn airborne(cx: &Ctx, _e: &Enemy) -> bool {
        cx.player_airborne
    }
    fn voice(&self, cx: &mut Ctx, id: u8) {
        if let Some(&command) = self.voices.get(usize::from(id)) {
            cx.play(command);
        }
    }
    fn misc(e: &Enemy, field: Field) -> i32 {
        get(e.step_x, field)
    }
    fn set_misc(e: &mut Enemy, field: Field, value: i32) {
        e.step_x = put(e.step_x, field, value);
    }
    fn present(e: &Enemy, field: Field) -> i32 {
        get(e.launch_y, field)
    }
    fn set_present(e: &mut Enemy, field: Field, value: i32) {
        e.launch_y = put(e.launch_y, field, value);
    }
    /// `Func_034696`: kept inside the arena; lands on terrain (Y & ~7).
    fn ground(&self, e: &mut Enemy, terrain: &dyn Terrain) {
        let left = e.home_x;
        let right = left + self.span;
        if e.x < left || e.x > right {
            e.x = e.x.clamp(left, right);
            e.motion.vx = 0;
            e.motion.ax = 0;
        }
        if terrain.solid_at(e.x, e.y) {
            e.y &= !7;
            e.motion.vy = 0;
            e.motion.gravity = 0;
        }
    }
    /// Back to routine `$C`: stand `+$28` = 35.
    fn stand(&self, e: &mut Enemy) {
        e.routine = 0x0c;
        e.vars[COUNT] = self.idle;
        Self::start(e, 0);
    }
    fn slot_kind(e: &Enemy) -> i32 {
        get(e.vars[SLOT], S_KIND)
    }
    fn open_slot(e: &mut Enemy, kind: i32, at: (i32, i32), flip: bool) {
        e.vars[SLOT_FRAME] = 0;
        e.vars[SLOT_POS] = i32::from(at.0 as u16) | (i32::from(at.1 as u16) << 16);
        let slot = put(0, S_KIND, kind);
        e.vars[SLOT] = put(slot, S_FLIP, i32::from(flip));
    }
    fn slot_at(e: &Enemy) -> (i32, i32) {
        let pos = e.vars[SLOT_POS];
        (
            i32::from(pos as u16 as i16),
            i32::from((pos >> 16) as u16 as i16),
        )
    }
}

impl Behavior for Boss7 {
    fn label(&self, _kind: u16) -> String {
        format!("Boss7 ${HANDLER:05X}")
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        let flags = e.vars[FLAGS];
        if flags & (DYING | GUARDLESS) != 0 {
            return HitResponse::Pass;
        }
        match e.routine {
            0x0c | 0x14 => HitResponse::Deflect,
            0x10 if self.guard.contains(&e.mapping) => HitResponse::Deflect,
            0x10 if flags & MIJIN != 0 => HitResponse::Ignore,
            0x10 => HitResponse::Accept,
            _ => HitResponse::Pass,
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        let me = cx.current;
        cx.projectiles
            .retain(|p| !(p.owner == Some(me) && p.life == ATTACHED));
        if e.routine == 0 {
            self.init(cx, e);
            return;
        }
        self.ground(e, terrain);
        self.body_contact(cx, e);
        self.hits(cx, e);
        self.special_reaction(cx, e, terrain);
        self.shove(cx, e, terrain);
        let timer = Self::misc(e, M_COMBO_TIMER);
        if timer > 0 {
            Self::set_misc(e, M_COMBO_TIMER, timer - 1);
        }
        let deflect = Self::misc(e, M_DEFLECT);
        if deflect > 0 {
            Self::set_misc(e, M_DEFLECT, deflect - 1);
        }
        match e.routine {
            4 => self.fall(cx, e),
            8 => {
                // $1FA7E: `Func_00346A` sets the backdrop to CRAM line 2
                // colour 0 (VDP register $07 = $20) with the new art.
                if e.vars[SUB] == 2 {
                    cx.palette_op(PaletteOp::Backdrop(self.backdrop));
                }
                self.intro(cx, e)
            }
            0x0c => self.idle(cx, e),
            0x10 => self.attack(cx, e),
            0x14 => self.parry_pose(cx, e),
            0x18 => self.mijin(cx, e),
            0x1c => self.teleport(cx, e),
            0x20 => self.knocked(cx, e),
            0x24 => self.fade_out(cx, e),
            0x28 => {
                cx.game_flag(0x1000);
                cx.player_event(crate::movement::PlayerEvent::Freeze);
            }
            _ => {
                self.finish(cx, e);
                return;
            }
        }
        self.slot(cx, e);
        self.attachments(cx, e);
        self.pillar(cx, e);
        // $38BE0 keeps delayed poses behind a fast dash. Native effects
        // retain each pose briefly rather than allocating a history object.
        if e.routine == 0x10
            && e.motion.vx.abs() >= 0x80000
            && e.age.is_multiple_of(3)
            && e.mapping != 0
        {
            let mut ghost =
                crate::combat::Effect::new(crate::combat::EffectKind::Ghost, e.x, e.y, e.mapping);
            ghost.base = 0x2000;
            ghost.flip = e.facing_left;
            ghost.back = true;
            ghost.counter = 6;
            cx.effect(ghost);
        }
        if e.vars[FLAGS] & SECOND == 0 && e.live() {
            e.health = POOL;
        }
    }
}
