//! Hits on the player: `Player_CheckDamage` (`$21C30`), the Ikazuchi
//! shield and the immunity each hit grants.
use super::{Blast, Combat, Effect, EffectKind, in_sound_window};
use crate::{movement::PlayerEvent, sound_events::SoundEvent};

/// One boss-owned CRAM operation, applied by `Game` after the combat pass.
/// Keeping it as data leaves the recovered boss logic independent of SDL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteOp {
    /// `$75C`: move one palette line one nibble step toward white.
    FadeWhite(u8),
    /// `$6E8`: move one palette line one nibble step toward a ROM target.
    FadeTo(u8, [u16; 16]),
    /// Fade only the first colours, preserving independently cycled entries.
    FadeFirst(u8, u8, [u16; 16]),
    /// Fade one colour (the mech's entrance eyes).
    FadeColor(u8, u8, u16),
    /// Directly fill one line (Boss4 writes `$0EEE0EEE`).
    Fill(u8, u16),
    /// Directly fill the first `n` colours of a line (the 3-2 laser blacks
    /// line 0's first eleven, `$2A168`).
    FillFirst(u8, u8, u16),
    /// Move one line back toward the stage palette loaded for this part.
    Restore(u8),
    /// Copy one line back from the stage palette loaded for this part
    /// (Boss2's `$FF16FA` fade target into CRAM, `$2B314`).
    Reload(u8),
    /// Copy a whole line (Boss3's `Func_02CE0A` health palette).
    Load(u8, [u16; 16]),
    /// Copy eight colours from index `first` (the samurai's `$FFF020` and
    /// `$FFF022` writes, `Func_01B9F8`/`Func_01BEDA`/`Func_01BEF6`).
    Words(u8, u8, [u16; 8]),
    /// Write one colour: line, index, value (Boss3's backdrop `$FFF000`).
    Color(u8, u8, u16),
    /// Rotate `count` colours from `first` one place toward the end, the
    /// last wrapping to `first` (Boss3's `Func_02D072`).
    Rotate(u8, u8, u8),
    /// Point the H-interrupt at a CRAM reload (`Some`) or at `IRQ_Default`
    /// (`None`). Boss4 rewrites `IRQ_JumpTarget` on every update; `Game`
    /// keeps the last choice.
    HInt(Option<CramReload>),
    /// Set VDP register `$07`, the backdrop's CRAM entry (Boss7's `$8720`).
    Backdrop(u8),
}

/// An H-interrupt handler's mid-frame CRAM reload (Boss4's `Func_02EF80`):
/// from display line `line` down, CRAM line `cram_line` holds `words`
/// until V-blank uploads the `$FFF000` copy again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CramReload {
    pub line: u16,
    pub cram_line: u8,
    pub words: [u16; 16],
}

/// One boss-owned VDP plane operation, applied by `Game` to the live
/// boss-drawn planes (`stage::BossRaster`) after the combat pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaneOp {
    /// `$32922`: one of the six revolving panels in the kabuki room.
    KabukiPanel { door: u8, frame: u8 },
    /// `$30634` and `$31AF2`: the mech's body map and world placement.
    MechBody { x: i32, y: i32, flip: bool },
    /// `Plane_DrawWordMapHeader` of the ROM block at this address.
    WordMap(u32),
    /// `Plane_DrawByteMapHeader` (`$4264`): the same header with byte
    /// cells.
    ByteMap(u32),
    /// `Func_00429A` (LZ word map) as called by the `lea`, `move.l`,
    /// `move.w`, `moveq`, `moveq` operand sequence at this ROM address
    /// (`stage::lz_word_map_call`).
    LzWordMap(u32),
    /// Write `cell` over a `width` x `height` block at a VRAM address.
    Fill {
        vram: u16,
        width: u8,
        height: u8,
        cell: u16,
    },
    /// Rewrite a block from the name table as the stage loaded it.
    Restore { vram: u16, width: u8, height: u8 },
    /// H-scroll of each 8-line band and V-scroll of each 2-cell column of
    /// a plane, as VDP values.
    Scroll {
        plane: Plane,
        bands: [i16; 28],
        columns: [i16; 20],
    },
    /// H-scroll of `count` lines from `first`, added to the plane's camera
    /// placement.
    HScroll {
        plane: Plane,
        first: u8,
        count: u8,
        value: i16,
    },
    /// VDP register `$0B` = 3 and Boss2's per-line plane A wave
    /// (`Func_02B832`, [`crate::stage::LineWave`]) on the stage's own plane.
    Wave { amplitude: u16, phase: u16 },
    /// VDP register `$0B` = 0 again: whole-screen scroll ends the wave.
    FullScroll,
    /// `Boss_LoadArt` with a zero-based round table index (Boss7's form changes).
    BossArt(u8),
    /// Boss7's intro writes both H-scroll planes; mode 7 enables $295E.
    FinalBossScroll {
        mode: u8,
        amplitude: u16,
        phase: u16,
    },
}

/// A VDP scroll plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plane {
    A,
    B,
}

/// `plr_hurt` after `Player_ApplyHit` on foot: 48 ticks for a hit of 0
/// (`$21E86`), 80 for 1 (`$21E5E`, the shocked state too). A knockdown sets
/// none; its handler holds 64 instead (`KNOCKDOWN_IMMUNITY`).
pub fn hit_immunity(damage: u8) -> Option<u16> {
    match damage {
        0 => Some(48),
        1 => Some(80),
        _ => None,
    }
}
/// `PlayerState_Knockdown` (`$20306`) writes 64 on every update, so the
/// immunity lasts the whole state and 64 ticks more.
pub const KNOCKDOWN_IMMUNITY: u16 = 64;
/// `PlayerState_Thrown` (`$203A8`) writes 96 on every update.
pub const THROWN_IMMUNITY: u16 = 96;
/// `PlayerState_HorseHurt`/`PlayerState_SurfHurt` (`$20358`, `$20376`)
/// write 32 on every update.
pub const MOUNTED_HURT_IMMUNITY: u16 = 32;

/// The path by which a hit reached `Player_ApplyHit` (`$21DB0`). Only one
/// is taken per update, in this order (`Player_CheckDamage`, `$21C30`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitSource {
    /// Body contact an enemy reported before the player update
    /// (`Object_ProcessHit` `$1285C` sets the player's bit 14).
    Contact,
    /// Material 5 at the feet, a wall or the head (`Player_SpikeDamage`,
    /// `$21C5A`). A head hit has the extra push-down after `Player_ApplyHit`.
    Spikes { head: bool },
    /// The first overlapping list-4 object (`Object_CollideL4`, `$21CA4`):
    /// an enemy shot or a time-bomb flame, at X `x`.
    Shot { x: i32 },
}
/// A hit on the player: its path, `obj_health` (the health it takes) and
/// damage code `$2F` (`obj_damage`), which selects the shocked state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerHit {
    pub source: HitSource,
    pub damage: u8,
    pub shocked: bool,
}
impl PlayerHit {
    pub fn contact(damage: u8) -> Self {
        Self {
            source: HitSource::Contact,
            damage,
            shocked: false,
        }
    }
    pub fn shot(x: i32, damage: u8) -> Self {
        Self {
            source: HitSource::Shot { x },
            damage,
            shocked: false,
        }
    }
    pub fn spikes(head: bool, shocked: bool) -> Self {
        Self {
            source: HitSource::Spikes { head },
            damage: 1,
            shocked,
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct CombatEvents {
    /// Last `plr_event` write made by the object update pass.
    pub player_event: Option<PlayerEvent>,
    /// A non-lethal hit: the hurt state and its voice, including damage 0.
    pub player_damage: bool,
    /// The non-lethal hit behind `player_damage`, for the player's reaction.
    pub hit: Option<PlayerHit>,
    /// Health reached zero (`Player_SubtractHealth` `$21EAA` calls
    /// `Player_Die`): the death sequence starts (`crate::death`). Lives are
    /// only spent once it ends, in `Mode_Game` state `$20`.
    pub lethal: bool,
    /// Resolved sound commands ($5B56 + effect ID * 16), ready for play_sfx.
    pub sound_ids: Vec<u8>,
    /// Damage of a hit the player's guard cancelled ($21D4E); 0 when none.
    pub blocked: u8,
    /// The air stab struck an enemy ($252EA sets player +$40).
    pub bounce: bool,
    /// `Player_ForceSurfJump` (`$2292C`, from Boss4's `Func_02F1EA`) put
    /// the surfer in `PS_SurfJump` at cursor `$100` with this `plr_drift`
    /// (whole pixels added to the X velocity while rising).
    pub surf_jump: Option<i32>,
    /// What the object handlers asked of the game this update, in object
    /// list order (all but [`Signal::RaisePlayerPriority`] and
    /// [`Signal::Shove`], which `Combat` applies itself).
    pub signals: Vec<Signal>,
}

impl CombatEvents {
    /// `$FF0029` bit 1 was requested ([`Signal::NextSubstage`]).
    pub fn next_substage(&self) -> bool {
        self.signals.contains(&Signal::NextSubstage)
    }
    /// The autoscroll velocity was cleared ([`Signal::StopAutoscroll`]).
    pub fn stop_autoscroll(&self) -> bool {
        self.signals.contains(&Signal::StopAutoscroll)
    }
    /// Bits written to `$FF0028` ([`Signal::GameFlags`]).
    pub fn game_flags(&self) -> u16 {
        self.signals.iter().fold(0, |flags, signal| match signal {
            Signal::GameFlags(bits) => flags | bits,
            _ => flags,
        })
    }
    /// CRAM operations, in order ([`Signal::Palette`]).
    pub fn palette_ops(&self) -> impl Iterator<Item = PaletteOp> + '_ {
        self.signals.iter().filter_map(|signal| match *signal {
            Signal::Palette(op) => Some(op),
            _ => None,
        })
    }
    /// Plane operations, in order ([`Signal::Plane`]).
    pub fn plane_ops(&self) -> impl Iterator<Item = PlaneOp> + '_ {
        self.signals.iter().filter_map(|signal| match *signal {
            Signal::Plane(op) => Some(op),
            _ => None,
        })
    }
}

/// A request an object handler makes of the game during the update pass
/// (`Ctx::signal`), carried out after it: the RAM flags and buffers the
/// ROM handlers write for the stage and the main loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// `$FF0029` bit 1: a boss or end guard finished its death sequence and
    /// the round clear / next substage is requested (`Boss_Update`, #52).
    /// The game raises `REQUEST_NEXT_SUBSTAGE` from it.
    NextSubstage,
    /// An end guard's first update wrote 0 to the autoscroll velocity
    /// `$FF0180` (the ported 4-1 `$18`, `$13F7E`; `crate::special`).
    StopAutoscroll,
    /// A handler ORed `$8000` into the player's `obj_tile` (`Boss5`
    /// `$30654`); it precedes this update's player collisions.
    RaisePlayerPriority,
    /// Bits ORed into `$FF0028` (`Game_Flags`).
    GameFlags(u16),
    /// A CRAM change made after the stage palette routine.
    Palette(PaletteOp),
    /// A boss-drawn plane or scroll change.
    Plane(PlaneOp),
    /// Foreground VSRAM words written by a stage object (`$1983C/$1988E`).
    PlaneAVScroll([i32; crate::stage::COLUMNS]),
    /// A list-7 item scheduled with `$361D2`, for `crate::stage_objects`.
    Drop(crate::stage_objects::Drop),
    /// `Func_00342E` called by a handler (`Boss1`): the player within
    /// camera X + 32 ..= + 288 and no higher than camera Y + 16.
    ClampPlayer,
    /// Boss5's unsigned X limit, independent of the screen clamp.
    PlayerMaxX(i32),
    /// Boss5 setup writes the arena's live maximum camera X (`$FF014A`).
    CameraMaxX(i32),
    /// `$3326`'s transient vertical jolt, also applied to sprites.
    VScrollOffset(i32),
    /// Positions of Boss5's two list-6 hand platforms; `None` frees them.
    MechHands(Option<[(i32, i32); 2]>),
    /// A `Layout_Buffer` metatile write (`Func_015054`), applied after the
    /// stage objects' update with theirs.
    Terrain(crate::stage::TerrainEdit),
    /// `Func_03775E` with `d6` (mapping nibble and X) and `d5` (Y): a
    /// list-1 "GO" marker (`crate::game`'s markers).
    Marker(u16, u16),
    /// Whole pixels added to the player's X word (`addq.w #n,$20(a3)`), as
    /// the 1-1 arena gate (`$1C51E`) shoves a player that touches it.
    PushPlayer(i32),
    /// `$19F56`: the ground grabber writes the player's position words
    /// and clears both velocities and accelerations.
    HoldPlayer((i32, i32)),
    /// `Func_037244` with a part's box: overlapping the player's, it re-arms
    /// the pusher (`combat::shove`). `Combat` applies it itself.
    Shove(super::Hitbox),
    /// `plr_push`, native units per tick, which the player's next
    /// `Player_Physics` adds to its X velocity once (the pusher's write).
    PushVelocity(i32),
    /// `$FF0029` bit 7 (Boss2's `Func_02B832`): `Input_ReadPort` (`$3996`)
    /// swaps Left and Right for the player's next input read.
    SwapLeftRight,
    /// `eori.w #$800,obj_tile` of the player (Boss2's flashes): the
    /// player turns around.
    TurnPlayer,
    /// `ori.w #$800,plr_ctrl` of the player (Boss2's flash fades):
    /// `Player_StartNinjutsu` (`$22E8E`) refuses A.
    BlockNinjutsu,
}

/// Where the tested list-4 object met the player, and its own reaction
/// when it replaces the spark ([`super::Projectile::contact_blast`]).
pub(super) type Struck = ((i32, i32), Option<Blast>);

impl Combat {
    /// `Player_CheckDamage` ran on the last `tick`: no immunity was left
    /// at its start and the player is not dying.
    /// Health loss from `plr_event` `$A` and the delayed thrown slam. It
    /// bypasses immunity and the shield, but keeps the invincibility cheat.
    pub(crate) fn subtract_event_health(&mut self, damage: u8) -> bool {
        if self.invincible || damage == 0 || self.dying {
            return false;
        }
        self.hud.health = self.hud.health.saturating_sub(damage);
        if self.hud.health == 0 {
            self.die();
            true
        } else {
            false
        }
    }
    /// `Player_CheckDamage` (`$21C30..$21D86`) on an update without
    /// immunity: body contact first, then spikes, and only then
    /// `Object_CollideL4`, which stops at
    /// the first overlapping list-4 object: this module's enemy shots in list
    /// order, then `pending` (a time-bomb flame). Shots that are not tested
    /// keep flying, as does one below the horse's height window or one met
    /// in the first 20 ticks of the dash slash. With +$2A
    /// bit 15 (a guard or a sword swing) a frontal shot whose hitbox centre
    /// is at most 40 px below the reference (feet-72, or feet-40 for the
    /// previous update's low body) is blocked: no hit, `blocked` recoil
    /// (`$21CFC..$21D62`). Also returns the tested shot's position for its
    /// spark, or its `$2C` explosion.
    pub(super) fn check_player_damage(
        &mut self,
        contact: Option<PlayerHit>,
        spikes: Option<PlayerHit>,
        pending: Option<PlayerHit>,
        player: (i32, i32),
        facing_left: bool,
        event: &mut CombatEvents,
    ) -> (Option<PlayerHit>, Option<Struck>) {
        if let Some(hit) = contact.or(spikes) {
            return (Some(hit), None);
        }
        // $21CB2..$21CCA: in the first five slots of the dash slash the
        // collided list-4 object's hit is cancelled on both sides: no
        // damage, no spark, and a shot or flame keeps flying.
        if self.weapons.dash_slashing {
            return (None, None);
        }
        let Some(index) = self
            .projectiles
            .iter()
            .position(|shot| shot.enemy && shot.touching && !shot.body_contact)
        else {
            return (pending, None);
        };
        let horse = self.mount == Some(crate::special::Mount::Horse);
        let shot = &self.projectiles[index];
        let hitbox = Self::shot_box(&self.mappings, shot);
        // $21CCC..$21CEE: on the horse, a shot whose hitbox centre is more
        // than 40 px below feet-72 is cancelled on both sides: no hit, and it
        // keeps flying (under the horse's legs).
        if horse && hitbox.y - (player.1 - 72) > 40 {
            return (None, None);
        }
        // $21D0C..$21D1E: crouching uses feet-40; the horse always uses
        // feet-72. Select it from the same previous-update body as collision.
        let body = self.player_body(player, facing_left);
        let low_body = body.y - body.half_height == player.1 - 38;
        let reference = player.1 - if low_body && !horse { 40 } else { 72 };
        let (x, damage) = (shot.x, shot.damage);
        let blocked =
            self.weapons.guarding && (x < player.0) == facing_left && hitbox.y - reference <= 40;
        let shot = &mut self.projectiles[index];
        if let Some((_, contacted)) = &mut shot.boss7_aura {
            *contacted = true;
        }
        self.struck_owner = shot.owner;
        let dart = shot.bounce_dart();
        let (y, stays, dust) = (shot.y, shot.stays, shot.contact_dust);
        // Freed by its own update after this test (a wall, which leaves
        // `$3690C`'s spark there, or the view): the handler never reads the
        // collision bit, so the player is hit but no hit spark or explosion
        // is left (`$1CDE4`).
        let walled = shot.walled;
        let freed = !shot.alive || walled;
        let mech_orb = if let Some(super::MechShot::Orb { hit, .. }) = &mut shot.mech {
            *hit = true;
            shot.no_collision = true;
            true
        } else {
            false
        };
        if mech_orb {
            // `$3170E` owns the four impact particles on the next handler call.
        } else if shot.home.is_some() {
            // $1A8CC: bit 13 stops further collisions and the chain turns back.
            shot.damage = 0;
            shot.no_collision = true;
            shot.motion.vx = -shot.motion.vx;
        } else if !stays && !walled {
            shot.alive = false;
        }
        let contact_puff = if shot.contact_puff && !freed {
            shot.shot_down
        } else {
            None
        };
        let spark = if dart || stays || dust || freed || mech_orb || contact_puff.is_some() {
            None
        } else {
            Some(((x, y), shot.contact_blast))
        };
        if let Some(puff) = contact_puff {
            self.effects.push(puff.puff((x, y), self.random));
        }
        if dust {
            self.spawn_player_dust((x, y));
        }
        if blocked {
            event.blocked = damage;
            return (None, spark);
        }
        (Some(PlayerHit::shot(x, damage)), spark)
    }
    /// The effect a shot's handler leaves when it has struck the player
    /// (`$1CE0E..$1CE44` and the other enemy shots): `$368EC` when the guard
    /// deflected it or the shield is still up after absorbing it, otherwise
    /// `$36900`, spark `$21`. `$368EC` also requests effect 8 while the
    /// shot is inside `$4B52`'s window (`$4B2E`).
    pub(super) fn shot_spark(
        &mut self,
        (x, y): (i32, i32),
        deflected: bool,
        camera: (i32, i32),
        event: &mut CombatEvents,
    ) {
        let (kind, first) = if deflected || self.weapons.shield != 0 {
            if in_sound_window((x, y), camera) {
                event.sound_ids.push(self.rules.deflect_sound);
            }
            (EffectKind::DeflectSpark, self.rules.deflect_seq[0])
        } else {
            (EffectKind::KillSpark, self.rules.kill_seq[0])
        };
        let mut spark = Effect::new(kind, x, y, first);
        // These contacts share Joe's priority; $36918 inherits bit 15.
        spark.high_priority = self.weapons.high_priority;
        spark.base = if spark.high_priority { 0x8000 } else { 0 };
        self.effects.push(spark);
    }
    /// `Player_ApplyHit` (`$21DB0..$21DC8`): a hit that deals damage calls
    /// `Player_PowerDown`. While powered (`plr_status` bit 7) its damage
    /// becomes 0 first, so it costs no health and enters the ordinary hurt
    /// state with 48 ticks of immunity (`$21E86`). Returns the damage that
    /// remains. The shield and the invincibility cheat absorb before this.
    /// `Player_PowerDown` unpacks the ordinary art into VRAM
    /// (`Func_0046DA`), which overruns the frame: `art_reloaded`.
    pub(crate) fn power_down(&mut self, damage: u8) -> u8 {
        if damage == 0 {
            return 0;
        }
        self.art_reloaded = true;
        if std::mem::take(&mut self.weapons.powered) {
            0
        } else {
            damage
        }
    }
    /// `$21D86..$21DBE`: while +$30 is nonzero a hit uses one shield charge;
    /// with `Cheat_Invincible == 'R'` it takes the same path without a charge.
    /// Both play effect 6 (`$B5`) and grant 64 ticks of protection.
    pub(super) fn absorb_with_shield(&mut self, event: &mut CombatEvents) -> bool {
        if self.invulnerability != 0 {
            return false;
        }
        if self.weapons.shield != 0 {
            self.weapons.shield -= 1;
        } else if !self.invincible {
            return false;
        }
        self.invulnerability = 64;
        event
            .sound_ids
            .push(self.rules.cues.get(SoundEvent::ShieldAbsorb));
        true
    }
}
