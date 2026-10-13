//! Round 7 enemy types (issue #51), ported from their handlers:
//!
//! | Type | Handler | Where | Native model |
//! | --- | --- | --- | --- |
//! | `$50` | `Enemy50` `$1C6EE` | 7-1 | [`Flyer`]: drops from above the screen, swoops, fires diagonal shots |
//! | `$DC` | `EnemyDC` `$1A146` | 7-1 | [`Turret`]: aims a 24-direction barrel (list 9) and fires |
//! | `$E8` | `EnemyE8` `$1ACA6` | 7-1 | [`BeamCannon`]: ceiling cannon, charges and fires a vertical beam |
//! | `$EC` | `EnemyEC` `$1B28A` | 7-1 | [`Launcher`]: charges, fires three accelerating missiles, recoils |
//! | `$F0` | `EnemyF0` `$1B4E6` | 7-1 | [`Battery`]: volleys of four diagonal shots |
//! | `$C4` | `EnemyC4` `$19918` | none | [`Crawler`]: climbs a wall, leaps to the opposite wall |
//!
//! `$C4` has no spawn record and no `Object_AllocL5` caller; its handler is
//! self-contained, so it is ported and tested with a placed record. `$74`
//! (`$303EC`) is not a list-5 enemy: the slot points at the shared handler of
//! list-4 projectile `$2A` and list-1 type `$19` (the `$B0` wall-gun bullet
//! and its muzzle flash, both native in `super::round2` and
//! `Shared::muzzle`). As a list-5 record it would read a `$3C` parent that
//! only `Object_AllocL4` callers set and free itself with
//! `Object_FreeL4`/`Object_FreeL1`, so it is not registered.
//!
//! Each routine is the byte offset into the handler's `bra.w` table (`$C4`
//! stores the index 0..4 as the cartridge does); the comments cite the
//! routine's address. Immediates and tables are read from the ROM at load
//! (each type's `tables_match_rom` test pins them). Native simplifications: list-1 and
//! list-9 decorations (smoke, charge particles, trails, the `$36FF2` puff
//! chain) are `Pieces`/`Puff` effects without their own handlers. The `$EC`
//! missile explodes on weapons and terrain; the `$E8` palette fade retains
//! its documented approximation.
use super::{Ctx, Roster};
use crate::{
    combat::{Effect, EffectKind, Enemy, Motion, Projectile},
    rom::Result,
};
use std::sync::Arc;

mod battery;
mod beam_cannon;
mod crawler;
mod flyer;
mod launcher;
mod turret;
use battery::Battery;
use beam_cannon::BeamCannon;
use crawler::Crawler;
use flyer::Flyer;
use launcher::Launcher;
use turret::Turret;

pub(crate) const KINDS: &[u16] = &[0x50, 0xc4, 0xdc, 0xe8, 0xec, 0xf0];

pub(crate) fn register(rom: &[u8], roster: &mut Roster) -> Result<()> {
    roster.add(0x50, Arc::new(Flyer::load(rom)?));
    roster.add(0xc4, Arc::new(Crawler::load(rom)?));
    roster.add(0xdc, Arc::new(Turret::load(rom)?));
    roster.add(0xe8, Arc::new(BeamCannon::load(rom)?));
    roster.add(0xec, Arc::new(Launcher::load(rom)?));
    roster.add(0xf0, Arc::new(Battery::load(rom)?));
    Ok(())
}

/// Consecutive equal entries of a frame list shown `period` ticks each, as
/// (mapping, ticks) runs.
pub(crate) fn runs(frames: &[u16], period: u16) -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = Vec::new();
    for &mapping in frames {
        match out.last_mut() {
            Some((last, ticks)) if *last == mapping => *ticks += period,
            _ => out.push((mapping, period)),
        }
    }
    out
}
/// A list-1/list-9 object that only animates and drifts: one `Pieces`
/// effect per (mapping, ticks) segment, delayed to its start and placed where
/// `motion` has carried the object by then. The first segment starts after
/// `skip` invisible ticks.
pub(crate) fn animate(
    cx: &mut Ctx,
    at: (i32, i32),
    motion: Motion,
    base: u16,
    skip: u16,
    segments: &[(u16, u16)],
    blink: bool,
) {
    let (mut x, mut y) = at;
    let mut m = motion;
    for _ in 0..skip {
        m.step(&mut x, &mut y);
    }
    let mut start = skip;
    for &(mapping, ticks) in segments {
        if ticks == 0 {
            continue;
        }
        let mut piece = Effect::new(EffectKind::Pieces, x, y, mapping);
        piece.base = base;
        piece.delay = start + 1;
        piece.counter = i32::from(ticks);
        piece.motion = m;
        piece.blink = blink;
        cx.effect(piece);
        for _ in 0..ticks {
            m.step(&mut x, &mut y);
        }
        start += ticks;
    }
}
/// `Func_01B004`: two puffs (d7 = 1, 0) around (x + d5, y + d6): position
/// `$1AF52` (x - 31..32, y + 0..31), direction `rand & $3E0` + 2 d7, twice
/// the `$94A0` speed, base `$8000`.
fn bursts(cx: &mut Ctx, e: &Enemy, offset: (i32, i32)) {
    for d7 in [1u16, 0] {
        let x = e.x + i32::from(cx.random() & 63) - 31 + offset.0;
        let y = e.y + i32::from(cx.random() & 31) + offset.1;
        let angle = (cx.random() & 0x3e0) + d7 * 2;
        let (c, s) = cx.shared.sine.polar(angle);
        let mut puff = Effect::new(EffectKind::Puff, x, y, 0);
        puff.base = 0x8000;
        puff.motion = Motion {
            vx: c << 1,
            vy: s << 1,
            ..Motion::default()
        };
        // $1B05C/$1B07C: an independent random countdown of 1 or 2,
        // advancing on underflow, so a frame lasts two or three updates.
        puff.period = (cx.random() & 1) + 2;
        puff.high_priority = true;
        cx.effect(puff);
    }
}

/// An aimed list-4 shot: mapping, damage (`$1FBF6 + ID`) and `$4B2E` effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shot {
    mapping: u16,
    damage: u8,
    sound: u8,
}
/// A shot that keeps its unflipped art in either direction (the handlers
/// never set the projectile's flip bit).
fn shot(x: i32, y: i32, spec: Shot, motion: Motion) -> Projectile {
    Projectile::new(x, y, 0, true, spec.mapping, spec.damage)
        .with_art(0, motion.vx < 0)
        .with_motion(motion)
}

/// Native lifetime of a projectile rebuilt by its owner every tick.
pub(crate) const ATTACHED: u16 = 2;

// First part of 7-1 and the index of the first record of `kind`.
