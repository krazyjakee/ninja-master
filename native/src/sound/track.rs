//! One SMPS-style sequence track: the driver's command interpreter, pitch, PSG and
//! pan envelopes, and the channel's FM, PSG or DAC output.
use super::dac::{Dac, Samples};
use super::ym2612::{ChipClock, Voice, fm_frequency};
use super::{ChannelKind, FM_RATE, Globals, PSG_CLOCK, PSG_TICK_RATE};
use crate::rom::{Result, byte, pointer, word};
use std::sync::OnceLock;

const MAX_COMMANDS: usize = 1024;

#[derive(Default)]
struct Modulation {
    params: [u8; 4],
    enabled: bool,
    delay: u8,
    speed: u8,
    steps: u8,
    delta: i8,
    offset: i16,
}
impl Modulation {
    fn reset(&mut self) {
        self.delay = self.params[0];
        self.speed = self.params[1];
        self.delta = self.params[2] as i8;
        self.steps = self.params[3] >> 1;
        self.offset = 0;
    }
    fn tick(&mut self) {
        if !self.enabled {
            return;
        }
        if self.delay != 0 {
            self.delay -= 1;
            return;
        }
        self.speed = self.speed.wrapping_sub(1);
        if self.speed != 0 {
            return;
        }
        self.speed = self.params[1];
        if self.steps == 0 {
            self.steps = self.params[3];
            self.delta = self.delta.wrapping_neg();
            return;
        }
        self.steps -= 1;
        self.offset = self.offset.wrapping_add(i16::from(self.delta));
    }
}

pub(super) struct Track {
    pub(super) kind: ChannelKind,
    pub(super) hardware: u8,
    pub(super) pc: usize,
    instruments: usize,
    pub(super) active: bool,
    transpose: u8,
    pub(super) volume: u8,
    /// Native output level, independent of the ROM's sequence attenuation.
    pub(super) mix_gain: f64,
    pub(super) multiplier: u8,
    duration: u8,
    pub(super) remaining: u16,
    note: u8,
    gate: u8,
    gate_left: u8,
    keyed: bool,
    detune: i8,
    loops: [u8; 8],
    calls: Vec<usize>,
    voice: Voice,
    modulation: Modulation,
    pub(super) psg_envelope: u8,
    envelope_cursor: u8,
    envelope_value: u8,
    pub(super) pitch_envelope: u8,
    pitch_cursor: u8,
    pitch_scale: i8,
    pitch_offset: i16,
    pan: u8,
    pan_program: [u8; 5],
    pan_cursor: u8,
    pan_clock: u8,
    noise: Option<u8>,
    /// A note or rest byte has been read. The `$B2` noise track sets noise
    /// mode (`$F3 $E4`) and then gives only durations; the emulator's PSG
    /// stem shows it sounding, so a fixed-rate noise track keys on before
    /// its first note.
    noted: bool,
    noise_lfsr: u16,
    /// PSG counter ticks until the next square edge or noise shift.
    psg_countdown: f64,
    psg_high: bool,
    hz: f64,
    /// Packed YM2612 `block << 11 | fnum` word, including driver pitch offsets.
    fm_word: u16,
    /// Track bytes 32..35: the SFX voice-table pointer, or the pan program
    /// id/cursor/bound/delay. `$E9` reads operator D1R bytes through it.
    pub(super) register_pointer: [u8; 4],
    /// Set when `$E3`/`$F2` stops the track; SFX stops release the driver priority.
    pub(super) stopped: bool,
    pub(super) dac: Option<Dac>,
}
impl Track {
    pub(super) fn new(
        kind: ChannelKind,
        hardware: u8,
        pc: usize,
        instruments: usize,
        multiplier: u8,
        transpose: u8,
        volume: u8,
    ) -> Self {
        Self {
            kind,
            hardware,
            pc,
            instruments,
            active: true,
            transpose,
            volume,
            mix_gain: 1.0,
            multiplier,
            duration: 0,
            remaining: 1,
            note: 0x80,
            gate: 0,
            gate_left: 0,
            keyed: false,
            detune: 0,
            loops: [0; 8],
            calls: Vec::new(),
            voice: Voice::default(),
            modulation: Modulation::default(),
            psg_envelope: 0,
            envelope_cursor: 0,
            envelope_value: 0,
            pitch_envelope: 0,
            pitch_cursor: 0,
            pitch_scale: 0,
            pitch_offset: 0,
            pan: 0xc0,
            pan_program: [0; 5],
            pan_cursor: 0,
            pan_clock: 0,
            noise: None,
            noted: false,
            noise_lfsr: 0x8000,
            psg_countdown: 0.0,
            psg_high: false,
            hz: 0.0,
            fm_word: 0,
            register_pointer: [0; 4],
            stopped: false,
            dac: None,
        }
    }
    pub(super) fn slot(&self) -> u8 {
        if self.hardware == 0xe0 {
            0xc0
        } else {
            self.hardware
        }
    }
    fn read(&mut self, rom: &[u8]) -> Result<u8> {
        let value = byte(rom, self.pc)?;
        self.pc += 1;
        Ok(value)
    }
    fn branch(&mut self, rom: &[u8]) -> Result<()> {
        let delta = word(rom, self.pc)? as i16;
        let target = self.pc as i64 + 1 + i64::from(delta);
        if target < 0 || target >= rom.len() as i64 {
            return Err("sequence branch outside ROM".into());
        }
        self.pc = target as usize;
        Ok(())
    }
    pub(super) fn off(&mut self) {
        self.keyed = false;
        self.voice.key_off();
    }
    fn set_pitch(&mut self, rom: &[u8]) -> Result<()> {
        let offset = i32::from(self.detune)
            + i32::from(self.pitch_offset)
            + if self.modulation.enabled {
                i32::from(self.modulation.offset)
            } else {
                0
            };
        self.hz = match self.kind {
            ChannelKind::Fm => {
                let n = self.note.wrapping_sub(0x80).wrapping_add(self.transpose) & 127;
                let fnum = word(rom, 0x6a55c + usize::from(n % 12) * 2)?;
                // $6996C..$69978 packs block << 11 | fnum; $69A58 adds the detune,
                // envelope and modulation offsets to that whole 16-bit word.
                self.fm_word = (fnum | (u16::from((n / 12) & 7) << 11)).wrapping_add(offset as u16);
                fm_frequency(self.fm_word)
            }
            ChannelKind::Psg => {
                let n = self.note.wrapping_sub(0x81).wrapping_add(self.transpose) & 127;
                // $6A5D8 masks to seven bits, without bounding the 70-word table.
                // Command $83 actually uses index 121 at $6F40C, reading $6000
                // from the dispatch code at $6A80E. Preserve this proven lookup;
                // the mask and checked ROM read bound its extent to 128 words.
                let raw_period = word(rom, 0x6a71c + usize::from(n) * 2)?;
                if raw_period & 0x8000 != 0 {
                    // $6A5FE..$6A608 treats a negative period as a rest.
                    self.off();
                    return Ok(());
                }
                let period = (i32::from(raw_period) + offset) & 1023;
                PSG_CLOCK / (32.0 * f64::from(period.max(1)))
            }
            ChannelKind::Dac => 0.0,
        };
        Ok(())
    }
    pub(super) fn tick(
        &mut self,
        rom: &[u8],
        samples: &Samples,
        globals: &mut Globals,
    ) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        self.remaining = self.remaining.saturating_sub(1);
        let new_event = self.remaining == 0;
        if new_event {
            let mut tied = false;
            let mut complete = false;
            for _ in 0..MAX_COMMANDS {
                let at = self.pc;
                let code = self.read(rom)?;
                if code < 0xe0 {
                    if code >= 0x80 {
                        self.note = code;
                        self.noted = true;
                        if byte(rom, self.pc)? < 0x80 {
                            let duration = self.read(rom)?;
                            self.duration = duration.wrapping_mul(self.multiplier);
                        }
                    } else {
                        self.duration = code.wrapping_mul(self.multiplier);
                    }
                    self.remaining = if self.duration == 0 {
                        256
                    } else {
                        u16::from(self.duration)
                    };
                    let mut retrigger = false;
                    // Fixed-rate noise (mode bits 0..2) needs no pitch.
                    let unpitched_noise = !self.noted
                        && self.kind == ChannelKind::Psg
                        && self.noise.is_some_and(|n| n & 3 != 3);
                    if self.note == 0x80 && !unpitched_noise {
                        self.off();
                    } else if !tied {
                        retrigger = true;
                        self.keyed = true;
                        self.gate_left = self.gate;
                        self.envelope_cursor = 0;
                        self.envelope_value = 0;
                        self.pitch_cursor = 0;
                        self.pitch_scale = 0;
                        self.pitch_offset = 0;
                        self.modulation.reset();
                        if self.kind == ChannelKind::Dac {
                            self.dac = Some(Dac::load(rom, samples, self.note)?);
                        }
                    }
                    if self.note != 0x80 {
                        self.set_pitch(rom)?;
                    }
                    if retrigger {
                        // After the pitch, so key scaling sees this note's key code.
                        self.voice.key_on(self.fm_word);
                    }
                    complete = true;
                    break;
                }
                match code {
                    0xe0 => self.pan = self.read(rom)? & 0xc0,
                    0xe1 => self.detune = self.read(rom)? as i8,
                    0xe2 => globals.communication = self.read(rom)?,
                    0xe3 | 0xf2 => {
                        self.active = false;
                        self.stopped = true;
                        self.off();
                        return Ok(());
                    }
                    0xe4 => {
                        self.pan_program[0] = self.read(rom)?;
                        if self.pan_program[0] != 0 {
                            for i in 1..5 {
                                self.pan_program[i] = self.read(rom)?;
                            }
                        }
                        if self.pan_program[0] != 0 {
                            let [_, id, cursor, end, delay] = self.pan_program;
                            self.register_pointer = [id, cursor, end, delay];
                        }
                        self.pan_cursor = self.pan_program[2];
                        self.pan_clock = self.pan_program[4];
                    }
                    0xe5 => {
                        let psg = self.read(rom)?;
                        let fm = self.read(rom)?;
                        self.volume = self.volume.wrapping_add(if self.kind == ChannelKind::Psg {
                            psg
                        } else {
                            fm
                        });
                    }
                    0xe6 | 0xec => {
                        let delta = self.read(rom)?;
                        self.volume = self.volume.wrapping_add(delta);
                    }
                    0xe7 => tied = true,
                    0xe8 => {
                        self.gate = self.read(rom)?;
                        self.gate_left = self.gate;
                    }
                    0xe9 => {
                        // $6A8D2: bits 7..4 of the first byte select operators 1..4 whose
                        // D1R register is rewritten with AM enabled; the whole byte is
                        // then written to LFO register $22. The second byte supplies
                        // AMS/PMS for $B4, keeping the pan bits.
                        let control = self.read(rom)?;
                        let sensitivity = self.read(rom)?;
                        // The D1R source is the track's bytes 32..35 as a 24-bit
                        // address, not the current voice (see sound-recovery.md).
                        let source =
                            usize::try_from(u32::from_be_bytes(self.register_pointer) & 0xff_ffff)
                                .unwrap_or(usize::MAX);
                        for (i, op) in self.voice.operators.iter_mut().enumerate() {
                            if control & (0x80 >> i) != 0 {
                                let d1r = source
                                    .checked_add(i)
                                    .and_then(|at| rom.get(at).copied())
                                    .unwrap_or(op.decay);
                                op.decay = d1r | 0x80;
                            }
                        }
                        globals.lfo = control;
                        self.voice.sensitivity = sensitivity & 0x37;
                    }
                    0xea => {
                        globals.tempo = self.read(rom)?;
                        globals.tempo_clock = globals.tempo;
                    }
                    0xeb => globals.mailbox = Some(self.read(rom)?),
                    0xed => {
                        let register = self.read(rom)?;
                        let value = self.read(rom)?;
                        if self.kind == ChannelKind::Fm {
                            self.voice.write_register(register, value, &mut self.pan);
                        }
                    }
                    0xee => {
                        let register = self.read(rom)?;
                        let value = self.read(rom)?;
                        if register == 0x22 {
                            globals.lfo = value;
                        }
                    }
                    0xef => {
                        let voice = self.read(rom)?;
                        self.voice
                            .load(rom, self.instruments + usize::from(voice) * 25)?;
                    }
                    0xf0 => {
                        for i in 0..4 {
                            self.modulation.params[i] = self.read(rom)?;
                        }
                        self.modulation.enabled = true;
                        self.modulation.reset();
                    }
                    0xf1 => {
                        let psg = self.read(rom)?;
                        let fm = self.read(rom)?;
                        self.pitch_envelope = if self.kind == ChannelKind::Psg {
                            psg
                        } else {
                            fm
                        };
                    }
                    0xf3 => self.noise = Some(self.read(rom)?),
                    0xf4 => self.pitch_envelope = self.read(rom)?,
                    0xf5 => self.psg_envelope = self.read(rom)?,
                    0xf6 => self.branch(rom)?,
                    0xf7 => {
                        let index = usize::from(self.read(rom)?);
                        let count = self.read(rom)?;
                        let value = self
                            .loops
                            .get_mut(index)
                            .ok_or("sequence loop index exceeds track storage")?;
                        if *value == 0 {
                            *value = count;
                        }
                        *value = value.wrapping_sub(1);
                        if *value != 0 {
                            self.branch(rom)?;
                        } else {
                            self.read(rom)?;
                            self.read(rom)?;
                        }
                    }
                    0xf8 => {
                        if self.calls.len() >= 8 {
                            return Err("sequence call stack overflow".into());
                        }
                        self.calls.push(self.pc + 2);
                        self.branch(rom)?;
                    }
                    0xf9 => self.pc = self.calls.pop().ok_or("sequence return with empty stack")?,
                    0xfa => self.multiplier = self.read(rom)?,
                    0xfb => {
                        let delta = self.read(rom)?;
                        self.transpose = self.transpose.wrapping_add(delta);
                    }
                    0xfc => self.modulation.enabled = true,
                    0xfd => self.modulation.enabled = false,
                    0xfe => {
                        for i in 0..4 {
                            let index = usize::from(self.read(rom)?);
                            if index > 3 {
                                return Err("invalid FM3 pitch index".into());
                            }
                            self.voice.special_pitch[i] = word(rom, 0x6ac22 + index * 2)?;
                        }
                    }
                    0xff => match self.read(rom)? {
                        0 => {
                            // $6AC2A writes $90/$98/$94/$9C (operators 1..4), each
                            // followed by $1F to the matching KS/AR register.
                            for i in 0..4 {
                                let value = self.read(rom)?;
                                let op = &mut self.voice.operators[i];
                                op.ssg = value & 15;
                                op.attack = 0x1f;
                            }
                        }
                        1 => globals.music_paused = self.read(rom)? != 0,
                        2 => globals.multiplier = Some(self.read(rom)?),
                        3 => {
                            globals.fade = Some((self.read(rom)?, self.read(rom)?));
                        }
                        4 => globals.fade = None,
                        sub => {
                            return Err(format!(
                                "unknown extended sound command ${sub:02x} at ${at:06x}"
                            ));
                        }
                    },
                    _ => return Err(format!("unknown sound command ${code:02x} at ${at:06x}")),
                }
            }
            if !complete {
                return Err(format!(
                    "sequence made no timed progress near ${:06x}",
                    self.pc
                ));
            }
        } else {
            if self.gate_left != 0 {
                self.gate_left -= 1;
                if self.gate_left == 0 {
                    self.off();
                }
            }
            self.modulation.tick();
        }
        if self.kind != ChannelKind::Dac && self.keyed {
            self.pitch_envelope_tick(rom)?;
            self.set_pitch(rom)?;
            if self.kind == ChannelKind::Psg {
                self.psg_envelope_tick(rom)?;
            }
            if self.kind == ChannelKind::Fm {
                self.pan_tick(rom, new_event)?;
            }
        }
        Ok(())
    }
    fn pitch_envelope_tick(&mut self, rom: &[u8]) -> Result<()> {
        let id = self.pitch_envelope & 127;
        if id == 0 {
            self.pitch_offset = 0;
            return Ok(());
        }
        if id > 8 {
            return Err(format!("unverified pitch envelope {id}"));
        }
        let base = pointer(rom, 0x6cc48 + usize::from(id - 1) * 4)?;
        for _ in 0..MAX_COMMANDS {
            let at = base + usize::from(self.pitch_cursor);
            let v = byte(rom, at)?;
            self.pitch_cursor = self.pitch_cursor.wrapping_add(1);
            match v {
                0x80 => self.pitch_cursor = 0,
                0x81 => self.pitch_cursor = self.pitch_cursor.wrapping_sub(2),
                0x82 => self.pitch_cursor = byte(rom, at + 1)?,
                0x83 => {
                    self.off();
                    return Ok(());
                }
                0x84 => {
                    self.pitch_scale = self.pitch_scale.wrapping_add(byte(rom, at + 1)? as i8);
                    self.pitch_cursor = self.pitch_cursor.wrapping_add(1);
                }
                _ => {
                    self.pitch_offset = i16::from(v as i8) * i16::from(self.pitch_scale);
                    return Ok(());
                }
            }
        }
        Err("pitch envelope has no timed progress".into())
    }
    fn psg_envelope_tick(&mut self, rom: &[u8]) -> Result<()> {
        if self.psg_envelope == 0 {
            self.envelope_value = 0;
            return Ok(());
        }
        if self.psg_envelope > 10 {
            return Err("unverified PSG envelope index".into());
        }
        let base = pointer(rom, 0x6cd0c + usize::from(self.psg_envelope - 1) * 4)?;
        for _ in 0..MAX_COMMANDS {
            let at = base + usize::from(self.envelope_cursor);
            let v = byte(rom, at)?;
            self.envelope_cursor = self.envelope_cursor.wrapping_add(1);
            match v {
                0x80 => self.envelope_cursor = 0,
                0x81 => {
                    self.envelope_cursor = self.envelope_cursor.wrapping_sub(2);
                    return Ok(());
                }
                0x82 => self.envelope_cursor = byte(rom, at + 1)?,
                0x83 => {
                    self.off();
                    return Ok(());
                }
                _ => {
                    self.envelope_value = v;
                    return Ok(());
                }
            }
        }
        Err("PSG envelope has no timed progress".into())
    }
    fn pan_tick(&mut self, rom: &[u8], new_note: bool) -> Result<()> {
        let [mode, id, _, end, delay] = self.pan_program;
        if mode == 0 || (mode == 1 && !new_note) {
            return Ok(());
        }
        if new_note && mode >= 2 {
            self.pan_cursor = 0;
            self.pan_clock = delay;
        }
        if self.pan_clock == delay {
            if self.pan_cursor > end {
                if mode == 2 {
                    return Ok(());
                }
                self.pan_cursor = 0;
            }
            self.pan_clock = 0;
            self.pan_cursor = self.pan_cursor.wrapping_add(1);
        }
        if !(1..=3).contains(&id) || self.pan_cursor == 0 {
            return Err("invalid panning envelope".into());
        }
        let base = pointer(rom, 0x69c3a + usize::from(id - 1) * 4)?;
        self.pan = byte(rom, base + usize::from(self.pan_cursor - 1))? & 0xc0;
        self.pan_clock = self.pan_clock.wrapping_add(1);
        self.register_pointer[1] = self.pan_cursor;
        Ok(())
    }
    /// One sample at the FM rate (`FM_RATE`), before panning.
    pub(super) fn render(&mut self, clock: &ChipClock) -> [f64; 2] {
        let value = match self.kind {
            ChannelKind::Dac => self.dac.as_mut().map_or(0.0, Dac::render),
            ChannelKind::Fm => {
                f64::from(self.voice.render(self.fm_word, self.volume, clock)) / 8192.0 * 0.22
            }
            ChannelKind::Psg => self.render_psg(PSG_TICK_RATE / FM_RATE),
        };
        if self.kind == ChannelKind::Fm {
            [
                if self.pan & 0x80 != 0 { value } else { 0.0 },
                if self.pan & 0x40 != 0 { value } else { 0.0 },
            ]
        } else {
            [value; 2]
        }
    }
    /// Average PSG output over `ticks` counter clocks. Square edges and noise shifts are
    /// integrated exactly (a box filter at the FM rate), so short periods do not alias.
    fn render_psg(&mut self, ticks: f64) -> f64 {
        if !self.keyed {
            return 0.0;
        }
        let interval = match self.noise.map(|n| n & 3) {
            // Noise shifts every 32/64/128 counter clocks or at twice tone 3's half period.
            Some(0) => 32.0,
            Some(1) => 64.0,
            Some(2) => 128.0,
            Some(_) if self.hz > 0.0 => PSG_TICK_RATE / self.hz,
            None if self.hz > 0.0 => PSG_TICK_RATE / (2.0 * self.hz),
            _ => 1.0,
        }
        .max(1.0);
        let attenuation = (u16::from(self.volume) + u16::from(self.envelope_value)).min(15);
        let amplitude = psg_amplitude()[usize::from(attenuation)];
        if self.psg_countdown <= 0.0 {
            self.psg_countdown = interval;
        }
        let mut left = ticks;
        let mut area = 0.0;
        while self.psg_countdown <= left {
            area += self.psg_sign() * self.psg_countdown;
            left -= self.psg_countdown;
            self.psg_advance();
            self.psg_countdown = interval;
        }
        area += self.psg_sign() * left;
        self.psg_countdown -= left;
        area / ticks * amplitude
    }
    fn psg_sign(&self) -> f64 {
        let high = if self.noise.is_some() {
            self.noise_lfsr & 1 != 0
        } else {
            self.psg_high
        };
        if high { 1.0 } else { -1.0 }
    }
    fn psg_advance(&mut self) {
        if let Some(noise) = self.noise {
            let feedback = if noise & 4 != 0 {
                (self.noise_lfsr ^ (self.noise_lfsr >> 3)) & 1
            } else {
                self.noise_lfsr & 1
            };
            self.noise_lfsr = (self.noise_lfsr >> 1) | (feedback << 15);
        } else {
            self.psg_high = !self.psg_high;
        }
    }
}

/// SN76489 output level per four-bit attenuation (2 dB steps; 15 is off).
fn psg_amplitude() -> &'static [f64; 16] {
    static LEVELS: OnceLock<[f64; 16]> = OnceLock::new();
    LEVELS.get_or_init(|| {
        std::array::from_fn(|attenuation| {
            if attenuation == 15 {
                0.0
            } else {
                0.10 * 10.0_f64.powf(-(attenuation as f64) / 10.0)
            }
        })
    })
}
