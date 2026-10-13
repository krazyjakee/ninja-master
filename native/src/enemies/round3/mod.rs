//! Round 3 enemy types (issue #51), ported from their handlers:
//!
//! | Type | Handler | Where | Native model |
//! | --- | --- | --- | --- |
//! | `$64` | `Enemy64` `$16D5E` | 3-1 | [`Flier`]: perches, flutters at the player within 128 px, flies home |
//! | `$98` | `Enemy98` `$18086` | 3-1 | [`Spitter`]: surges along the floor, surfaces and lobs a blob |
//! | `$A4` | `EnemyA4` `$18B0C` | 3-1 | [`Tank`]: glass tanks that release `$FC`; also `$C0`'s capsules |
//! | `$A8` | `EnemyA8` `$19064` | 3-1 | [`Crawler`]: crawls round block edges at 1 px/tick |
//! | `$C0` | `EnemyC0` `$19656` | 3-1 | [`Collapse`]: the end-of-3-1 capsule rain and floor collapse |
//! | `$CC` | `EnemyCC` `$19CD6` | 3-2 | [`Grabber`]: rises out of the floor and grabs |
//! | `$FC` | `EnemyFC` `$1B686` | 3-1 (from `$A4`) | [`Mutant`]: hops at the player |
//!
//! Each routine is the byte offset into the handler's `bra.w` table (`$A8`
//! dispatches `obj_routine` * 4, so its routines are indices); the comments
//! cite the routine's address. Immediates and tables are read from the ROM
//! at load (each type's `tables_match_rom` test pins them).
//!
//! The capsules `$C0` drops are list-1 objects of type `$29` (`Func_018DEC`)
//! in the ROM; natively they are `$A4` children in capsule mode, so weapons
//! can break them. The shared counters `$FF3D05` (live `$FC`) and `$FF3D06`
//! (`$FC` kills still wanted) are [`Mutants`], written where the ROM writes
//! them: `Func_01B8A8` (the `$FC` allocation, [`super::Ctx::spawn`]) counts
//! one in, `$FC` counts itself out when it is left behind or bursts dead on
//! landing (one kill), and `$C0` sets the kills wanted. `$C0` mirrors both
//! into its scratch words for the capsules, which read them from their
//! parent (one tick late for a `$FC` allocated after `$C0`'s update).
//!
//! A breaking tank's broken glass (`Func_01903A`: `Func_015054` with the
//! metatiles `Data_019014`) goes through `Ctx::write_metatiles` into the
//! layout, which is plane A's art and collision (#94).
//!
//! `$C0`'s completion plays STAGE CLEAR, ripples and sinks four foreground
//! columns (`$1983C/$1988E`), removes the floor and marks the hole, then
//! resumes INNER DARKSIDE. Its event 6 blocks attacks while Joe can walk/fall.
//!
//! Not modelled (no native hook): `$C0`'s arena walls (`Func_0198C0`),
//! player events (`$FC`'s freeze
//! event 6, `$CC`'s hold, pull and `$A` drain), `$98`'s bouncing blob (it ends at the first wall or floor),
//! other music requests, and effect animations that step faster than the shared
//! native sequences (see each type).
use super::{Ctx, Roster};
use crate::{
    combat::{Effect, EffectKind, Motion},
    rom::{Result, signed, word, word_i},
};
use std::sync::Arc;

mod collapse;
mod crawler;
mod flier;
mod grabber;
mod mutant;
mod spitter;
mod tank;
use collapse::Collapse;
use crawler::Crawler;
use flier::Flier;
use grabber::Grabber;
use mutant::Mutant;
use spitter::Spitter;
use tank::Tank;

pub(crate) const KINDS: &[u16] = &[0x64, 0x98, 0xa4, 0xa8, 0xc0, 0xcc, 0xfc];

pub(crate) fn register(rom: &[u8], roster: &mut Roster) -> Result<()> {
    roster.add(0x64, Arc::new(Flier::load(rom)?));
    roster.add(0x98, Arc::new(Spitter::load(rom)?));
    roster.add(0xa4, Arc::new(Tank::load(rom)?));
    roster.add(0xa8, Arc::new(Crawler::load(rom)?));
    roster.add(0xc0, Arc::new(Collapse::load(rom)?));
    roster.add(0xcc, Arc::new(Grabber::load(rom)?));
    roster.add(0xfc, Arc::new(Mutant::load(rom)?));
    Ok(())
}

/// `$FF3D05` and `$FF3D06`, signed bytes: the live `$FC` and the `$FC`
/// kills `$C0` still wants (it stores 7, so the eighth kill ends the rain).
/// Cleared with the part's RAM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mutants {
    pub(crate) alive: i8,
    pub(crate) wanted: i8,
}
impl Mutants {
    /// `Func_01B8A8`: `addq.b #1,($FF3D05)` for an allocated `$FC`.
    pub(crate) fn released(&mut self) {
        self.alive = self.alive.wrapping_add(1);
    }
    /// `$1B884`: `subq.b #1,($FF3D05)`, left behind or burst.
    fn gone(&mut self) {
        self.alive = self.alive.wrapping_sub(1);
    }
    /// `$1B85E`: `subq.b #1,($FF3D06)` for a kill, then [`Self::gone`].
    fn killed(&mut self) {
        self.wanted = self.wanted.wrapping_sub(1);
        self.gone();
    }
}

/// `Func_014E90`: eight glass shards (four list-1 `$25` objects of two
/// pieces) from `Data_014E14`, thrown away from the player, and the GLASS
/// effect of `Func_018CFA`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Glass {
    sound: u8,
    mappings: [u16; 2],
    /// (dx, dy, angle, shift): velocity = polar(angle) << (shift + 1).
    pieces: Vec<(i32, i32, u16, u32)>,
    gravity_mask: u16,
    gravity_base: i32,
    base: u16,
}
impl Glass {
    fn load(rom: &[u8]) -> Result<Self> {
        let mut pieces = Vec::with_capacity(8);
        for i in 0..8 {
            let p = 0x14e18 + i * 8;
            pieces.push((
                signed(rom, p)?,
                signed(rom, p + 2)?,
                word(rom, p + 4)?,
                u32::from(word(rom, p + 6)?),
            ));
        }
        Ok(Self {
            sound: word(rom, 0x18cfc)? as u8,
            mappings: [word(rom, 0x14e14)?, word(rom, 0x14e16)?],
            pieces,
            gravity_mask: word(rom, 0x14f50)?,
            gravity_base: word_i(rom, 0x14f54)?,
            base: word(rom, 0x14ec6)?,
        })
    }
    /// Shards only (`Func_014E90`); the pieces alternate frames in the ROM,
    /// natively both blink.
    fn shards(&self, cx: &mut Ctx, x: i32, y: i32) {
        let away = x >= cx.player.0;
        for pair in self.pieces.chunks(2) {
            // $FF001C & $FFF + $3000, one value per object.
            let gravity = i32::from(cx.random() & self.gravity_mask) + self.gravity_base;
            for (i, &(dx, dy, angle, shift)) in pair.iter().enumerate() {
                let (cos, sin) = cx.shared.sine.polar(angle);
                let vx = cos << (shift + 1);
                let mut shard = Effect::new(EffectKind::Pieces, x + dx, y + dy, self.mappings[i]);
                shard.base = self.base;
                shard.blink = true;
                shard.counter = 255;
                shard.motion = Motion {
                    vx: if away { vx } else { -vx },
                    vy: sin << (shift + 1),
                    gravity,
                    ..Motion::default()
                };
                cx.effect(shard);
            }
        }
    }
    /// `Func_018CFA`: the GLASS effect and the shards.
    fn smash(&self, cx: &mut Ctx, x: i32, y: i32) {
        cx.effect_sound(self.sound);
        self.shards(cx, x, y);
    }
}

// Live objects of `kind`.
