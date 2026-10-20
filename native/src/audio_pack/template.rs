//! A starter pack: every playable command rendered to WAV, with its ROM
//! name and detected loop, so a pack is made by replacing files one by one.
use super::*;
use crate::{
    sound::{Template, render_template, wav_bytes},
    sound_events::{EFFECT_TABLE, MUSIC_TABLE, VOICE_TABLE},
};
use std::fs;

/// Sound test record counts (`$1D`, `$42`, `$16`).
const TABLES: [(usize, usize); 3] = [
    (MUSIC_TABLE, 0x1d),
    (EFFECT_TABLE, 0x42),
    (VOICE_TABLE, 0x16),
];

const README: &str = "\
Audio pack template
===================

Every music track, sound effect and voice the game can play is rendered
here as 48 kHz WAV, named by driver command and its sound-test title.
Replace any file with your own .wav or .ogg (Vorbis), mono or stereo, at any
sample rate, and update its \"file\" in audio-pack.json. Delete the entries you
do not replace: missing entries keep the original synthesized sound.

Entries marked \"loop\": true repeat. The rendered WAVs carry the original
loop points in their own smpl chunk, so a file you swap in simply loops whole.
To loop part of your file, give it its own loop points (a WAV smpl loop, or
LOOPSTART with LOOPLENGTH tags in an Ogg file), or set \"loop_start\" and
\"loop_end\" in sample frames of that file; points that do not fit are
ignored. \"volume\" scales an entry (1.0 by default, up to 4.0). \"title\" is
only a note.

Select the pack in the F1 panel's Audio packs tab; Reload audio picks up
edits. The originals are rendered at the game's level, so files normalized
like them keep the original balance between music and effects.
";

/// Export into a fresh directory: existing files are never overwritten.
pub fn export_template(rom: &[u8], path: &Path) -> Result<(), String> {
    let titles = titles(rom);
    fs::create_dir(path).map_err(|e| format!("choose a new export folder: {e}"))?;
    let mut manifest = Manifest {
        version: 1,
        name: "My audio pack".into(),
        music: BTreeMap::new(),
        effects: BTreeMap::new(),
        voices: BTreeMap::new(),
    };
    for section in Section::ALL {
        let folder = section.name();
        fs::create_dir(path.join(folder)).map_err(|e| e.to_string())?;
        let entries = match section {
            Section::Music => &mut manifest.music,
            Section::Effects => &mut manifest.effects,
            Section::Voices => &mut manifest.voices,
        };
        for command in section.commands() {
            // Unassigned commands fail to load or render silence.
            let Ok(Template { pcm, looped }) = render_template(rom, command) else {
                continue;
            };
            if pcm.iter().all(|&sample| sample == 0) {
                continue;
            }
            let title = titles.get(&command).cloned().unwrap_or_default();
            let slug: String = title
                .to_ascii_lowercase()
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|word| !word.is_empty())
                .flat_map(|word| ["-", word])
                .collect();
            let file = format!("{folder}/{command:02X}{slug}.wav");
            fs::write(path.join(&file), looping_wav(&pcm, looped)?)
                .map_err(|e| format!("{file}: {e}"))?;
            // Loop points stay with the rendered file: a replacement that
            // lacks them loops whole instead of at the original's frames.
            entries.insert(
                format!("{command:02X}"),
                Entry {
                    file,
                    title,
                    volume: 1.0,
                    looping: looped.is_some(),
                    loop_start: None,
                    loop_end: None,
                },
            );
        }
    }
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(path.join(MANIFEST), json + "\n").map_err(|e| e.to_string())?;
    fs::write(path.join("README.txt"), README).map_err(|e| e.to_string())
}

/// 48 kHz stereo WAV with `looped` as a `smpl` chunk's forward loop.
pub(super) fn looping_wav(pcm: &[i16], looped: Option<(u64, u64)>) -> Result<Vec<u8>, String> {
    const RATE: u32 = 48_000;
    let mut bytes = wav_bytes(pcm, RATE)?;
    let Some((start, end)) = looped else {
        return Ok(bytes);
    };
    let frame = |n: u64| u32::try_from(n).map_err(|_| "loop beyond a WAV's range".to_string());
    let (start, last) = (frame(start)?, frame(end.saturating_sub(1))?);
    let period = 1_000_000_000 / RATE;
    // Manufacturer, product, sample period (ns), MIDI unity note, pitch
    // fraction, SMPTE format and offset, one loop, no sampler data; then the
    // loop: cue id, forward type, first and last frame, fraction, endless.
    let fields = [0, 0, period, 60, 0, 0, 0, 1, 0, 0, 0, start, last, 0, 0];
    bytes.extend(b"smpl");
    bytes.extend((fields.len() as u32 * 4).to_le_bytes());
    bytes.extend(fields.iter().flat_map(|field| field.to_le_bytes()));
    let riff = u32::try_from(bytes.len() - 8).map_err(|_| "WAV exceeds RIFF size limit")?;
    bytes[4..8].copy_from_slice(&riff.to_le_bytes());
    Ok(bytes)
}

/// Sound test names by driver command; the first record wins.
fn titles(rom: &[u8]) -> HashMap<u8, String> {
    let mut titles = HashMap::new();
    for (table, count) in TABLES {
        for index in 0..count {
            let at = table + index * 16;
            if let (Some(&command), Some(name)) = (rom.get(at), rom.get(at + 2..at + 16)) {
                let name = String::from_utf8_lossy(name).trim().to_owned();
                if !name.is_empty() {
                    titles.entry(command).or_insert(name);
                }
            }
        }
    }
    titles
}
