use ninja_master_native::audio_output::{AudioConsumer, AudioProducer, audio_ring};
use sdl2::audio::{AudioCallback, AudioDevice, AudioSpecDesired};

pub(super) struct PullAudio(AudioConsumer);

impl AudioCallback for PullAudio {
    type Channel = f32;

    fn callback(&mut self, output: &mut [f32]) {
        self.0.render(output);
    }
}

pub(super) struct AudioOutput {
    // Keep the device alive while the producer feeds its callback.
    pub(super) device: AudioDevice<PullAudio>,
    pub(super) producer: AudioProducer,
    pub(super) demo_remaining: usize,
}

impl AudioOutput {
    pub(super) fn push_tick(
        &mut self,
        samples: &[i16],
        rendered: bool,
        gain: f32,
        floats: &mut Vec<f32>,
    ) {
        floats.clear();
        for frame in samples.as_chunks::<2>().0 {
            let tone = if self.demo_remaining > 0 {
                let elapsed = 2400 - self.demo_remaining;
                let phase = elapsed as f32 * 660.0 * std::f32::consts::TAU / 48_000.0;
                let tone = phase.sin() * self.demo_remaining as f32 / 2400.0 * 0.08;
                self.demo_remaining -= 1;
                tone
            } else {
                0.0
            };
            floats.extend(frame.iter().map(|sample| {
                let value = if rendered {
                    f32::from(*sample) / 32768.0
                } else {
                    0.0
                };
                (value + tone) * gain
            }));
        }
        self.producer.push(floats);
    }
}

pub(super) fn open_audio(sdl: &sdl2::Sdl) -> Result<AudioOutput, String> {
    let audio = sdl.audio()?;
    let mut producer = None;
    let device = audio.open_playback(
        None,
        &AudioSpecDesired {
            freq: Some(48_000),
            channels: Some(2),
            samples: Some(512),
        },
        |spec| {
            let (writer, reader) = audio_ring(spec.freq as u32);
            producer = Some(writer);
            PullAudio(reader)
        },
    )?;
    if device.spec().channels != 2 {
        return Err("audio device must support stereo output".to_string());
    }
    device.resume();
    Ok(AudioOutput {
        device,
        producer: producer.expect("SDL initialized the audio callback"),
        demo_remaining: 0,
    })
}

pub(super) fn track_name(rom: &[u8], command: Option<u8>) -> String {
    let Some(command) = command else {
        return "None".into();
    };
    for index in 0..29 {
        let at = ninja_master_native::sound_events::MUSIC_TABLE + index * 16;
        if rom.get(at) == Some(&command)
            && let Some(name) = rom.get(at + 2..at + 16)
        {
            return format!("{} (${command:02X})", String::from_utf8_lossy(name).trim());
        }
    }
    format!("${command:02X}")
}
