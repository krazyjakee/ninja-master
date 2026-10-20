use super::*;
use crate::audio_pack::{Clip, Entry};

fn cartridge() -> Vec<u8> {
    let dir = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(std::fs::read(dir.join("shinobi3_us.bin")).unwrap()).unwrap()
}

/// `seconds` of a constant stereo `level`, looping from its start.
fn constant(level: i16, seconds: u32, looped: bool) -> Clip {
    let pcm = vec![level; 48_000 * seconds as usize * 2];
    let entry = Entry {
        file: "constant.wav".into(),
        title: String::new(),
        volume: 1.0,
        looping: looped,
        loop_start: None,
        loop_end: None,
    };
    Clip::load(wav_bytes(&pcm, 48_000).unwrap(), &entry).unwrap()
}

fn player(rom: &[u8], clips: impl IntoIterator<Item = (u8, Clip)>) -> SoundPlayer {
    let mut player = SoundPlayer::new(rom, 48_000).unwrap();
    player.set_pack(Some(Arc::new(AudioPack::with_clips(clips))));
    player
}

/// One tick of output and the communication byte it produced.
fn tick(player: &mut SoundPlayer) -> (Vec<i16>, u8) {
    let mut pcm = vec![0; 1600];
    player.render(&mut pcm).unwrap();
    (pcm, player.take_communication())
}

fn effect_priority(rom: &[u8], command: u8) -> u8 {
    rom[PRIORITY_TABLE + usize::from(command - 0x81)]
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn replaced_music_is_silent_but_keeps_its_markers() {
    let rom = cartridge();
    let intro = crate::sound_events::Table::Music
        .command(&rom, 0x0d)
        .unwrap();
    let mut original = SoundPlayer::new(&rom, 48_000).unwrap();
    let mut replaced = player(&rom, [(intro, constant(0, 1, true))]);
    original.play_command(intro);
    replaced.play_command(intro);
    let mut markers = Vec::new();
    for _ in 0..600 {
        let (sound, marker) = tick(&mut original);
        let (silence, replaced_marker) = tick(&mut replaced);
        assert_eq!(marker, replaced_marker);
        assert!(silence.iter().all(|&s| s == 0));
        markers.extend((marker != 0).then_some(marker));
        if markers.is_empty() {
            assert_eq!(replaced.current_music(), original.current_music());
        }
        let _ = sound;
    }
    assert_eq!(markers, [1, 2, 3, 4], "the logo's cues");
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn replaced_effects_keep_priority_and_leave_music_unmasked() {
    let rom = cartridge();
    // A latching effect, and one of lower priority it must block.
    let effects: Vec<_> = (0x9d..=0xd8)
        .filter(|&c| effect_priority(&rom, c) < 0x80)
        .collect();
    let high = *effects
        .iter()
        .max_by_key(|&&c| effect_priority(&rom, c))
        .unwrap();
    let low = *effects
        .iter()
        .min_by_key(|&&c| effect_priority(&rom, c))
        .unwrap();
    assert!(effect_priority(&rom, low) < effect_priority(&rom, high));
    let mut replaced = player(
        &rom,
        [(high, constant(0, 3, false)), (low, constant(0, 3, false))],
    );
    let mut music = SoundPlayer::new(&rom, 48_000).unwrap();
    for player in [&mut replaced, &mut music] {
        player.play_command(FOREST_MUSIC);
        for _ in 0..30 {
            tick(player);
        }
    }
    replaced.play_command(high);
    replaced.play_command(low);
    for _ in 0..3 {
        let (with_effects, _) = tick(&mut replaced);
        let (alone, _) = tick(&mut music);
        assert_eq!(
            with_effects, alone,
            "a pack effect does not mask music channels"
        );
    }
    let playing: Vec<_> = replaced.effect_voices.iter().map(|(c, _)| *c).collect();
    assert_eq!(playing, [high], "the lower priority effect is refused");
    assert_eq!(replaced.sfx_priority, effect_priority(&rom, high));
    // The file outlives its sequence, and a stop command ends it.
    for _ in 0..120 {
        tick(&mut replaced);
    }
    assert!(!replaced.effects.iter().any(|t| t.active));
    assert_eq!(replaced.effect_voices.len(), 1);
    replaced.play_command(0xfd);
    tick(&mut replaced);
    assert!(replaced.effect_voices.is_empty());
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn replacement_music_follows_pause_gain_fade_and_pack_changes() {
    let rom = cartridge();
    let mut player = player(&rom, [(FOREST_MUSIC, constant(16_384, 1, true))]);
    player.play_command(FOREST_MUSIC);
    let level = |player: &mut SoundPlayer| tick(player).0[1598];
    tick(&mut player);
    assert_eq!(level(&mut player), 16_384);
    player.set_music_gain(0.5);
    for _ in 0..10 {
        tick(&mut player);
    }
    assert_eq!(level(&mut player), 8_192);
    player.set_music_gain(1.0);
    for _ in 0..10 {
        tick(&mut player);
    }
    assert_eq!(level(&mut player), 16_384);
    player.play_command(0xfc);
    let mut fading = Vec::new();
    for _ in 0..200 {
        fading.push(level(&mut player));
    }
    assert!(
        fading.windows(2).all(|w| w[1] <= w[0]),
        "the fade only lowers"
    );
    assert!(fading[10] < 16_384 && fading[10] > 0);
    assert_eq!(fading.last(), Some(&0));
    assert!(player.music_voice.is_none() && player.current_music().is_none());

    player.set_pack(None);
    player.play_command(FOREST_MUSIC);
    tick(&mut player);
    let synthesized = (0..30).any(|_| level(&mut player) != 0);
    assert!(synthesized);
    player.set_pack(Some(Arc::new(AudioPack::with_clips([(
        FOREST_MUSIC,
        constant(1_000, 1, true),
    )]))));
    let restarted = level(&mut player);
    assert!(
        restarted.abs_diff(1_000) < 50,
        "{restarted}: a new pack restarts the music"
    );
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn templates_render_one_loop_of_music_and_whole_effects() {
    let rom = cartridge();
    // JAPONESQUE has no tempo ticks; MANDARA and INNER DARKSIDE have them,
    // and INNER DARKSIDE lowers a channel on every pass.
    for command in [FOREST_MUSIC, 0x99, 0x87] {
        let Template { pcm, looped } = render_template(&rom, command).unwrap();
        let (start, end) = looped.expect("the music loops");
        assert!(
            start < end && end as usize * 2 == pcm.len(),
            "{start}..{end}"
        );
        assert!(end - start > 48_000 * 20, "a full loop, not a repeated bar");
        // The music continues past the loop's end as it did past its start.
        let mut player = SoundPlayer::new(&rom, 48_000).unwrap();
        player.play_command(command);
        let mut more = Vec::new();
        while more.len() < (end as usize + 96_000) * 2 {
            more.extend(tick(&mut player).0);
        }
        assert!(
            more[..pcm.len()] == pcm,
            "templates render the game's sound"
        );
        // Oscillator phases differ between passes, so compare loudness in
        // 5 ms windows over a second: the same notes start at the same times,
        // and a loop end two ticks off matches clearly worse. (Tempo ticks
        // fall differently in later passes, so one tick off can match.)
        let envelope = |at: u64| -> Vec<f64> {
            more[at as usize * 2..(at as usize + 48_000) * 2]
                .chunks(480)
                .map(|w| w.iter().map(|&s| f64::from(s).abs()).sum::<f64>())
                .collect()
        };
        let distance = |a: u64, b: u64| {
            let (a, b) = (envelope(a), envelope(b));
            let total: f64 = a.iter().sum();
            a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum::<f64>() / total
        };
        let seam = distance(start, end);
        let misplaced = (2..=6)
            .flat_map(|k| [end - k * 800, end + k * 800])
            .map(|end| distance(start, end))
            .fold(f64::MAX, f64::min);
        assert!(
            seam < 0.3 && misplaced > seam * 1.5,
            "${command:02x}: seam {seam}, ticks off {misplaced}"
        );
    }
    let Template { pcm, looped } = render_template(&rom, 0x9d).unwrap();
    assert!(looped.is_none() && pcm.iter().any(|&s| s != 0));
    assert!(pcm.len() < 48_000 * 2 * 10);
}
