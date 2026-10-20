//! A playing clip, resampled to the output rate.
use super::clip::{Clip, Data};
use lewton::inside_ogg::OggStreamReader;
use std::{io::Cursor, sync::Arc};

pub(crate) struct Voice {
    clip: Arc<Clip>,
    reader: Reader,
    /// Source frame `reader` returns next.
    position: u64,
    /// Streamed frames from the loop start, kept on the first pass so later
    /// passes need no Vorbis seek.
    recording: Vec<[i16; 2]>,
    replay: Option<(Arc<[[i16; 2]]>, usize)>,
    /// Source frames per output frame.
    step: f64,
    fraction: f64,
    previous: [f32; 2],
    next: Option<[f32; 2]>,
}

enum Reader {
    Pcm(Arc<[i16]>),
    Vorbis {
        decoder: Box<OggStreamReader<Cursor<Arc<[u8]>>>>,
        packet: Vec<i16>,
        at: usize,
    },
    Ended,
}

impl Reader {
    fn frame(&mut self, position: u64, channels: usize) -> Option<[i16; 2]> {
        let samples = match self {
            Self::Pcm(pcm) => {
                let at = usize::try_from(position).ok()? * channels;
                pcm.get(at..at + channels)?
            }
            Self::Vorbis {
                decoder,
                packet,
                at,
            } => {
                while *at + channels > packet.len() {
                    // A damaged stream ends the clip instead of stopping the game.
                    match decoder.read_dec_packet_itl() {
                        Ok(Some(next)) => (*packet, *at) = (next, 0),
                        _ => {
                            *self = Self::Ended;
                            return None;
                        }
                    }
                }
                *at += channels;
                &packet[*at - channels..*at]
            }
            Self::Ended => return None,
        };
        Some([samples[0], samples[channels - 1]])
    }
}

impl Voice {
    pub(crate) fn new(clip: Arc<Clip>, sample_rate: u32) -> Self {
        let reader = match &clip.data {
            Data::Pcm(pcm) => Reader::Pcm(pcm.clone()),
            Data::Vorbis(bytes) => {
                OggStreamReader::new(Cursor::new(bytes.clone())).map_or(Reader::Ended, |decoder| {
                    Reader::Vorbis {
                        decoder: Box::new(decoder),
                        packet: Vec::new(),
                        at: 0,
                    }
                })
            }
        };
        let mut voice = Self {
            step: f64::from(clip.rate) / f64::from(sample_rate),
            clip,
            reader,
            position: 0,
            recording: Vec::new(),
            replay: None,
            fraction: 0.0,
            previous: [0.0; 2],
            next: None,
        };
        let first = voice.source();
        voice.previous = first.unwrap_or_default();
        // A one-frame clip holds its frame for one source period.
        voice.next = voice.source().or(first);
        voice
    }

    pub(crate) fn finished(&self) -> bool {
        self.next.is_none()
    }

    /// The next output frame at full scale 1.0, scaled by the entry volume.
    pub(crate) fn sample(&mut self) -> Option<[f32; 2]> {
        let next = self.next?;
        let t = self.fraction as f32;
        let volume = self.clip.volume;
        let out = [0, 1].map(|i| (self.previous[i] + (next[i] - self.previous[i]) * t) * volume);
        self.fraction += self.step;
        while self.fraction >= 1.0 && self.next.is_some() {
            self.fraction -= 1.0;
            self.previous = self.next.unwrap();
            self.next = self.source();
        }
        Some(out)
    }

    fn source(&mut self) -> Option<[f32; 2]> {
        let frame = self.source_frame()?;
        Some(frame.map(|s| f32::from(s) / 32768.0))
    }

    fn source_frame(&mut self) -> Option<[i16; 2]> {
        if let Some((frames, at)) = &mut self.replay {
            let frame = frames[*at];
            *at = (*at + 1) % frames.len();
            return Some(frame);
        }
        let start = self.clip.loop_start;
        if let Some(start) = start
            && self.position >= self.clip.loop_end
        {
            return self.restart_loop(start);
        }
        let frame = (self.position < self.clip.frames)
            .then(|| self.reader.frame(self.position, self.clip.channels))
            .flatten();
        let Some(frame) = frame else {
            // A stream shorter than its granule position loops at its end.
            return start.and_then(|start| self.restart_loop(start));
        };
        if matches!(self.reader, Reader::Vorbis { .. })
            && start.is_some_and(|start| self.position >= start)
        {
            self.recording.push(frame);
        }
        self.position += 1;
        Some(frame)
    }

    fn restart_loop(&mut self, start: u64) -> Option<[i16; 2]> {
        if matches!(self.reader, Reader::Pcm(_)) {
            self.position = start;
        } else {
            if self.recording.is_empty() {
                return None;
            }
            self.replay = Some((std::mem::take(&mut self.recording).into(), 0));
        }
        self.source_frame()
    }
}
