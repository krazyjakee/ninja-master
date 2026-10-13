//! Sine displacement shared by stage rows, boss planes and screen waves.
//! Callers select the affected lines/columns and advance their own phases.
use crate::rom::Sine;

#[derive(Clone, Copy)]
enum Scale {
    Shift(u32),
    Product(i16),
}

/// A spatial wave indexed in ROM sine-table byte offsets. One sample can
/// represent a scanline, an eight-line band or a two-cell column.
#[derive(Clone, Copy)]
pub(crate) struct SineWave {
    phase: u16,
    step: u16,
    scale: Scale,
}

impl SineWave {
    /// Fixed displacement: shift 12 gives the ±4 px 3-2/Hydra wobble.
    pub(crate) fn shifted(phase: u16, step: u16, shift: u32) -> Self {
        Self {
            phase,
            step,
            scale: Scale::Shift(shift),
        }
    }

    /// Boss2 and the final boss introduction use signed `muls.w`, `swap`,
    /// `asr.w #6`; amplitude `$7F00` gives a ±127 px wave.
    pub(crate) fn scaled(phase: u16, step: u16, amplitude: u16) -> Self {
        Self {
            phase,
            step,
            scale: Scale::Product(amplitude as i16),
        }
    }

    pub(crate) fn offset(self, sample: usize, sine: &Sine) -> i32 {
        let angle = self
            .phase
            .wrapping_add((sample as u16).wrapping_mul(self.step));
        let value = i32::from(sine.word(angle));
        match self.scale {
            Scale::Shift(shift) => value >> shift,
            Scale::Product(amplitude) => {
                let product = value * i32::from(amplitude);
                i32::from(((product >> 16) as i16) >> 6)
            }
        }
    }

    /// Presentation sampling keeps fractional pixels and interpolates the
    /// ROM table, including its wrap. Simulation uses the integer offset.
    pub(crate) fn smooth_offset(self, sample: f32, sine: &Sine) -> f32 {
        let angle = (f32::from(self.phase) + sample * f32::from(self.step)).rem_euclid(2048.0);
        let index = (angle / 2.0).floor() as u16 * 2;
        let fraction = (angle - f32::from(index)) / 2.0;
        let a = f32::from(sine.word(index));
        let b = f32::from(sine.word(index.wrapping_add(2)));
        let value = a + (b - a) * fraction;
        match self.scale {
            Scale::Shift(shift) => value / (1u32 << shift) as f32,
            Scale::Product(amplitude) => value * f32::from(amplitude) / 4_194_304.0,
        }
    }
}
