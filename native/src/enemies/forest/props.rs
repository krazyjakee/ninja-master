//! The 1-1 end arena's rock props: the list-1 records that
//! `EnemyB8_SamuraiLeader` (`$1C5F0`) allocates on its first update.
//!
//! Its routine 0 (`$1C664`, `$1C668`) calls `Func_01C2CC` and `Func_01C470`.
//! Each allocates list-1 records of type `$A0`, which the list-1 dispatch
//! (`Data_0365A2` entry 40) runs through the `$B4`/`$B8` routine table
//! (`Func_01BB48`). They are drawn with `obj_tile` `$6000`: stage tiles
//! `$107..$11B` on palette line 3, low priority.
//!
//! - Eight boulders (routine `$30`, `$1C36A`, mapping `$17F`) stack up the
//!   right wall of the arena at X `$6C0` + `Data_01C28C`. After 120 updates
//!   they burst out to the left: `Data_01C28C` vx, vy -2 and gravity
//!   `$1800`, with a 1/64 drag on vx every update. A boulder that meets
//!   solid terrain breaks into four blinking chips (`Func_01C3CA`, routine
//!   `$38`, `$1C446`: mapping `$181`, random velocity, gravity `$1000`, 32
//!   updates); the others are freed after 90 updates.
//! - Two gate halves (routine `$34`, `$1C4EA`, mapping `$180`, 32x32) stand
//!   at X `$6C0`, Y `$178`/`$198` from `Data_01C468`, the arena's left
//!   edge. After 48 updates they sink one pixel an update for 56 updates to
//!   the floor. While the player's box touches one, it pushes him 4 px
//!   right (`$1C51E`), which closes the arena behind him.
//!
//! Not modelled: when the hold ends, `$1C524` writes metatile 1 into the
//! 3x3 `Layout_Buffer` block at (`$6C0`, `$1B0`) without touching VRAM.
//! 1-1's collision byte for metatile 1 is 0, so the write changes neither
//! collision nor the cartridge's picture. Native draws plane A from the
//! layout, so repeating the write would paint metatile 1's cells there.
//!
//! Emulator evidence (Genesis Plus GX, 1-1 part 2, record spawn with the
//! camera lock still scrolling): the ten records appear with the leader,
//! boulder `$34` = 119 and gate `$34` = 47 after that first update; the
//! gates reach Y `$1B0`/`$1D0` 104 updates later, the boulders launch on
//! update 119, and a player walking left is held at X 1772..1776.
use crate::{
    combat::{Effect, EffectKind, Mapping, Signal, mapping_hitbox},
    game::Terrain,
    rom::{Result, byte, long, pointer, signed, word, word_i},
};
use std::collections::BTreeMap;

/// `$1C2CC`..`$1C4EA`, `$1C36A`, `$1C446` and `$1C4EA`, read from the ROM.
#[derive(Clone, Debug)]
pub(crate) struct Props {
    /// `Data_01C28C`: X from [`Self::boulder_x`], Y and the 16.16 vx
    /// (the table word `ext.l`, `asl.l #4`), one per boulder.
    pub(crate) boulders: Vec<(i32, i32, i32)>,
    pub(crate) boulder_x: i32,
    pub(crate) boulder: u16,
    pub(crate) boulder_vy: i32,
    pub(crate) boulder_gravity: i32,
    pub(crate) boulder_wait: i32,
    pub(crate) boulder_life: i32,
    pub(crate) chips: usize,
    /// `Data_01BA40`, `Data_01BA42` (a chip's first frame) and the blank
    /// `Data_01BA1E` word of its blink.
    pub(crate) chip: [u16; 3],
    pub(crate) chip_gravity: i32,
    pub(crate) chip_life: i32,
    /// `Data_01C468`: X from [`Self::gate_x`] and Y of each half.
    pub(crate) gates: Vec<(i32, i32)>,
    pub(crate) gate_x: i32,
    pub(crate) gate: u16,
    pub(crate) gate_hold: i32,
    pub(crate) gate_drop: i32,
    pub(crate) gate_push: i32,
    pub(crate) tile: u16,
}

/// Effect phases.
const HELD: u8 = 0;
const MOVING: u8 = 1;
const SETTLED: u8 = 2;

impl Props {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        let count = |at: usize| -> Result<usize> { Ok(usize::from(byte(rom, at)?) + 1) };
        let table = pointer(rom, 0x1c2d4)?;
        let mut boulders = Vec::new();
        for i in 0..count(0x1c2d1)? {
            let at = table + i * 8;
            boulders.push((
                word_i(rom, at)?,
                word_i(rom, at + 2)?,
                signed(rom, at + 4)? << 4,
            ));
        }
        let table = pointer(rom, 0x1c478)?;
        let mut gates = Vec::new();
        for i in 0..count(0x1c475)? {
            gates.push((word_i(rom, table + i * 4)?, word_i(rom, table + i * 4 + 2)?));
        }
        // $1C51E addq.w #n,$20(a3): the data field, 0 meaning 8.
        let push = i32::from((word(rom, 0x1c51e)? >> 9) & 7);
        Ok(Self {
            boulders,
            boulder_x: word_i(rom, 0x1c306)?,
            boulder: word(rom, pointer(rom, 0x1c2fe)?)?,
            boulder_vy: long(rom, 0x1c330)?,
            boulder_gravity: word_i(rom, 0x1c33c)?,
            boulder_wait: word_i(rom, 0x1c342)?,
            boulder_life: word_i(rom, 0x1c348)?,
            chips: count(0x1c3bb)?,
            chip: [
                word(rom, pointer(rom, 0x1c45e)?)?,
                word(rom, pointer(rom, 0x1c436)?)?,
                word(rom, pointer(rom, 0x1c448)?)?,
            ],
            chip_gravity: word_i(rom, 0x1c412)?,
            chip_life: word_i(rom, 0x1c440)?,
            gates,
            gate_x: word_i(rom, 0x1c4b6)?,
            gate: word(rom, pointer(rom, 0x1c4a2)?)?,
            gate_hold: word_i(rom, 0x1c4d2)?,
            gate_drop: word_i(rom, 0x1c4d8)?,
            gate_push: if push == 0 { 8 } else { push },
            tile: word(rom, 0x1c2f8)?,
        })
    }

    /// `Func_01C2CC` and `Func_01C470`: the ten records as allocated. List
    /// 1 runs after list 5, so each counts its first update in the frame
    /// that allocates it.
    pub(crate) fn spawn(&self) -> Vec<Effect> {
        // The ROM allocates boulders 7..0, then gates 1..0, and an earlier
        // list-1 sprite is in front. Natively a later effect is drawn on
        // top, so the list is built in the reverse order.
        let mut props = Vec::new();
        for &(dx, y) in &self.gates {
            let mut gate = Effect::new(EffectKind::Gate, self.gate_x + dx, y, self.gate);
            gate.base = self.tile;
            gate.counter = self.gate_hold;
            props.push(gate);
        }
        for (index, &(dx, y, _)) in self.boulders.iter().enumerate() {
            let mut boulder =
                Effect::new(EffectKind::Boulder, self.boulder_x + dx, y, self.boulder);
            boulder.base = self.tile;
            boulder.angle = index as u16;
            boulder.counter = self.boulder_wait;
            props.push(boulder);
        }
        props
    }

    /// `$1C36A`, before `Object_Integrate`. Returns false when the record
    /// frees itself; `chips` receives the pieces of a boulder that broke.
    pub(crate) fn boulder(
        &self,
        effect: &mut Effect,
        terrain: &dyn Terrain,
        random: &mut u16,
        chips: &mut Vec<Effect>,
    ) -> bool {
        if effect.phase == HELD {
            effect.counter -= 1;
            if effect.counter <= 0 {
                let vx = self
                    .boulders
                    .get(usize::from(effect.angle))
                    .map_or(0, |b| b.2);
                effect.phase = MOVING;
                effect.counter = self.boulder_life;
                effect.motion.vx = vx;
                effect.motion.vy = self.boulder_vy;
                effect.motion.gravity = self.boulder_gravity;
            }
            return true;
        }
        effect.motion.vx -= effect.motion.vx >> 6;
        if terrain.solid_at(effect.x, effect.y) {
            // Func_01C3BA: four chips at the boulder's position.
            for _ in 0..self.chips {
                chips.push(self.chip(effect.x, effect.y, random));
            }
            return false;
        }
        effect.counter -= 1;
        effect.counter > 0
    }

    /// `Func_01C3CA` and the chip's update of its allocation frame.
    fn chip(&self, x: i32, y: i32, random: &mut u16) -> Effect {
        let mut next = || crate::enemies::next_random(random);
        // ext.w, swap, asr.l #6: the low byte as signed 16.16 over 64.
        // (The fraction bits the swap brings down from d0's high word are
        // left out.)
        let vx = i32::from(next() as u8 as i8) << 10;
        let vy = i32::from((next() | 0x80) as u8 as i8) << 10;
        let first = if next() & 1 != 0 {
            self.chip[1]
        } else {
            self.chip[0]
        };
        let mut chip = Effect::new(EffectKind::Chip, x, y, first);
        chip.base = self.tile;
        chip.motion.vx = vx;
        chip.motion.vy = vy;
        chip.motion.gravity = self.chip_gravity;
        chip.counter = self.chip_life;
        chip.age = 1;
        self.chip_step(&mut chip);
        chip.motion.step(&mut chip.x, &mut chip.y);
        chip
    }

    /// `$1C446`: blink `$181` on odd counts; freed when the count ends.
    pub(crate) fn chip_step(&self, effect: &mut Effect) -> bool {
        effect.counter -= 1;
        effect.mapping = if effect.counter & 1 != 0 {
            self.chip[0]
        } else {
            self.chip[2]
        };
        effect.counter > 0
    }

    /// `$1C4EA`: hold, sink one pixel an update, then stay; every update
    /// but the hold's last pushes a touching player right.
    pub(crate) fn gate(
        &self,
        effect: &mut Effect,
        mappings: &BTreeMap<u16, Mapping>,
        player: (i32, i32),
        signals: &mut Vec<Signal>,
    ) {
        match effect.phase {
            HELD => {
                effect.counter -= 1;
                if effect.counter <= 0 {
                    // $1C524: the Layout_Buffer write (see the module notes).
                    effect.phase = MOVING;
                    effect.counter = self.gate_drop;
                    return;
                }
            }
            MOVING => {
                effect.counter -= 1;
                effect.y += 1;
                if effect.counter <= 0 {
                    effect.phase = SETTLED;
                }
            }
            _ => {}
        }
        // Object_CollidePlayer against the player's body box (mapping 1).
        let gate = mapping_hitbox(mappings, effect.mapping, effect.x, effect.y, false);
        let body = mapping_hitbox(mappings, 1, player.0, player.1, false);
        if gate.intersects(body) {
            signals.push(Signal::PushPlayer(self.gate_push));
        }
    }
}
