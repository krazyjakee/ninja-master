//! DAC drum and voice samples: the cartridge's differential PCM, decoded once per player.
use super::{FM_RATE, PSG_CLOCK};
use crate::rom::{Result, byte};
use std::sync::Arc;

const Z80: usize = 0x6ad3e;
const SAMPLE_ROOT: usize = 0x78d86;

fn little(rom: &[u8], at: usize) -> Result<usize> {
    Ok(usize::from(u16::from_le_bytes([
        byte(rom, at)?,
        byte(rom, at + 1)?,
    ])))
}

/// The original differential PCM format, including wrapping 8-bit accumulator arithmetic.
pub(super) fn decode_dpcm(data: &[u8]) -> Vec<i16> {
    const DELTA: [u8; 16] = [
        0, 1, 2, 4, 8, 16, 32, 64, 128, 255, 254, 252, 248, 240, 224, 192,
    ];
    let mut accumulator = 0x80u8;
    let mut pcm = Vec::with_capacity(data.len() * 2);
    for &packed in data {
        for nibble in [packed >> 4, packed & 15] {
            accumulator = accumulator.wrapping_add(DELTA[usize::from(nibble)]);
            pcm.push((i16::from(accumulator) - 128) * 256);
        }
    }
    pcm
}

struct Sample {
    pcm: Arc<[i16]>,
    /// Descriptor byte 11: the Z80 playback loop's delay.
    delay: u8,
}

/// Samples `$81..$86` (inside the Z80 driver) and `$88..$9D` (the external
/// bank at `$78D86`), indexed by `id - $81`; `$87` has no descriptor.
#[derive(Default)]
pub(super) struct Samples(Vec<Option<Sample>>);
impl Samples {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        (0x81..=0x9du8)
            .map(|id| {
                if id == 0x87 {
                    return Ok(None);
                }
                let internal = id <= 0x86;
                let descriptor = if internal {
                    Z80 + 0x16b + usize::from(id - 0x81) * 12
                } else {
                    SAMPLE_ROOT + usize::from(id - 0x88) * 12
                };
                let source = (if internal { Z80 } else { SAMPLE_ROOT }) + little(rom, descriptor)?;
                let length = little(rom, descriptor + 2)?;
                if byte(rom, descriptor + 4)? != 0 {
                    return Err(format!(
                        "looped DAC descriptor ${descriptor:06x} needs an explicit loop policy"
                    ));
                }
                let data = rom
                    .get(source..source + length)
                    .ok_or("DAC sample outside ROM")?;
                Ok(Some(Sample {
                    pcm: decode_dpcm(data).into(),
                    delay: byte(rom, descriptor + 11)?,
                }))
            })
            .collect::<Result<_>>()
            .map(Self)
    }
    fn get(&self, id: u8) -> Result<&Sample> {
        self.0
            .get(usize::from(id.wrapping_sub(0x81)))
            .and_then(Option::as_ref)
            .ok_or_else(|| format!("invalid DAC sample ID ${id:02x}"))
    }
}

pub(super) struct Dac {
    pcm: Arc<[i16]>,
    cursor: f64,
    hz: f64,
}
impl Dac {
    pub(super) fn load(rom: &[u8], samples: &Samples, note: u8) -> Result<Self> {
        // $698BC maps every bit-3 sample code to $85 and uses its unmasked delay lookup.
        let (id, delay_override) = if note & 8 != 0 {
            (
                0x85,
                Some(byte(rom, 0x698e8 + usize::from(note.wrapping_sub(0x88)))?),
            )
        } else {
            (note, None)
        };
        Self::load_raw(samples, id, delay_override)
    }
    pub(super) fn load_raw(samples: &Samples, id: u8, delay_override: Option<u8>) -> Result<Self> {
        let sample = samples.get(id)?;
        let delay = delay_override.unwrap_or(sample.delay);
        // Average instruction cost of the two nibble paths, excluding variable YM bus contention.
        let delay_cycles = if delay == 0 { 256.0 } else { f64::from(delay) } * 13.0 - 5.0;
        Ok(Self {
            pcm: Arc::clone(&sample.pcm),
            cursor: 0.0,
            hz: PSG_CLOCK / (204.5 + delay_cycles),
        })
    }
    /// The sample has not played to its end.
    pub(super) fn playing(&self) -> bool {
        self.cursor < self.pcm.len() as f64
    }
    /// One FM-rate sample, linearly interpolated between decoded DAC values.
    pub(super) fn render(&mut self) -> f64 {
        let whole = self.cursor.floor();
        let at = whole as usize;
        let fraction = self.cursor - whole;
        let a = f64::from(self.pcm.get(at).copied().unwrap_or(0));
        let b = f64::from(self.pcm.get(at + 1).copied().unwrap_or(0));
        self.cursor += self.hz / FM_RATE;
        (a + (b - a) * fraction) / 32768.0 * 0.45
    }
}
