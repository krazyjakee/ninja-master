use super::*;
use crate::sound::wav_bytes;

const OGG: &[u8] = include_bytes!("../../tests/data/audio/sine-left-32k.ogg");
/// The same audio tagged `LOOPSTART=8000`, `LOOPLENGTH=16000`.
const TAGGED_OGG: &[u8] = include_bytes!("../../tests/data/audio/sine-loop-tags-32k.ogg");

fn entry(file: &str) -> Entry {
    Entry {
        file: file.into(),
        title: String::new(),
        volume: 1.0,
        looping: false,
        loop_start: None,
        loop_end: None,
    }
}

/// Stereo frames whose left sample is `10 * index` and right is its negation.
fn ramp_wav(frames: i16, rate: u32) -> Vec<u8> {
    let pcm: Vec<i16> = (0..frames).flat_map(|i| [i * 10, -i * 10]).collect();
    wav_bytes(&pcm, rate).unwrap()
}

fn play(voice: &mut Voice, frames: usize) -> Vec<[f32; 2]> {
    std::iter::from_fn(|| voice.sample()).take(frames).collect()
}

fn level(sample: f32) -> f32 {
    (sample * 32768.0 * 1000.0).round() / 1000.0
}

#[test]
fn manifest_keys_are_driver_commands_in_their_section() {
    assert_eq!(command(Section::Music, "93"), Ok(0x93));
    assert_eq!(command(Section::Effects, "a5"), Ok(0xa5));
    assert_eq!(command(Section::Voices, "D9"), Ok(0xd9));
    for (section, key) in [
        (Section::Music, "9D"),
        (Section::Effects, "93"),
        (Section::Voices, "FC"),
        (Section::Music, "093"),
        (Section::Music, "+93"),
        (Section::Music, "forest"),
    ] {
        assert!(command(section, key).is_err(), "{key}");
    }
}

#[test]
fn wav_clips_resample_and_play_once() {
    let clip = Clip::load(ramp_wav(100, 24_000), &entry("a.wav")).unwrap();
    assert_eq!((clip.rate, clip.channels, clip.frames), (24_000, 2, 100));
    let mut voice = Voice::new(Arc::new(clip), 48_000);
    let out = play(&mut voice, 1000);
    assert!((197..=200).contains(&out.len()), "{}", out.len());
    assert!(voice.finished());
    let left: Vec<_> = out[..5].iter().map(|s| level(s[0])).collect();
    assert_eq!(left, [0.0, 5.0, 10.0, 15.0, 20.0]);
    assert_eq!(level(out[3][1]), -15.0);
}

#[test]
fn loops_repeat_between_their_frames() {
    let mut looped = entry("a.wav");
    looped.loop_start = Some(10);
    looped.loop_end = Some(20);
    let clip = Clip::load(ramp_wav(100, 48_000), &looped).unwrap();
    let mut voice = Voice::new(Arc::new(clip), 48_000);
    let left: Vec<_> = play(&mut voice, 45)
        .iter()
        .map(|s| level(s[0]) / 10.0)
        .collect();
    let expected: Vec<_> = (0..20)
        .chain(10..20)
        .chain(10..20)
        .chain(10..15)
        .map(|i| i as f32)
        .collect();
    assert_eq!(left, expected);
    assert!(!voice.finished());
}

#[test]
fn loops_that_do_not_fit_loop_the_whole_file_instead_of_failing() {
    let wav = ramp_wav(100, 48_000);
    let looped = |start, end| {
        let mut looped = entry("a.wav");
        (looped.loop_start, looped.loop_end) = (start, end);
        let clip = Clip::load(wav.clone(), &looped).unwrap();
        ((clip.loop_start, clip.loop_end), clip.notes.len())
    };
    // A replaced file keeps the original's points, measured in other frames.
    assert_eq!(looped(Some(312_800), Some(3_384_800)), ((Some(0), 100), 1));
    assert_eq!(looped(Some(50), Some(50)), ((Some(0), 100), 1));
    assert_eq!(looped(Some(0), Some(101)), ((Some(0), 100), 1));
    assert_eq!(looped(Some(100), None), ((Some(0), 100), 1));
    assert_eq!(looped(None, Some(10)), ((Some(0), 10), 0));
    assert_eq!(looped(Some(5), Some(10)), ((Some(5), 10), 0));
    let mut whole = entry("a.wav");
    whole.looping = true;
    let clip = Clip::load(wav.clone(), &whole).unwrap();
    assert_eq!(
        (clip.loop_start, clip.loop_end, clip.notes.len()),
        (Some(0), 100, 0)
    );
    let once = Clip::load(wav, &entry("a.wav")).unwrap();
    assert_eq!((once.loop_start, once.loop_end), (None, 100));
}

#[test]
fn files_carry_their_own_loop_points() {
    let pcm: Vec<i16> = (0..100).flat_map(|i| [i * 10, -i * 10]).collect();
    let wav = template::looping_wav(&pcm, Some((10, 20))).unwrap();
    assert_eq!(clip::wav_loop(&wav), Some((10, 20)));
    // Files with loop points loop even when the entry does not say so.
    let clip = Clip::load(wav.clone(), &entry("a.wav")).unwrap();
    assert_eq!(
        (clip.frames, clip.loop_start, clip.loop_end),
        (100, Some(10), 20)
    );
    let mut voice = Voice::new(Arc::new(clip), 48_000);
    let left: Vec<_> = play(&mut voice, 30)
        .iter()
        .map(|s| level(s[0]) / 10.0)
        .collect();
    let expected: Vec<_> = (0..20).chain(10..20).map(|i| i as f32).collect();
    assert_eq!(left, expected);
    // The manifest overrides them when its points fit, and falls back to them.
    let mut looped = entry("a.wav");
    (looped.loop_start, looped.loop_end) = (Some(30), Some(40));
    let clip = Clip::load(wav.clone(), &looped).unwrap();
    assert_eq!((clip.loop_start, clip.loop_end), (Some(30), 40));
    looped.loop_end = Some(4_000);
    let clip = Clip::load(wav, &looped).unwrap();
    assert_eq!(
        (clip.loop_start, clip.loop_end, clip.notes.len()),
        (Some(10), 20, 1)
    );

    let clip = Clip::load(TAGGED_OGG.to_vec(), &entry("a.ogg")).unwrap();
    assert_eq!((clip.loop_start, clip.loop_end), (Some(8_000), 24_000));
}

#[test]
fn unreadable_files_are_rejected() {
    assert!(Clip::load(b"ID3 not audio".to_vec(), &entry("a.mp3")).is_err());
    assert!(Clip::load(ramp_wav(0, 48_000), &entry("a.wav")).is_err());
    assert!(Clip::load(ramp_wav(10, 4_000), &entry("a.wav")).is_err());
}

#[test]
fn vorbis_streams_and_loops_from_its_recording() {
    let decoded = Clip::load(OGG.to_vec(), &entry("a.ogg")).unwrap();
    assert!(matches!(decoded.data, clip::Data::Pcm(_)));
    assert_eq!(
        (decoded.rate, decoded.channels, decoded.frames),
        (32_000, 2, 32_000)
    );
    let mut looped = entry("a.ogg");
    looped.loop_start = Some(16_000);
    let streamed = Clip::load_decoding(OGG.to_vec(), &looped, 0).unwrap();
    assert!(matches!(streamed.data, clip::Data::Vorbis(_)));
    // At the source rate every output frame is one decoded frame.
    let reference = play(&mut Voice::new(Arc::new(decoded), 32_000), 32_000);
    let mut voice = Voice::new(Arc::new(streamed), 32_000);
    let first = play(&mut voice, 32_000);
    // The one-shot voice ends when its last frame becomes current.
    assert_eq!(reference.len(), 31_999);
    assert!(
        first[..31_999] == reference,
        "streaming decodes like the whole file"
    );
    let second = play(&mut voice, 15_999);
    assert!(second == reference[16_000..], "the loop replays its frames");
    let energy = |channel: usize| first.iter().map(|s| s[channel].abs()).sum::<f32>();
    assert!(
        energy(0) > 1000.0 && energy(1) < 1.0,
        "the sine is left only"
    );
}

#[test]
fn packs_load_from_folders_and_validate_on_discovery() {
    let dir = std::env::temp_dir().join(format!("ninja-audio-pack-{}", std::process::id()));
    let _ = fs_remove(&dir);
    std::fs::create_dir_all(dir.join("music")).unwrap();
    std::fs::write(dir.join("music/forest.ogg"), OGG).unwrap();
    std::fs::write(dir.join("hit.wav"), ramp_wav(100, 22_050)).unwrap();
    std::fs::write(dir.join("junk.wav"), "not audio").unwrap();
    let manifest = r#"{"version": 1, "name": "Test pack",
        "music": {"93": {"file": "music/forest.ogg", "loop_start": 312800, "loop_end": 3384800}},
        "effects": {"A5": {"file": "hit.wav", "volume": 0.5, "title": "WARNING"},
                    "A6": {"file": "junk.wav"}, "A7": {"file": "effects/deleted.wav"}},
        "voices": {"DB": {"file": "hit.wav", "loop_start": 0, "loop_end": 80},
                   "DC": {"file": "voices/deleted.wav"}}}"#;
    std::fs::write(dir.join(MANIFEST), manifest).unwrap();
    let entry = PackEntry::read(&dir, &KIND).unwrap();
    assert_eq!(entry.name, "Test pack");
    let pack = AudioPack::load(&dir, None).unwrap();
    assert_eq!(Section::ALL.map(|s| pack.count(s)), [1, 1, 1]);
    assert_eq!(pack.clip(0xa5).unwrap().volume, 0.5);
    // Stale loop points loop the whole file and an unreadable file keeps
    // the original sound, with a note each; files the pack leaves out
    // simply keep the original sound.
    assert_eq!(pack.clip(0x93).unwrap().loop_start, Some(0));
    assert!(pack.clip(0xa6).is_none() && pack.clip(0xa7).is_none());
    assert!(pack.clip(0xdc).is_none());
    assert_eq!(pack.notes.len(), 2, "{:?}", pack.notes);
    // Voices play once, as the driver plays its samples.
    let voice = pack.clip(0xdb).unwrap();
    assert_eq!((voice.loop_start, voice.loop_end), (None, 100));

    for bad in [
        r#"{"version": 2, "name": ""}"#,
        r#"{"version": 1, "name": "", "music": {"A5": {"file": "hit.wav"}}}"#,
        r#"{"version": 1, "name": "", "voices": {"D9": {"file": "../hit.wav"}}}"#,
        r#"{"version": 1, "name": "", "effects": {"A5": {"file": "hit.wav", "volume": 9}}}"#,
        r#"{"version": 1, "name": "", "effects": {"A5": {"file": "hit.wav", "speed": 2}}}"#,
    ] {
        std::fs::write(dir.join(MANIFEST), bad).unwrap();
        assert!(PackEntry::read(&dir, &KIND).is_err(), "{bad}");
    }
    fs_remove(&dir).unwrap();
}

#[test]
fn levels_ignore_silence() {
    let tone = |level: f32, frames: usize| std::iter::repeat_n([level, -level], frames);
    let rms = level::gated_rms(tone(0.25, 4_800), 48_000);
    assert!((rms - 0.25).abs() < 1e-9);
    // A silent tail and a quiet intro do not lower the level.
    let padded = tone(0.0, 48_000)
        .chain(tone(0.25, 4_800))
        .chain(tone(0.0001, 9_600));
    assert!((level::gated_rms(padded, 48_000) - 0.25).abs() < 1e-9);
    assert_eq!(level::gated_rms(tone(0.0, 4_800), 48_000), 0.0);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn replacements_play_at_their_originals_loudness() {
    let dir = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    let rom =
        crate::rom::regions::prepare(std::fs::read(dir.join("shinobi3_us.bin")).unwrap()).unwrap();
    // A loud mastered track, a quiet effect, a loud voice, and an effect
    // too faint to raise fully.
    let tone = |level: i16| {
        let pcm: Vec<i16> = (0..48_000 * 30)
            .flat_map(|i| {
                let s = if i % 100 < 50 { level } else { -level };
                [s, s]
            })
            .collect();
        let mut looped = entry("a.wav");
        looped.looping = true;
        Clip::load(wav_bytes(&pcm, 48_000).unwrap(), &looped).unwrap()
    };
    let mut clips = vec![
        (0x93, tone(32_000)),
        (0xa5, tone(1_500)),
        (0xdb, tone(30_000)),
    ];
    clips[0].1.volume = 0.5;
    let mut faint = [(0xa5, tone(150))];
    level::match_levels(&rom, &mut clips);
    level::match_levels(&rom, &mut faint);
    assert_eq!(faint[0].1.volume, 4.0, "boosts stop at +12 dB");
    for (command, clip) in &clips {
        let heard = clip.level() * f64::from(clip.volume);
        let original = crate::sound::reference_level(&rom, *command).unwrap();
        let wanted = if *command == 0x93 {
            original * 0.5
        } else {
            original
        };
        assert!(
            (heard / wanted - 1.0).abs() < 0.02,
            "${command:02x}: {heard} vs {wanted}"
        );
    }
}

fn fs_remove(path: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn templates_round_trip_as_packs_with_every_sound() {
    let dir = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    let rom =
        crate::rom::regions::prepare(std::fs::read(dir.join("shinobi3_us.bin")).unwrap()).unwrap();
    let path = std::env::temp_dir().join(format!("ninja-audio-template-{}", std::process::id()));
    let _ = fs_remove(&path);
    export_template(&rom, &path).unwrap();
    assert!(export_template(&rom, &path).is_err(), "never overwrites");
    let pack = AudioPack::load(&path, Some(&rom)).unwrap();
    assert_eq!(Section::ALL.map(|s| pack.count(s)), [28, 59, 19]);
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(path.join(MANIFEST)).unwrap()).unwrap();
    let forest = &manifest.music["93"];
    assert!(forest.looping && forest.loop_start.is_none() && !forest.title.is_empty());
    let clip = pack.clip(0x93).unwrap();
    assert!(clip.loop_start.is_some_and(|start| start > 0) && clip.loop_end == clip.frames);
    assert!(pack.notes.is_empty(), "{:?}", pack.notes);
    assert!(
        pack.clip(0x91).unwrap().loop_start.is_none(),
        "ROUND CLEAR plays once"
    );
    assert!(manifest.voices.values().all(|voice| !voice.looping));
    // Renders of the originals already match them.
    for command in [0x93, 0x91, 0xa5, 0xdb] {
        let volume = pack.clip(command).unwrap().volume;
        assert!((volume - 1.0).abs() < 0.02, "${command:02x}: {volume}");
    }
    fs_remove(&path).unwrap();
}
