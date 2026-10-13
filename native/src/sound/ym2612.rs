//! YM2612 operator, envelope, LFO and channel model at the chip's own sample rate.
use super::FM_CLOCK;
use crate::rom::{Result, byte};
use std::f64::consts::TAU;
use std::sync::OnceLock;

const ENVELOPE_MAX: u16 = 1023;
const ATTACK: u8 = 1;
const DECAY: u8 = 2;
const SUSTAIN: u8 = 3;
const RELEASE: u8 = 4;
/// Key-code note bits from F-number bits 10..7 (YM2612 datasheet).
const FN_NOTE: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 3, 3, 3, 3, 3, 3];
/// YM2612 detune table in phase-increment units, indexed by DT1 & 3 and key code.
const DETUNE: [[u8; 32]; 4] = [
    [0; 32],
    [
        0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 8,
        8, 8,
    ],
    [
        1, 1, 1, 1, 2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12, 13, 14,
        16, 16, 16, 16,
    ],
    [
        2, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19,
        20, 22, 22, 22, 22,
    ],
];
/// Envelope increments per eight-step cycle (YM2612 rate tables as used by MAME/Genesis Plus).
const EG_INC: [[u8; 8]; 19] = [
    [0, 1, 0, 1, 0, 1, 0, 1],
    [0, 1, 0, 1, 1, 1, 0, 1],
    [0, 1, 1, 1, 0, 1, 1, 1],
    [0, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1, 1, 1],
    [1, 1, 1, 2, 1, 1, 1, 2],
    [1, 2, 1, 2, 1, 2, 1, 2],
    [1, 2, 2, 2, 1, 2, 2, 2],
    [2, 2, 2, 2, 2, 2, 2, 2],
    [2, 2, 2, 4, 2, 2, 2, 4],
    [2, 4, 2, 4, 2, 4, 2, 4],
    [2, 4, 4, 4, 2, 4, 4, 4],
    [4, 4, 4, 4, 4, 4, 4, 4],
    [4, 4, 4, 8, 4, 4, 4, 8],
    [4, 8, 4, 8, 4, 8, 4, 8],
    [4, 8, 8, 8, 4, 8, 8, 8],
    [8, 8, 8, 8, 8, 8, 8, 8],
    [16, 16, 16, 16, 16, 16, 16, 16],
    [0, 0, 0, 0, 0, 0, 0, 0],
];
/// FM samples per LFO step for register $22 frequencies 0..7 (3.98 Hz .. 72.2 Hz).
const LFO_PERIOD: [u32; 8] = [108, 77, 71, 67, 62, 44, 8, 5];
/// Tremolo shifts for AMS 0..3 (0, 1.4, 5.9 and 11.8 dB peak).
const AMS_SHIFT: [u32; 4] = [8, 3, 1, 0];
/// Peak vibrato for PMS 0..7 in cents (YM2612 datasheet).
const PMS_CENTS: [f64; 8] = [0.0, 3.4, 6.7, 10.0, 14.0, 20.0, 40.0, 80.0];
/// FM3 special-mode offsets are written to $A9, $A8, $AA, $A2 ($69B9A), i.e. slots 1, 3, 2, 4.
/// Index by logical operator to find its offset slot.
const SPECIAL_ORDER: [usize; 4] = [0, 2, 1, 3];

struct Tables {
    sine: [f64; 1024],
    gain: [f64; 1024],
    /// F-number scale per PMS and LFO vibrato step (-7..7, indexed from 0).
    vibrato: [[f64; 15]; 8],
}
fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut sine = [0.0; 1024];
        let mut gain = [0.0; 1024];
        for i in 0..1024 {
            // The chip's log-sine table samples the middle of each of its 1024 phase steps.
            sine[i] = (TAU * (i as f64 + 0.5) / 1024.0).sin();
            // Ten-bit attenuation: 64 units per 6.02 dB (0.094 dB per step).
            gain[i] = 2.0_f64.powf(-(i as f64) / 64.0);
        }
        let vibrato = PMS_CENTS.map(|peak| {
            std::array::from_fn(|i| {
                let step = i as i8 - 7;
                let position = f64::from(step.unsigned_abs()) / 7.0;
                let cents = peak * if step < 0 { -position } else { position };
                if cents == 0.0 {
                    1.0
                } else {
                    2.0_f64.powf(cents / 1200.0)
                }
            })
        });
        Tables {
            sine,
            gain,
            vibrato,
        }
    })
}

/// YM2612 frequency of a packed `block << 11 | fnum` word:
/// `fnum * 2^(block - 1) * clock / (144 * 2^20)`.
pub(super) fn fm_frequency(word: u16) -> f64 {
    let fnum = f64::from(word & 0x7ff);
    let block = i32::from((word >> 11) & 7);
    fnum * 2.0_f64.powi(block - 1) * FM_CLOCK / (144.0 * 1_048_576.0)
}
/// Five-bit key code used for detune and rate scaling.
fn key_code(word: u16) -> u8 {
    ((((word >> 11) & 7) as u8) << 2) | FN_NOTE[usize::from((word >> 7) & 15)]
}
/// Per-sample 20-bit phase increment. Detune is added to `(fnum << block) >> 1`
/// before MUL is applied, as on the chip; `vibrato` scales the F-number.
fn phase_increment(word: u16, detune_multiple: u8, vibrato: f64) -> u32 {
    let mut fnum = u32::from(word & 0x7ff);
    if vibrato != 1.0 {
        fnum = ((f64::from(fnum) * vibrato).round() as u32).min(0xfff);
    }
    let block = u32::from((word >> 11) & 7);
    let base = (fnum << block) >> 1;
    let dt = (detune_multiple >> 4) & 7;
    let delta = u32::from(DETUNE[usize::from(dt & 3)][usize::from(key_code(word))]);
    let detuned = (if dt & 4 != 0 {
        base.wrapping_sub(delta)
    } else {
        base + delta
    }) & 0x1ffff;
    let multiple = u32::from(detune_multiple & 15);
    if multiple == 0 {
        detuned >> 1
    } else {
        (detuned * multiple) & 0xfffff
    }
}
/// Six-bit envelope rate: zero stays zero, otherwise `2R + (KC >> (3 - KS))`.
fn envelope_rate(raw: u8, key_scale: u8, key_code: u8) -> u8 {
    if raw == 0 {
        0
    } else {
        (2 * raw + (key_code >> (3 - (key_scale & 3)))).min(63)
    }
}
/// Increment applied on this envelope clock; rates double every four steps.
fn envelope_increment(rate: u8, counter: u32) -> u16 {
    let shift = if rate < 48 {
        11 - u32::from(rate >> 2)
    } else {
        0
    };
    if counter & ((1 << shift) - 1) != 0 {
        return 0;
    }
    let row = match rate {
        0 | 1 => 18,
        2..=47 => usize::from(rate & 3),
        48..=59 => usize::from(rate - 44),
        _ => 16,
    };
    u16::from(EG_INC[row][((counter >> shift) & 7) as usize])
}

/// Chip-wide state shared by every FM channel for one 53267 Hz sample.
#[derive(Clone, Copy)]
pub(super) struct ChipClock {
    /// Global envelope counter when this sample carries an envelope clock (every third sample).
    envelope: Option<u32>,
    /// LFO tremolo attenuation before the AMS shift, 0..126 envelope units.
    tremolo: u16,
    /// LFO vibrato step, -7..7.
    vibrato: i8,
}

#[derive(Clone, Copy, Default)]
struct Lfo {
    control: u8,
    counter: u8,
    timer: u32,
}
impl Lfo {
    fn clock(&mut self) {
        if self.control & 8 == 0 {
            self.counter = 0;
            self.timer = 0;
            return;
        }
        self.timer += 1;
        if self.timer >= LFO_PERIOD[usize::from(self.control & 7)] {
            self.timer = 0;
            self.counter = (self.counter + 1) & 127;
        }
    }
    fn amplitude(&self) -> u16 {
        let step = u16::from(self.counter & 63);
        (if self.counter & 64 != 0 {
            step ^ 63
        } else {
            step
        }) << 1
    }
    fn vibrato(&self) -> i8 {
        let step = self.counter >> 2;
        let quarter = step & 15;
        let value = (if quarter < 8 { quarter } else { 15 - quarter }) as i8;
        if step & 16 != 0 { -value } else { value }
    }
}

#[derive(Default)]
pub(super) struct Chip {
    lfo: Lfo,
    envelope_divider: u8,
    envelope_counter: u32,
}
impl Chip {
    pub(super) fn clock(&mut self, lfo_control: u8) -> ChipClock {
        self.lfo.control = lfo_control;
        self.lfo.clock();
        self.envelope_divider += 1;
        let envelope = if self.envelope_divider == 3 {
            self.envelope_divider = 0;
            // The 12-bit envelope counter skips zero.
            self.envelope_counter = self.envelope_counter % 4095 + 1;
            Some(self.envelope_counter)
        } else {
            None
        };
        ChipClock {
            envelope,
            tremolo: self.lfo.amplitude(),
            vibrato: self.lfo.vibrato(),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Operator {
    /// 20-bit phase accumulator; the top ten bits address the sine.
    phase: u32,
    /// Ten-bit envelope attenuation, 0 loudest.
    envelope: u16,
    stage: u8,
    ssg_inverted: bool,
    detune_multiple: u8,
    /// KS (bits 6-7) and AR.
    pub(super) attack: u8,
    /// AM enable (bit 7) and D1R.
    pub(super) decay: u8,
    sustain_rate: u8,
    sustain_release: u8,
    total_level: u8,
    /// SSG-EG register $90 (low four bits).
    pub(super) ssg: u8,
}
impl Default for Operator {
    fn default() -> Self {
        Self {
            phase: 0,
            envelope: ENVELOPE_MAX,
            stage: RELEASE,
            ssg_inverted: false,
            detune_multiple: 0,
            attack: 0,
            decay: 0,
            sustain_rate: 0,
            sustain_release: 0,
            total_level: 0,
            ssg: 0,
        }
    }
}
impl Operator {
    fn key_scale(&self) -> u8 {
        self.attack >> 6
    }
    fn sustain_level(&self) -> u16 {
        let level = u16::from(self.sustain_release >> 4);
        if level == 15 { 992 } else { level * 32 }
    }
    fn attack_complete(&mut self) {
        self.envelope = 0;
        self.stage = if self.sustain_level() == 0 {
            SUSTAIN
        } else {
            DECAY
        };
    }
    fn key_on(&mut self, key_code: u8) {
        self.phase = 0;
        self.ssg_inverted = false;
        self.stage = ATTACK;
        // Attack starts from the current level; rates 62/63 are instantaneous.
        if envelope_rate(self.attack & 31, self.key_scale(), key_code) >= 62 {
            self.attack_complete();
        }
    }
    fn key_off(&mut self) {
        if self.stage == RELEASE {
            return;
        }
        if self.ssg_output_inverted() {
            self.envelope = 0x200u16.wrapping_sub(self.envelope) & 0x3ff;
        }
        self.ssg_inverted = false;
        self.stage = RELEASE;
    }
    fn ssg_output_inverted(&self) -> bool {
        self.ssg & 8 != 0 && (self.ssg_inverted ^ (self.ssg & 4 != 0))
    }
    /// SSG-EG repeat/hold/alternate transitions once the level passes $200.
    fn ssg_update(&mut self, key_code: u8) {
        if self.ssg & 8 == 0 || self.envelope < 0x200 || self.stage == RELEASE {
            return;
        }
        if self.ssg & 1 != 0 {
            if self.ssg & 2 != 0 {
                self.ssg_inverted = true;
            }
            if self.stage != ATTACK && !(self.ssg_inverted ^ (self.ssg & 4 != 0)) {
                self.envelope = ENVELOPE_MAX;
            }
        } else {
            if self.ssg & 2 != 0 {
                self.ssg_inverted = !self.ssg_inverted;
            } else {
                self.phase = 0;
            }
            if self.stage != ATTACK {
                if envelope_rate(self.attack & 31, self.key_scale(), key_code) >= 62 {
                    self.attack_complete();
                } else {
                    self.stage = ATTACK;
                }
            }
        }
    }
    fn envelope_step(&mut self, counter: u32, key_code: u8) {
        let key_scale = self.key_scale();
        match self.stage {
            ATTACK => {
                let rate = envelope_rate(self.attack & 31, key_scale, key_code);
                if rate >= 62 {
                    self.attack_complete();
                    return;
                }
                let increment = i32::from(envelope_increment(rate, counter));
                if increment != 0 {
                    let level = i32::from(self.envelope);
                    let next = level + (((!level) * increment) >> 4);
                    if next <= 0 {
                        self.attack_complete();
                    } else {
                        self.envelope = next as u16;
                    }
                }
            }
            DECAY | SUSTAIN | RELEASE => {
                let raw = match self.stage {
                    DECAY => self.decay & 31,
                    SUSTAIN => self.sustain_rate & 31,
                    _ => (self.sustain_release & 15) * 2 + 1,
                };
                let mut increment =
                    envelope_increment(envelope_rate(raw, key_scale, key_code), counter);
                if self.ssg & 8 != 0 && self.stage != RELEASE {
                    increment = if self.envelope < 0x200 {
                        increment * 4
                    } else {
                        0
                    };
                }
                self.envelope = (self.envelope + increment).min(ENVELOPE_MAX);
                if self.stage == DECAY && self.envelope >= self.sustain_level() {
                    self.stage = SUSTAIN;
                }
            }
            _ => {}
        }
    }
    fn attenuation(&self, total_level: u8, tremolo: u16) -> u16 {
        let envelope = if self.stage != RELEASE && self.ssg_output_inverted() {
            0x200u16.wrapping_sub(self.envelope) & 0x3ff
        } else {
            self.envelope
        };
        let tremolo = if self.decay & 0x80 != 0 { tremolo } else { 0 };
        (envelope + (u16::from(total_level) << 3) + tremolo).min(ENVELOPE_MAX)
    }
    /// One 14-bit signed output; `modulation` is in 1/1024-cycle phase units.
    fn output(&mut self, increment: u32, modulation: i32, attenuation: u16) -> i32 {
        let index = (((self.phase >> 10) as i32 + modulation) & 1023) as usize;
        self.phase = (self.phase + increment) & 0xfffff;
        if attenuation >= ENVELOPE_MAX {
            return 0;
        }
        let t = tables();
        (t.sine[index] * t.gain[usize::from(attenuation)] * 8191.0) as i32
    }
}

#[derive(Clone, Default)]
pub(super) struct Voice {
    pub(super) operators: [Operator; 4],
    algorithm: u8,
    feedback: u8,
    /// Register $B4 AMS (bits 4-5) and PMS (bits 0-2); voice loads preserve it.
    pub(super) sensitivity: u8,
    previous: [i32; 2],
    pub(super) special_pitch: [u16; 4],
}
impl Voice {
    pub(super) fn load(&mut self, rom: &[u8], at: usize) -> Result<()> {
        let config = byte(rom, at)?;
        self.algorithm = config & 7;
        self.feedback = (config >> 3) & 7;
        // The ROM register list $6AA52 stores logical operators 1,2,3,4.
        for (i, op) in self.operators.iter_mut().enumerate() {
            op.detune_multiple = byte(rom, at + 1 + i)?;
            op.attack = byte(rom, at + 5 + i)?;
            op.decay = byte(rom, at + 9 + i)?;
            op.sustain_rate = byte(rom, at + 13 + i)?;
            op.sustain_release = byte(rom, at + 17 + i)?;
            op.total_level = byte(rom, at + 21 + i)?;
        }
        Ok(())
    }
    /// Apply a channel register write whose low two bits address channel 0 (`$ED`).
    /// Writes to other registers are consumed without effect.
    pub(super) fn write_register(&mut self, register: u8, value: u8, pan: &mut u8) {
        let op = &mut self.operators[SPECIAL_ORDER[usize::from((register >> 2) & 3)]];
        match register & 0xf0 {
            0x30 => op.detune_multiple = value,
            0x40 => op.total_level = value,
            0x50 => op.attack = value,
            0x60 => op.decay = value,
            0x70 => op.sustain_rate = value,
            0x80 => op.sustain_release = value,
            0x90 => op.ssg = value & 15,
            0xb0 if register & 4 == 0 => {
                self.algorithm = value & 7;
                self.feedback = (value >> 3) & 7;
            }
            0xb0 => {
                *pan = value & 0xc0;
                self.sensitivity = value & 0x37;
            }
            _ => {}
        }
    }
    fn operator_word(&self, word: u16, i: usize) -> u16 {
        word.wrapping_add(self.special_pitch[SPECIAL_ORDER[i]])
    }
    pub(super) fn key_on(&mut self, word: u16) {
        for i in 0..4 {
            let key_code = key_code(self.operator_word(word, i));
            self.operators[i].key_on(key_code);
        }
    }
    pub(super) fn key_off(&mut self) {
        for op in &mut self.operators {
            op.key_off();
        }
    }
    /// One chip-rate sample, clamped to the 14-bit channel accumulator.
    pub(super) fn render(&mut self, word: u16, volume: u8, clock: &ChipClock) -> i32 {
        // Output operators for the YM2612's eight routing algorithms.
        let carriers = match self.algorithm & 7 {
            0..=3 => 0b1000,
            4 => 0b1010,
            5 | 6 => 0b1110,
            _ => 0b1111,
        };
        let tremolo = clock.tremolo >> AMS_SHIFT[usize::from((self.sensitivity >> 4) & 3)];
        let vibrato =
            tables().vibrato[usize::from(self.sensitivity & 7)][(clock.vibrato + 7) as usize];
        // Self-feedback: average of operator 1's last two outputs, (o1 + o2) >> (10 - FB).
        let feedback = if self.feedback == 0 {
            0
        } else {
            (self.previous[0] + self.previous[1]) >> (10 - self.feedback)
        };
        let mut values = [0i32; 4];
        for i in 0..4 {
            let input = if i == 0 {
                feedback
            } else {
                // Operator-to-operator modulation: 14-bit output >> 1 (about 8 pi peak).
                (match (self.algorithm, i) {
                    (0, 1) | (3, 1) | (4, 1) | (5, 1) | (6, 1) => values[0],
                    (0, 2) | (2, 2) => values[1],
                    (1, 2) => values[0] + values[1],
                    (5, 2) | (5, 3) => values[0],
                    (0, 3) | (1, 3) | (4, 3) => values[2],
                    (2, 3) => values[0] + values[2],
                    (3, 3) => values[1] + values[2],
                    _ => 0,
                }) >> 1
            };
            let op_word = self.operator_word(word, i);
            let key_code = key_code(op_word);
            let carrier = carriers & (1 << i) != 0;
            let op = &mut self.operators[i];
            if let Some(counter) = clock.envelope {
                op.envelope_step(counter, key_code);
            }
            op.ssg_update(key_code);
            let level = if carrier {
                (op.total_level & 127).saturating_add(volume).min(127)
            } else {
                op.total_level & 127
            };
            let attenuation = op.attenuation(level, tremolo);
            values[i] = op.output(
                phase_increment(op_word, op.detune_multiple, vibrato),
                input,
                attenuation,
            );
        }
        self.previous = [self.previous[1], values[0]];
        let sum: i32 = (0..4)
            .filter(|i| carriers & (1 << i) != 0)
            .map(|i| values[i])
            .sum();
        sum.clamp(-8192, 8191)
    }
}
