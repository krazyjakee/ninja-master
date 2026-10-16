//! Round 1 boss: `Boss1` (`$2A518`), called through `Boss_Jumps`.
//!
//! 1-3 waterfall cave: the armoured warrior with sword and spear. He walks in
//! from the right in 12-pixel steps, thrusts high at a standing player and
//! sweeps low at a crouching one, waits in a guarding stance while the player
//! jumps or attacks, and after every hit backs off three steps and charges
//! across the arena. Thrown shurikens bounce off him except while he winds
//! up an attack or is struck from behind; slashes land in his stance too.
//!
//! [`Boss1`] follows the handler's `bra.w` slot table (`$2A562`); each
//! routine constant is the `obj_routine` byte offset and cites its code.
//! Every immediate and table is read from the ROM at load
//! (`boss1_tables_match_rom` pins them). The two list-4 type `$0F` weapon
//! boxes (`Func_01E898`) that `Boss1` allocates at set-up are modelled as
//! data ([`Weapon`]): they follow the boss and hit while his pose is theirs.
//!
//! Hit counter: `Data_0362DA` gives `+$43` = 11; each accepted point
//! subtracts one (`Func_02AEDE`) and the fight ends when it drops below
//! zero, so twelve points win. Natively that is `Enemy::health` = 12 with
//! the framework's lethal hit; the death routine then runs the explosions,
//! the palette fade and sets [`Ctx::next_substage`] (`Game_Requests` bit 1,
//! then `Mode_RoundClear`). Each hurt allocates the list-1 `$37` flasher,
//! which alternates white and the `Data_0362DA` row of the counter in
//! palette line 1 for 20 updates, so line 1 reddens as the counter falls
//! (#92).
//!
//! Emulator evidence (`obj/emu/states/r1-3.state`, player idle at the left
//! clamp): the walk steps 12 px every 15 frames, the high attack enters at
//! |dx| < 104 and holds `$241` 33 frames, `$242` 8, `$243` 38 and `$23D` 8
//! before the stance; a poked charge accelerates 0.25 px/frame to the 8.0
//! cap and knocks the player down (`PS_Knockdown`, damage 2); a poked death
//! routine spends 128 frames exploding, 32 fading out, and requests the next
//! substage 120 frames later.
//!
//! The "throw": the charge writes `plr_event` 1/2. The US ROM then clears
//! that byte in `Player_CheckDamage` (`$21D7E`, `andi.l #$00FB0EFF`) and
//! enters `PS_Knockdown`; native deliberately connects the sender to the
//! recovered `PS_Thrown` receiver so the otherwise-dead lift/turn/slam
//! sequence can run. The intro, the death blast and the fade write
//! `plr_event` 6 (`Player_ApplyEvent` `$1FD06`: no player sounds, B starts
//! nothing while the player still walks).
//!
//! The state tables (`Data_02A798`, `Data_0344EC`) read Joe's actual state
//! and facing, including the crouching low sweep. Movement-only combat
//! callers without a player pose retain a height/weapon inference.
//! The death sequence applies its CRAM fades and explosion
//! cadence; the cartridge's distinct list-1 `$38`/`$3B` animations use the
//! shared puff artwork. Sparks, dust and debris, art decompression at death,
//! and `Object_ListParams` changes are not modelled.
//! The hurt boss's 2-pixel shove of a slashing player (`$2A97C`) checks
//! the leading foot's terrain. Its `Func_00342E` arena clamp
//! (camera + `$20` .. + `$120`, Y at least camera + `$10`) is
//! `Ctx::clamp_player`.
use super::{
    Flasher, HealthPalette, blast_offsets, cap_speed,
    contact::{player_box, strike},
    fade_white_lines,
};
use crate::{
    combat::{Effect, EffectKind, Enemy, EnemyState, Hitbox, Reaction, Spawn, pose_hitbox},
    enemies::{Behavior, Ctx, HitResponse, Roster},
    game::Terrain,
    movement::PlayerEvent,
    rom::{Result, byte, long, signed, signed_byte, word, word_i, words},
    sound_events::Table,
};
use std::sync::Arc;

/// `Boss_Jumps` target of this round.
pub(crate) const HANDLER: usize = 0x2a518;

pub(crate) fn fight(rom: &[u8]) -> Result<Arc<dyn Behavior>> {
    Ok(Arc::new(Boss1::load(rom)?))
}

/// List-5 types this boss spawns and owns (registered by [`register_parts`]).
/// `Boss1`'s helpers are list-4 (`$0F`) and list-1 objects, not list 5.
pub(crate) const PART_KINDS: &[u16] = &[];

pub(crate) fn register_parts(_rom: &[u8], _roster: &mut Roster) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Player-state inference for the round 1 boss.

/// The player's `plr_state` as the handler would read it. Inferred (the
/// framework's `Ctx` does not carry it): an attack object within reach of
/// the body is `PS_Slash` (5), feet above `floor` are `PS_Jump` (16),
/// otherwise `PS_Idle` (1).
pub(super) fn inferred_player_state(cx: &Ctx, floor: i32) -> usize {
    if let Some(state) = cx.player_state {
        return state;
    }
    let (px, py) = cx.player;
    let attacking = cx.attacks.iter().any(|&(ax, ay, high_priority)| {
        high_priority == cx.current_high_priority()
            && (ax - px).abs() <= MELEE_REACH
            && (py - 80..=py + 8).contains(&ay)
    });
    if attacking {
        5
    } else if py < floor {
        16
    } else {
        1
    }
}
/// Native: horizontal reach within which an attack object counts as the
/// player's own sword (the `Sword` offsets are at most 16 px).
const MELEE_REACH: i32 = 40;

// ---------------------------------------------------------------------------

/// `obj_routine` byte offsets of the `$2A562` table.
const SETUP: u16 = 0x00; // $2A596
const WALK: u16 = 0x04; // $2A702
const DECIDE: u16 = 0x08; // $2A75C
const HIGH: u16 = 0x0c; // $2A7D0
const LOW: u16 = 0x10; // $2A89A
const HURT: u16 = 0x14; // $2A90A
const STANCE: u16 = 0x18; // $2AAE6
const CHARGE: u16 = 0x1c; // $2AC10
const BLAST: u16 = 0x20; // $2ACBA
const FADE: u16 = 0x24; // $2AD56
const ENTER: u16 = 0x28; // $2A696
const INTRO_WAIT: u16 = 0x2c; // $2A6A0
const INTRO_FADE: u16 = 0x30; // $2A6CA

/// `Enemy::timer` = `$2A`, `Enemy::counter` = `$2E`, `Enemy::phase` = `$32`;
/// `vars`:
const POSE: usize = 0; // $2C: index into Data_02A504 (walk/retreat step)
const HOLD: usize = 1; // $30: hurt pose countdown
const WAIT: usize = 2; // $34: stance wait, blast debris counter, fade steps
const PATIENCE: usize = 3; // $36: stance patience / blast sound countdown
const FLAGS: usize = 4; // obj_flags+1 bits 1, 2, 4 and $3C bit 3
const DELAY: usize = 5; // $28: hurt delay / fade cadence
const GUARD: usize = 6; // native: the tick's HitResponse
const FLASHER: usize = 7; // native: the `$37` flasher's `$2C`/`$2A` (`Flasher`)

/// `obj_flags+1` bit 1: re-decide from the stance after an attack.
const RETURNING: i32 = 1 << 1;
/// `obj_flags+1` bit 2: backing off after a hit.
const RETREAT: i32 = 1 << 2;
/// `obj_flags+1` bit 4: faded out and hidden.
const FADED: i32 = 1 << 4;
/// `$3C` bit 3: the counter dropped below zero.
const DEFEATED: i32 = 1 << 8;

const PASS: i32 = 0;
const DEFLECT: i32 = 1;
const ACCEPT: i32 = 2;

/// One list-4 type `$0F` object (`Func_01E898`): a box at (+-`dx`, `dy`)
/// from the boss's feet, `dx` toward his facing, active while his pose is
/// `mapping`. `guarded` is `$2A` = 1 (the thrust): a guard facing the boss
/// blocks it. Otherwise (the low sweep, `$2A` = 0) it also carries the
/// charge's `plr_event`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Weapon {
    pub(crate) mapping: u16,
    pub(crate) dx: i32,
    pub(crate) dy: i32,
    pub(crate) half_width: i32,
    pub(crate) half_height: i32,
    pub(crate) guarded: bool,
    pub(crate) damage: u8,
}
impl Weapon {
    /// The `move.b #n,d(a6)` operands at `at` (hitbox x, y, w, h), the
    /// `Data_02A50E`/`Data_02A512` pose and the `$2A` word at `guard`.
    fn read(rom: &[u8], at: usize, pose: usize, guard: usize, damage: usize) -> Result<Self> {
        let b = |i: usize| signed_byte(rom, at + 1 + i * 6);
        Ok(Self {
            mapping: word(rom, pose)?,
            dx: b(0)?.abs(),
            dy: b(1)?,
            half_width: b(2)?,
            half_height: b(3)?,
            guarded: word(rom, guard)? != 0,
            damage: byte(rom, damage)?,
        })
    }
    fn area(&self, e: &Enemy) -> Hitbox {
        Hitbox {
            x: e.x + if e.facing_left { self.dx } else { -self.dx },
            y: e.y + self.dy,
            half_width: self.half_width,
            half_height: self.half_height,
        }
    }
}

/// `Boss1` (`$2A518`).
pub(crate) struct Boss1 {
    /// `Data_02A504`: the ten poses `$23D..$246`, indexed by `$2C`.
    poses: Vec<u16>,
    /// `Data_02A896` / `Data_02A906`: pose index per attack phase.
    high: [u8; 4],
    low: [u8; 4],
    /// `Data_02A798`: 1 high thrust, 2 low sweep, negative stance, by
    /// `plr_state`.
    reaction: Vec<i8>,
    /// `Data_0344EC`: 1 for the player's slashing states.
    slashing: Vec<i8>,
    /// `Data_0362DA`: the hit counter `+$43`.
    counter: u8,
    enter_x: i32,
    walk_ticks: u16,
    walk_step: i32,
    reach: i32,
    attack_ticks: u16,
    phase_ticks: u16,
    thrust_extra: u16,
    low_ticks: u16,
    patience: i32,
    stance_wait: i32,
    hurt_pose: i32,
    hurt_last: i32,
    hurt_first: i32,
    hurt_second: i32,
    retreat_ticks: u16,
    retreat_extra: u16,
    retreat_steps: i32,
    retreat_step: i32,
    charge_pose: i32,
    charge_vx: i32,
    charge_ax: i32,
    speed_cap: i32,
    charge_right: i32,
    charge_left: i32,
    intro_ticks: i32,
    intro_steps: i32,
    grow: (i32, i32),
    thrust_grow: i32,
    stance_limit: usize,
    blast_y: i32,
    blast_offsets: Vec<(i32, i32)>,
    blast_ticks: u16,
    blast_sound_ticks: i32,
    fade_hold: i32,
    fade_steps: u16,
    death_palette: [u16; 16],
    /// `Data_0362DA`: line 1 by the hit counter `+$43` (12 rows, redder
    /// as it falls), shown by the `$37` flasher a hurt allocates.
    health_palette: HealthPalette,
    flasher: Flasher,
    /// `$2A958`: the flasher's duration (`$2A`).
    flash_ticks: u16,
    contact_damage: u8,
    charge_damage: u8,
    weapons: [Weapon; 2],
    katana: u8,
    spear: u8,
    charge_sound: u8,
    blast_sound: u8,
    death_voice: u8,
    fade_out: u8,
}

/// `Func_00075C` calls until a line is white: 15 nibble steps plus the
/// call that reports no change (emulator: 32 frames at one call per two).
const FADE_CALLS: i32 = 16;

impl Boss1 {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        let mut poses = Vec::with_capacity(10);
        for i in 0..10 {
            poses.push(word(rom, 0x2a504 + i * 2)?);
        }
        let table = |p: usize, n: usize| -> Result<Vec<i8>> {
            (0..n).map(|i| Ok(byte(rom, p + i)? as i8)).collect()
        };
        let four = |p: usize| -> Result<[u8; 4]> {
            Ok([
                byte(rom, p)?,
                byte(rom, p + 1)?,
                byte(rom, p + 2)?,
                byte(rom, p + 3)?,
            ])
        };
        Ok(Self {
            poses,
            high: four(0x2a896)?,
            low: four(0x2a906)?,
            reaction: table(0x2a798, 0x38)?,
            slashing: table(0x344ec, 0x4a)?,
            counter: byte(rom, 0x362db)?,
            enter_x: word_i(rom, 0x2a634)?,
            walk_ticks: word(rom, 0x2a726)?,
            walk_step: word_i(rom, 0x2a73c)?,
            reach: word_i(rom, 0x2a712)?,
            attack_ticks: word(rom, 0x2a778)?,
            phase_ticks: word(rom, 0x2a860)?,
            thrust_extra: word(rom, 0x2a87e)?,
            low_ticks: word(rom, 0x2a8ec)?,
            patience: i32::from(byte(rom, 0x2ab21)?),
            stance_wait: i32::from(byte(rom, 0x2ab3f)?),
            hurt_pose: word_i(rom, 0x2a926)?,
            hurt_last: word_i(rom, 0x2a9a0)?,
            hurt_first: i32::from(byte(rom, 0x2a921)?),
            hurt_second: i32::from(byte(rom, 0x2a9a7)?),
            retreat_ticks: word(rom, 0x2aa52)?,
            retreat_extra: word(rom, 0x2aac4)?,
            retreat_steps: word_i(rom, 0x2aa66)?,
            retreat_step: word_i(rom, 0x2aaca)?,
            charge_pose: word_i(rom, 0x2aaa2)?,
            charge_vx: long(rom, 0x2aa7e)?,
            charge_ax: signed(rom, 0x2aa84)?,
            speed_cap: long(rom, 0x34b0e)?,
            charge_right: word_i(rom, 0x2ac28)?,
            charge_left: word_i(rom, 0x2ac38)?,
            intro_ticks: word_i(rom, 0x2a698)?,
            intro_steps: word_i(rom, 0x2a6c2)?,
            // $2AE94: addi.b #$14,$12(a0) / addi.b #$20,$13(a0).
            grow: (
                i32::from(byte(rom, 0x2ae97)?),
                i32::from(byte(rom, 0x2ae9d)?),
            ),
            // $2A7F6: addi.b #$14,$13(a0) while winding up.
            thrust_grow: i32::from(byte(rom, 0x2a7f9)?),
            stance_limit: usize::from(word(rom, 0x2ab74)?),
            blast_y: signed_byte(rom, 0x2a9e5)?,
            blast_offsets: blast_offsets(rom)?,
            blast_ticks: 0x80,
            blast_sound_ticks: word_i(rom, 0x2accc)?,
            fade_hold: word_i(rom, 0x2ae16)?,
            fade_steps: word(rom, 0x2ae44)?,
            death_palette: words(rom, 0x2a656)?,
            health_palette: HealthPalette::flasher(rom, usize::from(word(rom, 0x2a954)?))?,
            flasher: Flasher::load(rom)?,
            flash_ticks: word(rom, 0x2a95a)?,
            contact_damage: byte(rom, 0x2af7b)?,
            charge_damage: byte(rom, 0x2acb5)?,
            weapons: [
                Weapon::read(rom, 0x2a5d6, 0x2a50e, 0x2a5f6, 0x1e9b1)?,
                Weapon::read(rom, 0x2a60a, 0x2a512, 0x2a62a, 0x1e9b1)?,
            ],
            katana: Table::Effect.command_at(rom, 0x2a86b)?,
            spear: Table::Effect.command_at(rom, 0x2a8fd)?,
            charge_sound: Table::Effect.command_at(rom, 0x2aaad)?,
            blast_sound: Table::Effect.command_at(rom, 0x2acd3)?,
            // $2AA3E moveq #2 / Func_004A66: voice 2, BOSS DEATH.
            death_voice: Table::Voice.command_at(rom, 0x2aa3f)?,
            // Func_004A88: the direct fade-out command.
            fade_out: byte(rom, 0x4a8b)?,
        })
    }

    /// `Func_02AF46`: `obj_frame` = `Data_02A504[$2C]`.
    fn pose(&self, e: &mut Enemy) {
        let index = e.vars[POSE].clamp(0, self.poses.len() as i32 - 1) as usize;
        e.mapping = self.poses[index];
    }
    fn pose_at(&self, index: u8) -> u16 {
        self.poses[usize::from(index).min(self.poses.len() - 1)]
    }
    /// `Data_02A798[plr_state]`.
    fn reaction(&self, state: usize) -> i8 {
        self.reaction.get(state).copied().unwrap_or(-1)
    }
    /// The pose's hitbox grown by (`dw`, `dh`) (`addi.b` on `$12`/`$13`).
    fn grown(cx: &Ctx, e: &Enemy, dw: i32, dh: i32) -> Option<Hitbox> {
        cx.mappings.get(&e.mapping).map(|m| Hitbox {
            half_width: m.bounds.half_width + dw,
            half_height: m.bounds.half_height + dh,
            ..m.bounds
        })
    }
    /// `Func_02AE94`: the grown box deflects every player attack.
    fn deflect_box(&self, cx: &Ctx, e: &mut Enemy) {
        e.body = Self::grown(cx, e, self.grow.0, self.grow.1);
        e.vars[GUARD] = DEFLECT;
    }
    /// The boss touches the player's body with its pose box: `Func_02AF5A`
    /// runs `Object_LoadHitbox` and `Object_CollidePlayer` (`$2AF5E`,
    /// `$2AF64`).
    fn touches(cx: &Ctx, e: &Enemy) -> bool {
        cx.collide_player(pose_hitbox(cx.mappings, e))
    }
    /// `$2AE54`: after an attack, walk if the player is out of reach,
    /// otherwise wait in the stance.
    fn finish(&self, e: &mut Enemy, distance: i32) {
        if distance >= self.reach {
            e.vars[POSE] = 0;
            e.timer = self.walk_ticks;
            e.routine = WALK;
            self.pose(e);
        } else {
            e.routine = STANCE;
            e.vars[FLAGS] |= RETURNING;
            e.vars[POSE] = 0;
            e.mapping = self.poses[0];
        }
    }
    /// `$2AB72`: the stance's hit rules.
    fn stance_guard(&self, cx: &Ctx, e: &mut Enemy, state: usize) {
        if state >= self.stance_limit {
            e.body = None;
            e.vars[GUARD] = PASS;
            return;
        }
        // The original compares the two tile-word flip bits ($2AB7C).
        if e.facing_left == cx.player_facing_left {
            e.body = Self::grown(cx, e, self.grow.0, self.grow.1);
            // $2ABAC also accepts attack type $4C (the dash kick), which
            // is absent from the slashing-state table.
            let slash = self.slashing.get(state).copied() == Some(1)
                || state == crate::movement::state::DASH_KICK;
            e.vars[GUARD] = if slash { ACCEPT } else { DEFLECT };
        } else {
            e.body = Self::grown(cx, e, 0, self.grow.1);
            e.vars[GUARD] = ACCEPT;
        }
    }
    /// The two list-1 effects `Func_034B70` places around the box centre.
    fn explosion(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.counter as usize >= self.blast_offsets.len() {
            e.counter = 0;
        }
        let (dx, dy) = self.blast_offsets[usize::from(e.counter)];
        let first = cx.shared.puff_seq.first().copied().unwrap_or(0);
        let (x, y) = (e.x + dx, e.y + self.blast_y + dy);
        cx.effect(Effect::new(EffectKind::Puff, x, y, first));
        let mut second = Effect::new(EffectKind::Puff, x, y, first);
        second.flip = cx.random() & 1 != 0;
        cx.effect(second);
        e.counter += 1;
    }

    /// `$2ACEC`: the pair of list-1 type `$0D` blast records every 30 ticks.
    fn debris_pair(cx: &mut Ctx, e: &Enemy) {
        let first = cx.shared.puff_seq.first().copied().unwrap_or(0);
        for flip in [false, true] {
            let mut blast = Effect::new(EffectKind::Puff, e.x, e.y + 8, first);
            blast.flip = flip;
            cx.effect(blast);
        }
    }
    /// The accepted hit: `Func_02AEDE` routine `$14` with `$28` = 2.
    fn enter_hurt(e: &mut Enemy) {
        e.routine = HURT;
        e.vars[DELAY] = 2;
        e.motion.stop();
    }

    fn step(&self, cx: &mut Ctx, e: &mut Enemy, terrain: &dyn Terrain) {
        let player_left = cx.player_left(e.x);
        let distance = cx.player_dx(e.x);
        let state = inferred_player_state(cx, e.y);
        // Func_02AF5A for routines below $1C except $14: contact damage.
        if e.routine < CHARGE && e.routine != HURT {
            if Self::touches(cx, e) {
                cx.contact_player(self.contact_damage);
            }
            if e.routine < DECIDE {
                e.facing_left = !player_left;
            }
        }
        e.body = None;
        e.vars[GUARD] = PASS;
        match e.routine {
            SETUP => {
                // $2A596.
                e.health = u16::from(self.counter) + 1;
                e.hurt_routine = HURT;
                e.death_routine = HURT;
                e.x = cx.camera.0 + self.enter_x;
                e.timer = self.walk_ticks;
                e.vars[POSE] = 0;
                e.vars[PATIENCE] = self.patience;
                e.routine = ENTER;
                self.pose(e);
            }
            ENTER | INTRO_WAIT => {
                if e.routine == ENTER {
                    e.vars[WAIT] = self.intro_ticks;
                    e.routine = INTRO_WAIT;
                }
                cx.player_event(PlayerEvent::Freeze);
                // subq.w / bpl: the wait ends when the counter goes negative.
                e.vars[WAIT] -= 1;
                if e.vars[WAIT] < 0 {
                    // Data_02A656 becomes the fade target of palette line 2.
                    e.vars[WAIT] = self.intro_steps;
                    e.routine = INTRO_FADE;
                    self.intro_fade(cx, e);
                }
            }
            INTRO_FADE => self.intro_fade(cx, e),
            WALK => {
                self.deflect_box(cx, e);
                if distance < self.reach {
                    e.routine = DECIDE;
                } else {
                    e.timer = e.timer.saturating_sub(1);
                    if e.timer == 0 {
                        e.timer = self.walk_ticks;
                        e.x += if player_left {
                            -self.walk_step
                        } else {
                            self.walk_step
                        };
                        e.vars[POSE] = if e.vars[POSE] >= 3 {
                            0
                        } else {
                            e.vars[POSE] + 1
                        };
                        self.pose(e);
                    }
                }
            }
            DECIDE => {
                self.deflect_box(cx, e);
                let class = self.reaction(state);
                if class < 0 {
                    e.routine = STANCE;
                    e.vars[FLAGS] |= RETURNING;
                } else {
                    e.counter = self.attack_ticks;
                    e.phase = 0;
                    e.routine = if class == 1 { HIGH } else { LOW };
                }
            }
            HIGH => {
                if e.phase == 3 {
                    self.deflect_box(cx, e);
                } else {
                    e.body = Self::grown(cx, e, 0, self.thrust_grow);
                    e.vars[GUARD] = ACCEPT;
                }
                e.mapping = self.pose_at(self.high[usize::from(e.phase.min(3))]);
                e.counter = e.counter.saturating_sub(1);
                if e.counter == 0 {
                    e.counter = self.phase_ticks;
                    match e.phase {
                        0 => cx.play(self.katana),
                        1 => e.counter += self.thrust_extra,
                        3 => {
                            self.finish(e, distance);
                            return;
                        }
                        _ => {}
                    }
                    e.phase += 1;
                }
            }
            LOW => {
                e.body = Self::grown(cx, e, 0, self.thrust_grow);
                e.vars[GUARD] = ACCEPT;
                e.mapping = self.pose_at(self.low[usize::from(e.phase.min(3))]);
                e.counter = e.counter.saturating_sub(1);
                if e.counter == 0 {
                    if e.phase == 2 {
                        self.finish(e, distance);
                        return;
                    }
                    e.counter = self.low_ticks;
                    e.phase += 1;
                    if e.phase == 1 {
                        cx.play(self.spear);
                    }
                }
            }
            HURT => self.hurt(cx, e, player_left, terrain),
            STANCE => {
                if e.vars[FLAGS] & RETURNING == 0 {
                    e.vars[WAIT] -= 1;
                    if e.vars[WAIT] <= 0 {
                        e.routine = WALK;
                        return;
                    }
                } else if self.reaction(state) < 0 {
                    e.vars[PATIENCE] -= 1;
                    if e.vars[PATIENCE] <= 0 {
                        e.routine = HIGH;
                        e.vars[PATIENCE] = self.patience;
                        e.counter = self.attack_ticks;
                        e.phase = 0;
                        e.vars[FLAGS] &= !RETURNING;
                        return;
                    }
                } else {
                    e.vars[FLAGS] &= !RETURNING;
                    e.vars[WAIT] = self.stance_wait;
                    // Facing the player: walk at once.
                    if e.facing_left != player_left {
                        e.routine = WALK;
                        e.vars[PATIENCE] = self.patience;
                        return;
                    }
                }
                self.stance_guard(cx, e, state);
            }
            CHARGE => {
                cap_speed(e, self.speed_cap);
                let screen = e.x - cx.camera.0;
                let gone = if e.facing_left {
                    screen >= self.charge_right
                } else {
                    screen < 0 && -screen >= self.charge_left
                };
                if gone {
                    e.routine = WALK;
                    e.vars[FLAGS] &= !RETREAT;
                    e.timer = self.walk_ticks;
                    e.motion.stop();
                    e.vars[POSE] = 0;
                    self.pose(e);
                    return;
                }
                e.vars[GUARD] = DEFLECT;
                if Self::touches(cx, e) {
                    cx.player_event(Self::throw(e));
                    cx.contact_player(self.charge_damage);
                }
            }
            BLAST => {
                cx.player_event(PlayerEvent::Freeze);
                e.timer = e.timer.wrapping_add(1);
                e.vars[PATIENCE] -= 1;
                if e.vars[PATIENCE] <= 0 {
                    e.vars[PATIENCE] = self.blast_sound_ticks;
                    cx.play(self.blast_sound);
                }
                // $34 counts to 30 for the list-1 $0D debris pair (not drawn).
                e.vars[WAIT] += 1;
                if e.vars[WAIT] == 30 {
                    e.vars[WAIT] = 0;
                    Self::debris_pair(cx, e);
                }
                if e.timer & 3 == 0 {
                    self.explosion(cx, e);
                }
                if e.timer & self.blast_ticks != 0 {
                    // Func_02A466 adds the score (done at the lethal hit).
                    e.vars[DELAY] = 2;
                    e.vars[WAIT] = 0;
                    e.routine = FADE;
                }
                self.pose(e);
            }
            FADE => self.fade(cx, e),
            _ => {}
        }
    }

    /// `$2A6CA`: every fourth frame one fade step of palette line 2.
    fn intro_fade(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.player_event(PlayerEvent::Freeze);
        if e.age & 3 != 0 {
            return;
        }
        e.vars[WAIT] -= 1;
        if e.vars[WAIT] < 0 {
            e.timer = self.walk_ticks;
            e.routine = WALK;
        }
    }

    /// `$2A90A`: the hurt poses, the retreat and the start of the charge or
    /// of the death sequence.
    fn hurt(&self, cx: &mut Ctx, e: &mut Enemy, player_left: bool, terrain: &dyn Terrain) {
        if e.vars[DELAY] != 0 {
            e.vars[DELAY] -= 1;
            if e.vars[DELAY] != 0 {
                return;
            }
            e.vars[PATIENCE] = self.patience;
            e.vars[HOLD] = self.hurt_first;
            e.vars[POSE] = self.hurt_pose;
            self.pose(e);
            e.facing_left = !player_left;
            // $2A944: the list-1 `$37` flasher, table 1, 20 updates.
            Flasher::start(&mut e.vars[FLASHER], self.flash_ticks);
        }
        if e.vars[FLAGS] & RETREAT != 0 {
            self.retreat(cx, e, player_left);
            return;
        }
        if cx
            .player_state
            .is_some_and(|state| state == 29 || self.slashing.get(state) == Some(&1))
        {
            // $2A97C: a slashing Joe recoils two pixels, after $1F520's
            // leading-foot terrain probe. Preserve the boss's facing sign.
            let dx: i32 = if player_left { -2 } else { 2 };
            let width = cx.player_body.map_or(15, |body| body.half_width);
            if !terrain.object_solid_at(
                cx.player_at.0 + width * dx.signum() + dx,
                cx.player_at.1 - 1,
                cx.current_high_priority(),
            ) {
                cx.signals.push(crate::combat::Signal::PushPlayer(dx));
            }
        }
        e.vars[HOLD] -= 1;
        if e.vars[HOLD] > 0 {
            return;
        }
        if e.vars[POSE] != self.hurt_last {
            e.vars[HOLD] = self.hurt_second;
            e.vars[POSE] += 1;
            self.pose(e);
        } else if e.vars[FLAGS] & DEFEATED != 0 {
            // $2A9BE: the death sequence.
            e.routine = BLAST;
            e.timer = 0;
            e.counter = 0;
            e.vars[WAIT] = 0;
            e.vars[PATIENCE] = 1;
            cx.play(self.death_voice);
        } else {
            e.vars[FLAGS] |= RETREAT;
            e.vars[POSE] = 0;
            e.timer = self.retreat_ticks;
            self.pose(e);
        }
    }

    /// `$2AA5C`: three steps back, then the charge.
    fn retreat(&self, cx: &mut Ctx, e: &mut Enemy, player_left: bool) {
        e.timer = e.timer.saturating_sub(1);
        if e.timer != 0 {
            self.pose(e);
            return;
        }
        if e.vars[POSE] == self.retreat_steps {
            e.routine = CHARGE;
            e.vars[FLAGS] &= !RETREAT;
            // Away from the player at 5 px/frame, accelerating back at him.
            e.facing_left = !player_left;
            let away = if player_left { 1 } else { -1 };
            e.motion.vx = away * self.charge_vx;
            e.motion.ax = -away * self.charge_ax.abs();
            e.vars[POSE] = self.charge_pose;
            self.pose(e);
            cx.play(self.charge_sound);
            return;
        }
        e.timer = self.retreat_ticks;
        if e.vars[POSE] == 2 {
            e.timer += self.retreat_extra;
        }
        e.x += if e.facing_left {
            -self.retreat_step
        } else {
            self.retreat_step
        };
        e.vars[POSE] += 1;
        self.pose(e);
    }

    /// `$2AD56`: explosions until both palette lines are white (`Func_00075C`
    /// every second frame), the hidden wait, the fade back and the request.
    fn fade(&self, cx: &mut Ctx, e: &mut Enemy) {
        cx.player_event(PlayerEvent::Freeze);
        let faded = e.vars[FLAGS] & FADED != 0;
        if !faded {
            e.vars[PATIENCE] -= 1;
            if e.vars[PATIENCE] <= 0 {
                e.vars[PATIENCE] = self.blast_sound_ticks;
                cx.play(self.blast_sound);
            }
            if e.vars[PATIENCE] & 3 == 0 {
                self.explosion(cx, e);
            }
        }
        e.vars[DELAY] -= 1;
        if e.vars[DELAY] > 0 {
            return;
        }
        e.vars[DELAY] = 2;
        if !faded {
            fade_white_lines(cx, 1..=2);
            e.vars[WAIT] += 1;
            if e.vars[WAIT] >= FADE_CALLS {
                cx.play(self.fade_out);
                e.vars[FLAGS] |= FADED;
                e.timer = 0;
                e.mapping = 0;
                e.vars[POSE] = self.fade_hold;
            }
        } else if e.vars[POSE] > 0 {
            e.vars[POSE] -= 1;
        } else {
            cx.fade_palette(2, self.death_palette);
            e.timer += 1;
            if e.timer == self.fade_steps {
                // Game_Requests bit 1: Mode_RoundClear, then round 2.
                cx.next_substage();
                Ctx::free(e);
            }
        }
    }

    /// The two `Func_01E898` boxes, updated after the boss (`Func_01CC0A`).
    fn weapons(&self, cx: &mut Ctx, e: &Enemy) {
        if !matches!(e.state, EnemyState::Waiting | EnemyState::Hurt) {
            return;
        }
        let body = player_box(cx);
        for weapon in &self.weapons {
            if e.mapping != weapon.mapping || !weapon.area(e).intersects(body) {
                continue;
            }
            let guard_from = weapon.guarded.then_some(e.x);
            if guard_from.is_none() && e.routine == CHARGE {
                cx.player_event(Self::throw(e));
            }
            strike(cx, weapon.damage, guard_from);
        }
    }
    /// The charge's `plr_event`: 1 (thrown left) charging right, 2 charging
    /// left.
    fn throw(e: &Enemy) -> PlayerEvent {
        if e.facing_left {
            PlayerEvent::ThrownRight
        } else {
            PlayerEvent::ThrownLeft
        }
    }
}

impl Behavior for Boss1 {
    fn label(&self, _kind: u16) -> String {
        format!("Boss1 ${HANDLER:05X}")
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        self.poses[0]
    }
    fn touches_player(&self) -> bool {
        false // Contact comes from Func_02AF5A, not Boss_Records' zero damage.
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        cx.clamp_player(); // Boss1's first instruction
        // The flasher reads `+$43` (the counter, 0 once it goes negative).
        let row = usize::from(e.health.saturating_sub(1));
        self.flasher
            .step(cx, &mut e.vars[FLASHER], &self.health_palette, row);
        match cx.process_hit(e) {
            Some(Reaction::Hurt) => Self::enter_hurt(e),
            Some(Reaction::Death) => {
                e.vars[FLAGS] |= DEFEATED;
                Self::enter_hurt(e);
            }
            _ => {}
        }
        self.step(cx, e, terrain);
        if e.state == EnemyState::Defeated && !e.dying {
            return; // freed at the request
        }
        self.weapons(cx, e);
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        // Func_02AEDE enters HURT immediately. The native framework
        // defers that dispatch, so block additional objects in this pass.
        if matches!(enemy.reaction, Some(Reaction::Hurt | Reaction::Death)) {
            return HitResponse::Pass;
        }
        match enemy.vars[GUARD] {
            DEFLECT => HitResponse::Deflect,
            ACCEPT => HitResponse::Accept,
            _ => HitResponse::Pass,
        }
    }
    fn hit_by(&self, enemy: &Enemy, kind: u16, damage: u8) -> HitResponse {
        // Func_02AF5A runs before the routine's weapon guard. Its
        // Func_032F00 result of 8 is Kariu/Mijin and goes straight to
        // Func_02AEDE, even while Joe's ninjutsu state skips stance_guard.
        if damage == 8
            && enemy.routine < CHARGE
            && enemy.routine != HURT
            && !matches!(enemy.reaction, Some(Reaction::Hurt | Reaction::Death))
        {
            HitResponse::Accept
        } else {
            self.hit(enemy, kind)
        }
    }
    fn ninjutsu_damage(&self, _kind: u16, _damage: u8) -> u8 {
        8 // Func_032F00, independent of the dragon/blast damage byte.
    }
}
