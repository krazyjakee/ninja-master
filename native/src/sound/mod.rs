//! ROM sequence playback with a portable YM2612/SN76489-style FM, PSG and DAC renderer.
//! Sequence timing follows the cartridge driver. FM runs at the chip's own 53267 Hz rate
//! with YM2612 phase, detune, envelope, feedback, SSG-EG and LFO semantics; PSG edges are
//! integrated exactly over each FM sample, and the mix is low-passed and resampled to the
//! host rate. It is not a cycle-accurate chip emulator.
use crate::assets::SoundAssets;
use crate::rom::{Result, byte, pointer, word};
use dac::{Dac, Samples};
use std::collections::VecDeque;
use std::f64::consts::TAU;
use track::Track;
use ym2612::Chip;

mod dac;

mod track;
mod ym2612;

pub const FOREST_MUSIC: u8 = 0x93;
const FM_CLOCK: f64 = 7_670_454.0;
const PSG_CLOCK: f64 = 3_579_545.0;
/// YM2612 output rate: one sample per 144 master clocks (about 53267 Hz).
const FM_RATE: f64 = FM_CLOCK / 144.0;
/// `$6A20E`: `$FC` fades over `$28` steps, with `$6A208`'s three idle
/// updates between them.
const FADE_OUT_STEPS: u8 = 0x28;
const FADE_OUT_DELAY: u8 = 3;
/// SN76489 counter clock: the PSG clock divided by 16 (about 223722 Hz).
const PSG_TICK_RATE: f64 = PSG_CLOCK / 16.0;
/// Driver command priorities, indexed by `(command - $81) & $7F` ($69D04).
const PRIORITY_TABLE: usize = 0x6ceac;
/// `Sound_Queue` (`$FFFC00`): the game's 32-byte request ring.
const REQUEST_RING: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChannelKind {
    Dac,
    Fm,
    Psg,
}

/// Which chip `SoundPlayer::render` mixes. Every track is still rendered, so the stems of
/// one command sum to its full mix (up to rounding). `Fm` includes the DAC (YM2612 channel
/// 6), matching the Genesis Plus GX FM/PSG pre-amplifier split used for emulator stems.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stem {
    #[default]
    Mix,
    Fm,
    Psg,
}
impl Stem {
    fn gain(self, kind: ChannelKind) -> f64 {
        match (self, kind) {
            (Stem::Mix, _) | (Stem::Fm, ChannelKind::Fm | ChannelKind::Dac) => 1.0,
            (Stem::Psg, ChannelKind::Psg) => 1.0,
            _ => 0.0,
        }
    }
}

/// Render one driver command in isolation, the way the cartridge's sound test plays it
/// (`Options_Music` / `Options_SE` after `Options_Init` has stopped everything): a fresh
/// player, the command started before the first driver update, then `ticks` 60 Hz updates
/// of interleaved stereo at `sample_rate`. Accepts music `$81..$9C` and SFX `$9D..$D8`.
pub fn render_cue(
    rom: &[u8],
    command: u8,
    ticks: u32,
    sample_rate: u32,
    stem: Stem,
) -> Result<Vec<i16>> {
    let mut player = SoundPlayer::new(rom, sample_rate)?;
    player.set_stem(stem);
    match command {
        0x81..=0x9c => player.play_music(command)?,
        0x9d..=0xd8 => player.play_sfx(command)?,
        _ => return Err(format!("${command:02x} is not a music or SFX command")),
    }
    let frames = u64::from(sample_rate) * u64::from(ticks) / 60;
    let mut pcm = vec![0i16; usize::try_from(frames * 2).map_err(|e| e.to_string())?];
    player.render(&mut pcm)?;
    Ok(pcm)
}

/// A 16-bit stereo PCM WAV file holding interleaved `pcm` at `sample_rate`.
pub fn wav_bytes(pcm: &[i16], sample_rate: u32) -> Result<Vec<u8>> {
    if !pcm.len().is_multiple_of(2) {
        return Err("PCM buffer must contain complete stereo frames".into());
    }
    let length = u32::try_from(pcm.len())
        .ok()
        .and_then(|n| n.checked_mul(2))
        .filter(|n| *n <= u32::MAX - 36)
        .ok_or("WAV exceeds RIFF size limit")?;
    let mut bytes = Vec::with_capacity(44 + pcm.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(length + 36).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend(pcm.iter().flat_map(|s| s.to_le_bytes()));
    Ok(bytes)
}

/// Stereo biquad low-pass (transposed direct form II) applied at the FM rate before resampling.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: [f64; 2],
    z2: [f64; 2],
}
impl Biquad {
    fn low_pass(cutoff: f64, rate: f64, q: f64) -> Self {
        let (sin, cos) = (TAU * cutoff / rate).sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: (1.0 - cos) / 2.0 / a0,
            b1: (1.0 - cos) / a0,
            b2: (1.0 - cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            ..Self::default()
        }
    }
    fn process(&mut self, input: [f64; 2]) -> [f64; 2] {
        let mut output = [0.0; 2];
        for i in 0..2 {
            let y = self.b0 * input[i] + self.z1[i];
            self.z1[i] = self.b1 * input[i] - self.a1 * y + self.z2[i];
            self.z2[i] = self.b2 * input[i] - self.a2 * y;
            output[i] = y;
        }
        output
    }
}

/// Driver-wide state that sequence commands change.
#[derive(Default)]
struct Globals {
    tempo: u8,
    tempo_clock: u8,
    communication: u8,
    music_paused: bool,
    /// `sndQueue` (`$FFF80A`, `Sound_Mailbox`): the command the next update
    /// examines. Sequence command `$EB` writes it directly.
    mailbox: Option<u8>,
    multiplier: Option<u8>,
    fade: Option<(u8, u8)>,
    /// Command `$FC` (`SndDrv_FadeOutMusic`, `$6A200`): steps left
    /// (`sndFadeOutSteps`) and driver updates until the next one
    /// (`sndFadeOutDelay`). Starting music clears both (`$6A352`).
    fade_out: (u8, u8),
    /// YM2612 register $22: LFO enable (bit 3) and frequency (bits 0-2).
    lfo: u8,
}

/// Owned ROM and playback state; safe to move into an audio producer thread.
/// `render` accepts interleaved stereo and keeps a fractional 60 Hz clock across calls.
pub struct SoundPlayer {
    rom: Vec<u8>,
    samples: Samples,
    sample_rate: u32,
    sample_clock: u64,
    music: Vec<Track>,
    music_command: Option<u8>,
    effects: Vec<Track>,
    globals: Globals,
    /// Game requests not yet moved to the mailbox (`play_command`).
    requests: VecDeque<u8>,
    /// Latched driver priority (`$FFF800`); zero when no SFX holds the channels.
    sfx_priority: u8,
    chip: Chip,
    /// Fraction of an FM sample between `history[1]` and `history[2]`.
    resample_position: f64,
    history: [[f64; 2]; 4],
    filters: [Biquad; 2],
    stem: Stem,
    /// Native: the music's share of the mix (`set_music_gain`), ramped
    /// toward `music_gain_target` to avoid clicks.
    music_gain: f64,
    music_gain_target: f64,
}
/// Seconds a music gain change takes to complete.
const MUSIC_GAIN_RAMP: f64 = 0.08;
impl SoundPlayer {
    /// The currently active music selection, including front-end screens.
    pub fn current_music(&self) -> Option<u8> {
        self.music
            .iter()
            .any(|track| track.active)
            .then_some(self.music_command)
            .flatten()
    }

    pub fn new(rom: &[u8], sample_rate: u32) -> Result<Self> {
        SoundAssets::load(rom)?;
        if !(8_000..=192_000).contains(&sample_rate) {
            return Err("sound sample rate must be 8000..192000 Hz".into());
        }
        Ok(Self {
            rom: rom.to_vec(),
            samples: Samples::load(rom)?,
            sample_rate,
            sample_clock: 0,
            music: Vec::new(),
            music_command: None,
            effects: Vec::new(),
            globals: Globals::default(),
            requests: VecDeque::new(),
            sfx_priority: 0,
            chip: Chip::default(),
            resample_position: 0.0,
            history: [[0.0; 2]; 4],
            // Fourth-order Butterworth below both the host and FM Nyquist limits.
            filters: {
                let cutoff = 0.45 * f64::from(sample_rate).min(FM_RATE);
                [
                    Biquad::low_pass(cutoff, FM_RATE, 0.541_196_1),
                    Biquad::low_pass(cutoff, FM_RATE, 1.306_563),
                ]
            },
            stem: Stem::Mix,
            music_gain: 1.0,
            music_gain_target: 1.0,
        })
    }
    /// Native: scale the music tracks (not the effects), for example while
    /// the pause window is open. The change ramps in over 80 ms.
    pub fn set_music_gain(&mut self, gain: f64) {
        self.music_gain_target = gain.clamp(0.0, 1.0);
    }
    /// Consume the game communication byte written by sequence command `$E2`
    /// (`$FFF803` in the cartridge). Game logic sees it after this driver
    /// update, as it would on the following main-loop frame.
    pub fn take_communication(&mut self) -> u8 {
        std::mem::take(&mut self.globals.communication)
    }
    /// Mix only one chip's tracks (for FM/PSG balance measurements); `Stem::Mix` is the default.
    fn set_stem(&mut self, stem: Stem) {
        self.stem = stem;
    }
    /// Request any driver command the way game code does (`$4B80` appends it
    /// to `Sound_Queue`): music, effects, voices `$D9..$FB`, fade `$FC`, stop
    /// effects `$FD/$FE` and stop `$FF`. `Sound_ConsumeQueue` (`$4BA6`) hands
    /// one request to each driver update, in order, so several made in one
    /// frame start on consecutive updates.
    pub fn play_command(&mut self, command: u8) {
        // Native: past a full ring the oldest request is dropped.
        if self.requests.len() >= REQUEST_RING {
            self.requests.pop_front();
        }
        self.requests.push_back(command);
    }
    pub fn play_music(&mut self, command: u8) -> Result<()> {
        if !(0x81..=0x9c).contains(&command) {
            return Err(format!("invalid music command ${command:02x}"));
        }
        let header = pointer(&self.rom, 0x6ce3c + usize::from(command - 0x81) * 4)?;
        let instruments = header + usize::from(word(&self.rom, header)?);
        let fm_count = byte(&self.rom, header + 2)?;
        let psg_count = byte(&self.rom, header + 3)?;
        if fm_count > 7 || psg_count > 3 {
            return Err("invalid music channel count".into());
        }
        let multiplier = byte(&self.rom, header + 4)?;
        let mut tracks = Vec::new();
        let mut cursor = header + 6;
        for i in 0..fm_count + psg_count {
            let psg = i >= fm_count;
            let kind = if psg {
                ChannelKind::Psg
            } else if i == 0 {
                ChannelKind::Dac
            } else {
                ChannelKind::Fm
            };
            let hardware = byte(
                &self.rom,
                if psg {
                    0x69f3c + usize::from(i - fm_count)
                } else {
                    0x69f34 + usize::from(i)
                },
            )?;
            let mut track = Track::new(
                kind,
                hardware,
                header + usize::from(word(&self.rom, cursor)?),
                instruments,
                multiplier,
                byte(&self.rom, cursor + 2)?,
                byte(&self.rom, cursor + 3)?,
            );
            if psg {
                track.pitch_envelope = byte(&self.rom, cursor + 4)?;
                track.psg_envelope = byte(&self.rom, cursor + 5)?;
            }
            tracks.push(track);
            cursor += if psg { 6 } else { 4 };
        }
        self.music = tracks;
        self.music_command = Some(command);
        // $69DC8 stops SFX through $6A0F2, which also clears the priority latch.
        self.effects.clear();
        self.sfx_priority = 0;
        self.globals = Globals::default();
        self.globals.tempo = byte(&self.rom, header + 5)?;
        self.globals.tempo_clock = self.globals.tempo;
        self.sample_clock = 0;
        Ok(())
    }
    pub fn play_sfx(&mut self, command: u8) -> Result<()> {
        if !(0x9d..=0xd8).contains(&command) {
            return Err(format!("invalid SFX command ${command:02x}"));
        }
        let header = pointer(&self.rom, 0x6cf2c + usize::from(command - 0x9d) * 4)?;
        let instruments = header + usize::from(word(&self.rom, header)?);
        let multiplier = byte(&self.rom, header + 2)?;
        let count = byte(&self.rom, header + 3)?;
        if count > 6 {
            return Err("invalid SFX channel count".into());
        }
        let mut tracks = Vec::new();
        for i in 0..count {
            let cursor = header + 4 + usize::from(i) * 6;
            let hardware = byte(&self.rom, cursor + 1)?;
            let kind = if hardware & 128 != 0 {
                ChannelKind::Psg
            } else {
                ChannelKind::Fm
            };
            let mut track = Track::new(
                kind,
                hardware,
                header + usize::from(word(&self.rom, cursor + 2)?),
                instruments,
                multiplier,
                byte(&self.rom, cursor + 4)?,
                byte(&self.rom, cursor + 5)?,
            );
            track.active = byte(&self.rom, cursor)? & 128 != 0;
            // Native: lower the 2-2 WARNING alarm by 30%.
            if command == 0xa5 {
                track.mix_gain = 0.7;
            }
            // $69FAE stores the SFX voice-table pointer in track bytes 32..35.
            track.register_pointer = (instruments as u32).to_be_bytes();
            tracks.push(track);
        }
        if !self.admit(command)? {
            return Ok(());
        }
        for track in tracks {
            self.effects.retain(|old| old.slot() != track.slot());
            self.effects.push(track);
        }
        Ok(())
    }
    /// `$69D04`: a command is accepted when its `$6CEAC` priority is not below the latched
    /// priority. Priorities below `$80` are latched until an SFX track stops (`$6AAC8`) or
    /// SFX are cleared (`$6A0F2`); `$80` (music, control and some effects) always passes
    /// without latching. The game-side table `$5B56` carries no priority (its second byte is 0).
    fn admit(&mut self, command: u8) -> Result<bool> {
        let priority = byte(
            &self.rom,
            PRIORITY_TABLE + usize::from(command.wrapping_sub(0x81) & 0x7f),
        )?;
        if priority < self.sfx_priority {
            return Ok(false);
        }
        if priority < 0x80 {
            self.sfx_priority = priority;
        }
        Ok(true)
    }
    /// Command `$FC` (`SndDrv_FadeOutMusic`, `$6A200`): stop FM/PSG effects
    /// and the music DAC track, then fade music over 40 steps. The external
    /// Z80 voice continues independently (Karura requests `$E4`, then `$FC`).
    fn fade_out_music(&mut self) {
        self.effects.retain(|track| track.kind == ChannelKind::Dac);
        self.sfx_priority = 0;
        for track in &mut self.music {
            if track.kind == ChannelKind::Dac {
                // clr.b sndMusicDAC(a6): the drum track stops reading events.
                track.active = false;
            }
        }
        self.globals.fade_out = (FADE_OUT_STEPS, FADE_OUT_DELAY);
    }
    /// `SndDrv_UpdateFadeOut` (`$6A21A`), before the tracks update: every
    /// fourth update raises each running FM and PSG music track's
    /// attenuation by one, stopping an FM track that passes `$7F` and a PSG
    /// track that reaches `$10`. The last step stops everything
    /// (`SndDrv_StopAll`).
    fn update_fade_out(&mut self) {
        let (steps, delay) = self.globals.fade_out;
        if steps == 0 {
            return;
        }
        if delay > 0 {
            self.globals.fade_out.1 = delay - 1;
            return;
        }
        if steps == 1 {
            self.stop();
            return;
        }
        self.globals.fade_out = (steps - 1, FADE_OUT_DELAY);
        for track in &mut self.music {
            if !track.active || track.kind == ChannelKind::Dac {
                continue;
            }
            track.volume = track.volume.wrapping_add(1);
            let limit = if track.kind == ChannelKind::Psg {
                track.volume >= 0x10
            } else {
                track.volume >= 0x80
            };
            if limit {
                track.active = false;
                track.off();
            }
        }
    }
    pub fn stop(&mut self) {
        self.music_command = None;
        self.music.clear();
        self.effects.clear();
        self.sfx_priority = 0;
        self.globals = Globals::default();
    }
    fn tick(&mut self) -> Result<()> {
        let mut skip_sequence_clock = false;
        if self.globals.tempo != 0 {
            self.globals.tempo_clock = self.globals.tempo_clock.wrapping_sub(1);
            if self.globals.tempo_clock == 0 {
                self.globals.tempo_clock = self.globals.tempo;
                skip_sequence_clock = true;
            }
        }
        self.update_fade_out();
        // Sound_Update ($4A08): Sound_ConsumeQueue moves one request into the
        // mailbox (over any `$EB` command), then SndDrv_Update ($697A0) runs
        // it before the tracks. Starting music returns past the rest.
        if let Some(command) = self.requests.pop_front() {
            self.globals.mailbox = Some(command);
        }
        if let Some(command) = self.globals.mailbox.take()
            && self.process_command(command)?
        {
            return Ok(());
        }
        if !self.globals.music_paused {
            for track in &mut self.music {
                // $6A288 compensates the duration counter only; envelopes keep running.
                if skip_sequence_clock {
                    track.remaining = (track.remaining % 256) + 1;
                }
                track.tick(&self.rom, &self.samples, &mut self.globals)?;
            }
        }
        for track in &mut self.effects {
            track.tick(&self.rom, &self.samples, &mut self.globals)?;
            if std::mem::take(&mut track.stopped) {
                self.sfx_priority = 0;
            }
        }
        if let Some(multiplier) = self.globals.multiplier.take() {
            for track in &mut self.music {
                track.multiplier = multiplier;
            }
        }
        if let Some((fm, psg)) = self.globals.fade.take() {
            for track in &mut self.music {
                track.volume = track
                    .volume
                    .saturating_add(if track.kind == ChannelKind::Psg {
                        psg
                    } else {
                        fm
                    });
            }
        }
        Ok(())
    }
    /// `SndDrv_ProcessCommand` (`$69D3E`) for one mailbox command; true when
    /// it started music.
    fn process_command(&mut self, command: u8) -> Result<bool> {
        match command {
            0x81..=0x9c => {
                self.play_music(command)?;
                return Ok(true);
            }
            0x9d..=0xd8 => self.play_sfx(command)?,
            0xd9..=0xfb => {
                if !self.admit(command)? {
                    return Ok(false);
                }
                let sample = Dac::load_raw(&self.samples, command.wrapping_add(0xae), None)?;
                let mut track = Track::new(ChannelKind::Dac, 6, 0, 0, 1, 0, 0);
                track.active = false;
                track.dac = Some(sample);
                self.effects.retain(|old| old.kind != ChannelKind::Dac);
                self.effects.push(track);
            }
            0xfc => self.fade_out_music(),
            0xfd | 0xfe => {
                self.effects.clear();
                self.sfx_priority = 0;
            }
            0xff => self.stop(),
            _ => {}
        }
        Ok(false)
    }
    /// Fill exactly `out.len()/2` stereo frames. Odd buffers are rejected before advancing time.
    /// On a decode error the unrendered suffix is silenced and the error is returned to the caller.
    pub fn render(&mut self, out: &mut [i16]) -> Result<()> {
        if !out.len().is_multiple_of(2) {
            return Err("PCM buffer must contain complete stereo frames".into());
        }
        out.fill(0);
        let step = FM_RATE / f64::from(self.sample_rate);
        for frame in out.as_chunks_mut::<2>().0 {
            if self.sample_clock < 60 {
                self.tick()?;
            }
            self.resample_position += step;
            while self.resample_position >= 1.0 {
                self.resample_position -= 1.0;
                let sample = self.chip_sample();
                self.history.rotate_left(1);
                self.history[3] = sample;
            }
            // Cubic Hermite (Catmull-Rom) interpolation between history[1] and history[2].
            let t = self.resample_position;
            for (i, slot) in frame.iter_mut().enumerate() {
                let [p0, p1, p2, p3] = self.history.map(|s| s[i]);
                let value = p1
                    + 0.5
                        * t
                        * (p2 - p0
                            + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3
                                + t * (3.0 * (p1 - p2) + p3 - p0)));
                *slot = (value.clamp(-1.0, 1.0) * 28_000.0).round() as i16;
            }
            self.sample_clock += 60;
            if self.sample_clock >= u64::from(self.sample_rate) {
                self.sample_clock -= u64::from(self.sample_rate);
            }
        }
        Ok(())
    }
    /// Mix one FM-rate (53267 Hz) stereo sample and low-pass it for resampling.
    fn chip_sample(&mut self) -> [f64; 2] {
        let clock = self.chip.clock(self.globals.lfo);
        let mut mixed = [0.0; 2];
        let ramp = 1.0 / (FM_RATE * MUSIC_GAIN_RAMP);
        self.music_gain += (self.music_gain_target - self.music_gain).clamp(-ramp, ramp);
        let music_gain = self.music_gain;
        for track in &mut self.music {
            let value = track.render(&clock);
            let overridden = self.effects.iter().any(|effect| {
                (effect.active || effect.dac.as_ref().is_some_and(Dac::playing))
                    && effect.slot() == track.slot()
            });
            if !overridden && !self.globals.music_paused {
                let gain = self.stem.gain(track.kind) * music_gain;
                mixed[0] += value[0] * gain;
                mixed[1] += value[1] * gain;
            }
        }
        for track in &mut self.effects {
            let value = track.render(&clock);
            let gain = self.stem.gain(track.kind) * track.mix_gain;
            mixed[0] += value[0] * gain;
            mixed[1] += value[1] * gain;
        }
        for filter in &mut self.filters {
            mixed = filter.process(mixed);
        }
        mixed
    }
}
