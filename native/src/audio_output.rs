//! SDL-independent, single-producer/single-consumer stereo output.
//!
//! A callback cannot manufacture future game audio during a producer stall.
//! We therefore keep 117 ms of PCM (initially silence), explicitly trading
//! that much sound latency for surviving a 90 ms compositor stall. Capacity
//! alone would not protect an almost-empty ring. Catch-up/fast-forward output
//! is dropped above this reserve plus three ticks, bounding additional delay.
use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicUsize, Ordering},
};

const SAMPLE_RATE: usize = 48_000;
const TICK_FRAMES: usize = SAMPLE_RATE / 60;
const RESERVE: usize = TICK_FRAMES * 7;
const BACKLOG: usize = RESERVE + TICK_FRAMES * 3;
const CAPACITY: usize = BACKLOG + TICK_FRAMES * 4;

struct Shared {
    frames: Box<[[AtomicU32; 2]]>,
    read: AtomicUsize,
    write: AtomicUsize,
    underruns: AtomicUsize,
}

/// Only this endpoint writes samples; it must not be cloned.
pub struct AudioProducer {
    shared: Arc<Shared>,
}

/// Only this endpoint reads samples. Resampling runs on the audio thread,
/// without locks, allocation, or access to simulation state.
pub struct AudioConsumer {
    shared: Arc<Shared>,
    position: f64,
    integral: f64,
    output_rate: f64,
}

pub fn audio_ring(output_rate: u32) -> (AudioProducer, AudioConsumer) {
    assert!(output_rate > 0);
    let shared = Arc::new(Shared {
        frames: (0..CAPACITY)
            .map(|_| [AtomicU32::new(0), AtomicU32::new(0)])
            .collect(),
        read: AtomicUsize::new(0),
        write: AtomicUsize::new(RESERVE),
        underruns: AtomicUsize::new(0),
    });
    (
        AudioProducer {
            shared: shared.clone(),
        },
        AudioConsumer {
            shared,
            position: 0.0,
            integral: 0.0,
            output_rate: f64::from(output_rate),
        },
    )
}

impl AudioProducer {
    pub fn queued_frames(&self) -> usize {
        let read = self.shared.read.load(Ordering::Acquire);
        self.shared.write.load(Ordering::Relaxed).wrapping_sub(read)
    }

    /// Drop a complete tick above the backlog; never publish partial stereo
    /// frames. The sound driver still advances when output is dropped.
    pub fn push(&mut self, samples: &[f32]) -> bool {
        assert!(samples.len().is_multiple_of(2));
        let count = samples.len() / 2;
        let queued = self.queued_frames();
        if queued >= BACKLOG || count > CAPACITY - queued {
            return false;
        }
        let write = self.shared.write.load(Ordering::Relaxed);
        for (offset, sample) in samples.as_chunks::<2>().0.iter().enumerate() {
            let frame = &self.shared.frames[write.wrapping_add(offset) % CAPACITY];
            frame[0].store(sample[0].to_bits(), Ordering::Relaxed);
            frame[1].store(sample[1].to_bits(), Ordering::Relaxed);
        }
        self.shared
            .write
            .store(write.wrapping_add(count), Ordering::Release);
        true
    }

    pub fn underrun_frames(&self) -> usize {
        self.shared.underruns.load(Ordering::Relaxed)
    }
}

impl AudioConsumer {
    fn rate(&mut self, queued: usize, frames: usize) -> f64 {
        let error = (queued as f64 - RESERVE as f64) / RESERVE as f64;
        // Integral feedback removes persistent display/device clock error;
        // proportional feedback recovers the reserve after an isolated stall.
        self.integral = (self.integral + error * frames as f64 / self.output_rate * 0.0005)
            .clamp(-0.005, 0.005);
        let correction = (error * 0.01 + self.integral).clamp(-0.005, 0.005);
        SAMPLE_RATE as f64 / self.output_rate * (1.0 + correction)
    }

    pub fn render(&mut self, output: &mut [f32]) {
        assert!(output.len().is_multiple_of(2));
        let mut read = self.shared.read.load(Ordering::Relaxed);
        let write = self.shared.write.load(Ordering::Acquire);
        let step = self.rate(write.wrapping_sub(read), output.len() / 2);
        let mut missing = 0;
        for sample in output.as_chunks_mut::<2>().0 {
            let advance = (self.position + step).floor() as usize;
            // Keep the second interpolation frame, even for non-48k devices.
            if write.wrapping_sub(read) < advance.max(1) + 1 {
                sample.fill(0.0);
                missing += 1;
                continue;
            }
            let a = &self.shared.frames[read % CAPACITY];
            let b = &self.shared.frames[read.wrapping_add(1) % CAPACITY];
            for channel in 0..2 {
                let a = f32::from_bits(a[channel].load(Ordering::Relaxed));
                let b = f32::from_bits(b[channel].load(Ordering::Relaxed));
                sample[channel] = a + (b - a) * self.position as f32;
            }
            self.position += step;
            self.position -= advance as f64;
            read = read.wrapping_add(advance);
        }
        self.shared.read.store(read, Ordering::Release);
        self.shared.underruns.fetch_add(missing, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserve_survives_ninety_millisecond_producer_stall() {
        let (producer, mut consumer) = audio_ring(48_000);
        consumer.render(&mut [0.0; 48_000 * 90 / 1000 * 2]);
        assert_eq!(producer.underrun_frames(), 0);
        assert!(producer.queued_frames() > 0);
    }

    #[test]
    fn overflow_drops_whole_ticks_and_underflow_is_silence() {
        let (mut producer, mut consumer) = audio_ring(48_000);
        let tick = [0.25; TICK_FRAMES * 2];
        for _ in 0..3 {
            assert!(producer.push(&tick));
        }
        assert!(!producer.push(&tick));
        let mut output = vec![1.0; CAPACITY * 4];
        consumer.render(&mut output);
        assert!(producer.underrun_frames() > 0);
        assert_eq!(*output.last().unwrap(), 0.0);
    }

    #[test]
    fn rate_control_is_bounded_and_restores_fill_at_5995_hz() {
        let (mut producer, mut consumer) = audio_ring(48_000);
        let tick = [0.25; TICK_FRAMES * 2];
        let mut output = [0.0; 512 * 2];
        let mut next_tick = 0.0;
        let mut time = 0.0;
        let mut dropped = 0;
        // Ten simulated minutes, with no SDL or wall-clock waits.
        while time < 600.0 {
            while next_tick <= time {
                if !producer.push(&tick) {
                    dropped += 1;
                }
                next_tick += 1.0 / 59.95;
            }
            consumer.render(&mut output);
            time += 512.0 / 48_000.0;
        }
        assert_eq!(producer.underrun_frames(), 0);
        assert_eq!(dropped, 0);
        assert!(producer.queued_frames().abs_diff(RESERVE) < TICK_FRAMES);
        assert!((0.995..=1.005).contains(&consumer.rate(0, 512)));
        assert!((0.995..=1.005).contains(&consumer.rate(CAPACITY, 512)));
        consumer.render(&mut [0.0; 48_000 * 90 / 1000 * 2]);
        assert_eq!(producer.underrun_frames(), 0);
    }

    #[test]
    fn display_catch_up_preserves_audio_through_repeated_stalls() {
        use crate::timing::FixedClock;
        use std::time::Duration;
        let (mut producer, mut consumer) = audio_ring(48_000);
        let mut clock = FixedClock::default();
        let tick = [0.25; TICK_FRAMES * 2];
        let mut dropped = 0;
        for frame in 0..3600 {
            let frames = if frame % 120 == 119 { 4320 } else { 800 };
            consumer.render(&mut vec![0.0; frames * 2]);
            let elapsed = Duration::from_secs_f64(frames as f64 / 48_000.0);
            for _ in 0..clock.advance_display(elapsed) {
                dropped += usize::from(!producer.push(&tick));
            }
        }
        assert_eq!(producer.underrun_frames(), 0);
        assert_eq!(dropped, 0);
        assert!(producer.queued_frames().abs_diff(RESERVE) < TICK_FRAMES * 2);
    }

    #[test]
    fn stereo_samples_survive_ring_wrap_and_device_rate_conversion() {
        let (mut producer, mut consumer) = audio_ring(44_100);
        let tick: Vec<_> = (0..TICK_FRAMES).flat_map(|_| [0.25, -0.5]).collect();
        let mut output = [0.0; 735 * 2];
        for _ in 0..120 {
            producer.push(&tick);
            consumer.render(&mut output);
        }
        for frame in output.as_chunks::<2>().0 {
            assert_eq!(*frame, [0.25, -0.5]);
        }
        assert_eq!(producer.underrun_frames(), 0);
    }
}
