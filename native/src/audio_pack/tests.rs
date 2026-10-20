use super::*;
use crate::sound::wav_bytes;

const OGG: &[u8] = include_bytes!("../../tests/data/audio/sine-left-32k.ogg");

fn entry(file: &str) -> Entry {
    Entry {
        file: file.into(),
        title: String::new(),
        volume: 1.0,
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
fn invalid_loops_and_files_are_rejected() {
    let wav = ramp_wav(100, 48_000);
    for (start, end) in [
        (Some(50), Some(50)),
        (Some(0), Some(101)),
        (None, Some(10)),
        (Some(100), None),
    ] {
        let mut looped = entry("a.wav");
        (looped.loop_start, looped.loop_end) = (start, end);
        assert!(
            Clip::load(wav.clone(), &looped).is_err(),
            "{start:?}..{end:?}"
        );
    }
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
    let manifest = r#"{"version": 1, "name": "Test pack",
        "music": {"93": {"file": "music/forest.ogg", "loop_start": 0}},
        "effects": {"A5": {"file": "hit.wav", "volume": 0.5, "title": "WARNING"}}}"#;
    std::fs::write(dir.join(MANIFEST), manifest).unwrap();
    let entry = PackEntry::read(&dir, &KIND).unwrap();
    assert_eq!(entry.name, "Test pack");
    let pack = AudioPack::load(&dir).unwrap();
    assert_eq!(
        Section::ALL.map(|s| pack.count(s)),
        [1, 1, 0],
        "music and effects"
    );
    assert_eq!(pack.clip(0xa5).unwrap().volume, 0.5);
    assert_eq!(pack.clip(0x93).unwrap().loop_start, Some(0));

    for bad in [
        r#"{"version": 2, "name": ""}"#,
        r#"{"version": 1, "name": "", "music": {"A5": {"file": "hit.wav"}}}"#,
        r#"{"version": 1, "name": "", "voices": {"D9": {"file": "missing.wav"}}}"#,
        r#"{"version": 1, "name": "", "voices": {"D9": {"file": "../hit.wav"}}}"#,
        r#"{"version": 1, "name": "", "effects": {"A5": {"file": "hit.wav", "volume": 9}}}"#,
        r#"{"version": 1, "name": "", "effects": {"A5": {"file": "hit.wav", "speed": 2}}}"#,
    ] {
        std::fs::write(dir.join(MANIFEST), bad).unwrap();
        assert!(PackEntry::read(&dir, &KIND).is_err(), "{bad}");
    }
    fs_remove(&dir).unwrap();
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
    let pack = AudioPack::load(&path).unwrap();
    let counts = Section::ALL.map(|s| pack.count(s));
    eprintln!("{counts:?}");
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(path.join(MANIFEST)).unwrap()).unwrap();
    let forest = &manifest.music["93"];
    assert!(forest.loop_start.is_some() && !forest.title.is_empty());
    fs_remove(&path).unwrap();
}
