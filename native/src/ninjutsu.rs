//! The four ninjutsu: recovered start rules, timelines and effects.
//!
//! `$22E8E` starts a technique as player state `64 + $FF37E7` (Ikazuchi,
//! Kariu, Fushin, Mijin) and switches the game to mode `$22A96`, where only
//! the player's sequence advances. Each state handler acts when it sees a
//! particular sequence cursor; `effects` lists those cursors. The sequences
//! themselves are `movement::sequence` (speed 32, eight ticks per frame).
//! What the techniques draw (effect objects, palette writes, the shake) is
//! `crate::ninjutsu_fx`, run by the `Cast`.

use crate::movement::state::NINJUTSU;
use crate::sound_events::SoundEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// State 64, `$20B28`: thunderbolt and a four-hit shield.
    Ikazuchi,
    /// State 65, `$20994`: fire dragons across the screen.
    Kariu,
    /// State 66, `$208F8`: higher jumps for the rest of the life.
    Fushin,
    /// State 67, `$206B2`: explosion that costs a life.
    Mijin,
}
impl Kind {
    pub const ALL: [Self; 4] = [Self::Ikazuchi, Self::Kariu, Self::Fushin, Self::Mijin];
    /// `$FF37E7` value.
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    pub fn index(self) -> usize {
        self as usize
    }
    /// `$22F4A`: player state 64 + selection (ordinary, grounded variant).
    pub fn state(self) -> usize {
        NINJUTSU + self.index()
    }
}

/// Something a technique's state handler does at one cursor value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// A sound request (`crate::sound_events` cites its ROM site).
    Sound(SoundEvent),
    /// +$30 hits the shield absorbs (`$20C0C`).
    Shield(u8),
    /// Damage every live enemy on screen: the native stand-in for the effect
    /// objects' hits (`crate::ninjutsu_fx` draws the objects).
    Strike(u8),
    /// `$20730..$20778`: spend a spare life, or die without one.
    LifeCost,
    /// Ticks the whole technique pauses while its scene runs.
    Hold(u16),
    /// `Player_LoadRootAsset` (`$21918`): `Func_0046DA` unpacks the
    /// technique's player art straight into VRAM with interrupts masked
    /// (`Func_00460E`, `$463E..$4658`). The vertical interrupt waits for the
    /// copy and the update ends after it: `ART_LOAD_LAG` frames without one.
    ArtLoad,
}

/// Frames an `Effect::ArtLoad` update overruns. Genesis Plus GX: every
/// Ikazuchi of the 16 round tapes that cast one (on foot, on the horse, on
/// the surfboard) and every Kariu and Mijin cast from four of their states
pub const ART_LOAD_LAG: u8 = 1;
/// `Ninjutsu_Finish` waits for `Frame_Counter` to change (`$21A06..$21A12`)
/// in the middle of its update, whose logic then ends after the next
/// vertical interrupt: one frame without an update after every technique.
pub const FINISH_LAG: u8 = 1;

/// Kariu switches to mode `$229D8` for its dragons (`$20AFE`). The round 1-1
/// capture resumed the player's sequence 304 ticks later than an unbroken
/// eight-ticks-per-frame run: 441 ticks from A to idle instead of 137.
pub const KARIU_HOLD_TICKS: u16 = 304;
/// `$2193E`: 64 ticks of protection after every technique.
pub const PROTECTION_TICKS: u16 = 64;

/// The handler actions at `cursor`, read before the stepper advances it.
/// Every technique starts with the `$4A66` voices "katana set" at `$0500`
/// and "ninjitsu" at `$0900` (Mijin: `$0A00`).
pub fn effects(kind: Kind, cursor: u16) -> &'static [Effect] {
    use SoundEvent::*;
    match (kind, cursor) {
        // $206FA: Mijin loads its art (root asset $14) with the voice.
        (Kind::Mijin, 0x0500) => &[Effect::Sound(KatanaSet), Effect::ArtLoad],
        (_, 0x0500) => &[Effect::Sound(KatanaSet)],
        (Kind::Mijin, 0x0a00) => &[Effect::Sound(NinjutsuVoice)],
        (Kind::Ikazuchi | Kind::Kariu | Kind::Fushin, 0x0900) => &[Effect::Sound(NinjutsuVoice)],
        // $20BEC..$20C0C: effect 31 (thunderbolt), root asset $10 ($20C06)
        // and four shield charges. The bolt object 20 ($25570) is
        // presentation natively.
        (Kind::Ikazuchi, 0x0d00) => &[
            Effect::Sound(Thunderbolt),
            Effect::ArtLoad,
            Effect::Shield(4),
        ],
        // $20A62..$20A76 create four object-25 dragons with +$06 = 1
        // ($25956); each dragon plays effect 12 ($259AE). Natively every
        // on-screen enemy takes one hit per dragon, and the first dragon's
        // sound stands for all four. $20AF4 pauses the music for the wave;
        // `Game` restarts it when the hold ends ($22A78).
        // Root asset 8 loads first ($20A6C).
        (Kind::Kariu, 0x0d00) => &[
            Effect::ArtLoad,
            Effect::Sound(KariuMusicPause),
            Effect::Sound(KariuDragon),
            Effect::Strike(1),
            Effect::Strike(1),
            Effect::Strike(1),
            Effect::Strike(1),
            Effect::Hold(KARIU_HOLD_TICKS),
        ],
        // $2092E..$20942: effect 25. The flag is set when the state ends.
        (Kind::Fushin, 0x0d00) => &[Effect::Sound(Fushin)],
        // $20898..$208F4: effect 47 and six object-24 pieces with +$06 = 4
        // ($257C4); natively every on-screen enemy takes 4.
        (Kind::Mijin, 0x1400) => &[Effect::Sound(Mijin), Effect::Strike(4)],
        (Kind::Mijin, 0x1500) => &[Effect::LifeCost],
        // $20792..$207A6: effect 0 (SE STOP), then effect 58, Mijin return.
        (Kind::Mijin, 0x1c00) => &[Effect::Sound(MijinStopEffects), Effect::Sound(MijinReturn)],
        _ => &[],
    }
}

/// A running technique.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cast {
    pub kind: Kind,
    /// Remaining paused ticks (Kariu's dragon scene).
    pub hold: u16,
    /// The sequence has ended (`plr_status` bit 0): the next update is
    /// `Ninjutsu_Finish`.
    pub finishing: bool,
    /// Its effect objects, palette and shake (issue #56).
    pub fx: crate::ninjutsu_fx::Effects,
}
impl Cast {
    pub fn new(kind: Kind) -> Self {
        Self {
            kind,
            hold: 0,
            finishing: false,
            fx: crate::ninjutsu_fx::Effects::default(),
        }
    }
}
