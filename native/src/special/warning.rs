//! The blinking "!" sign that announces each obstacle of the mounted stages.
//!
//! `Func_0377D2` (stage load and `SectionChange_S30`) points `$FF0106` at a
//! camera-X schedule when `Stage_ExitFlags` bit 7 is set: `$377BC` for
//! rounds 1-2 (the 2-1 fences) and `$377C4` otherwise (the 4-1 wave ramps).
//! `Func_037814`, run by the list-1 pass (`$366A6`) every frame, allocates
//! list-1 type `$22` once `Camera_X + $1A0` reaches the next entry. Its
//! update `Func_037878` holds it at camera + (256, 80) with mapping `$3A6`
//! (tile word `$8000`: the `$3C646` art at tiles `$30..$38`, loaded by
//! `$A32E` in place of `$3C570` on mounted stages), hides it while
//! `timer & $C` is 0, requests effect `$B` through `Func_004B2E` while it is
//! `$C`, and frees it after `$A0 + 1` updates.
use crate::rom::{Result, word};

/// `Data_0377BC` (2-1) and `Data_0377C4` (4-1), each ended by a negative word.
const SCHEDULES: [usize; 2] = [0x377bc, 0x377c4];
/// `Func_037814`: `Camera_X + $1A0` must reach the entry.
const LEAD: i32 = 0x1a0;
/// `Func_037878`: camera + (`$100`, `$50`).
pub const POSITION: (i32, i32) = (0x100, 0x50);
pub const MAPPING: u16 = 0x3a6;
/// High priority, palette line 0, absolute tiles.
pub const TILE: u16 = 0x8000;
/// `$2C` = `$A0`; the sign is freed when it goes negative.
const LIFE: i32 = 0xa0;
/// `Func_004B2E` effect ID.
pub const EFFECT: u8 = 0x0b;
/// List-1 records (`Object_ListParams+5`); a full list drops the sign
/// but the schedule has already advanced.
const LIST1_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sign {
    /// `$28`, incremented first on every update.
    timer: u16,
    /// `$2C`.
    life: i32,
}

#[derive(Clone, Debug)]
pub struct Warnings {
    schedule: Vec<i32>,
    next: usize,
    signs: Vec<Sign>,
}

impl Warnings {
    /// `Func_0377D2`'s choice by `Round_Index`: below 2 the horse table.
    pub fn load(rom: &[u8], round_index: u8) -> Result<Self> {
        let mut at = SCHEDULES[usize::from(round_index >= 2)];
        let mut schedule = Vec::new();
        loop {
            let entry = word(rom, at)?;
            if entry & 0x8000 != 0 {
                break;
            }
            schedule.push(i32::from(entry));
            at += 2;
        }
        Ok(Self {
            schedule,
            next: 0,
            signs: Vec::new(),
        })
    }

    /// No schedule (`$FF0106` cleared): a surfed part without mode bit 7.
    pub fn none() -> Self {
        Self {
            schedule: Vec::new(),
            next: 0,
            signs: Vec::new(),
        }
    }

    /// One frame at the already scrolled camera X: `Func_037814`, then the
    /// signs' updates. Returns whether effect [`EFFECT`] was requested.
    pub fn tick(&mut self, camera_x: i32) -> bool {
        if let Some(&entry) = self.schedule.get(self.next)
            && camera_x + LEAD >= entry
        {
            self.next += 1;
            if self.signs.len() < LIST1_CAPACITY {
                self.signs.push(Sign {
                    timer: 0,
                    life: LIFE,
                });
            }
        }
        let mut sound = false;
        for sign in &mut self.signs {
            sign.timer = sign.timer.wrapping_add(1);
            sound |= sign.timer & 0xc == 0xc;
            sign.life -= 1;
        }
        // Func_036716 frees the record before this frame's sprite build.
        self.signs.retain(|sign| sign.life >= 0);
        sound
    }

    /// Screen positions of the signs shown this frame.
    pub fn visible(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.signs
            .iter()
            .filter(|sign| sign.timer & 0xc != 0)
            .map(|_| POSITION)
    }
}
