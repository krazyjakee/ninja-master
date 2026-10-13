//! Game events that request a sound, each tied to its ROM request site.
//!
//! The cartridge names sounds through three 16-byte record tables (command
//! byte, flag byte, 14-character sound-test name) and one direct command:
//!
//! | Entry | Table | Commands |
//! | --- | --- | --- |
//! | `$4A36` | music `$5986` | `$81..$9C`, `$FC` |
//! | `$4A4E`, `$4B2E` (only within 352 px of the camera, `$4B52`) | effects `$5B56` | `$9D..$D8`, `$FD` |
//! | `$4A66`, `Player_PlaySfx` `$21A7A` (skipped while player +$29 bit 5) | voices `$5F76` | `$DA..$FB` (external DAC samples) |
//! | `$4A88` | direct | `$FC` fade |
//!
//! Every entry ends at `$4B80`, which appends the command to the 32-byte ring
//! them the native port reaches. Commands are read from the cartridge; the
//! `recovered` values are the emulator-confirmed results, checked by tests.

use crate::rom::{Result, byte};

/// The request tables: 16-byte records whose first byte is the driver
/// command and whose second is nonzero for entries the sound test skips;
/// a 14-character name follows.
pub const MUSIC_TABLE: usize = 0x5986;
pub const EFFECT_TABLE: usize = 0x5b56;
pub const VOICE_TABLE: usize = 0x5f76;

/// `$5B56 + id * 16`: all 66 effect entries, IDs `$00..$41`, as
/// `$4A4E` (and `$4B2E`) request it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Effects(Vec<u8>);
impl Effects {
    pub(crate) fn load(rom: &[u8]) -> crate::rom::Result<Self> {
        (0..((VOICE_TABLE - EFFECT_TABLE) / 16) as u8)
            .map(|id| Table::Effect.command(rom, id))
            .collect::<crate::rom::Result<_>>()
            .map(Self)
    }
    /// The driver command of effect `id` (`None` past the table).
    pub(crate) fn command(&self, id: u8) -> Option<u8> {
        self.0.get(usize::from(id)).copied()
    }
}

/// Where an event's command comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    /// `$5986`, requested through `$4A36`.
    Music,
    /// `$5B56`, requested through `$4A4E` or `$4B2E`.
    Effect,
    /// `$5F76`, requested through `$4A66` or `$21A7A`.
    Voice,
    /// The index is the command itself (`$4A88` loads `$FC`).
    Direct,
}
impl Table {
    /// The driver command of entry `index`, as the request entry reads it.
    pub fn command(self, rom: &[u8], index: u8) -> Result<u8> {
        let base = match self {
            Self::Music => MUSIC_TABLE,
            Self::Effect => EFFECT_TABLE,
            Self::Voice => VOICE_TABLE,
            Self::Direct => return Ok(index),
        };
        byte(rom, base + usize::from(index) * 16)
    }
    /// The command of the entry whose index is the byte at `id_at` (the
    /// `moveq`/`move.b` operand of a request site).
    pub(crate) fn command_at(self, rom: &[u8], id_at: usize) -> Result<u8> {
        self.command(rom, byte(rom, id_at)?)
    }
}

/// A request site: `index` is the value in d0 when the site calls `entry`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub table: Table,
    pub index: u8,
    /// PC of the `jsr`/`jmp` to the request entry.
    pub site: usize,
    /// Request routine the site calls.
    pub entry: usize,
    /// Instruction that loads the index: `moveq #index,dn` (d0 unless noted).
    pub setup: Option<(usize, [u8; 2])>,
}

/// Sound-requesting events the native port implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundEvent {
    /// Ordinary jump, `Player_HoldAttack` `$23464` (the somersault uses the
    /// same effect at `$210B6`).
    Jump,
    /// Wall-kick launch, player sound 7 in `PlayerState_WallKick` `$213C4`.
    WallKick,
    /// Single shuriken, `Player_StartThrow` `$232F0`.
    Throw,
    /// Somersault eight-way spread, `Player_ThrowSpread` `$231C0`.
    SpreadThrow,
    /// Sword swing. `Player_StartSlash` (`$23314`) stores effect 3 in the
    /// sword object's +$32; the object plays it (`$24D02`) on the frame whose
    /// mapping word has bit 15 set, when `+$3A & 3 == 3` (`+$3A` = 11).
    Slash,
    /// Powered sword swing: `Player_StartPoweredSlash` (`$2331A`) stores
    /// effect `$1A`, played the same way at `+$3A` = 7. It shares
    /// command `$B1` with effect 3.
    PoweredSlash,
    /// A hit absorbed by the Ikazuchi shield, `$21D9A`.
    ShieldAbsorb,
    /// Ninjutsu selection change in the pause menu, `$B638`.
    NinjutsuSelect,
    /// Ikazuchi bolt at cursor `$0D00`, `$20BFC`.
    Thunderbolt,
    /// A Kariu dragon (object 25), `$259B0`.
    KariuDragon,
    /// Kariu pauses the music as the screen wave starts, `$20AF4`.
    KariuMusicPause,
    /// `Mode_KariuWave` restarts it when the wave ends, `$22A78`.
    KariuMusicRestart,
    /// Fushin at cursor `$0D00`, `$2093C`.
    Fushin,
    /// Mijin explosion at cursor `$1400`, `$208A8`.
    Mijin,
    /// Mijin return at `$1C00`: effect 0 (SE STOP) at `$2079E`, then this.
    MijinStopEffects,
    MijinReturn,
    /// Ninja star (type `$14` windup), `$14C78`.
    EnemyStar,
    /// Ordinary enemy explosion: `$36DAE` passes d6 = 1, played at `$36DF0`.
    Explosion,
    /// The `$B8` finale's large explosion: `$36D90` passes d6 = `$12`.
    FinaleExplosion,
    /// Type `$24` debris burst, `$36F38`.
    Debris,
    /// A weapon deflected by the heavy actor, `$368EE`.
    Deflect,
    /// Heavy actor's thrown sickle, `$1C19A`.
    SickleThrow,
    /// `$B8` encounter entrance, `$1C6BE`.
    SamuraiEntry,
    /// STAGE CLEAR after the `$B8` finale explosion, `$1BFE8`.
    StageClear,
    /// WABI when the last life is lost (`$20552` selects 11 while `$FF37E0`
    /// is zero), `$2055E`.
    GameOver,
    /// SABI when a life is lost with a spare one left (`$20552` selects 12
    /// while `$FF37E0` is nonzero), the same `$2055E` request.
    DeathJingle,
    /// Music fade when the player dies, `$20594` (`$4A88`).
    DeathFade,
    /// The horse stage replaces its table-selected IDATEN with UMA during
    /// `Player_Init`, `$22CC4`.
    HorseStart,
    /// UMA raises the sound-driver communication byte; the player handler
    /// consumes it and starts IDATEN, `$205D6`.
    HorseTransition,
    /// "Katana set" voice at ninjutsu cursor `$0500` (Ikazuchi `$20B5C`,
    /// Kariu `$209FA`, Fushin `$20924`, Mijin `$206F0`).
    KatanaSet,
    /// "Ninjitsu" voice at `$0900` (Mijin: `$0A00`; Ikazuchi `$20B98`,
    /// Kariu `$20A12`, Fushin `$20956`, Mijin `$2070E`).
    NinjutsuVoice,
    /// Damage voice, `Player_SubtractHealth` `$21EEA`.
    PlayerHurt,
    /// Damage code `$2F` (the shocked state) plays this effect instead of
    /// the voice, `$21EDC`.
    Shocked,
    /// Air stab start (object 15), `$23078`.
    AirStabKiai,
    /// Standing/air slash object hit (objects 9 and 13, `$24C92`), `$24C9A`.
    SlashHit,
    /// Crouching attack hit (object 11, `$251EE`), `$251F6`.
    CrouchHit,
    /// Air stab hit, `$252F2` (with the +$40 bounce).
    AirStabHit,
    /// Dash kick object 19 on its bit-15 frame, `$25544`.
    DashAttack,
    /// Dash kick hit, `$254E0`.
    DashKickHit,
    /// Shuriken hit, `Shuriken_Update` `$24DE0`.
    ShurikenHit,
    /// A shuriken knocked back from a wall, `Func_024AFE` `$24B0E`.
    ShurikenBounce,
    /// A shuriken entering material 15, `Shuriken_Update` `$24E36`.
    ShurikenSplash,
    /// The player's feet entering material 15, `Player_CollideVertical` `$21F5C`.
    WaterEnter,
    /// The player rising out of material 15, `Player_CollideCeiling` `$22156`.
    WaterLeave,
    /// A sword breaking an item box (list-6 type 8), `$297B8`.
    ItemBoxHit,
    /// The item-box time bomb's blast (list-4 type 30), `$1D134`.
    BombExplosion,
}

const SFX: usize = 0x4a4e;
const SFX_VISIBLE: usize = 0x4b2e;
const MUSIC: usize = 0x4a36;
const VOICE: usize = 0x4a66;
const PLAYER_VOICE: usize = 0x21a7a;
const FADE: usize = 0x4a88;

const fn at(table: Table, index: u8, site: usize, entry: usize) -> Source {
    Source {
        table,
        index,
        site,
        entry,
        setup: Some((site - 2, [0x70, index])),
    }
}

impl SoundEvent {
    pub const COUNT: usize = 46;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Jump,
        Self::WallKick,
        Self::Throw,
        Self::SpreadThrow,
        Self::Slash,
        Self::PoweredSlash,
        Self::ShieldAbsorb,
        Self::NinjutsuSelect,
        Self::Thunderbolt,
        Self::KariuDragon,
        Self::KariuMusicPause,
        Self::KariuMusicRestart,
        Self::Fushin,
        Self::Mijin,
        Self::MijinStopEffects,
        Self::MijinReturn,
        Self::EnemyStar,
        Self::Explosion,
        Self::FinaleExplosion,
        Self::Debris,
        Self::Deflect,
        Self::SickleThrow,
        Self::SamuraiEntry,
        Self::StageClear,
        Self::GameOver,
        Self::DeathJingle,
        Self::DeathFade,
        Self::HorseStart,
        Self::HorseTransition,
        Self::KatanaSet,
        Self::NinjutsuVoice,
        Self::PlayerHurt,
        Self::Shocked,
        Self::AirStabKiai,
        Self::SlashHit,
        Self::CrouchHit,
        Self::AirStabHit,
        Self::DashAttack,
        Self::DashKickHit,
        Self::ShurikenHit,
        Self::ShurikenBounce,
        Self::ShurikenSplash,
        Self::WaterEnter,
        Self::WaterLeave,
        Self::ItemBoxHit,
        Self::BombExplosion,
    ];
    /// The request site this event reproduces.
    pub const fn source(self) -> Source {
        use Table::*;
        match self {
            Self::Jump => at(Effect, 5, 0x23464, SFX),
            Self::WallKick => at(Voice, 7, 0x213c4, PLAYER_VOICE),
            Self::Throw => at(Effect, 2, 0x232f0, SFX),
            Self::SpreadThrow => at(Effect, 4, 0x231c0, SFX),
            Self::Slash => Source {
                table: Effect,
                index: 3,
                site: 0x24d02,
                entry: SFX,
                // moveq #3,d2, stored to the sword object's +$32 at $23350.
                setup: Some((0x23314, [0x74, 0x03])),
            },
            Self::PoweredSlash => Source {
                table: Effect,
                index: 0x1a,
                site: 0x24d02,
                entry: SFX,
                setup: Some((0x2331a, [0x74, 0x1a])),
            },
            Self::ShieldAbsorb => at(Effect, 6, 0x21d9a, SFX),
            Self::NinjutsuSelect => at(Effect, 0x0b, 0xb638, SFX),
            Self::Thunderbolt => at(Effect, 0x1f, 0x20bfc, SFX),
            Self::KariuDragon => at(Effect, 0x0c, 0x259b0, SFX),
            Self::KariuMusicPause => at(Effect, 0x3f, 0x20af4, SFX),
            Self::KariuMusicRestart => at(Effect, 0x40, 0x22a78, SFX),
            Self::Fushin => at(Effect, 0x19, 0x2093c, SFX),
            Self::Mijin => at(Effect, 0x2f, 0x208a8, SFX),
            Self::MijinStopEffects => at(Effect, 0, 0x2079e, SFX),
            Self::MijinReturn => at(Effect, 0x3a, 0x207a6, SFX),
            Self::EnemyStar => at(Effect, 0x0a, 0x14c78, SFX_VISIBLE),
            Self::Explosion => Source {
                table: Effect,
                index: 1,
                site: 0x36df0,
                entry: SFX,
                // moveq #1,d6 in the $36DAE entry; $36DEE moves d6 to d0.
                setup: Some((0x36db4, [0x7c, 0x01])),
            },
            Self::FinaleExplosion => Source {
                table: Effect,
                index: 0x12,
                site: 0x36df0,
                entry: SFX,
                setup: Some((0x36d96, [0x7c, 0x12])),
            },
            Self::Debris => at(Effect, 1, 0x36f38, SFX_VISIBLE),
            Self::Deflect => at(Effect, 8, 0x368ee, SFX_VISIBLE),
            Self::SickleThrow => at(Effect, 0x1b, 0x1c19a, SFX),
            Self::SamuraiEntry => at(Effect, 0x20, 0x1c6be, SFX),
            Self::StageClear => at(Music, 0x16, 0x1bfe8, MUSIC),
            Self::GameOver => Source {
                table: Music,
                index: 0x0b,
                site: 0x2055e,
                entry: MUSIC,
                setup: Some((0x20552, [0x70, 0x0b])),
            },
            Self::DeathJingle => Source {
                table: Music,
                index: 0x0c,
                site: 0x2055e,
                entry: MUSIC,
                setup: Some((0x2055c, [0x70, 0x0c])),
            },
            Self::DeathFade => Source {
                table: Direct,
                index: 0xfc,
                site: 0x20594,
                entry: FADE,
                // $4A88: move.b #$FC,d0.
                setup: Some((0x4a88, [0x10, 0x3c])),
            },
            Self::HorseStart => at(Music, 0x18, 0x22cc4, MUSIC),
            Self::HorseTransition => at(Music, 0x10, 0x205d6, MUSIC),
            Self::KatanaSet => at(Voice, 0x0c, 0x20b5c, VOICE),
            Self::NinjutsuVoice => at(Voice, 1, 0x20b98, VOICE),
            Self::PlayerHurt => at(Voice, 4, 0x21eea, PLAYER_VOICE),
            Self::Shocked => at(Effect, 0x31, 0x21edc, SFX),
            Self::AirStabKiai => at(Voice, 0x13, 0x23078, PLAYER_VOICE),
            Self::SlashHit => at(Voice, 6, 0x24c9a, PLAYER_VOICE),
            Self::CrouchHit => at(Voice, 0x0a, 0x251f6, PLAYER_VOICE),
            Self::AirStabHit => at(Voice, 0x0a, 0x252f2, PLAYER_VOICE),
            Self::DashAttack => at(Voice, 0x12, 0x25544, PLAYER_VOICE),
            Self::DashKickHit => at(Voice, 6, 0x254e0, PLAYER_VOICE),
            Self::ShurikenHit => at(Voice, 0x0e, 0x24de0, PLAYER_VOICE),
            Self::ShurikenBounce => at(Effect, 8, 0x24b0e, SFX),
            Self::ShurikenSplash => at(Effect, 0x27, 0x24e36, SFX),
            Self::WaterEnter => at(Effect, 0x27, 0x21f5c, SFX),
            Self::WaterLeave => at(Effect, 0x27, 0x22156, SFX),
            Self::ItemBoxHit => at(Voice, 0x0e, 0x297b8, VOICE),
            Self::BombExplosion => at(Effect, 0x12, 0x1d134, SFX_VISIBLE),
        }
    }
    /// Command written to `$FFFC00`, as recorded by the emulator (see tests).
    /// The command byte this event requests, read from the cartridge table.
    pub fn command(self, rom: &[u8]) -> Result<u8> {
        let Source { table, index, .. } = self.source();
        table.command(rom, index)
    }
}

/// Sound-driver command groups (`SndDrv_ProcessCommand`, `$69D3E`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// `$81..$9C`: starts a music header.
    Music,
    /// `$9D..$D8`: an SFX header (`SoundPlayer::play_sfx`).
    Effect,
    /// `$D9..$FF` and anything else: special SFX, external DAC samples
    /// (voices), fade and stop commands.
    Driver,
}
pub fn route(command: u8) -> Route {
    match command {
        0x81..=0x9c => Route::Music,
        0x9d..=0xd8 => Route::Effect,
        _ => Route::Driver,
    }
}

/// Every event's command, resolved once from the cartridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cues([u8; SoundEvent::COUNT]);
impl Cues {
    pub fn load(rom: &[u8]) -> Result<Self> {
        let mut commands = [0; SoundEvent::COUNT];
        for (command, event) in commands.iter_mut().zip(SoundEvent::ALL) {
            *command = event.command(rom)?;
        }
        Ok(Self(commands))
    }
    pub fn get(&self, event: SoundEvent) -> u8 {
        let index = SoundEvent::ALL
            .iter()
            .position(|&e| e == event)
            .expect("SoundEvent::ALL lists every event");
        self.0[index]
    }
}
