mod art;
// OpenGL/SDL interop is confined to this presentation module.
#[allow(unsafe_code)]
mod crt;
mod gallery;
mod gui;
mod panel;
mod rom_picker;

pub(crate) use rom_picker::pick_rom;

use crate::{Options, Scene};
use ninja_master_native::{
    HEIGHT, WIDTH,
    art_pack::{ArtPack, export_template, library_directory, resolve_selection, selection_value},
    audio_output::{AudioConsumer, AudioProducer, audio_ring},
    devtools::DevTools,
    input::{Button, InputLatch},
    render::Frame,
    settings::{self, Bindings, Device, Rebinder, Settings},
    timing::{FixedClock, RefreshProbe},
};
use sdl2::{
    audio::{AudioCallback, AudioDevice, AudioSpecDesired},
    controller::{Axis, Button as PadButton, GameController},
    event::{Event, WindowEvent},
    keyboard::Scancode,
    pixels::PixelFormatEnum,
    rect::Rect,
    render::WindowCanvas,
    video::FullscreenType,
};
use std::{
    collections::HashSet,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

/// Bindings resolved to SDL values; indexed by `Button as usize`.
struct InputMap {
    keys: [Vec<Scancode>; Button::COUNT],
    pads: [Vec<PadButton>; Button::COUNT],
}

impl InputMap {
    /// Unknown names are reported and dropped. A button whose configured
    /// names are all unknown falls back to its default binding.
    fn new(settings: &Settings) -> Self {
        fn resolve<T>(
            bindings: &Bindings,
            defaults: &Bindings,
            device: &str,
            parse: impl Fn(&str) -> Option<T>,
        ) -> [Vec<T>; Button::COUNT] {
            std::array::from_fn(|index| {
                let button = Button::ALL[index];
                let names = bindings.get(button);
                let mut resolved = Vec::new();
                for name in names {
                    match parse(name.as_str()) {
                        Some(value) => resolved.push(value),
                        None => eprintln!("settings: unknown {device} input {name:?} ignored"),
                    }
                }
                if resolved.is_empty() && !names.is_empty() {
                    resolved = defaults
                        .get(button)
                        .iter()
                        .filter_map(|name| parse(name.as_str()))
                        .collect();
                }
                resolved
            })
        }
        Self {
            keys: resolve(
                &settings.keyboard,
                &Bindings::default_keyboard(),
                "keyboard",
                Scancode::from_name,
            ),
            pads: resolve(
                &settings.gamepad,
                &Bindings::default_gamepad(),
                "gamepad",
                PadButton::from_string,
            ),
        }
    }

    fn keyboard_mask(&self, keys: &HashSet<Scancode>) -> u16 {
        Button::ALL.iter().fold(0, |mask, button| {
            mask | if self.keys[*button as usize]
                .iter()
                .any(|key| keys.contains(key))
            {
                button.mask()
            } else {
                0
            }
        })
    }

    fn controller_mask(&self, pad: &GameController) -> u16 {
        let mut mask = Button::ALL.iter().fold(0, |mask, button| {
            mask | if self.pads[*button as usize]
                .iter()
                .any(|binding| pad.button(*binding))
            {
                button.mask()
            } else {
                0
            }
        });
        // The left stick always mirrors the D-pad; it is not rebindable.
        let x = pad.axis(Axis::LeftX);
        let y = pad.axis(Axis::LeftY);
        for (active, button) in [
            (x < -16_000, Button::Left),
            (x > 16_000, Button::Right),
            (y < -16_000, Button::Up),
            (y > 16_000, Button::Down),
        ] {
            if active {
                mask |= button.mask();
            }
        }
        mask
    }

    fn mask(&self, keys: &HashSet<Scancode>, pads: &[GameController], controller: &str) -> u16 {
        pads.iter()
            .filter(|pad| controller.is_empty() || pad.name() == controller)
            .fold(self.keyboard_mask(keys), |mask, pad| {
                mask | self.controller_mask(pad)
            })
    }
}

/// Front-end keys that are never bound to pad controls.
const RESERVED_KEYS: [Scancode; 11] = [
    Scancode::Escape,
    Scancode::Backspace,
    Scancode::F1,
    Scancode::F2,
    Scancode::F5,
    Scancode::F6,
    Scancode::F7,
    Scancode::F8,
    Scancode::F9,
    Scancode::F10,
    Scancode::F11,
];

fn save_settings(path: Option<&Path>, settings: &Settings) {
    if let Some(path) = path
        && let Err(error) = settings.save(path)
    {
        eprintln!("settings: could not save {}: {error}", path.display());
    }
}

fn set_title(canvas: &mut WindowCanvas, title: &str) -> Result<(), String> {
    canvas
        .window_mut()
        .set_title(title)
        .map_err(|error| error.to_string())
}

/// The largest whole multiple of the frame that fits the window, centred.
fn game_rect(canvas: &WindowCanvas, settings: &Settings) -> Result<Rect, String> {
    let (width, height) = canvas.output_size()?;
    Ok(presentation_rect(width, height, settings))
}

fn art_output_scale(rect: Rect, width: usize) -> usize {
    (rect.width() as usize)
        .div_ceil(width)
        .max((rect.height() as usize).div_ceil(HEIGHT))
        .max(1)
}

fn fit_art_to_window(
    frame: &mut Frame,
    canvas: &WindowCanvas,
    settings: &Settings,
) -> Result<bool, String> {
    Ok(frame.set_output_scale(art_output_scale(
        game_rect(canvas, settings)?,
        frame.width(),
    )))
}

/// Change window modes only after presenting the frame, when SDL has no queued
/// game or panel draws using the previous drawable dimensions.
fn apply_window_settings(canvas: &mut WindowCanvas, settings: &Settings) -> Result<(), String> {
    let fullscreen = if settings.fullscreen {
        FullscreenType::Desktop
    } else {
        FullscreenType::Off
    };
    if canvas.window().fullscreen_state() != fullscreen {
        canvas.window_mut().set_fullscreen(fullscreen)?;
    }
    if !settings.fullscreen {
        let size = (
            settings.frame_width() as u32 * settings.scale,
            HEIGHT as u32 * settings.scale,
        );
        if canvas.window().size() != size {
            canvas
                .window_mut()
                .set_size(size.0, size.1)
                .map_err(|error| error.to_string())?;
        }
    }
    // Discard the panel's clipping and the old window's viewport. Windows can
    // resize the drawable synchronously while changing fullscreen styles.
    canvas.set_clip_rect(None);
    canvas.set_viewport(None);
    Ok(())
}

fn presentation_rect(width: u32, height: u32, settings: &Settings) -> Rect {
    if !settings.widescreen && settings.aspect == 2 {
        return Rect::new(0, 0, width.max(1), height.max(1));
    }
    let base_width = if settings.widescreen {
        settings.frame_width() as f64
    } else if settings.aspect == 1 {
        HEIGHT as f64 * 4.0 / 3.0
    } else {
        WIDTH as f64
    };
    let mut scale = (width as f64 / base_width).min(height as f64 / HEIGHT as f64);
    if settings.integer_scale && scale >= 1.0 {
        scale = scale.floor();
    }
    let (w, h) = (
        (base_width * scale).round().max(1.0) as u32,
        (HEIGHT as f64 * scale).round().max(1.0) as u32,
    );
    Rect::new(
        (width as i32 - w as i32) / 2,
        (height as i32 - h as i32) / 2,
        w,
        h,
    )
}

/// Clock-paced fallback runs the tick early and presents on its deadline.
const PRESENT_LEAD: Duration = Duration::from_millis(5);

struct PullAudio(AudioConsumer);

impl AudioCallback for PullAudio {
    type Channel = f32;

    fn callback(&mut self, output: &mut [f32]) {
        self.0.render(output);
    }
}

struct AudioOutput {
    // Keep the device alive while the producer feeds its callback.
    device: AudioDevice<PullAudio>,
    producer: AudioProducer,
    demo_remaining: usize,
}

impl AudioOutput {
    fn push_tick(&mut self, samples: &[i16], rendered: bool, gain: f32, floats: &mut Vec<f32>) {
        floats.clear();
        for frame in samples.as_chunks::<2>().0 {
            let tone = if self.demo_remaining > 0 {
                let elapsed = 2400 - self.demo_remaining;
                let phase = elapsed as f32 * 660.0 * std::f32::consts::TAU / 48_000.0;
                let tone = phase.sin() * self.demo_remaining as f32 / 2400.0 * 0.08;
                self.demo_remaining -= 1;
                tone
            } else {
                0.0
            };
            floats.extend(frame.iter().map(|sample| {
                let value = if rendered {
                    f32::from(*sample) / 32768.0
                } else {
                    0.0
                };
                (value + tone) * gain
            }));
        }
        self.producer.push(floats);
    }
}

fn open_audio(sdl: &sdl2::Sdl) -> Result<AudioOutput, String> {
    let audio = sdl.audio()?;
    let mut producer = None;
    let device = audio.open_playback(
        None,
        &AudioSpecDesired {
            freq: Some(48_000),
            channels: Some(2),
            samples: Some(512),
        },
        |spec| {
            let (writer, reader) = audio_ring(spec.freq as u32);
            producer = Some(writer);
            PullAudio(reader)
        },
    )?;
    if device.spec().channels != 2 {
        return Err("audio device must support stereo output".to_string());
    }
    device.resume();
    Ok(AudioOutput {
        device,
        producer: producer.expect("SDL initialized the audio callback"),
        demo_remaining: 0,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pacing {
    Display,
    Clock,
}

pub(super) fn run(
    options: &Options,
    rom: &[u8],
    scene: &mut Scene,
    frame: &mut Frame,
    soundtrack: &mut Option<ninja_master_native::sound::SoundPlayer>,
) -> Result<u64, String> {
    let config_path = settings::config_path();
    let mut settings = match &config_path {
        Some(path) => match Settings::load(path) {
            Ok((settings, warnings)) => {
                for warning in warnings {
                    eprintln!("{}: {warning}", path.display());
                }
                settings
            }
            Err(error) => {
                eprintln!(
                    "settings: could not read {}: {error}; using defaults",
                    path.display()
                );
                Settings::default()
            }
        },
        None => {
            eprintln!(
                "settings: no config directory; set {} to persist",
                settings::CONFIG_ENV
            );
            Settings::default()
        }
    };
    if options.widescreen {
        settings.widescreen = true;
    }
    let mut map = InputMap::new(&settings);
    *frame = Frame::new(settings.frame_width());
    let mut art_status = String::new();
    if !settings.art_pack.is_empty() {
        match ArtPack::load(&resolve_selection(&library_directory(), &settings.art_pack)) {
            Ok(pack) => frame.set_art_pack(Some(Rc::new(pack))),
            Err(error) => art_status = format!("Could not load artwork: {error}"),
        }
    }
    frame.smooth_wave_effects = settings.smooth_wave_effects;

    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let base_title = if options.assets {
        "Ninja Master — Assets"
    } else if options.boot || options.play {
        "Ninja Master"
    } else {
        "Ninja Master — Demo"
    };
    let mut builder = video.window(
        base_title,
        settings.frame_width() as u32 * settings.scale,
        HEIGHT as u32 * settings.scale,
    );
    // A scaled desktop (macOS, Wayland) would otherwise stretch the window's
    // backing store with bilinear filtering; `game_rect` sizes the frame in
    // real pixels from `output_size`.
    builder.position_centered().resizable().allow_highdpi();
    if settings.fullscreen {
        builder.fullscreen_desktop();
    }
    let mut window = builder.build().map_err(|error| error.to_string())?;
    set_icon(&mut window)?;
    sdl2::hint::set("SDL_RENDER_SCALE_QUALITY", "0");
    // Prefer SDL's compatibility OpenGL backend for the CRT pass. An explicit
    // SDL_RENDER_DRIVER override still wins; other backends retain flat output.
    if std::env::var_os("SDL_RENDER_DRIVER").is_none() {
        sdl2::hint::set("SDL_RENDER_DRIVER", "opengl");
    }
    // The frame is scaled by `game_rect`; the window's own resolution is
    // left for the F1 dev panel.
    // NINJA_VSYNC=0 provides an explicit clock-paced path for VRR displays.
    let request_vsync = std::env::var("NINJA_VSYNC").as_deref() != Ok("0");
    let builder = window.into_canvas();
    let builder = if request_vsync {
        builder.present_vsync()
    } else {
        builder
    };
    let mut canvas = builder.build().map_err(|error| error.to_string())?;
    fit_art_to_window(frame, &canvas, &settings)?;
    let mut events = sdl.event_pump()?;
    // rust-sdl2 has no safe runtime vsync setter. Probe before long-lived
    // textures exist, then rebuild the renderer if a clock is required.
    // Calibration displays the initial scene for about two seconds.
    let pacing = if request_vsync {
        scene.draw(frame);
        let locked = {
            let creator = canvas.texture_creator();
            let mut texture = creator
                .create_texture_streaming(
                    PixelFormatEnum::RGB24,
                    frame.width() as u32,
                    HEIGHT as u32,
                )
                .map_err(|error| error.to_string())?;
            texture
                .update(None, frame.rgb(), frame.width() * 3)
                .map_err(|error| error.to_string())?;
            let mut probe = RefreshProbe::default();
            let mut last = None;
            loop {
                if events
                    .poll_iter()
                    .any(|event| matches!(event, Event::Quit { .. }))
                {
                    return Ok(0);
                }
                canvas.set_draw_color(sdl2::pixels::Color::BLACK);
                canvas.clear();
                canvas.copy(&texture, None, game_rect(&canvas, &settings)?)?;
                let before = Instant::now();
                canvas.present();
                let after = Instant::now();
                if let Some(last) = last
                    && let Some(locked) = probe.observe(after.duration_since(last), after - before)
                {
                    break locked;
                }
                last = Some(after);
            }
        };
        if locked {
            Pacing::Display
        } else {
            canvas = canvas
                .into_window()
                .into_canvas()
                .build()
                .map_err(|error| error.to_string())?;
            Pacing::Clock
        }
    } else {
        Pacing::Clock
    };
    let mut last_present = None;
    // Opt-in CSV allows hardware cadence and callback underruns to be checked
    // with the real compositor, rather than inferred from simulation timing.
    let timing_log = std::env::var_os("NINJA_PRESENT_LOG").is_some();
    let timing_start = Instant::now();
    if timing_log {
        eprintln!("present_us,period_us,ticks,pacing,audio_underrun_frames");
    }
    let creator = canvas.texture_creator();
    let mut texture = creator
        .create_texture_streaming(
            PixelFormatEnum::RGB24,
            frame.output_width() as u32,
            frame.output_height() as u32,
        )
        .map_err(|error| error.to_string())?;
    let ini = config_path
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| dir.join("imgui.ini"));
    let mut gui = gui::Gui::new(&creator, ini)?;
    let mut crt = crt::Crt::new(&video, &mut canvas);
    let mut panel = panel::Panel::default();
    panel.set_art_selection(if frame.art_pack().is_some() {
        &settings.art_pack
    } else {
        ""
    });
    panel.gallery.set_art_pack(frame.art_pack());
    let mut show_panel = options.devtools;
    let mut tools = matches!(scene, Scene::Play(_)).then(|| DevTools::new(rom));
    if let (Some(tools), Scene::Play(modes)) = (&mut tools, &mut *scene) {
        let effects = ["palette", "parallax", "priority", "vegetation"]
            .map(|name| !options.disabled_effects.iter().any(|n| n == name));
        let dir = config_path
            .as_deref()
            .and_then(Path::parent)
            .map(|dir| dir.join("saves"));
        tools.enable_disk_saves(modes, effects, dir);
    }

    let controllers = sdl.game_controller()?;
    let mut pads = Vec::new();
    for id in 0..controllers.num_joysticks()? {
        if controllers.is_game_controller(id)
            && let Ok(pad) = controllers.open(id)
        {
            pads.push(pad);
        }
    }
    let mut audio = if options.mute || options.assets {
        None
    } else {
        match open_audio(&sdl) {
            Ok(device) => Some(device),
            Err(error) => {
                eprintln!("Audio unavailable; running muted: {error}");
                None
            }
        }
    };

    let mut keys = HashSet::new();
    let mut physical_keys = HashSet::new();
    let mut latch = InputLatch::default();
    let mut clock = FixedClock::default();
    // Ticks fall due PRESENT_LEAD before their frames are presented.
    clock.advance(PRESENT_LEAD);
    let mut previous = Instant::now();
    let mut total_ticks = 0;
    let mut focused = canvas.window().has_input_focus();
    if !focused && let Some(audio) = &audio {
        audio.device.pause();
    }
    let mut rebinder: Option<Rebinder> = None;
    let mut capture: Option<(Button, Device)> = None;
    let mut release_inputs = false;
    let mut text_input_active = false;
    let mut thumbnail_versions = [None; ninja_master_native::devtools::SLOTS];
    let mut shown_title = base_title.to_string();
    // Escape in play opens the pause window (a Start press on the next
    // tick); quitting is the title menu's QUIT item or closing the window.
    let mut escape_pause = false;
    let mut samples = [0i16; crate::TICK_SAMPLES];
    let mut floats = Vec::with_capacity(crate::TICK_SAMPLES);
    let mut inactive_pixels = Vec::with_capacity(frame.width() * HEIGHT * 3);
    // The frame changes only when the scene ticks; the window is repainted
    // then, or when the window system asks for it.
    scene.draw(frame);
    let mut upload = true;
    let mut repaint = true;

    'running: loop {
        let mut status = None;
        let mut settings_changed = false;
        let mut window_settings_changed = false;
        let mut had_event = false;
        for event in events.poll_iter() {
            had_event = true;
            match event {
                Event::KeyDown {
                    scancode: Some(key),
                    ..
                } => {
                    physical_keys.insert(key);
                }
                Event::KeyUp {
                    scancode: Some(key),
                    ..
                } => {
                    physical_keys.remove(&key);
                }
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => {
                    physical_keys.clear();
                }
                _ => {}
            }
            if let Some((button, device)) = capture {
                let binding = match event {
                    Event::KeyDown {
                        scancode: Some(key),
                        repeat: false,
                        ..
                    } if focused && key != Scancode::F1 => {
                        if key == Scancode::Escape {
                            capture = None;
                            release_inputs = true;
                            continue;
                        }
                        if key == Scancode::Backspace {
                            Some(None)
                        } else if device == Device::Keyboard
                            && !RESERVED_KEYS.contains(&key)
                            && !key.name().contains(',')
                            && !key.name().is_empty()
                        {
                            Some(Some(key.name().to_string()))
                        } else {
                            None
                        }
                    }
                    Event::ControllerButtonDown { button, which, .. }
                        if focused
                            && device == Device::Gamepad
                            && pads.iter().any(|pad| {
                                pad.instance_id() == which
                                    && (settings.controller.is_empty()
                                        || pad.name() == settings.controller)
                            }) =>
                    {
                        Some(Some(button.string()))
                    }
                    _ => None,
                };
                if let Some(name) = binding {
                    let bindings = if device == Device::Keyboard {
                        &mut settings.keyboard
                    } else {
                        &mut settings.gamepad
                    };
                    if let Some(name) = name {
                        bindings.assign(button, &name);
                    } else {
                        bindings.set(button, Vec::new());
                    }
                    capture = None;
                    release_inputs = true;
                    keys.clear();
                    latch.clear();
                    map = InputMap::new(&settings);
                    settings_changed = true;
                    continue;
                }
            }
            if show_panel {
                gui.handle_event(&event);
                // A focused ImGui text field keeps typed keys from the game.
                if capture.is_none()
                    && rebinder.is_none()
                    && gui.wants_keyboard()
                    && matches!(event, Event::KeyDown { scancode: Some(key), .. } if !RESERVED_KEYS.contains(&key) || matches!(key, Scancode::Escape | Scancode::Backspace))
                {
                    continue;
                }
            }
            match event {
                Event::Quit { .. } => break 'running,
                Event::ControllerButtonDown { button, .. } if focused && rebinder.is_some() => {
                    if let Some(session) = rebinder.as_mut() {
                        session.assign(Device::Gamepad, &button.string());
                    }
                }
                Event::KeyDown {
                    scancode: Some(key),
                    repeat,
                    ..
                } if focused && rebinder.is_some() => {
                    keys.insert(key);
                    match key {
                        _ if repeat => {}
                        Scancode::Escape => {
                            rebinder = None;
                            // Keys held during the capture must not act as presses.
                            latch.set_held(map.mask(&keys, &pads, &settings.controller));
                            latch.take_tick();
                            status = Some(String::new());
                        }
                        Scancode::Backspace => {
                            if let Some(session) = rebinder.as_mut() {
                                session.skip();
                            }
                        }
                        key if RESERVED_KEYS.contains(&key)
                            || key.name().is_empty()
                            || key.name().contains(',') => {}
                        key => {
                            if let Some(session) = rebinder.as_mut() {
                                session.assign(Device::Keyboard, key.name());
                            }
                        }
                    }
                }
                Event::KeyDown {
                    scancode: Some(key),
                    repeat,
                    ..
                } if focused => {
                    keys.insert(key);
                    match key {
                        _ if repeat => {}
                        Scancode::Escape if options.play => escape_pause = true,
                        Scancode::Escape => break 'running,
                        Scancode::F1 => {
                            show_panel = !show_panel;
                            if capture.take().is_some() {
                                release_inputs = true;
                            }
                            keys.clear();
                            latch.clear();
                            status = Some(match &tools {
                                Some(tools) if tools.paused && !show_panel => {
                                    "PAUSED by dev tools (F1)".into()
                                }
                                _ => String::new(),
                            });
                            repaint = true;
                        }
                        Scancode::F5 | Scancode::F6 => {
                            if let (Some(tools), Scene::Play(modes)) = (tools.as_mut(), &mut *scene)
                            {
                                if key == Scancode::F5 {
                                    tools.save(modes, total_ticks);
                                } else {
                                    tools.load(modes);
                                    scene.draw(frame);
                                    upload = true;
                                }
                                status = Some(tools.status.clone());
                            }
                        }
                        Scancode::F2 => {
                            capture = None;
                            rebinder = Some(Rebinder::new(&settings));
                        }
                        Scancode::F11 => {
                            settings.fullscreen = !settings.fullscreen;
                            window_settings_changed = true;
                            settings_changed = true;
                            repaint = true;
                        }
                        Scancode::F7 | Scancode::F8 => {
                            let scale = if key == Scancode::F7 {
                                settings.scale.saturating_sub(1)
                            } else {
                                settings.scale + 1
                            };
                            settings.scale = scale.clamp(settings::MIN_SCALE, settings::MAX_SCALE);
                            if !settings.fullscreen {
                                window_settings_changed = true;
                            }
                            status = Some(format!("scale {}x", settings.scale));
                            settings_changed = true;
                            repaint = true;
                        }
                        Scancode::F9 | Scancode::F10 => {
                            settings.volume = if key == Scancode::F9 {
                                settings.volume.saturating_sub(10)
                            } else {
                                settings.volume.saturating_add(10).min(settings::MAX_VOLUME)
                            };
                            status = Some(format!("volume {}%", settings.volume));
                            settings_changed = true;
                        }
                        _ => {}
                    }
                }
                Event::KeyUp {
                    scancode: Some(key),
                    ..
                } => {
                    keys.remove(&key);
                }
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Ok(pad) = controllers.open(which)
                        && !pads
                            .iter()
                            .any(|existing| existing.instance_id() == pad.instance_id())
                    {
                        pads.push(pad);
                    }
                }
                Event::ControllerDeviceRemoved { which, .. } => {
                    pads.retain(|pad| pad.instance_id() != which);
                }
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => {
                    focused = false;
                    keys.clear();
                    latch.clear();
                    escape_pause = false;
                    if let Some(audio) = &audio {
                        audio.device.pause();
                    }
                    upload = true;
                }
                Event::Window {
                    win_event: WindowEvent::FocusGained,
                    ..
                } => {
                    focused = true;
                    // Discard time spent away, including a suspended event loop.
                    clock = FixedClock::default();
                    clock.advance(PRESENT_LEAD);
                    previous = Instant::now();
                    if let Some(audio) = &audio {
                        audio.device.resume();
                    }
                    upload = true;
                }
                Event::Window { .. } => repaint = true,
                _ => {}
            }
            if rebinder.as_ref().is_some_and(Rebinder::is_done) {
                if let Some(session) = rebinder.take() {
                    session.apply(&mut settings);
                }
                map = InputMap::new(&settings);
                // Controls still held from the capture must not act as presses.
                latch.set_held(map.mask(&keys, &pads, &settings.controller));
                latch.take_tick();
                status = Some("controls saved".into());
                settings_changed = true;
            }
            // Preserve a press even when down and up events arrive before the
            // next simulation tick.
            if focused && rebinder.is_none() {
                latch.set_held(map.mask(&keys, &pads, &settings.controller));
            }
        }

        if settings_changed {
            save_settings(config_path.as_deref(), &settings);
        }
        let title = match (&rebinder, status) {
            (Some(session), _) => session.prompt().map(|(step, total, label)| {
                format!(
                    "Ninja Master — Bind {label} ({step}/{total}): press key/button | Backspace: keep | Esc: cancel"
                )
            }),
            (None, Some(status)) if status.is_empty() => Some(base_title.to_string()),
            (None, Some(status)) => Some(format!("{base_title} | {status}")),
            (None, None) => None,
        };
        if let Some(title) = title
            && title != shown_title
        {
            set_title(&mut canvas, &title)?;
            shown_title = title;
        }

        let now = Instant::now();
        let real_ticks = if pacing == Pacing::Display {
            clock.advance_display(now.duration_since(previous))
        } else {
            clock.advance(now.duration_since(previous))
        };
        previous = now;
        let ticks = match &mut tools {
            _ if !focused => 0,
            Some(tools) => tools.scale_ticks(real_ticks),
            None => real_ticks,
        };
        let mut ticked = false;
        if release_inputs && map.mask(&physical_keys, &pads, &settings.controller) == 0 {
            release_inputs = false;
        }
        let suppress_input = capture.is_some() || release_inputs;
        if !focused || rebinder.is_some() {
            // Focus suspension and binding capture leave scene and sound untouched.
            latch.take_tick();
        } else {
            if suppress_input {
                keys.clear();
                latch.clear();
            } else if focused {
                if show_panel && gui.wants_keyboard() {
                    keys.clear();
                }
                latch.set_held(map.mask(&keys, &pads, &settings.controller));
            }
            let gain = settings.gain();
            for _ in 0..ticks {
                let mut input = if options.scripted {
                    if options.sound {
                        crate::sound_demo_input(total_ticks)
                    } else {
                        scene.scripted_input(total_ticks)
                    }
                } else {
                    latch.take_tick()
                };
                if std::mem::take(&mut escape_pause) {
                    input.pressed |= Button::Start.mask();
                }
                if scene.tick(input, frame.width())
                    && let Some(audio) = &mut audio
                {
                    audio.demo_remaining = 2400;
                }
                if let (Some(tools), Scene::Play(modes)) = (&tools, &mut *scene) {
                    tools.after_tick(modes);
                }
                ticked = true;
                let rendered = crate::render_sound_tick(
                    soundtrack,
                    scene,
                    input,
                    options.sound,
                    &mut samples,
                )?;
                // The callback keeps draining through blocking present. Output
                // beyond the bounded backlog is dropped, but synthesis advances.
                if let Some(audio) = &mut audio {
                    audio.push_tick(&samples, rendered, gain, &mut floats);
                }
                total_ticks += 1;
                if options.frames.is_some_and(|limit| total_ticks >= limit)
                    || scene.quit_requested()
                {
                    break;
                }
            }
        }
        if scene.quit_requested() {
            break 'running;
        }

        if fit_art_to_window(frame, &canvas, &settings)? {
            texture = creator
                .create_texture_streaming(
                    PixelFormatEnum::RGB24,
                    frame.output_width() as u32,
                    frame.output_height() as u32,
                )
                .map_err(|error| error.to_string())?;
            scene.draw(frame);
            upload = true;
        }
        if ticked {
            scene.draw(frame);
            upload = true;
        }
        if std::mem::take(&mut upload) {
            let pixels = if focused {
                frame.presentation_rgb()
            } else {
                gray_out(frame.presentation_rgb(), &mut inactive_pixels);
                &inactive_pixels
            };
            let mut presentation_pixels = Vec::new();
            let crt_enabled = settings.crt && crt.is_ok();
            let pixels = if settings.scanlines == 0 || crt_enabled {
                pixels
            } else {
                scanlines(
                    pixels,
                    frame.output_width(),
                    frame.output_scale(),
                    settings.scanlines,
                    &mut presentation_pixels,
                );
                &presentation_pixels
            };
            texture
                .update(None, pixels, frame.output_width() * 3)
                .map_err(|error| error.to_string())?;
            if crt_enabled {
                crt.as_mut().unwrap().upload(
                    &mut canvas,
                    pixels,
                    frame.output_width(),
                    frame.output_height(),
                )?;
            }
            repaint = true;
        }
        // The panel redraws at the tick rate and on input.
        if show_panel && (real_ticks > 0 || had_event) {
            repaint = true;
        }
        if std::mem::take(&mut repaint) || pacing != Pacing::Clock {
            canvas.set_draw_color(sdl2::pixels::Color::BLACK);
            canvas.clear();
            let rect = game_rect(&canvas, &settings)?;
            if settings.crt
                && let Ok(crt) = &crt
            {
                crt.draw(
                    &mut canvas,
                    rect,
                    frame.width(),
                    settings.crt_curvature,
                    settings.scanlines,
                )?;
            } else {
                canvas.copy(&texture, None, rect)?;
            }
            if show_panel {
                if let Some(tools) = &tools {
                    if panel.gallery.prepare(rom, &tools.stages) {
                        gui.set_image(
                            &creator,
                            1,
                            panel.gallery.frame.presentation_rgb(),
                            panel.gallery.frame.output_width() as u32,
                            panel.gallery.frame.output_height() as u32,
                        )?;
                    }
                    for (slot, state) in tools.slots.iter().enumerate() {
                        if let Some(state) = state {
                            let version = (state.tick, state.frame_checksum());
                            if thumbnail_versions[slot] != Some(version) {
                                gui.set_image(
                                    &creator,
                                    slot + 2,
                                    &state.thumbnail,
                                    ninja_master_native::devtools::THUMB_WIDTH as u32,
                                    ninja_master_native::devtools::THUMB_HEIGHT as u32,
                                )?;
                                thumbnail_versions[slot] = Some(version);
                            }
                        } else {
                            thumbnail_versions[slot] = None;
                        }
                    }
                }
                let before = settings.clone();
                let controller_names: Vec<_> = pads.iter().map(GameController::name).collect();
                let axes = pads
                    .iter()
                    .find(|pad| settings.controller.is_empty() || pad.name() == settings.controller)
                    .map_or([0; 2], |pad| [pad.axis(Axis::LeftX), pad.axis(Axis::LeftY)]);
                let track = track_name(
                    rom,
                    soundtrack
                        .as_ref()
                        .and_then(ninja_master_native::sound::SoundPlayer::current_music),
                );
                let info = panel::PanelInfo {
                    crt_error: crt.as_ref().err().map(String::as_str),
                    art_name: &frame.art_pack().map_or_else(
                        || "Original".to_owned(),
                        |p| {
                            format!(
                                "{} ({}x, {} tiles, {} player frames)",
                                p.name,
                                p.scale,
                                p.replacement_count(),
                                p.player_frame_count()
                            )
                        },
                    ),
                    art_status: &art_status,
                    ticks: total_ticks,
                    controllers: &controller_names,
                    capture,
                    held: map.mask(&physical_keys, &pads, &settings.controller),
                    axes,
                    track: &track,
                    audio_available: audio.is_some(),
                    queued_audio: audio.as_ref().map_or(0, |a| a.producer.queued_frames()),
                    underruns: audio.as_ref().map_or(0, |a| a.producer.underrun_frames()),
                };
                let mut actions = panel::Actions::default();
                gui.draw(&mut canvas, |ui| {
                    actions = panel.show(ui, scene, tools.as_mut(), &mut settings, &info);
                })?;
                if let Some(action) = actions.art {
                    let mut replace = None;
                    match action {
                        art::Action::Load(path) => match ArtPack::load(&path) {
                            Ok(pack) => {
                                art_status = format!("Loaded {} at {}x.", pack.name, pack.scale);
                                settings.art_pack = selection_value(&library_directory(), &path);
                                replace = Some(Some(Rc::new(pack)));
                            }
                            Err(error) => art_status = format!("Could not load pack: {error}"),
                        },
                        art::Action::Original => {
                            settings.art_pack.clear();
                            art_status = "Using original artwork.".into();
                            replace = Some(None);
                        }
                        art::Action::Export {
                            folder,
                            stage,
                            scale,
                        } => {
                            let result = folder
                                .parent()
                                .filter(|p| !p.as_os_str().is_empty())
                                .map_or(Ok(()), |p| {
                                    std::fs::create_dir_all(p).map_err(|e| e.to_string())
                                })
                                .and_then(|()| export_template(rom, stage, &folder, scale));
                            art_status = match result {
                                Ok(()) => {
                                    panel.refresh_art_packs();
                                    format!(
                                        "Template exported to {}. Paint the PNG sheets, then select it from Artwork.",
                                        folder.display()
                                    )
                                }
                                Err(error) => format!("Could not export template: {error}"),
                            };
                        }
                    }
                    if let Some(pack) = replace {
                        panel.set_art_selection(&settings.art_pack);
                        panel.gallery.set_art_pack(pack.clone());
                        frame.set_art_pack(pack);
                        fit_art_to_window(frame, &canvas, &settings)?;
                        texture = creator
                            .create_texture_streaming(
                                PixelFormatEnum::RGB24,
                                frame.output_width() as u32,
                                frame.output_height() as u32,
                            )
                            .map_err(|e| e.to_string())?;
                        scene.draw(frame);
                        upload = true;
                    }
                }
                if let Some(binding) = actions.capture {
                    capture = Some(binding);
                    keys.clear();
                    latch.clear();
                }
                let wants_text = gui.wants_text();
                if text_input_active != wants_text {
                    if wants_text {
                        video.text_input().start();
                    } else {
                        video.text_input().stop();
                    }
                    text_input_active = wants_text;
                }
                if before != settings {
                    if before.widescreen != settings.widescreen {
                        let pack = frame.art_pack();
                        *frame = Frame::new(settings.frame_width());
                        frame.set_art_pack(pack);
                        fit_art_to_window(frame, &canvas, &settings)?;
                        frame.smooth_wave_effects = settings.smooth_wave_effects;
                        texture = creator
                            .create_texture_streaming(
                                PixelFormatEnum::RGB24,
                                frame.output_width() as u32,
                                frame.output_height() as u32,
                            )
                            .map_err(|error| error.to_string())?;
                        scene.draw(frame);
                    }
                    if before.fullscreen != settings.fullscreen
                        || (!settings.fullscreen
                            && (before.scale != settings.scale
                                || before.widescreen != settings.widescreen))
                    {
                        window_settings_changed = true;
                    }
                    map = InputMap::new(&settings);
                    if before.smooth_wave_effects != settings.smooth_wave_effects {
                        frame.smooth_wave_effects = settings.smooth_wave_effects;
                        scene.draw(frame);
                    }
                    if before.keyboard != settings.keyboard
                        || before.gamepad != settings.gamepad
                        || before.controller != settings.controller
                    {
                        keys.clear();
                        latch.clear();
                        release_inputs = true;
                    }
                    save_settings(config_path.as_deref(), &settings);
                    upload = true;
                }
                // Panel actions (a load, a warp) show on the next repaint.
                if actions.scene_changed {
                    scene.draw(frame);
                    upload = true;
                }
            } else if text_input_active {
                video.text_input().stop();
                text_input_active = false;
            }
            if ticked && pacing == Pacing::Clock {
                let due = (now + PRESENT_LEAD)
                    .checked_sub(clock.since_tick())
                    .unwrap_or(now);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
            }
            canvas.present();
            let presented = Instant::now();
            if let Some(previous_present) = last_present {
                let period = presented.duration_since(previous_present);
                panel.observe_present(period.as_secs_f32() * 1000.0);
                if timing_log {
                    eprintln!(
                        "{},{},{},{},{}",
                        presented.duration_since(timing_start).as_micros(),
                        period.as_micros(),
                        total_ticks,
                        match pacing {
                            Pacing::Display => "display",
                            Pacing::Clock => "clock",
                        },
                        audio
                            .as_ref()
                            .map_or(0, |audio| audio.producer.underrun_frames()),
                    );
                }
            }
            last_present = Some(presented);
            if window_settings_changed {
                apply_window_settings(&mut canvas, &settings)?;
                upload = true;
                repaint = true;
            }
        }
        if options.frames.is_some_and(|limit| total_ticks >= limit) {
            break;
        }
        // Events wait for the next tick; the input latch keeps presses.
        if pacing == Pacing::Clock {
            std::thread::sleep(clock.until_tick().saturating_sub(previous.elapsed()));
        }
    }
    Ok(total_ticks)
}

fn set_icon(window: &mut sdl2::video::Window) -> Result<(), String> {
    let mut data = sdl2::rwops::RWops::from_bytes(include_bytes!("../resources/ninja-master.bmp"))?;
    let mut icon = sdl2::surface::Surface::load_bmp_rw(&mut data)?;
    icon.set_color_key(true, sdl2::pixels::Color::BLACK)?;
    window.set_icon(icon);
    Ok(())
}

/// Desaturate and dim the presentation copy; the paused game frame keeps its colour.
fn scanlines(pixels: &[u8], width: usize, scale: usize, strength: u8, output: &mut Vec<u8>) {
    output.clear();
    let gain = 100 - u16::from(strength.min(100));
    for (row, line) in pixels.chunks(width * 3).enumerate() {
        if (row / scale).is_multiple_of(2) {
            output.extend_from_slice(line);
        } else {
            output.extend(
                line.iter()
                    .map(|&channel| (u16::from(channel) * gain / 100) as u8),
            );
        }
    }
}

fn track_name(rom: &[u8], command: Option<u8>) -> String {
    let Some(command) = command else {
        return "None".into();
    };
    for index in 0..29 {
        let at = ninja_master_native::sound_events::MUSIC_TABLE + index * 16;
        if rom.get(at) == Some(&command)
            && let Some(name) = rom.get(at + 2..at + 16)
        {
            return format!("{} (${command:02X})", String::from_utf8_lossy(name).trim());
        }
    }
    format!("${command:02X}")
}

fn gray_out(pixels: &[u8], output: &mut Vec<u8>) {
    output.clear();
    for pixel in pixels.as_chunks::<3>().0 {
        let luminance =
            (u32::from(pixel[0]) * 77 + u32::from(pixel[1]) * 150 + u32::from(pixel[2]) * 29) >> 8;
        output.extend_from_slice(&[(luminance * 3 / 5) as u8; 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a desktop OpenGL display"]
    fn fullscreen_round_trips_keep_game_and_panel_rendering() {
        use sdl2::{pixels::Color, render::ClippingRect};

        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let driver = sdl2::render::drivers()
            .position(|driver| driver.name == "opengl")
            .unwrap() as u32;
        let mut settings = Settings {
            widescreen: false,
            scale: 1,
            ..Settings::default()
        };
        let window = video
            .window(
                "Fullscreen rendering regression",
                WIDTH as u32,
                HEIGHT as u32,
            )
            .resizable()
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().index(driver).build().unwrap();
        let mut events = sdl.event_pump().unwrap();
        let creator = canvas.texture_creator();
        let mut texture = creator
            .create_texture_streaming(PixelFormatEnum::RGB24, 2, 2)
            .unwrap();
        let mut gui = gui::Gui::new(&creator, None).unwrap();
        let mut crt = crt::Crt::new(&video, &mut canvas).unwrap();
        for shader in [false, true] {
            for fullscreen in [true, false, true, false] {
                canvas.clear();
                gui.draw(&mut canvas, |ui| {
                    ui.window("Display")
                        .position([0.0, 0.0], imgui::Condition::Always)
                        .size([150.0, 80.0], imgui::Condition::Always)
                        .build(|| {
                            ui.checkbox("Fullscreen", &mut settings.fullscreen);
                            settings.fullscreen = fullscreen;
                        });
                })
                .unwrap();
                // Match both the F1 and F11 paths: finish the old frame before
                // changing modes, then upload and draw new frames immediately.
                canvas.present();
                apply_window_settings(&mut canvas, &settings).unwrap();
                events.pump_events();
                assert_eq!(
                    canvas.window().fullscreen_state(),
                    if fullscreen {
                        FullscreenType::Desktop
                    } else {
                        FullscreenType::Off
                    }
                );
                assert_eq!(canvas.clip_rect(), ClippingRect::None);
                // The first swap lets the window system resize its backing
                // buffer. Subsequent frames must continue accepting new pixels.
                for color in [[255, 255, 255], [255, 0, 0], [0, 255, 0], [0, 0, 255]] {
                    std::thread::sleep(Duration::from_millis(16));
                    events.pump_events();
                    let pixels = color.repeat(4);
                    texture.update(None, &pixels, 6).unwrap();
                    crt.upload(&mut canvas, &pixels, 2, 2).unwrap();
                    canvas.set_draw_color(Color::BLACK);
                    canvas.clear();
                    let rect = game_rect(&canvas, &settings).unwrap();
                    if shader {
                        crt.draw(&mut canvas, rect, WIDTH, 0, 0).unwrap();
                    } else {
                        canvas.copy(&texture, None, rect).unwrap();
                    }
                    gui.draw(&mut canvas, |ui| {
                        ui.window("Display")
                            .build(|| ui.text("Rendering after fullscreen"));
                    })
                    .unwrap();
                    if color == [255, 255, 255] {
                        canvas.present();
                        continue;
                    }
                    let sample = Rect::new(
                        rect.x() + rect.width() as i32 * 9 / 10,
                        rect.y() + rect.height() as i32 * 9 / 10,
                        1,
                        1,
                    );
                    let actual = canvas.read_pixels(sample, PixelFormatEnum::RGB24).unwrap();
                    for channel in 0..3 {
                        if color[channel] == 0 {
                            assert_eq!(actual[channel], 0, "fullscreen={fullscreen}, CRT={shader}");
                        } else {
                            assert!(
                                actual[channel] > 20,
                                "fullscreen={fullscreen}, CRT={shader}: {actual:?}"
                            );
                        }
                    }
                    canvas.present();
                }
            }
        }
        assert_eq!(canvas.window().size(), (WIDTH as u32, HEIGHT as u32));
    }

    #[test]
    fn hd_scale_tracks_actual_viewport_including_4k_and_stretched_windows() {
        assert_eq!(art_output_scale(Rect::new(0, 0, 1194, 672), 398), 3);
        assert_eq!(art_output_scale(Rect::new(0, 0, 3840, 2160), 398), 10);
        assert_eq!(art_output_scale(Rect::new(0, 0, 160, 112), 320), 1);
        assert_eq!(art_output_scale(Rect::new(0, 0, 1920, 448), 320), 6);
    }

    #[test]
    fn presentation_fits_small_windows_and_respects_aspect_modes() {
        let mut settings = Settings {
            widescreen: false,
            ..Settings::default()
        };
        assert_eq!(
            presentation_rect(960, 672, &settings),
            Rect::new(0, 0, 960, 672)
        );
        assert_eq!(
            presentation_rect(160, 112, &settings),
            Rect::new(0, 0, 160, 112)
        );
        assert_eq!(
            presentation_rect(1000, 700, &settings),
            Rect::new(20, 14, 960, 672)
        );
        settings.integer_scale = false;
        assert_eq!(
            presentation_rect(1000, 700, &settings),
            Rect::new(0, 0, 1000, 700)
        );
        settings.aspect = 1;
        let rect = presentation_rect(960, 672, &settings);
        assert_eq!(rect, Rect::new(32, 0, 896, 672));
        settings.aspect = 2;
        assert_eq!(
            presentation_rect(900, 500, &settings),
            Rect::new(0, 0, 900, 500)
        );
    }

    #[test]
    fn scanlines_change_only_alternating_presentation_rows() {
        for width in [WIDTH, ninja_master_native::WIDE_WIDTH] {
            let pixels = vec![200; width * HEIGHT * 3];
            let mut output = Vec::new();
            scanlines(&pixels, width, 1, 50, &mut output);
            assert!(output[..width * 3].iter().all(|&v| v == 200));
            assert!(output[width * 3..width * 6].iter().all(|&v| v == 100));
            assert!(pixels.iter().all(|&v| v == 200));
            scanlines(&pixels, width, 1, 0, &mut output);
            assert_eq!(pixels, output);
            scanlines(&pixels, width, 2, 50, &mut output);
            assert!(output[..width * 6].iter().all(|&v| v == 200));
            assert!(output[width * 6..width * 12].iter().all(|&v| v == 100));
        }
    }

    #[test]
    fn widescreen_preserves_pixels_even_with_a_saved_legacy_aspect() {
        let mut settings = Settings::default();
        for aspect in 0..=2 {
            settings.aspect = aspect;
            assert_eq!(
                presentation_rect(1194, 672, &settings),
                Rect::new(0, 0, 1194, 672)
            );
            assert_eq!(
                presentation_rect(199, 112, &settings),
                Rect::new(0, 0, 199, 112)
            );
            assert_eq!(
                presentation_rect(1920, 1080, &settings),
                Rect::new(164, 92, 1592, 896)
            );
        }
        settings.integer_scale = false;
        assert_eq!(
            presentation_rect(1920, 1080, &settings),
            Rect::new(0, 0, 1919, 1080)
        );
    }

    #[test]
    fn unfocused_display_is_gray_and_dim_without_changing_the_game_frame() {
        let pixels = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let mut output = vec![255; 24];
        gray_out(&pixels, &mut output);
        assert_eq!(output, [45, 45, 45, 89, 89, 89, 16, 16, 16, 153, 153, 153]);
        assert_eq!(&pixels[..3], &[255, 0, 0]);
    }
}
