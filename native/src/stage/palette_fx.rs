//! The `$9C0` palette routines, run once per gameplay frame by `$99A`.
use crate::render::fade_step;
use crate::rom::{Result, byte};
use crate::sound_events::Table;
use std::ops::Range;

const TABLE_START: usize = 0xa86;

/// Flattened palette entries, frame mask, and rightward rotation.
#[derive(Clone, Debug)]
struct PaletteCycle(Range<usize>, u8, bool);
const RIGHT_THREE: &[PaletteCycle] = &[PaletteCycle(45..48, 7, true)];

/// Palette routines selected through `$9C0` (round*8 + substage*2) by `$894`
/// and called once per gameplay frame by `$99A` after it increments the
/// frame byte `$FF16EA`. `$0A00` is `rts`.
pub const PALETTE_ROUTINES: [u16; 14] = [
    0x0a00, 0x0bc8, 0x0c5c, 0x0d38, 0x0fa6, 0x1036, 0x106c, 0x10a2, 0x10c4, 0x10ea, 0x1120, 0x114c,
    0x11e0, 0x12b0,
];

/// Inputs a palette routine reads besides its own workspace.
#[derive(Clone, Copy, Debug, Default)]
pub struct PaletteContext {
    /// `Camera_XFixed` (`$FFE010`), continuous across 1-1 parts.
    pub background_x: i32,
    /// `$FF37AA`.
    pub part: usize,
    /// `VDP_Reg0B` (`$FF005F`) & 7.
    pub vdp_mode_3: u8,
    /// `$FF379C` bit 5, set by `Pause_S00` and cleared by `Pause_S10`.
    pub paused: bool,
}

/// Palette cycles are selected once at load; distinct table-driven effects
/// retain their ROM routine as provenance. Only `$A86..$1300` table bytes are
/// retained, and palette writes address native entries rather than RAM.
#[derive(Clone, Debug)]
pub struct PaletteEffect {
    routine: u16,
    tables: Vec<u8>,
    cycles: &'static [PaletteCycle],
    /// `$4A4E` effect 7 (`moveq #7,d0` at `$E74`) as a driver command.
    warning_sound: u8,
    frame: u8,
    phase: u16,
    flash_countdown: u16,
    previous_flash: u8,
    ramp_stopped: bool,
    storm_finished: bool,
}
impl PaletteEffect {
    pub fn load(rom: &[u8], routine: u16) -> Result<Self> {
        if !PALETTE_ROUTINES.contains(&routine) {
            return Err(format!("palette routine ${routine:04x} is not interpreted"));
        }
        let effect = byte(rom, 0xe75)?;
        Ok(Self {
            routine,
            tables: rom
                .get(TABLE_START..0x1300)
                .ok_or("short palette tables")?
                .to_vec(),
            cycles: match routine {
                0x0c5c => &[
                    PaletteCycle(44..48, 7, false),
                    PaletteCycle(60..64, 3, false),
                ],
                0x0fa6 | 0x1036 | 0x106c => RIGHT_THREE,
                0x10c4 => &[
                    PaletteCycle(42..45, 7, true),
                    PaletteCycle(45..48, 3, false),
                ],
                0x10ea => &[PaletteCycle(45..48, 7, true), PaletteCycle(60..64, 3, true)],
                0x11e0 => &[PaletteCycle(44..48, 7, false)],
                _ => &[],
            },
            warning_sound: Table::Effect.command(rom, effect)?,
            frame: 0,
            phase: 0,
            flash_countdown: 0,
            previous_flash: 0,
            ramp_stopped: false,
            storm_finished: false,
        })
    }
    fn byte(&self, at: usize) -> u8 {
        self.tables[at - TABLE_START]
    }
    fn word(&self, at: usize) -> u16 {
        u16::from_be_bytes([self.byte(at), self.byte(at + 1)])
    }
    /// Copy `count` table words to native palette entries.
    fn copy(&self, words: &mut [u16], start: usize, table: usize, count: usize) {
        for (i, w) in words[start..start + count].iter_mut().enumerate() {
            *w = self.word(table + i * 2);
        }
    }
    /// `$37108`: the 1-1 event sets `$FF16F0`, which stops the `$BC8` ramp.
    pub fn stop_ramp(&mut self) {
        self.previous_flash = 0;
        self.ramp_stopped = true;
    }
    /// `$3721A`: `$FF16EE = 4` starts the `$C08` flash countdown.
    pub fn start_flash(&mut self) {
        self.flash_countdown = 4;
    }
    /// One `$99A` call. Returns a sound effect command the routine requested.
    pub fn step(&mut self, words: &mut [[u16; 16]; 4], cx: &PaletteContext) -> Option<u8> {
        self.frame = self.frame.wrapping_add(1);
        let f = self.frame;
        let words = words.as_flattened_mut();
        // These ranges do not overlap the table writes below. Palette-dependent
        // rotations (the 7-2 red level) stay beside their corresponding lookup.
        for PaletteCycle(entries, mask, right) in self.cycles {
            if f & mask == 0 {
                if *right {
                    words[entries.clone()].rotate_right(1);
                } else {
                    words[entries.clone()].rotate_left(1);
                }
            }
        }
        match self.routine {
            0x0bc8 => self.forest(words, cx.background_x),
            0x0d38 => return self.storm(words, cx.part, cx.paused),
            0x0fa6 => {
                self.copy(words, 39, 0xe9e + usize::from(f & 3) * 10, 5);
                // $1010: both ramps follow the `$F86` triangle wave.
                let level = usize::from(self.byte(0xf86 + usize::from((f >> 3) & 15)));
                self.copy(words, 34, 0xec6 + level * 10, 5);
                self.copy(words, 50, 0xf16 + level * 14, 7);
            }
            0x1036 => {
                if f & 0x1f == 0 {
                    // The old byte counter advanced every 32 ticks; its low
                    // two bits are already present in the wrapping frame byte.
                    let i = usize::from((f >> 5) & 3) * 2;
                    let (a, b) = (self.word(0x1026 + i), self.word(0x102e + i));
                    words[56..58].copy_from_slice(&[a, b]);
                }
            }
            0x10a2 => {
                let i = usize::from((f >> 3) & 15) * 2;
                words[47] = self.word(0x1082 + i);
            }
            0x1120 => {
                let v = u16::from(self.byte(0x1110 + usize::from((f >> 2) & 15)));
                words[44..46].copy_from_slice(&[v, v >> 1]);
            }
            0x114c => {
                let a = usize::from(self.byte(0x1190 + usize::from(f & 0x3f)));
                let b = usize::from(self.byte(0x11d0 + usize::from(f & 15)));
                let (a, b) = (self.word(0x1180 + a * 2), self.word(0x1180 + b * 2));
                words[62..64].copy_from_slice(&[a, b]);
            }
            0x11e0 => {
                let i = usize::from(f & 15);
                let a = self.byte(0x1110 + i);
                let b = self.byte(0x1110 + ((i + 10) & 15)); // (i - 6) & 15
                words[59..61].copy_from_slice(&[u16::from(a), u16::from(b)]);
                if b & 1 == 0 {
                    words[61..64].rotate_left(1);
                }
            }
            // Active only while VDP register $0B is 7 (the 7-3 boss sets
            // it). `$FF379C` bit 5 and `Game_Flags` bit 4 are not modelled.
            0x12b0 if cx.vdp_mode_3 == 7 => {
                self.phase = self.phase.wrapping_add(1);
                let row = usize::from((self.phase >> 8) & 7) * 16;
                if self.phase & 7 == 0 {
                    for i in 0..8 {
                        let target = self.word(0x1230 + row + i * 2);
                        let w = &mut words[34 + i];
                        *w = fade_step(*w, target); // $6E8
                    }
                }
            }
            _ => {} // $0A00: rts
        }
        None
    }
    /// `$BC8`: camera ramp of bank two, then the `$C08` event flash.
    fn forest(&mut self, words: &mut [u16], background_x: i32) {
        let x = background_x - 0x1000;
        if !self.ramp_stopped && x >= 0 {
            let row = usize::try_from((x >> 4).min(7)).unwrap_or(0);
            self.copy(words, 32, 0xa86 + row * 32, 16);
        }
        if self.flash_countdown == 0 {
            return;
        }
        if self.flash_countdown != 1 {
            self.flash_countdown -= 1;
            return;
        }
        // $C20 increments the cursor BEFORE selecting the eight-tick row.
        self.phase = self.phase.wrapping_add(1);
        let row = 0xb86 + usize::from((self.phase >> 3) & 15) * 8;
        if self.word(row) & 0x8000 != 0 {
            self.flash_countdown = 0; // $FFFF: stop, keeping the last colors.
        } else {
            self.copy(words, 43, row, 4);
        }
    }
    /// `$D38` (2-2): two color cycles and, outside part 0, the lightning
    /// flash with its `$4A4E` warning effect on each rising flash flag.
    fn storm(&mut self, words: &mut [u16], part: usize, paused: bool) -> Option<u8> {
        let f = self.frame;
        if f & 3 == 0 {
            let phase = f >> 2;
            let a = usize::from(phase >> 1) & 7;
            let b = (a + 4) & 7;
            let colors = [
                u16::from(self.byte(0xd12 + a)) << 4,
                u16::from(self.byte(0xd12 + b)) << 4,
                self.word(0xd1a + usize::from(phase & 15) * 2),
            ];
            words[45..48].copy_from_slice(&colors);
            self.copy(words, 60, 0xcf2 + usize::from((f >> 2) & 7) * 4, 2);
        }
        // `$DBC`: normal color cycles run, but pause holds the lightning
        // workspace before either its countdown or its flash advances.
        if part == 0 || paused || self.storm_finished {
            return None;
        }
        if self.flash_countdown == 0 {
            self.phase = self.phase.wrapping_add(1);
            if (self.phase as i16) < 0x12c {
                return None;
            }
            self.phase = 0;
            self.previous_flash = 0;
            self.flash_countdown = 1;
        }
        if f & 3 != 0 {
            return None;
        }
        // `addq.b #1,($FF16EC)` advances the word's high byte.
        self.phase = self.phase.wrapping_add(0x100);
        let i = usize::from(self.phase >> 8) & 15;
        let level = usize::from(self.byte(0xcd2 + i));
        let flash = self.byte(0xce2 + i);
        self.copy(words, 52, 0xc82 + level * 16, 8);
        let rising = flash & !self.previous_flash;
        self.previous_flash = flash;
        if part != 1 && i == 0 {
            self.storm_finished = true; // Other parts flash once.
        }
        (rising != 0).then_some(self.warning_sound)
    }
}
