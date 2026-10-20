//! One replacement file, validated and ready to play.
use super::Entry;
use lewton::inside_ogg::OggStreamReader;
use std::{io::Cursor, sync::Arc};

/// Vorbis files up to this long are decoded on load; longer ones stream.
const DECODE_SECONDS: u64 = 15;

pub struct Clip {
    pub(super) data: Data,
    pub rate: u32,
    pub channels: usize,
    /// Length in sample frames.
    pub frames: u64,
    pub volume: f32,
    pub loop_start: Option<u64>,
    pub loop_end: u64,
}

pub(super) enum Data {
    /// Interleaved samples, `channels` per frame.
    Pcm(Arc<[i16]>),
    /// A whole Ogg Vorbis file, decoded while it plays.
    Vorbis(Arc<[u8]>),
}

impl Clip {
    pub(crate) fn load(bytes: Vec<u8>, entry: &Entry) -> Result<Self, String> {
        Self::load_decoding(bytes, entry, DECODE_SECONDS)
    }
    /// `decode_seconds`: the longest Vorbis file decoded up front.
    pub(super) fn load_decoding(
        bytes: Vec<u8>,
        entry: &Entry,
        decode_seconds: u64,
    ) -> Result<Self, String> {
        let (data, rate, channels, frames) = if bytes.starts_with(b"OggS") {
            vorbis(bytes, decode_seconds)?
        } else if bytes.starts_with(b"RIFF") {
            wav(bytes)?
        } else {
            return Err("not an Ogg Vorbis or WAV file".into());
        };
        if !(8_000..=192_000).contains(&rate) {
            return Err(format!("sample rate {rate} Hz is outside 8000..192000"));
        }
        if !(1..=2).contains(&channels) {
            return Err(format!("{channels} channels; use mono or stereo"));
        }
        if frames == 0 {
            return Err("file contains no audio".into());
        }
        let loop_end = entry.loop_end.unwrap_or(frames);
        match entry.loop_start {
            Some(start) if start >= loop_end || loop_end > frames => {
                return Err(format!(
                    "loop {start}..{loop_end} must lie within the file's {frames} frames"
                ));
            }
            None if entry.loop_end.is_some() => return Err("loop_end needs loop_start".into()),
            _ => {}
        }
        Ok(Self {
            data,
            rate,
            channels,
            frames,
            volume: entry.volume,
            loop_start: entry.loop_start,
            loop_end,
        })
    }
}

fn vorbis(bytes: Vec<u8>, decode_seconds: u64) -> Result<(Data, u32, usize, u64), String> {
    let bytes: Arc<[u8]> = bytes.into();
    let mut reader = OggStreamReader::new(Cursor::new(bytes.clone())).map_err(|e| e.to_string())?;
    let rate = reader.ident_hdr.audio_sample_rate;
    let channels = usize::from(reader.ident_hdr.audio_channels);
    // The last page's granule position is the stream's length in frames.
    let frames = bytes
        .windows(14)
        .rposition(|w| w.starts_with(b"OggS"))
        .map(|at| u64::from_le_bytes(bytes[at + 6..at + 14].try_into().unwrap()))
        .ok_or("no Ogg pages")?;
    if frames > u64::from(rate) * decode_seconds || !(1..=2).contains(&channels) {
        return Ok((Data::Vorbis(bytes), rate, channels, frames));
    }
    let mut pcm = Vec::new();
    while let Some(packet) = reader.read_dec_packet_itl().map_err(|e| e.to_string())? {
        pcm.extend(packet);
    }
    // lewton trims the final packet only across pages; a one-page stream
    // keeps the encoder's padding past its granule length.
    pcm.truncate(
        usize::try_from(frames)
            .unwrap_or(usize::MAX)
            .saturating_mul(channels),
    );
    let frames = (pcm.len() / channels) as u64;
    Ok((Data::Pcm(pcm.into()), rate, channels, frames))
}

fn wav(bytes: Vec<u8>) -> Result<(Data, u32, usize, u64), String> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    let channels = usize::from(spec.channels);
    let pcm: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Int if spec.bits_per_sample <= 16 => {
            let shift = 16 - spec.bits_per_sample;
            reader
                .samples::<i16>()
                .map(|s| s.map(|s| s << shift))
                .collect::<Result<_, _>>()
        }
        hound::SampleFormat::Int => {
            let shift = spec.bits_per_sample - 16;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| (s >> shift) as i16))
                .collect::<Result<_, _>>()
        }
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| s.map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16))
            .collect::<Result<_, _>>(),
    }
    .map_err(|e| e.to_string())?;
    let frames = (pcm.len() / channels.max(1)) as u64;
    Ok((Data::Pcm(pcm.into()), spec.sample_rate, channels, frames))
}
