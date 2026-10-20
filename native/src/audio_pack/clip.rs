//! One replacement file, validated and ready to play.
use super::Entry;
use lewton::inside_ogg::OggStreamReader;
use std::{io::Cursor, sync::Arc};

/// Vorbis files up to this long are decoded on load; longer ones stream.
const DECODE_SECONDS: u64 = 15;

#[derive(Clone)]
pub struct Clip {
    pub(super) data: Data,
    pub rate: u32,
    pub channels: usize,
    /// Length in sample frames.
    pub frames: u64,
    pub volume: f32,
    pub loop_start: Option<u64>,
    pub loop_end: u64,
    /// Problems worked around while loading, such as loop points that do
    /// not fit a replaced file.
    pub notes: Vec<String>,
}

/// Loop start and end frames a file carries itself.
type FileLoop = Option<(u64, u64)>;

#[derive(Clone)]
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
        let (data, rate, channels, frames, file_loop) = if bytes.starts_with(b"OggS") {
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
        let mut notes = Vec::new();
        let looped = loop_points(entry, file_loop, frames, &mut notes);
        Ok(Self {
            data,
            rate,
            channels,
            frames,
            volume: entry.volume,
            loop_start: looped.map(|(start, _)| start),
            loop_end: looped.map_or(frames, |(_, end)| end),
            notes,
        })
    }
}

/// The manifest's loop when it fits the file, else the file's own loop,
/// else the whole file for an entry meant to loop. Points never stop a
/// pack loading: they often belong to a file the entry no longer names.
fn loop_points(
    entry: &Entry,
    file_loop: FileLoop,
    frames: u64,
    notes: &mut Vec<String>,
) -> Option<(u64, u64)> {
    let fits = |&(start, end): &(u64, u64)| start < end && end <= frames;
    let manifest = (entry.loop_start.is_some() || entry.loop_end.is_some()).then(|| {
        (
            entry.loop_start.unwrap_or(0),
            entry.loop_end.unwrap_or(frames),
        )
    });
    if let Some(points) = manifest.filter(fits) {
        return Some(points);
    }
    let file = file_loop.filter(fits);
    let fallback = if file.is_some() {
        "the file's own loop"
    } else {
        "the whole file"
    };
    if let Some((start, end)) = manifest {
        notes.push(format!(
            "loop {start}..{end} does not fit the file's {frames} frames; looping {fallback}"
        ));
    } else if let Some((start, end)) = file_loop.filter(|points| !fits(points)) {
        notes.push(format!(
            "the file's loop {start}..{end} does not fit its {frames} frames; looping the whole file"
        ));
    }
    file.or((entry.looping || manifest.is_some() || file_loop.is_some()).then_some((0, frames)))
}

fn vorbis(
    bytes: Vec<u8>,
    decode_seconds: u64,
) -> Result<(Data, u32, usize, u64, FileLoop), String> {
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
    let file_loop = vorbis_loop(&reader.comment_hdr.comment_list, frames);
    if frames > u64::from(rate) * decode_seconds || !(1..=2).contains(&channels) {
        return Ok((Data::Vorbis(bytes), rate, channels, frames, file_loop));
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
    Ok((Data::Pcm(pcm.into()), rate, channels, frames, file_loop))
}

/// The `LOOPSTART` tag with `LOOPLENGTH` or `LOOPEND` (the convention RPG
/// Maker and many game engines read), in frames.
fn vorbis_loop(comments: &[(String, String)], frames: u64) -> FileLoop {
    let tag = |name: &str| {
        comments
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| value.trim().parse::<u64>().ok())
    };
    let start = tag("LOOPSTART")?;
    let end = tag("LOOPLENGTH")
        .map(|length| start.saturating_add(length))
        .or_else(|| tag("LOOPEND"))
        .unwrap_or(frames);
    Some((start, end))
}

/// The first loop of a WAV `smpl` chunk, whose end frame is inclusive.
pub(super) fn wav_loop(bytes: &[u8]) -> FileLoop {
    let word = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = word(at + 4)? as usize;
        if &bytes[at..at + 4] == b"smpl" {
            let data = at + 8;
            if word(data + 28)? == 0 {
                return None;
            }
            let (start, end) = (word(data + 36 + 8)?, word(data + 36 + 12)?);
            return Some((u64::from(start), u64::from(end) + 1));
        }
        at = at.checked_add(8 + size + size % 2)?;
    }
    None
}

fn wav(bytes: Vec<u8>) -> Result<(Data, u32, usize, u64, FileLoop), String> {
    let file_loop = wav_loop(&bytes);
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
    Ok((
        Data::Pcm(pcm.into()),
        spec.sample_rate,
        channels,
        frames,
        file_loop,
    ))
}
