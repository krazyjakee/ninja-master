//! Loudness matching: a replacement plays at its original's level, so a
//! mastered track does not drown the synthesized effects (or vice versa).
use super::{Clip, Section, Voice};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

/// Seconds of music measured from its start; effects and voices are
/// measured whole up to this length.
pub(crate) const SECONDS: u32 = 20;
/// Quieter 50 ms blocks (below -60 dBFS) are silence and are not measured.
const GATE: f64 = 1e-6;
/// The furthest a replacement is turned down or up (-24 dB, +12 dB).
const GAIN: std::ops::RangeInclusive<f64> = 0.0625..=4.0;

/// RMS of the audible 50 ms blocks of stereo frames at `rate`, as a fraction
/// of full scale; 0 when everything is silent.
pub(crate) fn gated_rms(frames: impl Iterator<Item = [f32; 2]>, rate: u32) -> f64 {
    let block = (rate / 20).max(1) as usize;
    let (mut sum, mut count, mut blocks, mut loud) = (0.0, 0, 0, 0.0);
    for [left, right] in frames {
        sum += (f64::from(left).powi(2) + f64::from(right).powi(2)) / 2.0;
        count += 1;
        if count == block {
            let mean = sum / block as f64;
            if mean > GATE {
                loud += mean;
                blocks += 1;
            }
            (sum, count) = (0.0, 0);
        }
    }
    if count > 0 && sum / count as f64 > GATE {
        loud += sum / count as f64;
        blocks += 1;
    }
    if blocks == 0 {
        0.0
    } else {
        (loud / f64::from(blocks)).sqrt()
    }
}

impl Clip {
    /// The file's loudness at volume 1, over the window originals use.
    pub(crate) fn level(&self) -> f64 {
        let clip = Clip {
            volume: 1.0,
            notes: Vec::new(),
            ..self.clone()
        };
        let frames = u64::from(clip.rate * SECONDS).min(clip.frames) as usize;
        let mut voice = Voice::new(Arc::new(clip), self.rate);
        gated_rms(
            std::iter::from_fn(|| voice.sample()).take(frames),
            self.rate,
        )
    }
}

/// Originals' loudness by ROM checksum and command, measured once per run.
fn reference(rom: &[u8], command: u8) -> Result<f64, String> {
    static LEVELS: OnceLock<Mutex<HashMap<(u32, u8), f64>>> = OnceLock::new();
    let key = (crate::rom::regions::checksum(rom), command);
    let levels = LEVELS.get_or_init(Default::default);
    if let Some(&level) = levels.lock().unwrap().get(&key) {
        return Ok(level);
    }
    let level = crate::sound::reference_level(rom, command)?;
    levels.lock().unwrap().insert(key, level);
    Ok(level)
}

/// Scale each clip's volume so it plays as loud as the original command it
/// replaces; `volume` from the manifest then adjusts from there. Originals
/// render in parallel.
pub(super) fn match_levels(rom: &[u8], clips: &mut [(u8, Clip)]) {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = clips.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for chunk in clips.chunks_mut(chunk) {
            scope.spawn(move || {
                for (command, clip) in chunk {
                    let level = clip.level();
                    let Ok(original) = reference(rom, *command) else {
                        continue;
                    };
                    if original > 0.0 && level > 0.0 {
                        clip.volume *= (original / level).clamp(*GAIN.start(), *GAIN.end()) as f32;
                    }
                }
            });
        }
    });
}

/// Sections whose files may loop.
pub(super) fn loops(section: Section) -> bool {
    section != Section::Voices
}
