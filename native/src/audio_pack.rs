//! Versioned OGG/WAV audio packs that replace the synthesized soundtrack.
//! Entries are keyed by driver command, so a pack can replace any subset of
//! the music, effects and voices; everything else keeps the FM/PSG sound.
//! Replacement only changes what is heard: the original sequences still run
//! silently, keeping their `$E2` markers, fades and timing deterministic.
use crate::pack::{Kind, Source, check_name};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

mod clip;
mod level;
mod template;
mod voice;
pub use crate::pack::{Library, PackEntry, resolve_selection, selection_value};
pub use clip::Clip;
pub(crate) use level::{SECONDS as LEVEL_SECONDS, gated_rms};
pub use template::export_template;
pub(crate) use voice::Voice;

pub const MANIFEST: &str = "audio-pack.json";
const MAX_PACK_BYTES: usize = 1024 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;

pub static KIND: Kind = Kind {
    manifest: MANIFEST,
    folder: "audio-packs",
    env: "NINJA_MASTER_AUDIO_PACKS",
    max_bytes: MAX_PACK_BYTES,
    name: |source| Ok(manifest(source)?.name),
};

pub fn library_directory() -> PathBuf {
    crate::pack::library_directory(&KIND)
}

/// Driver commands each manifest section accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Music,
    Effects,
    Voices,
}
impl Section {
    pub const ALL: [Self; 3] = [Self::Music, Self::Effects, Self::Voices];
    pub fn commands(self) -> std::ops::RangeInclusive<u8> {
        match self {
            Self::Music => 0x81..=0x9c,
            Self::Effects => 0x9d..=0xd8,
            Self::Voices => 0xd9..=0xfb,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Music => "music",
            Self::Effects => "effects",
            Self::Voices => "voices",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub name: String,
    /// Driver command (two hex digits, `81`..`9C`) to replacement.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub music: BTreeMap<String, Entry>,
    /// `9D`..`D8`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub effects: BTreeMap<String, Entry>,
    /// Sampled voices and drums started by the game, `D9`..`FB`; these
    /// play once, as the driver plays them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub voices: BTreeMap<String, Entry>,
}
impl Manifest {
    fn section(&self, section: Section) -> &BTreeMap<String, Entry> {
        match section {
            Section::Music => &self.music,
            Section::Effects => &self.effects,
            Section::Voices => &self.voices,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// An `.ogg` (Vorbis) or `.wav` file, mono or stereo, at any sample rate.
    pub file: String,
    /// The original's name, for people editing the pack; ignored on load.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default = "full_volume", skip_serializing_if = "is_full_volume")]
    pub volume: f32,
    /// Repeat the file: between its own loop points (a WAV `smpl` loop, or
    /// Ogg `LOOPSTART` with `LOOPLENGTH`/`LOOPEND` tags) or else whole.
    /// Files with their own loop points always loop.
    #[serde(default, rename = "loop", skip_serializing_if = "is_false")]
    pub looping: bool,
    /// Sample frame playback returns to after `loop_end`, overriding the
    /// file's loop. Points that do not fit the file are ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_start: Option<u64>,
    /// Sample frame that ends each loop; the end of the file by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_end: Option<u64>,
}
fn full_volume() -> f32 {
    1.0
}
fn is_full_volume(volume: &f32) -> bool {
    *volume == 1.0
}
fn is_false(value: &bool) -> bool {
    !value
}

/// A manifest key: the driver command as two hex digits.
fn command(section: Section, key: &str) -> Result<u8, String> {
    u8::from_str_radix(key, 16)
        .ok()
        .filter(|c| key.len() == 2 && section.commands().contains(c))
        .ok_or_else(|| {
            let range = section.commands();
            format!(
                "{} key {key:?} must be a command {:02X}..{:02X}",
                section.name(),
                range.start(),
                range.end()
            )
        })
}

/// Discovery validates keys and file names without reading audio. Listed
/// files may be missing: those commands keep the original sound.
fn manifest(source: &mut Source) -> Result<Manifest, String> {
    let manifest: Manifest = serde_json::from_slice(&source.manifest_bytes()?)
        .map_err(|e| format!("{MANIFEST}: {e}"))?;
    if manifest.version != 1 {
        return Err(format!(
            "unsupported audio pack version {}",
            manifest.version
        ));
    }
    if manifest.name.contains('\0') {
        return Err("pack name cannot contain a null character".into());
    }
    for section in Section::ALL {
        for (key, entry) in manifest.section(section) {
            command(section, key)?;
            check_name(&entry.file)?;
            if !(0.0..=4.0).contains(&entry.volume) {
                return Err(format!("{key}: volume must be 0..4"));
            }
        }
    }
    Ok(manifest)
}

pub struct AudioPack {
    pub name: String,
    clips: HashMap<u8, Arc<Clip>>,
    /// Entries skipped or adjusted while loading, one line each.
    pub notes: Vec<String>,
}

impl AudioPack {
    /// A pack directory, audio-pack.json, or a ZIP with one root/wrapped
    /// manifest. With the ROM, every file plays at its original's loudness
    /// (which takes a few seconds of rendering).
    pub fn load(path: &Path, rom: Option<&[u8]>) -> Result<Self, String> {
        let mut source = Source::open(path, &KIND)?;
        let manifest = manifest(&mut source)?;
        let mut clips = Vec::new();
        let mut notes = Vec::new();
        let mut total = 0;
        for section in Section::ALL {
            for (key, entry) in manifest.section(section) {
                // A pack need not include every file it lists: missing ones
                // keep the original sound. Unreadable ones do too, noted.
                if source.location(&entry.file).is_err() {
                    continue;
                }
                let bytes = source.read(&entry.file, MAX_FILE_BYTES)?;
                total += bytes.len();
                if total > MAX_PACK_BYTES {
                    return Err("audio pack exceeds 1 GiB".into());
                }
                match Clip::load(bytes, entry) {
                    Ok(mut clip) => {
                        notes.extend(clip.notes.iter().map(|n| format!("{}: {n}", entry.file)));
                        // The driver plays a voice sample once.
                        if !level::loops(section) {
                            (clip.loop_start, clip.loop_end) = (None, clip.frames);
                        }
                        clips.push((command(section, key)?, clip));
                    }
                    Err(error) => notes.push(format!(
                        "{}: {error}; keeping the original sound",
                        entry.file
                    )),
                }
            }
        }
        if let Some(rom) = rom {
            level::match_levels(rom, &mut clips);
        }
        Ok(Self {
            name: manifest.name,
            clips: clips
                .into_iter()
                .map(|(command, clip)| (command, Arc::new(clip)))
                .collect(),
            notes,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_clips(clips: impl IntoIterator<Item = (u8, Clip)>) -> Self {
        Self {
            name: "test".into(),
            notes: Vec::new(),
            clips: clips
                .into_iter()
                .map(|(c, clip)| (c, Arc::new(clip)))
                .collect(),
        }
    }
    pub fn clip(&self, command: u8) -> Option<&Arc<Clip>> {
        self.clips.get(&command)
    }

    /// Replacements in one section.
    pub fn count(&self, section: Section) -> usize {
        self.clips
            .keys()
            .filter(|c| section.commands().contains(c))
            .count()
    }
}

#[cfg(test)]
mod tests;
