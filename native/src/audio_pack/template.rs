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

Music loops between loop_start and loop_end (sample frames of that file;
loop_end defaults to the end of the file). Entries without loop_start play
once. A new file needs loop points measured in its own sample frames.
\"volume\" scales an entry (1.0 by default, up to 4.0). \"title\" is only a note.

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
            fs::write(path.join(&file), wav_bytes(&pcm, 48_000)?)
                .map_err(|e| format!("{file}: {e}"))?;
            entries.insert(
                format!("{command:02X}"),
                Entry {
                    file,
                    title,
                    volume: 1.0,
                    loop_start: looped.map(|(start, _)| start),
                    loop_end: looped.map(|(_, end)| end),
                },
            );
        }
    }
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(path.join(MANIFEST), json + "\n").map_err(|e| e.to_string())?;
    fs::write(path.join("README.txt"), README).map_err(|e| e.to_string())
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
