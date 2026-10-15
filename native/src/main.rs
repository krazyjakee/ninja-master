#![deny(unsafe_code)]

#[cfg(feature = "desktop")]
mod desktop;
mod rom_source;

use ninja_master_native::{
    assets::{SoundAssets, StageAssets},
    demo::{Demo, scripted_input},
    front_end::{Front, SegaLogo},
    game::Game,
    input::{Button, Input},
    modes::{Mode, Modes},
    render::Frame,
    sound::{FOREST_MUSIC, SoundPlayer, Stem, render_cue, wav_bytes},
    viewer::{View, Viewer},
};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};

/// Interleaved stereo samples per 60 Hz tick at the 48 kHz output rate.
const TICK_SAMPLES: usize = 48_000 / 60 * 2;

#[derive(Default)]
struct Options {
    input_script: Option<PathBuf>,
    trace: Option<PathBuf>,
    sound: bool,
    wav: Option<PathBuf>,
    /// `--sound-cue HEX`: render one driver command in isolation to `--wav`.
    sound_cue: Option<u8>,
    stem: Stem,
    part: usize,
    round: Option<u8>,
    substage: Option<u8>,
    disabled_effects: Vec<String>,
    headless: bool,
    /// Request the wider view for deterministic headless captures too.
    widescreen: bool,
    frames: Option<u64>,
    capture: Option<PathBuf>,
    /// `--dump-frames DIR`: every headless tick's frame as `DIR/frameNNNNNN.ppm`.
    dump_frames: Option<PathBuf>,
    scripted: bool,
    mute: bool,
    assets: bool,
    play: bool,
    /// Start at the SEGA logo and run the whole front end (`Mode_SegaLogo`).
    boot: bool,
    demo: bool,
    /// Open the F1 developer panel at start.
    devtools: bool,
    rom: Option<PathBuf>,
    view: Option<View>,
    page: Option<usize>,
    palette: Option<usize>,
    camera: Option<(i32, i32)>,
    asset_report: Option<PathBuf>,
}

enum Scene {
    Demo(Demo),
    Assets(Box<Viewer>),
    /// Play runs through the `$FF002A` mode layer: the front end, gameplay,
    /// pause, the round transition screens, game over and the ending.
    Play(Box<Modes>),
}
impl Scene {
    fn scripted_input(&self, tick: u64) -> Input {
        match self {
            Self::Demo(_) => scripted_input(tick),
            Self::Assets(_) => Input::default(),
            Self::Play(modes) => match modes.game() {
                Some(game) => game.scripted_input(tick),
                None => ninja_master_native::game::scripted_input(tick),
            },
        }
    }
    fn tick(&mut self, input: Input, width: usize) -> bool {
        match self {
            Self::Demo(demo) => demo.tick(input),
            Self::Assets(viewer) => viewer.tick(input),
            Self::Play(modes) => {
                modes.set_view_width(width);
                modes.tick(input);
                false
            }
        }
    }
    fn draw(&self, frame: &mut Frame) {
        match self {
            Self::Demo(demo) => demo.draw(frame),
            Self::Assets(viewer) => viewer.draw(frame),
            Self::Play(modes) => modes.draw(frame),
        }
    }
    /// The title menu's QUIT was chosen.
    #[cfg_attr(not(feature = "desktop"), allow(dead_code))]
    fn quit_requested(&self) -> bool {
        matches!(self, Self::Play(modes) if modes.quit_requested())
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input-script" => {
                options.input_script =
                    Some(args.next().ok_or("--input-script needs a file")?.into())
            }
            "--trace" => options.trace = Some(args.next().ok_or("--trace needs a file")?.into()),
            "--sound" => options.sound = true,
            "--wav" => options.wav = Some(args.next().ok_or("--wav needs a file")?.into()),
            "--sound-cue" => {
                let value = args.next().ok_or("--sound-cue needs a hex command")?;
                let value = value.trim_start_matches('$').trim_start_matches("0x");
                options.sound_cue = Some(
                    u8::from_str_radix(value, 16)
                        .map_err(|_| "--sound-cue needs a hex command such as b0")?,
                );
            }
            "--stem" => {
                options.stem = match args.next().ok_or("--stem needs mix|fm|psg")?.as_str() {
                    "mix" => Stem::Mix,
                    "fm" => Stem::Fm,
                    "psg" => Stem::Psg,
                    _ => return Err("--stem needs mix|fm|psg".into()),
                }
            }
            "--part" => {
                options.part = args
                    .next()
                    .ok_or("--part needs a part index (0-based)")?
                    .parse()
                    .map_err(|_| "invalid part")?
            }
            "--round" => {
                options.round = Some(
                    args.next()
                        .ok_or("--round needs 1..7")?
                        .parse()
                        .map_err(|_| "invalid round")?,
                )
            }
            "--substage" => {
                options.substage = Some(
                    args.next()
                        .ok_or("--substage needs 1..3")?
                        .parse()
                        .map_err(|_| "invalid substage")?,
                )
            }
            "--disable-effect" => options.disabled_effects.push(
                args.next()
                    .ok_or("--disable-effect needs palette|parallax|priority|vegetation")?,
            ),
            "--headless" => options.headless = true,
            "--widescreen" => options.widescreen = true,
            "--scripted" => options.scripted = true,
            "--mute" => options.mute = true,
            "--assets" => options.assets = true,
            "--play" => options.play = true,
            "--boot" => options.boot = true,
            "--demo" => options.demo = true,
            "--devtools" => options.devtools = true,
            "--rom" => options.rom = Some(args.next().ok_or("--rom needs a path")?.into()),
            "--view" => {
                options.view = Some(View::parse(&args.next().ok_or("--view needs a name")?)?)
            }
            "--page" => {
                options.page = Some(
                    args.next()
                        .ok_or("--page needs a number (1-based)")?
                        .parse()
                        .map_err(|_| "invalid page")?,
                )
            }
            "--palette" => {
                options.palette = Some(
                    args.next()
                        .ok_or("--palette needs 0..3")?
                        .parse()
                        .map_err(|_| "invalid palette")?,
                )
            }
            "--camera" => {
                let value = args.next().ok_or("--camera needs X,Y")?;
                let (x, y) = value.split_once(',').ok_or("--camera needs X,Y")?;
                options.camera = Some((
                    x.parse().map_err(|_| "invalid camera X")?,
                    y.parse().map_err(|_| "invalid camera Y")?,
                ));
            }
            "--asset-report" => {
                options.asset_report =
                    Some(args.next().ok_or("--asset-report needs a path")?.into())
            }
            "--frames" => {
                let count = args
                    .next()
                    .ok_or("--frames needs a positive tick count")?
                    .parse::<u64>()
                    .map_err(|_| "--frames needs a positive tick count")?;
                if count == 0 {
                    return Err("--frames must be greater than zero".into());
                }
                options.frames = Some(count);
            }
            "--capture" => {
                options.capture = Some(args.next().ok_or("--capture needs a PPM path")?.into())
            }
            "--dump-frames" => {
                options.dump_frames =
                    Some(args.next().ok_or("--dump-frames needs a directory")?.into())
            }
            "--help" | "-h" => {
                println!(
                    "Ninja Master native playable slice, engine demo and ROM asset viewer\n\
                    Usage: ninja-master-native [--headless] [--frames N] [--scripted] [--mute] [--capture FILE.ppm] [--dump-frames DIR]\n\
                    With no mode flag, the desktop build boots the whole game (--boot); headless runs and --round/--substage/--part start play directly (--play).\n\
                    Full game: --boot [--rom FILE] (SEGA logo, story, title, options, attract demo, game, game over/continue, ending)\n\
                    Engine demo: --demo (synthetic test scene)\n\
                    Playable slice: --play [--round R --substage S [--part P]] [--rom FILE] [--asset-report FILE] [--scripted] (move, run, jump, attack, ninjutsu, pause)\n\
                    Play continues through substage and round transitions (1-1 -> 1-2 ...), with ROUND CLEAR and the round card between rounds.\n\
                    Start (Enter) or Escape pauses: Up/Down/Left/Right select the ninjutsu, Start resumes, B then C quits to the title menu (its QUIT ends the program).\n\
                    Replay: --headless --play --input-script FILE [--trace FILE] [--frames N]\n\
                    Display: desktop widescreen is on by default (F1 Display to turn off); --widescreen expands headless captures too.\n\
                    Sound demo: --sound [--scripted] [--headless --frames N --wav FILE] (X plays an effect)\n\
                    Isolated cue (sound-test comparison): --headless --sound --sound-cue HEX [--stem mix|fm|psg] [--frames N] --wav FILE\n\
                    Asset viewer: --assets [--rom FILE] [--view scene|foreground|background|objects|metatiles|palettes]\n\
                    Viewer options: --round 1..7 --substage 1..3 --part P (0-based), --page N (1-based), --palette 0..3, --camera X,Y, --asset-report FILE\n\
                    Effects: --disable-effect palette|parallax|priority|vegetation (repeatable)\n\
                    Viewer controls: arrows pan/page, Z / Space next view, X palette/layer, Enter reset.\n\
                    Viewer scope: every substage's art, palettes, layout, plane maps, scenery sprites and recovered palette/tile-animation effects.\n\
                    Keyboard (play): arrows move, D / C / Space jump, S / X attack, A / Z ninjutsu, Enter or Escape pause.\n\
                    Dev tools (play): F1 panel (save states, pause/step/speed, cheats, skip and warp), F5 save, F6 load; --devtools opens it at start.\n\
                    Controller: D-pad / left stick move, east face jump, south attack, west ninjutsu, Start pause.\n\
                    Headless defaults: demo/play = 600 scripted ticks; --input-script = the tape's length; viewer = 1 tick."
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    let stage_flags = options.part != 0 || options.round.is_some() || options.substage.is_some();
    let modes = [
        options.assets,
        options.play,
        options.sound,
        options.demo,
        options.boot,
    ];
    match modes.iter().filter(|&&mode| mode).count() {
        // The desktop starts at the SEGA logo; headless runs (tests, CI
        // smoke, input tapes) and stage selections keep starting in play.
        0 if !options.headless
            && !stage_flags
            && !options.scripted
            && options.input_script.is_none() =>
        {
            options.boot = true
        }
        0 => options.play = true,
        1 => {}
        _ => return Err("--assets, --play, --boot, --sound and --demo are separate modes".into()),
    }
    if options.boot {
        if stage_flags {
            return Err(
                "--boot starts at the SEGA logo; use --play with --round/--substage/--part".into(),
            );
        }
        // Boot is play mode that begins in the front end.
        options.play = true;
    }
    if options.rom.is_some() && !options.assets && !options.play && !options.sound {
        return Err("--rom requires --assets, --play or --sound".into());
    }
    let rom_path = rom_source::resolve(options.rom.as_deref());
    let read_rom = || rom_source::read(&rom_path);
    // Resolve before loading any scenes. The picker owns a temporary SDL UI,
    // which is dropped before the game initializes its own window and audio.
    let rom_data = if options.assets || options.play || options.sound {
        match read_rom() {
            Ok(data) => Some(data),
            Err(error) => {
                #[cfg(feature = "desktop")]
                if !options.headless {
                    match desktop::pick_rom(&rom_path, &error)? {
                        Some(data) => Some(data),
                        None => return Ok(()),
                    }
                } else {
                    return Err(format!(
                        "{error}\nSupply a US ROM with --rom FILE or NINJA_MASTER_ROM."
                    ));
                }
                #[cfg(not(feature = "desktop"))]
                return Err(format!(
                    "{error}\nSupply a US ROM with --rom FILE or NINJA_MASTER_ROM."
                ));
            }
        }
    } else {
        None
    };
    let read_rom = || {
        rom_data
            .clone()
            .ok_or_else(|| "This mode has no ROM.".to_owned())
    };
    if options.sound_cue.is_some() || options.stem != Stem::Mix {
        let (Some(command), true, true, Some(wav)) = (
            options.sound_cue,
            options.headless,
            options.sound,
            options.wav.as_ref(),
        ) else {
            return Err(
                "--sound-cue/--stem require --headless --sound --sound-cue HEX --wav FILE".into(),
            );
        };
        let ticks =
            u32::try_from(options.frames.unwrap_or(180)).map_err(|_| "--frames too large")?;
        let pcm = render_cue(&read_rom()?, command, ticks, 48_000, options.stem)?;
        return write_wav(wav, &pcm);
    }
    if !options.assets
        && (options.view.is_some()
            || options.page.is_some()
            || options.palette.is_some()
            || options.camera.is_some())
    {
        return Err("asset options require --assets".into());
    }
    if options.asset_report.is_some() && !options.assets && !options.play && !options.sound {
        return Err("--asset-report requires --assets, --play or --sound".into());
    }
    if options.assets && options.scripted {
        return Err("--scripted applies only to the engine demo or playable slice".into());
    }
    let mut effects = [true; 4];
    for name in &options.disabled_effects {
        let index = ["palette", "parallax", "priority", "vegetation"]
            .iter()
            .position(|n| n == name)
            .ok_or("unknown effect")?;
        effects[index] = false;
    }
    if stage_flags && !options.assets && !options.play {
        return Err("--part, --round and --substage require --assets or --play".into());
    }
    let stage = ninja_master_native::stage::StageId::new(
        options.round.unwrap_or(1),
        options.substage.unwrap_or(1),
        u8::try_from(options.part).map_err(|_| "invalid part")?,
    );
    let rom = if options.assets || options.play || options.sound {
        read_rom()?
    } else {
        Vec::new()
    };
    let mut scene = if options.assets || options.sound {
        let mut viewer = Viewer::new(
            StageAssets::load_stage(&rom, stage)?,
            options.view.unwrap_or_default(),
        );
        viewer.effects_enabled = effects;
        if let Some(page) = options.page {
            if page == 0 || page > viewer.pages() {
                return Err(format!("page must be 1..{} for this view", viewer.pages()));
            }
            viewer.page = page - 1;
        }
        if let Some(palette) = options.palette {
            if palette > 3 {
                return Err("palette must be 0..3".into());
            }
            viewer.palette = palette;
        }
        if let Some((x, y)) = options.camera {
            let (width, height) = viewer.assets.data.pixel_size();
            let max_x = (width - ninja_master_native::WIDTH as i32).max(0);
            let max_y = (height - 192).max(0);
            if !(0..=max_x).contains(&x) || !(0..=max_y).contains(&y) {
                return Err(format!("camera must be X=0..{max_x}, Y=0..{max_y}"));
            }
            viewer.x = x;
            viewer.y = y;
        }
        if let Some(path) = &options.asset_report {
            std::fs::write(path, viewer.assets.report())
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Scene::Assets(Box::new(viewer))
    } else if options.play {
        let mut game = (!options.boot || options.asset_report.is_some())
            .then(|| Game::load_stage(&rom, stage))
            .transpose()?;
        if let (Some(path), Some(game)) = (&options.asset_report, &game) {
            let report = game.report() + &SoundAssets::load(&rom)?.report();
            std::fs::write(path, report).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        let first: Box<dyn Mode> = match game.take() {
            Some(mut game) if !options.boot => {
                game.set_effects(effects);
                Box::new(game)
            }
            _ => {
                let mut front = Front::load(&rom)?;
                front.effects = effects;
                Box::new(SegaLogo::new(front))
            }
        };
        Scene::Play(Box::new(Modes::new(first)))
    } else {
        Scene::Demo(Demo::default())
    };
    if let Some(dir) = &options.dump_frames {
        if !options.headless {
            return Err("--dump-frames requires --headless".into());
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if options.wav.is_some() && (!options.headless || (!options.sound && !options.play)) {
        return Err("--wav requires --headless with --sound or --play".into());
    }
    let mut soundtrack = if options.sound || options.play {
        let mut player = SoundPlayer::new(&rom, 48000)?;
        // Play mode starts the stage's own music through Game::take_music.
        if !options.play {
            player.play_music(FOREST_MUSIC)?;
        }
        Some(player)
    } else {
        None
    };
    if (options.input_script.is_some() || options.trace.is_some())
        && (!options.headless || !options.play)
    {
        return Err("--input-script and --trace require --headless --play".into());
    }
    let replay = options
        .input_script
        .as_ref()
        .map(|path| read_input_script(path))
        .transpose()?;
    let mut trace = options
        .trace
        .as_ref()
        .map(File::create)
        .transpose()
        .map_err(|e| e.to_string())?
        .map(BufWriter::new);
    if let Some(file) = &mut trace {
        writeln!(
            file,
            "tick,part,x_fixed,y_fixed,vy,camera_x,camera_y,grounded,complete,round,substage,health,score,lives"
        )
        .map_err(|e| e.to_string())?;
    }
    let mut pcm = Vec::<i16>::new();
    let mut samples = [0i16; TICK_SAMPLES];
    let mut frame = Frame::new(if options.widescreen {
        ninja_master_native::WIDE_WIDTH
    } else {
        ninja_master_native::WIDTH
    });
    let ticks;
    if options.headless {
        ticks = options.frames.unwrap_or(match &replay {
            Some(replay) => replay.len() as u64,
            None if options.assets => 1,
            None => 600,
        });
        for tick in 0..ticks {
            let input = if let Some(replay) = &replay {
                replay.get(tick as usize).copied().unwrap_or_default()
            } else if options.sound && options.scripted {
                sound_demo_input(tick)
            } else {
                scene.scripted_input(tick)
            };
            scene.tick(input, frame.width());
            if let (Some(file), Scene::Play(modes)) = (&mut trace, &scene) {
                let written = match modes.game() {
                    Some(game) => writeln!(
                        file,
                        "{tick},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                        game.part(),
                        game.player.x,
                        game.player.y,
                        game.player.velocity_y,
                        game.player.camera_x,
                        game.player.camera_y,
                        game.player.grounded,
                        game.complete,
                        game.stage_id.round,
                        game.stage_id.substage,
                        game.health().0,
                        game.score(),
                        game.lives()
                    ),
                    // Front-end modes hold no gameplay; only the ending
                    // marks the run complete.
                    None => writeln!(file, "{tick},,,,,,,,{},,,,,", modes.name() == "Ending"),
                };
                written.map_err(|e| e.to_string())?;
            }
            let rendered = render_sound_tick(
                &mut soundtrack,
                &mut scene,
                input,
                options.sound,
                &mut samples,
            )?;
            if rendered && options.wav.is_some() {
                pcm.extend_from_slice(&samples);
            }
            if let Some(dir) = &options.dump_frames {
                scene.draw(&mut frame);
                let path = dir.join(format!("frame{:06}.ppm", tick + 1));
                frame
                    .write_ppm(File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?)
                    .map_err(|e| e.to_string())?;
            }
        }
        scene.draw(&mut frame);
    } else {
        #[cfg(feature = "desktop")]
        {
            ticks = desktop::run(&options, &rom, &mut scene, &mut frame, &mut soundtrack)?;
        }
        #[cfg(not(feature = "desktop"))]
        {
            return Err(
                "desktop support is disabled; use --headless or enable the desktop feature".into(),
            );
        }
    }
    if let Some(file) = &mut trace {
        file.flush().map_err(|e| e.to_string())?;
    }
    if let Some(path) = &options.wav {
        write_wav(path, &pcm)?;
    }
    if let Some(path) = options.capture {
        frame
            .write_ppm(File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?)
            .map_err(|e| e.to_string())?;
    }
    match scene {
        Scene::Demo(demo) => println!(
            "Engine demo: ticks={ticks} frame={:016x} position=({}, {})",
            frame.checksum(),
            demo.x,
            demo.y
        ),
        Scene::Play(modes) => match modes.game() {
            Some(game) => println!(
                "Playable slice: round={}-{} part={} ticks={ticks} frame={:016x} position=({}, {}) camera=({}, {}) grounded={} music=${:02x} mode={}",
                game.stage_id.round,
                game.stage_id.substage,
                game.part(),
                frame.checksum(),
                game.player.position().0,
                game.player.position().1,
                game.player.camera_x,
                game.player.camera_y,
                game.player.grounded,
                game.music(),
                modes.name()
            ),
            None => println!(
                "Playable slice: ticks={ticks} frame={:016x} mode={}",
                frame.checksum(),
                modes.name()
            ),
        },
        Scene::Assets(_) if options.sound => {
            println!("Sound demo: ticks={ticks} frame={:016x}", frame.checksum())
        }
        Scene::Assets(viewer) => println!(
            "Asset viewer: round={}-{} part={} view={:?} page={} ticks={ticks} frame={:016x} camera=({}, {})",
            viewer.assets.data.id.round,
            viewer.assets.data.id.substage,
            viewer.assets.data.id.part_index(),
            viewer.view,
            viewer.page + 1,
            frame.checksum(),
            viewer.x,
            viewer.y
        ),
    }
    Ok(())
}

/// Advance the soundtrack by one tick into `samples`; false when there is no soundtrack.
fn render_sound_tick(
    sound: &mut Option<SoundPlayer>,
    scene: &mut Scene,
    input: Input,
    demo: bool,
    samples: &mut [i16; TICK_SAMPLES],
) -> Result<bool, String> {
    let Some(sound) = sound else {
        return Ok(false);
    };
    // Start restarts the demo and viewer scenes; in play it pauses, and the
    // pause requests its own PAUSE / MUSIC RESTART effects.
    if input.pressed(Button::Start) && !matches!(scene, Scene::Play(_)) {
        sound.stop();
        sound.play_music(FOREST_MUSIC)?;
    }
    if demo && input.pressed(Button::Attack) {
        sound.play_sfx(0x9d)?;
    }
    if let Scene::Play(modes) = scene {
        // Native: the pause window lowers the music.
        sound.set_music_gain(modes.music_gain());
        // Stage loads, respawns, the camera lock and the transition screens.
        // Every request goes through the driver's queue in the order the game
        // made it, so a stop or fade cannot cancel an effect requested after it.
        for command in modes.take_audio().commands {
            sound.play_command(command);
        }
    }
    sound.render(samples)?;
    let communication = sound.take_communication();
    if communication != 0
        && let Scene::Play(modes) = scene
    {
        modes.sound_communication(communication);
    }
    Ok(true)
}
/// 48 kHz stereo WAV.
fn write_wav(path: &std::path::Path, pcm: &[i16]) -> Result<(), String> {
    std::fs::write(path, wav_bytes(pcm, 48_000)?).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_input_script(path: &std::path::Path) -> Result<Vec<Input>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    ninja_master_native::input::parse_tape(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn sound_demo_input(tick: u64) -> Input {
    let mask = if tick.is_multiple_of(120) {
        Button::Attack.mask()
    } else {
        0
    };
    Input {
        held: mask,
        pressed: mask,
    }
}
