//! Round 2 boss: `Boss2` (`$2B104`), called through `Boss_Jumps`.
//!
//! 2-3 base corridor: the flying machine with its energy orb. It spirals in
//! as four flickering images, hovers across the upper screen easing toward
//! a point it re-picks every 33 frames, then vanishes and reappears for one
//! of three attacks, chosen at random (`Func_02B53A`):
//!
//! - `$2C` = 0, routines 5-6: charges up (CHARGE, 96 frames), then four
//!   times fires a vertical energy beam (POWER SHOT, list-4 `$33` routine
//!   `$38`, LASER L) and drifts 32 frames toward the player.
//! - `$2C` = 2, routines 7, 8, 10: a white flash turns the player around
//!   (YAJIMA POWER), the screen waves, and the boss hovers above the player,
//!   alternating sides every 32 frames and firing diagonal shots (GUN);
//!   a second flash ends it.
//! - `$2C` = 4, routine 9: follows the player, drops to Y `$1A8` (MASAO
//!   MISSILE), throws four orbs that rest on a line and burst into eight
//!   shots once it hovers again, and rises behind a flash.
//!
//! While charging, flashing or following (`+$2A` bit 7) a field of half
//! extents `$60` x `$30` around it turns weapons away (`$2B126`, [`HitResponse::Reflect`], #91):
//! it clears the weapon's collision bit, so nothing is spent or heard (no
//! `$24DE0` voice, no `$368EC` deflect), and a shuriken not yet turned is
//! set to routine `$10` (`Func_024BE0`): one of 28 directions away from the
//! boss every other update, four times, flying on at 8 px a tick. A sword
//! passes silently. Otherwise weapons hit its pose box, except during the
//! immunity (`$34`), when the field's bit clearing also lets them pass. A
//! hit gives 64 frames of immunity. The health word `+$32` = 10 dies below
//! zero, so eleven points win: natively `Enemy::health` = 11. The machine
//! then falls bouncing through explosions for 256 frames and sets
//! [`Ctx::next_substage`] (routine 12, `Game_Requests` bit 1).
//!
//! `obj_routine` here is an index (`$2B242` multiplies it by four for the
//! `bra.w` table at `$2B25C`); the routine constants cite each target. After
//! most routines the handler runs `Func_02BD12` (vertical bob) and always
//! `Func_02BD3C` (palette cycle, then the `Data_02BE92` animation). Immediates
//! and tables are read from the ROM (`boss2_tables_match_rom`).
//!
//! Emulator evidence (`obj/emu/states/r2-3.state`, idle player): hover 232
//! frames, warp 32, then routine 5 for 96 and 6 for 129 frames, 7 for 128,
//! 8 for 226, 10 for 128, 9 for 335; plane B stays blank (its routine
//! `$1AB2` is an `rts`; the plane B name table holds 4096 zero cells).
//!
//! Palette line 1 follows the health (`Func_02BF46`, `Data_02AFA4`, #92).
//! `Func_02BD3C` pulses the walls (line 3 colours 5..13) and a lamp (line 2
//! colour 9) every fourth tick, except behind the orb attack's rising flash.
//!
//! The `$2C` = 2 attack, as the emulator shows it with an idle player: the
//! flash (frame 743) fills CRAM with `$EEE`, turns the player
//! ([`Signal::TurnPlayer`]), and `Func_0006E8` fades all 64 colours back to
//! `$FF16FA` while `plr_ctrl` bit 11 refuses A for 16 frames
//! ([`Signal::BlockNinjutsu`]). From then on `Func_02B832` sets VDP register
//! `$0B` = 3 and waves lines 12..211 of plane A ([`PlaneOp::Wave`]: `+$2E`
//! amplitude grows `$100` a frame to `$7F00`, ±127 px; `+$30` phase `$20`
//! a frame). Each call also sets `Game_Flags` bit 7 (`$FF0029` negative),
//! which the handler clears at its top: while it is set `Input_ReadPort`
//! swaps Left and Right ([`Signal::SwapLeftRight`]), frames 744..=1095. The
//! second flash (1095) turns the player back; the wave then dies down with
//! the pad normal and flattens (register `$0B` = 0) when the hover returns
//! (1221). A hit ends the wave at once (`$2B356`) but not the turn.
//!
//! A hit shows the hurt pose, copies `$FF16FA` back into CRAM, and bursts
//! (`Func_02BEE2`): six fragments arc away and eight flares spread and
//! return after random delays, beside the explosion. A hover hit then warps:
//! two streaks of the machine fly out, and two more close in on its new
//! place (`Func_02BCDA`, `Func_02BC3A`). The same burst repeats every 16
//! frames of the fall. The hover leaves a list-9 afterimage every fourth
//! frame and the orb attack every eighth (`Func_02BF68`); sparks circle into
//! the charging orb (`$2B6C0`) and the gun ejects shell casings (`$2B942`).
//!
//! MASAO MISSILE's orbs (#90, list-4 `$33` routine `$28`, `$25F28`) fly
//! through terrain in an arc (vx `Data_025F20`, vy -5, gravity `$3400`)
//! until Y >= `$1B0`, a fixed line, where they stop without bouncing and
//! bob ±16 px about it (`$25FCE`: the `$94A0` sine of `$2C`, from `d7 ror
//! 7`, `$10` a tick), their tile spinning through three frames. The
//! emulator shows a resting orb between Y 416 and 448. Their own
//! `Object_CollideL3` lets a shuriken or the sword destroy one
//! ([`ShotDown`]: the `$2623A` puff and effect `$1B`, the shuriken spent
//! with its voice). An orb clears its collision bit before reading it, so
//! touching the player costs it nothing: it stays and leaves no spark. The
//! gun's shots end on the floor or the player in `Player_SpawnDust`
//! (`$25E6E`).
//!
//! In the ROM, death and clear write `plr_event` 6 and `plr_timer` 2:
//! player sounds and ninjutsu are suppressed, but Joe can still move and
//! dash kick (confirmed in Genesis Plus GX during the fall). The
//! type `$88` weapon that damages through the field is not produced by the
//! native player. The lines outside 12..211 keep the camera's scroll (the
//! ROM freezes them at the flash's value, which the arena's fixed camera
//! hides).
use super::{
    HealthPalette,
    contact::{player_box, strike},
    sine,
};
use crate::{
    combat::{
        Effect, EffectKind, Enemy, EnemyState, Hitbox, Motion, PaletteOp, PlaneOp, Projectile,
        Reaction, ShotDown, Signal, Spawn, advance, pose_hitbox,
    },
    enemies::{Behavior, Ctx, HitResponse, Roster, set_whole},
    game::Terrain,
    movement::PlayerEvent,
    rom::{Result, byte, signed, signed_byte, word, word_i},
    sound_events::Table,
};
use std::sync::Arc;

/// `Boss_Jumps` target of this round.
pub(crate) const HANDLER: usize = 0x2b104;

pub(crate) fn fight(rom: &[u8]) -> Result<Arc<dyn Behavior>> {
    Ok(Arc::new(Boss2::load(rom)?))
}

/// List-5 types this boss spawns and owns (registered by [`register_parts`]).
/// `Boss2`'s helpers are list-1, list-4 (`$33`) and list-9 objects.
pub(crate) const PART_KINDS: &[u16] = &[];

pub(crate) fn register_parts(_rom: &[u8], _roster: &mut Roster) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------

/// `obj_routine` indices of the `$2B25C` table.
const SETUP: u16 = 0; // $2B290
const HURT: u16 = 1; // $2B30C
const DEATH: u16 = 2; // $2B38C
const SPIRAL: u16 = 3; // $2B4BC
const HOVER: u16 = 4; // $2B51C
const CHARGE: u16 = 5; // $2B670
const BEAM: u16 = 6; // $2B704
const FLASH_IN: u16 = 7; // $2B76A
const SWEEP: u16 = 8; // $2B892
const BURST: u16 = 9; // $2B960
const FLASH_OUT: u16 = 10; // $2BAB0
const WARP: u16 = 11; // $2BBA0
const CLEAR: u16 = 12; // $2B4A4

/// `Enemy::timer` = `$34` (immunity), `Enemy::counter` = `$3A` (frames),
/// `Enemy::home_x` = `$42`, `Enemy::launch_y` = the `$44` long (16.16),
/// `Enemy::step_x` = the `$36`/`$38` target (X << 16 | Y); `vars`:
const ANIM: usize = 0; // $28: animation counter (high byte = frame / sub-phase)
const FLAGS: usize = 1; // $2A, plus native bits
const P2C: usize = 2; // $2C
const P2E: usize = 3; // $2E
const P30: usize = 4; // $30
const BOB: usize = 5; // $3C angle | $3E step << 16
const AMP: usize = 6; // $40: bob amplitude

/// `$2B` bit 0: the animation reached its end marker.
const ANIM_END: i32 = 1 << 0;
/// `$2B` bit 1: `Func_02BD3C` leaves the palette alone (the rising flash).
const HOLD_PALETTE: i32 = 1 << 1;
/// `$2B` bit 7: the reflecting field and the flicker poses.
const SHIELD: i32 = 1 << 7;

/// One `Data_02BE92` animation: `$28` advance per frame and frame bytes
/// (mapping = `$256` + byte).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Animation {
    speed: i32,
    frames: Vec<u8>,
}

/// `Boss2` (`$2B104`).
pub(crate) struct Boss2 {
    animations: Vec<Animation>,
    /// `Data_02BE8A`: the field's flicker frames by `$3A & 3`.
    flicker: [u16; 4],
    base: u16,
    health: i32,
    /// `Data_02AFA4`: palette line 1 by the health word `$32` (0..=10),
    /// copied by `Func_02BF46` into CRAM (`$FFF020`) and the fade target
    /// (`$FF171A`); it reddens as the health falls. Entry 0, shown at the
    /// death, is the stage's own line 1.
    health_palette: HealthPalette,
    center: (i32, i32),
    radius: i32,
    spiral_end: i32,
    hover_ticks: i32,
    hover_step: i32,
    hover_speed: i32,
    hover_bounds: (i32, i32),
    hover_band: (i32, i32),
    bob_amplitude: i32,
    field: (i32, i32),
    hurt_immunity: u16,
    hit_immunity: u16,
    charge_y: i32,
    beam_timing: i32,
    beam_shots: i32,
    beam_drift: i32,
    beam: Projectile,
    beam_y: i32,
    sweep_offset: i32,
    sweep_height: i32,
    shot: Projectile,
    shot_speed: (i32, i32),
    shot_offset: (i32, i32),
    shot_floor: i32,
    burst_y: i32,
    burst_ticks: i32,
    rise_y: i32,
    flash_ticks: i32,
    orb: Projectile,
    orb_vx: [i32; 4],
    orb_rest: i32,
    /// `$25FCE`: a resting orb bobs ±16 px on `$2C`, `$10` a tick; its tile
    /// spins through three frames of four patterns, one per `$100` / `$10`
    /// ticks (`$25F74`).
    orb_bob_step: u16,
    orb_spin: (u16, u16),
    burst: Vec<(i32, i32)>,
    warp_y: i32,
    warp_side: i32,
    warp_center: i32,
    attacks: [(u16, i32, i32); 3],
    death_fall: (i32, i32),
    arena: (i32, i32, i32),
    contact: [u8; 2],
    list4: usize,
    sounds: Sounds,
    pieces: Pieces,
    wave: Wave,
    cycle: Cycle,
    /// `move.l #$0EEE0EEE,d0`: the flashes' CRAM colour.
    white: u16,
    /// `cmpi.w #$24,$28(a0)`: the flashes fade while the animation is below.
    fade_end: i32,
}

/// The list-1 type `$2B` (handler `$26178`) and list-9 pieces the handler
/// allocates. Life counts are the `$28` countdowns: a piece is drawn on
/// `life - 1` frames.
struct Pieces {
    /// `Func_02BEE2`: six fragments (routine `$20`, `$26406`), mapping
    /// `$25A + (r & 1)`, velocity from the random word, gravity, life.
    debris: (usize, u16, i32, i32),
    /// Then eight flares (routine `$18`, `$2634E`) after `(r & $F) + 1`
    /// ticks: first mapping, frame count to `$264` and `Data_0263C6`'s
    /// velocity and acceleration by index.
    flares: (usize, u16, u8, [Motion; 8]),
    delay_mask: u16,
    /// `$2637C`: `move.w #7,$28(a0)`, ticks per flare frame.
    flare_period: u16,
    /// `Func_02BCDA`: the warp streak (routine `$14`), life, speed and the
    /// arriving pair's offset (`Func_02BC3A`).
    streak: (u16, i32, i32, i32),
    /// `Func_02BF68`: the list-9 afterimage's tile, the hover's mapping and
    /// life (`$2B58A`) and the orb attack's (`$2BFAA`).
    trail_tile: u16,
    hover_trail: (u16, i32),
    burst_trail: (u16, i32),
    /// `$2B6C0`: the charge's circling spark (routine 8, `$262B6`): tile,
    /// Y offset, life and first mapping (the radius is life + 8).
    orbit: (u16, i32, i32, u16),
    /// `$2B942`: the gun's shell casing (routine `$30`, `$2650E`): mapping,
    /// Y offset, X speed, Y speed and gravity; it lands on Y `$1F0`.
    casing: (u16, i32, i32, i32, i32),
    /// `move.w #$8000,d1` (`$2651A`): the casing's tile.
    casing_tile: u16,
}

/// `Func_02B832`: lines, phase step and amplitude growth and cap.
struct Wave {
    phase_step: i32,
    growth: i32,
    cap: i32,
}

/// `Func_02BD3C`: line 3 colours 5..11 (`Data_02BDAA` rows picked by
/// `Data_02BDF0`) and 12..13 (`Data_02BE00`), every fourth tick.
struct Cycle {
    rows: [[u16; 7]; 5],
    order: [u8; 16],
    pairs: [[u16; 2]; 8],
}
/// Commands of the `$5B56` effects (and the `$5986` fade) the handler requests.
struct Sounds {
    entry: u8,
    hurt: [u8; 2],
    death_music: u8,
    fall: u8,
    boom: u8,
    beam: u8,
    laser: u8,
    flash: u8,
    gun: u8,
    drop: u8,
    charge: u8,
    burst: u8,
}

/// A still list-4 record: `Combat::tick` sweeps a record only while it
/// moves, so every resting shot carries a 1/65536 px drift.
fn still() -> Motion {
    Motion {
        vx: 1,
        ..Motion::default()
    }
}

impl Boss2 {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        let table = 0x2be92;
        let mut animations = Vec::with_capacity(13);
        for routine in 0..13 {
            let at = table + usize::from(word(rom, table + routine * 2)?);
            let mut frames = Vec::new();
            for i in 1..16 {
                let frame = byte(rom, at + i)?;
                if frame == 0xff {
                    break;
                }
                frames.push(frame);
            }
            animations.push(Animation {
                speed: i32::from(byte(rom, at)?),
                frames,
            });
        }
        let base = word(rom, 0x2be7e)?;
        let mut flicker = [0; 4];
        for (i, frame) in flicker.iter_mut().enumerate() {
            *frame = base + word(rom, 0x2be8a + i * 2)?;
        }
        let mut orb_vx = [0; 4];
        for (i, vx) in orb_vx.iter_mut().enumerate() {
            *vx = signed(rom, 0x25f20 + i * 2)? << 16;
        }
        let mut burst = Vec::with_capacity(8);
        for i in 0..8 {
            let b = |p: usize| -> Result<i32> { Ok((signed_byte(rom, p)? << 16) >> 2) };
            burst.push((b(0x26068 + i * 2)?, b(0x26069 + i * 2)?));
        }
        let shot_damage = byte(rom, 0x25e62)?;
        // List-4 `$33` (`$25E06`) loads its box before a routine sets the
        // frame, and no spawner sets it: each first box is a point.
        let mut beam = Projectile::new(0, 0, 0, true, word(rom, 0x26114)?, byte(rom, 0x2611a)?)
            .with_motion(still())
            .unset_frame();
        // `move.w #4,$28(a0)` countdown after the set-up frame.
        beam.life = 5;
        beam.passes_terrain = true;
        let mut shot =
            Projectile::new(0, 0, 0, true, word(rom, 0x25e56)?, shot_damage).unset_frame();
        shot.passes_terrain = true;
        // $25E6E: the floor or the player (bit 14) ends it in dust.
        shot.contact_dust = true;
        let mut orb = Projectile::new(0, 0, 0, true, word(rom, 0x25f2a)?, byte(rom, 0x25f36)?)
            .with_motion(Motion {
                vy: signed(rom, 0x25f48)? << 16,
                gravity: word_i(rom, 0x25f5c)?,
                ..Motion::default()
            })
            .unset_frame();
        orb.passes_terrain = true;
        orb.life = u16::MAX;
        orb.stays = true;
        // $2603A: a weapon leaves the list-1 `$2B` puff and effect $1B.
        orb.shot_down = Some(ShotDown {
            blast: None,
            first: word(rom, 0x2623c)?,
            frames: (word(rom, 0x262a8)? - word(rom, 0x2623c)?) as u8,
            period: word(rom, 0x2629e)? + 1,
            sound: Some(Table::Effect.command_at(rom, 0x26057)?),
            smoke: None,
        });
        let effect = |id_at: usize| Table::Effect.command_at(rom, id_at);
        let mut flare_motions = [Motion::default(); 8];
        for (i, motion) in flare_motions.iter_mut().enumerate() {
            let at = 0x263c6 + i * 8;
            *motion = Motion {
                vx: signed(rom, at)? << 16,
                vy: signed(rom, at + 2)? << 16,
                ax: signed(rom, at + 4)?,
                gravity: signed(rom, at + 6)?,
                ..Motion::default()
            };
        }
        let flare_first = word(rom, 0x2635a)?;
        let mut rows = [[0; 7]; 5];
        for (i, row) in rows.iter_mut().enumerate() {
            *row = crate::rom::words(rom, 0x2bdaa + i * 14)?;
        }
        let mut order = [0; 16];
        for (i, offset) in order.iter_mut().enumerate() {
            *offset = byte(rom, 0x2bdf0 + i)? / 14;
        }
        let mut pairs = [[0; 2]; 8];
        for (i, pair) in pairs.iter_mut().enumerate() {
            *pair = crate::rom::words(rom, 0x2be00 + i * 4)?;
        }
        Ok(Self {
            animations,
            flicker,
            base,
            health: word_i(rom, 0x2b2ec)?,
            health_palette: HealthPalette::table(rom, 1, 0x2afa4, 11)?,
            center: (word_i(rom, 0x2b2ce)?, word_i(rom, 0x2b2da)?),
            radius: word_i(rom, 0x2b2e2)?,
            spiral_end: word_i(rom, 0x2b50c)?,
            hover_ticks: word_i(rom, 0x2b64a)?,
            hover_step: word_i(rom, 0x2b652)?,
            hover_speed: word_i(rom, 0x2b5fa)?,
            hover_bounds: (word_i(rom, 0x2b5f4)?, word_i(rom, 0x2b600)?),
            hover_band: (word_i(rom, 0x2b610)?, word_i(rom, 0x2b61a)?),
            bob_amplitude: word_i(rom, 0x2b658)?,
            field: (
                i32::from(byte(rom, 0x2b128)?),
                i32::from(byte(rom, 0x2b129)?),
            ),
            hurt_immunity: word(rom, 0x2b37a)?,
            hit_immunity: word(rom, 0x2b212)?,
            charge_y: word_i(rom, 0x2b6ac)?,
            beam_timing: word_i(rom, 0x2b696)?,
            beam_shots: word_i(rom, 0x2b69c)?,
            beam_drift: word_i(rom, 0x2b75c)?,
            beam,
            beam_y: word_i(rom, 0x26120)?,
            sweep_offset: word_i(rom, 0x2b818)?,
            sweep_height: word_i(rom, 0x2b8dc)?,
            shot,
            shot_speed: (
                i32::from(byte(rom, 0x2b927)?) << 16,
                word_i(rom, 0x2b920)? << 16,
            ),
            shot_offset: (i32::from(byte(rom, 0x2b92b)?), word_i(rom, 0x2b918)?),
            shot_floor: word_i(rom, 0x25e7c)?,
            burst_y: word_i(rom, 0x2b9ba)?,
            burst_ticks: word_i(rom, 0x2b9d6)?,
            rise_y: word_i(rom, 0x2ba3c)?,
            flash_ticks: word_i(rom, 0x2ba74)?,
            orb,
            orb_vx,
            orb_rest: word_i(rom, 0x25fb2)?,
            orb_bob_step: word(rom, 0x25fe6)?,
            orb_spin: (word(rom, 0x25f7c)?, 0x100 / word(rom, 0x25f90)?),
            burst,
            warp_y: word_i(rom, 0x2bbf0)?,
            warp_side: word_i(rom, 0x2bc16)?,
            warp_center: word_i(rom, 0x2bc32)?,
            attacks: [
                (CHARGE, word_i(rom, 0x2bc92)?, word_i(rom, 0x2bc98)?),
                (FLASH_IN, word_i(rom, 0x2bcb8)?, word_i(rom, 0x2bcbe)?),
                (BURST, word_i(rom, 0x2bcce)?, word_i(rom, 0x2bcd4)?),
            ],
            death_fall: (word_i(rom, 0x2b40c)?, signed(rom, 0x2b412)?),
            arena: (
                word_i(rom, 0x2b41c)?,
                word_i(rom, 0x2b424)?,
                word_i(rom, 0x2b438)?,
            ),
            // $2B1CA / $2B1C2: move.b #1 or #2,obj_health(a3).
            contact: [byte(rom, 0x2b1cd)?, byte(rom, 0x2b1c5)?],
            // $2B29A: move.b #$30,Object_ListParams+2 (a dbf count).
            list4: usize::from(byte(rom, 0x2b29d)?) + 1,
            sounds: Sounds {
                entry: effect(0x2b2f5)?,
                hurt: [effect(0x2b33d)?, effect(0x2b341)?],
                // $2B230 moveq #6 / Func_004A36: music 6, FADE OUT.
                death_music: Table::Music.command_at(rom, 0x2b231)?,
                fall: effect(0x2b3eb)?,
                boom: effect(0x2b493)?,
                beam: effect(0x2b74d)?,
                laser: effect(0x26137)?,
                flash: effect(0x2b7c1)?,
                gun: effect(0x2b8fd)?,
                drop: effect(0x2b9df)?,
                charge: effect(0x2bca1)?,
                burst: effect(0x26057)?,
            },
            pieces: Pieces {
                // $2BEE2 moveq #5,d7 / $2BF0E moveq #7,d7: dbf counts.
                debris: (
                    usize::from(byte(rom, 0x2bee3)?) + 1,
                    word(rom, 0x26418)?,
                    word_i(rom, 0x2643a)?,
                    word_i(rom, 0x26440)?,
                ),
                flares: (
                    usize::from(byte(rom, 0x2bf0f)?) + 1,
                    flare_first,
                    (word(rom, 0x263b8)? - flare_first) as u8,
                    flare_motions,
                ),
                delay_mask: word(rom, 0x2bf32)?,
                flare_period: word(rom, 0x2637e)?,
                streak: (
                    word(rom, 0x2bcec)?,
                    word_i(rom, 0x2bcfe)?,
                    word_i(rom, 0x2bbd6)?,
                    word_i(rom, 0x2bc42)?,
                ),
                trail_tile: word(rom, 0x2bfb2)?,
                hover_trail: (word(rom, 0x2b58c)?, word_i(rom, 0x2b592)?),
                burst_trail: (word(rom, 0x2bfac)?, word_i(rom, 0x2bfc4)?),
                orbit: (
                    word(rom, 0x2b6dc)?,
                    word_i(rom, 0x2b6e4)?,
                    word_i(rom, 0x262c8)?,
                    word(rom, 0x262ce)?,
                ),
                casing: (
                    word(rom, 0x26510)?,
                    word_i(rom, 0x26516)?,
                    i32::from(byte(rom, 0x2651f)?),
                    signed(rom, 0x26536)?,
                    word_i(rom, 0x2653e)?,
                ),
                casing_tile: word(rom, 0x2651c)?,
            },
            wave: Wave {
                phase_step: word_i(rom, 0x2b876)?,
                growth: word_i(rom, 0x2b880)?,
                cap: word_i(rom, 0x2b884)?,
            },
            cycle: Cycle { rows, order, pairs },
            white: word(rom, 0x2b77e)?,
            fade_end: word_i(rom, 0x2b7d6)?,
        })
    }

    fn center_y(e: &Enemy) -> i32 {
        e.launch_y >> 16
    }
    fn set_center_y(e: &mut Enemy, y: i32) {
        e.launch_y = set_whole(e.launch_y, y);
    }
    fn target(e: &Enemy) -> (i32, i32) {
        (e.step_x >> 16, i32::from(e.step_x as i16))
    }
    fn bob_angle(e: &Enemy) -> i32 {
        e.vars[BOB] & 0xffff
    }
    fn set_bob(e: &mut Enemy, angle: i32, step: i32) {
        e.vars[BOB] = (angle & 0xffff) | (step << 16);
    }
    /// `sign(d) | 1` (`ext.l; swap; ori.w #1`).
    fn sign(d: i32) -> i32 {
        if d < 0 { -1 } else { 1 }
    }

    /// `Func_02B624`: hover, drifting away from the player.
    fn hover(&self, cx: &Ctx, e: &mut Enemy) {
        e.routine = HOVER;
        e.vars[ANIM] = 0;
        e.vars[P2E] = -Self::sign(cx.player.0 - e.x) * 4;
        e.vars[P2C] = self.hover_ticks;
        Self::set_bob(e, 0, self.hover_step);
        e.vars[AMP] = self.bob_amplitude;
        e.home_x = e.x;
        e.launch_y = (e.y << 16) | i32::from(e.motion.fy);
        e.timer = 2;
    }
    /// `Func_02B53A`: stop and warp for a random attack (`$2C` 0, 2 or 4).
    fn choose(cx: &mut Ctx, e: &mut Enemy) {
        e.motion.stop();
        e.vars[ANIM] = 0;
        e.routine = WARP;
        let mut pick = i32::from(cx.random() & 6);
        while pick == 6 {
            pick = i32::from(cx.random() & 6);
        }
        e.vars[P2C] = pick;
    }

    /// `Object_AllocL4` for this boss. Its set-up raises the list-4 limit
    /// (`Object_ListParams+2` = `$30`, 49 records) above the framework's
    /// eight, so the four orbs can each burst into eight shots.
    fn fire(&self, cx: &mut Ctx, template: &Projectile, x: i32, y: i32, motion: Motion) {
        if cx
            .projectiles
            .iter()
            .filter(|shot| shot.enemy && shot.alive)
            .count()
            >= self.list4
        {
            return;
        }
        let mut shot = template.clone();
        shot.x = x;
        shot.y = y;
        shot.motion = motion;
        shot.owner = Some(cx.current);
        cx.push_projectile_unchecked(shot);
    }
    /// The orbs and shots after the boss (`$25E06`): shots end on the floor
    /// (`$25E7A`), orbs rest at Y `$1B0` and burst into eight shots while the
    /// boss hovers (`$25FEE`).
    fn manage_shots(&self, cx: &mut Ctx, e: &Enemy) {
        let me = Some(cx.current);
        let mut bursts = Vec::new();
        let mut dust = Vec::new();
        for shot in cx.projectiles.iter_mut() {
            if shot.owner != me || !shot.alive {
                continue;
            }
            if shot.mapping == self.shot.mapping && shot.y >= self.shot_floor {
                shot.alive = false;
                dust.push((shot.x, shot.y));
            } else if shot.mapping == self.orb.mapping {
                let (frames, ticks) = self.orb_spin;
                shot.base = (shot.age / ticks % frames) * 4;
                match shot.rest {
                    // $25FB4: stopped where it crossed the line, no bounce.
                    None if shot.y >= self.orb_rest => {
                        shot.motion = still();
                        shot.rest = Some(self.orb_rest);
                    }
                    None => {}
                    Some(centre) => {
                        let (_, sin) = cx.shared.sine.polar(shot.angle);
                        shot.y = centre + ((sin << 4) >> 16);
                        shot.angle = shot.angle.wrapping_add(self.orb_bob_step);
                        if e.routine == HOVER {
                            shot.alive = false;
                            bursts.push((shot.x, shot.y));
                        }
                    }
                }
            }
        }
        for (x, y) in dust {
            cx.dust(x, y);
        }
        for (x, y) in bursts {
            if let Some(down) = self.orb.shot_down {
                let puff = down.puff((x, y), *cx.random);
                cx.effect(puff);
            }
            for &(vx, vy) in &self.burst {
                self.fire(
                    cx,
                    &self.shot,
                    x,
                    y,
                    Motion {
                        vx,
                        vy,
                        ..Motion::default()
                    },
                );
            }
            cx.play(self.sounds.burst);
        }
    }
    /// `Func_02F146`: one explosion within 16 px of the boss.
    fn explosion(cx: &mut Ctx, e: &Enemy) {
        let r = cx.random();
        let dx = i32::from((r >> 8) as u8 as i8) >> 3;
        let dy = i32::from(r as u8 as i8) >> 3;
        let first = cx.shared.puff_seq.first().copied().unwrap_or(0);
        cx.effect(Effect::new(EffectKind::Puff, e.x + dx, e.y + dy, first));
    }

    /// `Func_02BEE2`: six fragments fly up and fall, then eight flares
    /// spread and return, each after a random delay (the hit and the fall).
    fn burst_pieces(&self, cx: &mut Ctx, e: &Enemy) {
        let (count, first, gravity, life) = self.pieces.debris;
        for _ in 0..count {
            // $26406: $25A + (r & 1); vx = low byte >> 6, vy = (high | $80) >> 5.
            let r = cx.random();
            let mut piece = Effect::new(EffectKind::Pieces, e.x, e.y, first + (r & 1));
            piece.motion = Motion {
                vx: (i32::from(r as u8 as i8) >> 6) << 16,
                vy: (i32::from(((r >> 8) as u8 | 0x80) as i8) >> 5) << 16,
                gravity,
                ..Motion::default()
            };
            piece.counter = life - 1;
            cx.effect(piece);
        }
        let (count, first, frames, motions) = self.pieces.flares;
        for index in (0..count).rev() {
            let delay = (cx.random() & self.pieces.delay_mask) + 1;
            let mut flare = Effect::new(EffectKind::Flare, e.x, e.y, first);
            flare.delay = delay;
            flare.motion = motions[index % motions.len()];
            flare.period = self.pieces.flare_period;
            flare.counter = i32::from(flare.period);
            flare.phase = frames;
            cx.effect(flare);
        }
    }
    /// `Func_02BCDA`: a streak of the machine at (x, y), `vx` px a tick.
    fn streak(&self, cx: &mut Ctx, x: i32, y: i32, vx: i32) {
        let (mapping, life, ..) = self.pieces.streak;
        let mut streak = Effect::new(EffectKind::Pieces, x, y, mapping);
        streak.motion.vx = vx << 16;
        streak.counter = life - 1;
        cx.effect(streak);
    }
    /// `Func_02BF68`: a still list-9 afterimage behind every object.
    /// It links the record at the head of list 9 (`$2BF7C`), so the newest
    /// afterimage is the first list-9 sprite.
    fn trail(&self, cx: &mut Ctx, e: &Enemy, (mapping, life): (u16, i32)) {
        let mut ghost = Effect::new(EffectKind::Pieces, e.x, e.y, mapping);
        ghost.base = self.pieces.trail_tile;
        ghost.back = true;
        ghost.head = true;
        ghost.counter = life - 1;
        cx.effect(ghost);
    }
    /// `$2B6C0`: a spark circling in toward the orb, from the angle and
    /// step in the random word (`$FF001C`, read without advancing it).
    ///
    /// Nothing advances the generator during the charge, but the main
    /// loop turns it (`rol.w $FF001C`, `Main_WaitFrame` `$512`) until each
    /// VBlank: Genesis Plus GX, 2-3, 600 to 1,560 turns a frame, so every
    /// spark reads a new rotation and they close in from all around the
    /// orb. The idle time is not modelled; the charge's frame count stands
    /// in for the turns (mod 16), on a copy, so the shared sequence the
    /// other handlers draw from is unchanged.
    fn orbit(&self, cx: &mut Ctx, e: &Enemy) {
        let (tile, dy, life, first) = self.pieces.orbit;
        let r = crate::ninjutsu_fx::orbit_random(*cx.random, (e.counter >> 2) as u64);
        let mut spark = Effect::new(EffectKind::Orbit, e.x, e.y, first + (life >> 3) as u16);
        spark.base = tile;
        spark.angle = r >> 4;
        spark.phase = (r & 0xf) as u8;
        spark.counter = life;
        spark.origin = (e.x, e.y + dy);
        // Placed by its first update, in this frame's effect pass.
        cx.effect(spark);
    }
    /// `$2B942`: the gun's shell casing, bouncing on the floor until slow.
    fn casing(&self, cx: &mut Ctx, e: &Enemy) {
        let (mapping, dy, vx, vy, gravity) = self.pieces.casing;
        let mut casing = Effect::new(EffectKind::Pieces, e.x, e.y + dy, mapping);
        casing.base = self.pieces.casing_tile;
        casing.motion = Motion {
            vx: vx << 16,
            vy: vy << 16,
            gravity,
            ..Motion::default()
        };
        // $26574: it halves on each landing and is freed at the third.
        casing.phase = 3;
        casing.counter = i32::from(u16::MAX);
        cx.effect(casing);
    }

    /// Line 1 as `Func_02BF46` last wrote it into the `$FF16FA` fade
    /// target: the stage's until the first hit, then `Data_02AFA4` by the
    /// health before each hit (the native health after it).
    fn health_line(&self, e: &Enemy) -> Option<[u16; 16]> {
        (i32::from(e.health) <= self.health)
            .then(|| self.health_palette.colors(usize::from(e.health)))
    }
    /// `$2B314`: the whole `$FF16FA` target back into CRAM.
    fn reload_palette(&self, cx: &mut Ctx, e: &Enemy) {
        for line in 0..4 {
            match self
                .health_line(e)
                .filter(|_| line == self.health_palette.line)
            {
                Some(colors) => cx.palette_op(PaletteOp::Load(line, colors)),
                None => cx.palette_op(PaletteOp::Reload(line)),
            }
        }
    }
    /// `Func_0006E8` with d7 = `$3F`: one step of all 64 colours toward it.
    fn fade_palette(&self, cx: &mut Ctx, e: &Enemy) {
        for line in 0..4 {
            match self
                .health_line(e)
                .filter(|_| line == self.health_palette.line)
            {
                Some(colors) => cx.fade_palette(line, colors),
                None => cx.restore_palette(line),
            }
        }
    }
    /// `$2B77A`: every CRAM colour white.
    fn white_flash(&self, cx: &mut Ctx) {
        for line in 0..4 {
            cx.fill_palette(line, self.white);
        }
    }
    /// `Func_02BD3C`: line 2 colour 9's green falls every fourth tick
    /// (`(-$3A << 3) & $E0` into `$FFF052`), and every fourth tick line 3
    /// colours 5..11 and 12..13 step through their tables: the walls pulse
    /// and the lamps blink.
    fn cycle_palette(&self, cx: &mut Ctx, e: &Enemy) {
        let t = e.counter;
        cx.palette_op(PaletteOp::Color(2, 9, (t.wrapping_neg() << 3) & 0xe0));
        if t & 3 != 0 {
            return;
        }
        let pair = self.cycle.pairs[usize::from((t & 0x1c) >> 2)];
        let row = self.cycle.rows[usize::from(self.cycle.order[usize::from((t >> 2) & 0xf)])];
        let mut words = [pair[0]; 8];
        words[..7].copy_from_slice(&row);
        cx.palette_op(PaletteOp::Words(3, 5, words));
        cx.palette_op(PaletteOp::Color(3, 13, pair[1]));
    }
    /// `Func_02B832`: this tick's wave, then its phase and amplitude grow.
    /// Its `Game_Flags` bit 7 makes the pad read Left and Right swapped.
    fn wave(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.signal(Signal::SwapLeftRight);
        self.draw_wave(cx, e);
        e.vars[P2E] = (e.vars[P2E] + self.wave.growth).min(self.wave.cap);
    }
    /// `$2BB4E`: the wave dies down after the second flash, the pad normal.
    fn calm(&self, cx: &mut Ctx, e: &mut Enemy) {
        self.draw_wave(cx, e);
        e.vars[P2E] = (e.vars[P2E] - self.wave.growth).max(0);
    }
    fn draw_wave(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.plane_op(PlaneOp::Wave {
            amplitude: e.vars[P2E] as u16,
            phase: e.vars[P30] as u16,
        });
        e.vars[P30] = (e.vars[P30] + self.wave.phase_step) & 0xffff;
    }

    /// `Func_02BE20`: advance the routine's animation and set the pose.
    fn animate(&self, e: &mut Enemy) {
        let Some(animation) = self.animations.get(usize::from(e.routine)) else {
            return;
        };
        e.vars[FLAGS] &= !ANIM_END;
        let index = ((e.vars[ANIM] >> 8) & 0xff) as usize;
        let frame = match animation.frames.get(index) {
            Some(&frame) => frame,
            None => {
                e.vars[ANIM] = 0;
                animation.frames.first().copied().unwrap_or(0)
            }
        };
        e.vars[ANIM] = (e.vars[ANIM] + animation.speed) & 0xffff;
        if animation
            .frames
            .get(((e.vars[ANIM] >> 8) & 0xff) as usize)
            .is_none()
        {
            e.vars[FLAGS] |= ANIM_END;
        }
        e.mapping = if e.vars[FLAGS] & SHIELD != 0 {
            self.flicker[usize::from(e.counter & 3)]
        } else {
            self.base + u16::from(frame)
        };
    }
    /// `Func_02BD12`: Y = `$44` + sine(`$3C`) * `$40`; `$3C` += `$3E`.
    fn bob(cx: &Ctx, e: &mut Enemy) {
        let angle = Self::bob_angle(e);
        e.y = Self::center_y(e) + ((sine(cx, angle) * e.vars[AMP]) >> 16);
        let step = e.vars[BOB] >> 16;
        Self::set_bob(e, angle + step, step);
    }

    /// One routine; true when the bob (`Func_02BD12`) follows.
    fn dispatch(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        match e.routine {
            SETUP => {
                e.health = (self.health + 1) as u16;
                e.hurt_routine = HURT;
                e.death_routine = DEATH;
                e.state = EnemyState::Emerging;
                e.motion = Motion::default();
                // move.w d0,obj_tile(a0): tile base 0 (Boss_Spawn gave $2000).
                e.tile = Some(0);
                e.facing_left = false;
                e.home_x = cx.camera.0 + self.center.0;
                Self::set_center_y(e, cx.camera.1 + self.center.1);
                e.vars[P2E] = self.radius;
                e.vars[P30] = 0;
                cx.play(self.sounds.entry);
                e.routine = SPIRAL;
                e.vars[ANIM] = 0;
                false
            }
            HURT => {
                if e.vars[P2C] == 0 {
                    // $2B314: $FF16FA back into CRAM (ends a flash).
                    self.reload_palette(cx, e);
                    e.vars[P2C] = 1;
                    return false;
                }
                let sound = self.sounds.hurt[usize::from(e.health % 3 != 1)];
                cx.play(sound);
                self.burst_pieces(cx, e);
                Self::explosion(cx, e);
                // $2B356: VDP register $0B = 0 ends a wave; the player keeps
                // the turn of a flash that no second flash undoes.
                cx.plane_op(PlaneOp::FullScroll);
                if e.saved_routine == HOVER {
                    Self::choose(cx, e);
                } else {
                    self.hover(cx, e);
                }
                e.timer = self.hurt_immunity;
                false
            }
            DEATH => {
                self.death(cx, e);
                false
            }
            SPIRAL => {
                e.timer = 1;
                let angle = (i32::from(e.counter & 3) << 8) + e.vars[P30];
                let (cos, sin) = cx.shared.sine.polar(angle as u16);
                let radius = e.vars[P2E];
                e.x = e.home_x + ((((cos >> 2) as i16 as i32) * radius) >> 16);
                e.y = Self::center_y(e) + ((((sin >> 2) as i16 as i32) * radius) >> 16);
                e.vars[P2E] = (radius - 2).max(0);
                e.vars[P30] += 8;
                if e.vars[P30] >= self.spiral_end {
                    self.hover(cx, e);
                }
                false
            }
            HOVER => {
                if cx.projectiles.iter().any(|shot| shot.enemy && shot.alive) {
                    return true;
                }
                e.vars[AMP] = 0;
                if e.vars[P2C] == 0 {
                    Self::choose(cx, e);
                } else {
                    self.drift(cx, e);
                }
                false
            }
            CHARGE => {
                if e.counter & 3 == 0 {
                    self.orbit(cx, e);
                }
                e.vars[FLAGS] |= SHIELD;
                if e.vars[FLAGS] & ANIM_END != 0 {
                    e.vars[P2C] = Self::sign(cx.player.0 - e.x) * 2;
                    e.vars[P2E] = self.beam_timing;
                    e.vars[P30] = self.beam_shots;
                    e.routine = BEAM;
                    e.vars[ANIM] = 0;
                }
                let dy = self.charge_y - e.y;
                if dy != 0 {
                    e.launch_y += Self::sign(dy) << 16;
                }
                true
            }
            BEAM => {
                e.vars[FLAGS] |= SHIELD;
                let (count, drift) = ((e.vars[P2E] >> 8) & 0xff, e.vars[P2E] & 0xff);
                if drift != 0 {
                    e.vars[P2E] = (count << 8) | (drift - 1);
                    e.x += e.vars[P2C];
                    return true;
                }
                let count = count - 1;
                e.vars[P2E] = count << 8;
                if count == 0 {
                    e.vars[P2E] = self.beam_drift;
                    e.vars[P30] -= 1;
                    if e.vars[P30] == 0 {
                        self.hover(cx, e);
                    }
                } else if count == 4 {
                    let y = e.y + self.beam_y;
                    self.fire(cx, &self.beam, e.x, y, still());
                    cx.play(self.sounds.beam);
                    cx.play(self.sounds.laser);
                }
                true
            }
            FLASH_IN | FLASH_OUT => {
                let out = e.routine == FLASH_OUT;
                if out && cx.projectiles.iter().any(|shot| shot.enemy && shot.alive) {
                    e.vars[ANIM] = 0;
                    self.wave(cx, e);
                    return true;
                }
                let anim = e.vars[ANIM];
                if anim < 4 {
                    if out {
                        self.wave(cx, e);
                    }
                    return true;
                }
                if anim == 4 {
                    // $2B774 / $2BAC0: the white flash turns the player.
                    cx.signal(Signal::BlockNinjutsu);
                    self.white_flash(cx);
                    cx.signal(Signal::TurnPlayer);
                    if out {
                        self.wave(cx, e);
                    } else {
                        // $2B78E: every line at line 0's scroll, register
                        // $0B = 3; YAJIMA POWER.
                        e.vars[P2E] = 0;
                        e.vars[P30] = 0;
                        cx.plane_op(PlaneOp::Wave {
                            amplitude: 0,
                            phase: 0,
                        });
                        cx.play(self.sounds.flash);
                    }
                    return true;
                }
                if anim < self.fade_end {
                    cx.signal(Signal::BlockNinjutsu);
                    self.fade_palette(cx, e);
                }
                e.vars[FLAGS] |= SHIELD;
                if out {
                    if e.vars[FLAGS] & ANIM_END != 0 {
                        // $2BB1E: the lines flattened, register $0B = 0.
                        cx.plane_op(PlaneOp::FullScroll);
                        self.hover(cx, e);
                    } else {
                        self.calm(cx, e);
                    }
                    return true;
                }
                if e.vars[FLAGS] & ANIM_END != 0 {
                    // ±32, three times: ±96 from the player.
                    e.vars[P2C] = Self::sign(cx.player.0 - e.x) * self.sweep_offset * 3;
                    e.routine = SWEEP;
                    e.vars[ANIM] = 0;
                }
                self.wave(cx, e);
                true
            }
            SWEEP => {
                self.wave(cx, e);
                if e.vars[FLAGS] & ANIM_END != 0 {
                    e.routine = FLASH_OUT;
                    e.vars[ANIM] = 0;
                    return true;
                }
                let anim = e.vars[ANIM];
                if (anim >> 8) & 1 == 0 {
                    if anim & 0xff == 0 {
                        e.vars[P2C] = -e.vars[P2C];
                    }
                    // Ease 1/16 toward the player's X less $2C.
                    let here = (i64::from(e.x) << 16) + i64::from(e.motion.fx);
                    let goal = i64::from(cx.player.0 - e.vars[P2C]) << 16;
                    advance(&mut e.x, &mut e.motion.fx, ((goal - here) >> 4) as i32);
                    let dy = cx.player.1 - Self::center_y(e) - self.sweep_height;
                    if dy != 0 {
                        e.launch_y += Self::sign(dy) << 16;
                    }
                } else if anim & 0x1f == 0 {
                    cx.play(self.sounds.gun);
                    let right = e.vars[P2C] >= 0;
                    let (vx, vy) = self.shot_speed;
                    let (dx, dy) = self.shot_offset;
                    let x = e.x + if right { dx } else { -dx };
                    let motion = Motion {
                        vx: if right { vx } else { -vx },
                        vy,
                        ..Motion::default()
                    };
                    self.fire(cx, &self.shot, x, e.y + dy, motion);
                    self.casing(cx, e);
                }
                true
            }
            BURST => {
                self.burst(cx, e);
                true
            }
            WARP => {
                self.warp(cx, e);
                false
            }
            CLEAR => {
                cx.player_event(PlayerEvent::Freeze);
                cx.next_substage();
                Ctx::free(e);
                false
            }
            _ => true,
        }
    }

    /// `$2B564`: drift across the upper screen, re-targeting every 33
    /// frames and leaving an afterimage every fourth.
    fn drift(&self, cx: &mut Ctx, e: &mut Enemy) {
        let count = e.vars[P2C] - 1;
        e.vars[P2C] = count;
        if (count as u8 as i8) < 0 {
            e.vars[P2C] = (count & !0xff) | 0x20;
            e.step_x = (e.home_x << 16) | (Self::center_y(e) & 0xffff);
        }
        if e.counter & 3 == 0 {
            self.trail(cx, e, self.pieces.hover_trail);
        }
        e.home_x += e.vars[P2E];
        let angle = Self::bob_angle(e);
        e.launch_y += (sine(cx, angle) << 4) & !0xffff;
        let step = e.vars[BOB] >> 16;
        Self::set_bob(e, angle + 0x30, step);
        let (tx, ty) = Self::target(e);
        e.motion.vx = ((tx - e.x) << 16) >> 4;
        e.motion.vy = ((ty - e.y) << 16) >> 4;
        let screen = e.home_x - (cx.camera.0 - 0x80);
        if screen < self.hover_bounds.0 {
            e.vars[P2E] = self.hover_speed;
        }
        if screen >= self.hover_bounds.1 {
            e.vars[P2E] = -self.hover_speed;
        }
        if e.y < self.hover_band.0 {
            e.launch_y += 1 << 16;
        }
        if e.y >= self.hover_band.1 {
            e.launch_y -= 1 << 16;
        }
    }

    /// `$2B960`: follow, drop, throw the orbs, rise behind a flash.
    fn burst(&self, cx: &mut Ctx, e: &mut Enemy) {
        let low = e.vars[ANIM] & 0xff;
        let mut phase = (e.vars[ANIM] >> 8) & 0xff;
        if phase == 0 {
            e.vars[FLAGS] |= SHIELD;
            e.vars[ANIM] = low;
            e.timer = 2;
            let here = (i64::from(e.x) << 16) + i64::from(e.motion.fx);
            let goal = i64::from(cx.player.0) << 16;
            advance(&mut e.x, &mut e.motion.fx, ((goal - here) >> 4) as i32);
            if low & 0xf == 0 {
                self.trail(cx, e, self.pieces.burst_trail);
            }
            return;
        }
        if phase == 1 {
            // move.w #$100,$28: sub-phases 1..4 hold the whole word.
            e.vars[ANIM] = 0x100;
            let y = (e.y << 16) + i32::from(e.motion.fy);
            e.launch_y += ((self.burst_y << 16) - y) >> 4;
            if Self::center_y(e) < self.burst_y {
                return;
            }
            e.vars[P2C] = self.burst_ticks;
            cx.play(self.sounds.drop);
            phase = 2;
        }
        if phase == 2 {
            e.vars[ANIM] = 0x200;
            if e.vars[P2C] == self.burst_ticks {
                // d7 = 3..0 selects Data_025F20[d7]; `$2C` = d7 ror 7.
                for (d7, &vx) in self.orb_vx.iter().enumerate().rev() {
                    let motion = Motion {
                        vx,
                        ..self.orb.motion
                    };
                    let mut orb = self.orb.clone();
                    orb.angle = (d7 as u16).rotate_right(7);
                    self.fire(cx, &orb, e.x, e.y, motion);
                }
            }
            e.vars[P2C] -= 1;
            if e.vars[P2C] != 0 {
                return;
            }
            phase = 3;
        }
        if phase == 3 {
            e.vars[ANIM] = 0x300;
            e.launch_y -= 2 << 16;
            if Self::center_y(e) >= self.rise_y {
                return;
            }
            // $2BA42: line 0's colours 0..10 black, lines 1..3 white.
            cx.signal(Signal::BlockNinjutsu);
            cx.palette_op(PaletteOp::FillFirst(0, 11, 0));
            for line in 1..4 {
                cx.fill_palette(line, self.white);
            }
            e.vars[P2C] = self.flash_ticks;
        }
        e.vars[ANIM] = 0x400;
        e.vars[P2C] -= 1;
        if e.vars[P2C] == 0 {
            self.hover(cx, e);
            return;
        }
        e.vars[FLAGS] |= HOLD_PALETTE;
        cx.signal(Signal::BlockNinjutsu);
        self.fade_palette(cx, e);
    }

    /// `$2BBA0`: hidden, reappear, and set up the chosen attack (`$2BC66`).
    fn warp(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.state = EnemyState::Emerging;
        e.timer = 2;
        let low = e.vars[ANIM] & 0xff;
        let (_, _, speed, offset) = self.pieces.streak;
        match (e.vars[ANIM] >> 8) & 0xff {
            0 => {
                // Two streaks fly out of the machine.
                self.streak(cx, e.x, e.y, speed);
                self.streak(cx, e.x, e.y, -speed);
                e.vars[ANIM] = 0x100 | low;
            }
            1 => e.vars[ANIM] = 0x100 | low,
            2 => {
                e.y = self.warp_y;
                e.x = match e.vars[P2C] {
                    0 => {
                        let left = cx.random() & 0x8000 != 0;
                        cx.camera.0
                            + if left {
                                self.warp_side & 0xff
                            } else {
                                self.warp_side
                            }
                    }
                    2 => cx.player.0,
                    _ => cx.camera.0 + self.warp_center,
                };
                // Func_02BC3A: two more close in on the new place.
                self.streak(cx, e.x + offset, e.y, -speed);
                self.streak(cx, e.x - offset, e.y, speed);
                e.vars[ANIM] = 0x300 | low;
            }
            3 => e.vars[ANIM] = 0x300 | low,
            _ => {
                e.vars[ANIM] = 0;
                e.home_x = e.x;
                e.launch_y = (e.y << 16) | i32::from(e.motion.fy);
                let (routine, step, amplitude) =
                    self.attacks[(e.vars[P2C] / 2).clamp(0, 2) as usize];
                e.routine = routine;
                Self::set_bob(e, 0, step);
                e.vars[AMP] = amplitude;
                if routine == CHARGE {
                    cx.play(self.sounds.charge);
                }
            }
        }
    }

    /// `$2B38C`: bounce and burn until the animation ends, then routine 12.
    fn death(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.player_event(PlayerEvent::Freeze);
        cx.plane_op(PlaneOp::FullScroll);
        e.timer = 2;
        if e.vars[FLAGS] & ANIM_END != 0 {
            e.routine = CLEAR;
            return;
        }
        if e.vars[ANIM] & 0xff == 4 {
            cx.play(self.sounds.fall);
            e.motion.vx = Self::sign(e.x - e.vars[P2C]) << 16;
            e.motion.gravity = self.death_fall.0;
            e.motion.vy = self.death_fall.1 << 16;
        }
        let (left, right, floor) = self.arena;
        if e.x < left || e.x >= right {
            e.x = if e.x < left { left } else { right };
            e.motion.vx = -e.motion.vx;
        }
        if e.y >= floor {
            e.y = floor;
            let mut vy = e.motion.vy;
            if (vy + 0x8000) >> 16 == 0 {
                vy = 0;
                e.motion.ax = 0;
                e.motion.gravity = 0;
            }
            e.motion.vy = -(vy >> 1);
            e.motion.vx >>= 1;
        }
        if e.motion.vy != 0 {
            if e.counter & 3 == 0 {
                Self::explosion(cx, e);
            }
            if e.counter & 0xf == 0 {
                cx.play(self.sounds.boom);
                self.burst_pieces(cx, e);
            }
        }
    }
}

impl Behavior for Boss2 {
    fn label(&self, _kind: u16) -> String {
        format!("Boss2 ${HANDLER:05X}")
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.base
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.counter = e.counter.wrapping_add(1);
        if e.state == EnemyState::Emerging {
            e.state = EnemyState::Waiting; // andi.w #$B7DF: shown again
        }
        match cx.process_hit(e) {
            Some(Reaction::Hurt) => {
                // $2B1E8: Func_02BF46 with `$32` before the damage is
                // subtracted, so each hit shows the previous hit's health.
                let before = e.health_before_hit() - 1;
                self.health_palette.show(cx, usize::from(before));
                // Hurt for two frames, 4 frames of immunity.
                e.vars[ANIM] = 0;
                e.vars[P2C] = 0;
                e.timer = self.hit_immunity;
            }
            Some(Reaction::Death) => {
                // $2B218: `$32` cleared, Func_02BF46 (the wreck's line),
                // then fall away from the blow.
                self.health_palette.show(cx, 0);
                e.vars[ANIM] = 0;
                e.vars[P2C] = cx.player.0;
                e.motion.stop();
                cx.play(self.sounds.death_music);
            }
            _ => {}
        }
        // $2B1AA: contact, 2 inside the field's states, else 1 (queued after
        // the routine so the hover's list-4 test does not see it). Its
        // `Object_CollidePlayer` (`$2B1B0`) is `Object_CollideList`: an
        // empty box is a point.
        let shield = e.vars[FLAGS] & SHIELD != 0;
        let touching = !matches!(e.routine, DEATH | CLEAR)
            && pose_hitbox(cx.mappings, e).collides(player_box(cx));
        e.timer = e.timer.saturating_sub(1);
        e.vars[FLAGS] &= !(SHIELD | HOLD_PALETTE);
        let bob = self.dispatch(cx, e);
        if e.state == EnemyState::Defeated && !e.dying {
            return; // freed at the request
        }
        if bob {
            Self::bob(cx, e);
        }
        if e.vars[FLAGS] & HOLD_PALETTE == 0 {
            self.cycle_palette(cx, e);
        }
        self.animate(e);
        // The charge's sparks read their parent's place on every update.
        let (_, dy, ..) = self.pieces.orbit;
        for spark in cx.effects.iter_mut() {
            if spark.kind == EffectKind::Orbit {
                spark.origin = (e.x, e.y + dy);
            }
        }
        // $2B126: the field is the pose's hitbox centre with $60 x $30.
        let bounds = cx
            .mappings
            .get(&e.mapping)
            .map_or(Hitbox::NONE, |mapping| mapping.bounds);
        e.body = if e.vars[FLAGS] & SHIELD != 0 {
            Some(Hitbox {
                half_width: self.field.0,
                half_height: self.field.1,
                ..bounds
            })
        } else {
            None
        };
        self.manage_shots(cx, e);
        if touching {
            strike(cx, self.contact[usize::from(shield)], None);
        }
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        if matches!(e.routine, SETUP | DEATH | CLEAR) {
            HitResponse::Pass
        } else if e.vars[FLAGS] & SHIELD != 0 {
            HitResponse::Reflect
        } else if e.timer > 0 {
            HitResponse::Pass
        } else {
            HitResponse::Accept
        }
    }
}
