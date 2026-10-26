mod art;
mod audio;
mod audio_packs;
mod benchmark;
mod capture;
mod editor;
mod input;
mod performance;
mod presentation;
// OpenGL/SDL interop is confined to the CRT and experimental tile-batch modules.
#[allow(unsafe_code)]
mod crt;
mod gallery;
mod gui;
mod packs;
mod panel;
mod rom_picker;
#[allow(unsafe_code)]
mod tile_batch;
mod transfer;

use audio::{open_audio, track_name};
pub(crate) use benchmark::run as benchmark;
pub(crate) use editor::run as editor;
use input::{InputMap, RESERVED_KEYS};
use presentation::{
    apply_window_settings, fit_frame_to_window, game_rect, menu_point, set_icon, set_title,
};
pub(crate) use rom_picker::pick_rom;

use crate::{Options, Scene};
use ninja_master_native::{
    HEIGHT,
    art_pack::{ArtPack, export_template, library_directory, resolve_selection, selection_value},
    audio_pack::{self, AudioPack, Section},
    devtools::DevTools,
    input::{Button, InputLatch},
    render::Frame,
    settings::{self, Device, Rebinder, Settings},
    timing::{FixedClock, RefreshProbe},
};
use sdl2::{
    controller::{Axis, GameController},
    event::{Event, WindowEvent},
    keyboard::Scancode,
    pixels::PixelFormatEnum,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

/// What loading an audio pack skipped or adjusted, for the panel status.
fn pack_notes(pack: &AudioPack) -> String {
    pack.notes
        .iter()
        .fold(String::new(), |text, note| text + note + ". ")
}

/// The title menu's EDITOR: the expansion editor runs as its own process,
/// with its own window, beside the game.
fn open_editor(rom: Option<&Path>) -> Result<(), String> {
    let mut command =
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?);
    command.arg("--editor");
    if let Some(rom) = rom {
        command.arg("--rom").arg(rom);
    }
    command.spawn().map(drop).map_err(|e| e.to_string())
}

fn save_settings(path: Option<&Path>, settings: &Settings) {
    if let Some(path) = path
        && let Err(error) = settings.save(path)
    {
        eprintln!("settings: could not save {}: {error}", path.display());
    }
}

/// Clock-paced fallback runs the tick early and presents on its deadline.
const PRESENT_LEAD: Duration = Duration::from_millis(5);

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
    if let Scene::Play(modes) = &mut *scene {
        modes.set_options(settings.game);
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
    let mut audio_status = String::new();
    // The original sound plays until the pack has loaded.
    let mut audio_loading = None;
    if soundtrack.is_some() && !settings.audio_pack.is_empty() {
        let path = resolve_selection(&audio_pack::library_directory(), &settings.audio_pack);
        audio_status = "Loading the audio pack...".into();
        audio_loading = Some(audio_packs::Loading::start(path, Some(rom.to_vec())));
    }
    // Rendering a template takes a minute, so it runs beside the game.
    let mut audio_export: Option<std::thread::JoinHandle<Result<PathBuf, String>>> = None;

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
    // adjusted through Options or the desktop shortcuts.
    // NINJA_VSYNC=0 provides an explicit clock-paced path for VRR displays.
    let request_vsync = std::env::var("NINJA_VSYNC").as_deref() != Ok("0");
    let builder = window.into_canvas();
    let builder = if request_vsync {
        builder.present_vsync()
    } else {
        builder
    };
    let mut canvas = builder.build().map_err(|error| error.to_string())?;
    fit_frame_to_window(frame, &canvas, &settings)?;
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
    panel.set_audio_selection(&settings.audio_pack);
    let mut show_panel = options.devtools;
    let mut performance = performance::Performance::default();
    let mut screenshots = capture::Capture::default();
    let screenshot_directory = config_path
        .as_deref()
        .and_then(Path::parent)
        .map(|directory| directory.join("screenshots"))
        .unwrap_or_else(|| std::path::PathBuf::from("screenshots"));
    let mut screenshot_requested = false;
    let mut toast: Option<(String, Instant)> = None;
    let mut tools = matches!(scene, Scene::Play(_)).then(|| DevTools::new(rom));
    if let (Some(tools), Scene::Play(modes)) = (&mut tools, &mut *scene) {
        let effects = ["palette", "parallax", "priority", "vegetation"]
            .map(|name| !options.disabled_effects.iter().any(|n| n == name));
        let dir = config_path
            .as_deref()
            .and_then(Path::parent)
            .map(|dir| dir.join("saves"));
        tools.enable_disk_saves(modes, effects, dir);
        modes.enable_progress(
            config_path
                .as_deref()
                .and_then(Path::parent)
                .map(|dir| dir.join("continue.nms")),
            rom,
        );
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
    let mut transfer_buffers = transfer::Buffers::default();
    let mut reset_renderer = false;
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
        if let Some(message) = screenshots.poll() {
            eprintln!("{message}");
            toast = Some((message, Instant::now() + Duration::from_secs(5)));
            repaint = true;
        }
        if toast
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            toast = None;
            repaint = true;
        }
        if let Scene::Play(modes) = &mut *scene {
            modes.display_context((&settings).into(), crt.as_ref().err().map(String::as_str));
        }
        for event in events.poll_iter() {
            had_event = true;
            if matches!(
                event,
                Event::RenderTargetsReset { .. } | Event::RenderDeviceReset { .. }
            ) {
                reset_renderer = true;
                continue;
            }
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
            // Desktop shortcuts remain available during both binding flows.
            if let Event::KeyDown {
                scancode: Some(key),
                repeat: false,
                ..
            } = event
                && focused
                && matches!(key, Scancode::F2 | Scancode::F12)
            {
                if key == Scancode::F2 {
                    performance.visible = !performance.visible;
                } else {
                    screenshot_requested = true;
                }
                repaint = true;
                continue;
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
            if focused
                && !show_panel
                && rebinder.is_none()
                && capture.is_none()
                && let Scene::Play(modes) = &mut *scene
            {
                use ninja_master_native::menu::MenuEvent;
                let pointer = match event {
                    Event::MouseMotion { x, y, .. } => Some((x, y, None)),
                    Event::MouseButtonDown {
                        x, y, mouse_btn, ..
                    } if matches!(
                        mouse_btn,
                        sdl2::mouse::MouseButton::Left | sdl2::mouse::MouseButton::Right
                    ) =>
                    {
                        Some((x, y, Some(mouse_btn == sdl2::mouse::MouseButton::Right)))
                    }
                    _ => None,
                };
                if let Some((x, y, click)) = pointer {
                    let rect = game_rect(&canvas, &settings)?;
                    let output = canvas.output_size()?;
                    if let Some((x, y)) = menu_point(
                        x,
                        y,
                        canvas.window().size(),
                        output,
                        rect,
                        frame.width(),
                        if settings.crt && crt.is_ok() {
                            f64::from(settings.crt_curvature) * 0.0018
                        } else {
                            0.0
                        },
                    ) {
                        modes.menu_event(match click {
                            Some(alternate) => MenuEvent::Click(x, y, alternate),
                            None => MenuEvent::Hover(x, y),
                        });
                        modes.draw(frame);
                        upload = true;
                    }
                }
                if let Event::MouseWheel { y, direction, .. } = event {
                    let delta = if direction == sdl2::mouse::MouseWheelDirection::Flipped {
                        y
                    } else {
                        -y
                    };
                    modes.menu_event(MenuEvent::Scroll(delta));
                    scene.draw(frame);
                    upload = true;
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
                        Scancode::Escape if options.play => {
                            if let Scene::Play(modes) = &mut *scene {
                                if modes.name() == "Game" {
                                    escape_pause = true;
                                } else {
                                    modes.menu_event(ninja_master_native::menu::MenuEvent::Back);
                                }
                            }
                        }
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
                        Scancode::F3 => {
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
            if let Scene::Play(modes) = &mut *scene {
                modes.display_context((&settings).into(), crt.as_ref().err().map(String::as_str));
            }
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
        performance.ticks = ticks as u32;
        performance.raster_ms = 0.0;
        performance.upload_ms = 0.0;
        performance.upload_bytes = 0;
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
                if let (Some(tools), Scene::Play(modes)) = (&tools, &mut *scene) {
                    tools.before_tick(modes);
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
                if let Scene::Play(modes) = &mut *scene {
                    if total_ticks.is_multiple_of(300) {
                        modes.checkpoint();
                    }
                    if let Some(mut game_options) = modes.options() {
                        // Secret invincibility is run state, never an application default.
                        game_options.invincible = false;
                        if settings.game != game_options {
                            settings.game = game_options;
                            save_settings(config_path.as_deref(), &settings);
                        }
                    }
                }
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
        if let Scene::Play(modes) = &mut *scene
            && modes.take_editor_request()
        {
            let message = match open_editor(options.rom.as_deref()) {
                Ok(()) => "The expansion editor opens in its own window.".to_string(),
                Err(error) => format!("Could not open the editor: {error}"),
            };
            eprintln!("{message}");
            toast = Some((message, Instant::now() + Duration::from_secs(5)));
            repaint = true;
        }

        // Native Options owns the controls; the desktop applies and persists
        // presentation changes through the same path used by shortcuts.
        if let Scene::Play(modes) = &mut *scene
            && let Some(display) = modes.take_display_settings()
        {
            let before = settings.clone();
            display.apply(&mut settings);
            window_settings_changed |= before.fullscreen != settings.fullscreen
                || (!settings.fullscreen
                    && (before.scale != settings.scale
                        || before.widescreen != settings.widescreen));
            frame.smooth_wave_effects = settings.smooth_wave_effects;
            save_settings(config_path.as_deref(), &settings);
            scene.draw(frame);
            upload = true;
            repaint = true;
        }

        if reset_renderer {
            // Recreate GPU resources before drawing again. Game/replay state is untouched.
            drop(std::mem::replace(&mut crt, Err("Renderer reset".into())));
            texture = creator
                .create_texture_streaming(
                    PixelFormatEnum::RGB24,
                    frame.output_width() as u32,
                    frame.output_height() as u32,
                )
                .map_err(|error| error.to_string())?;
            gui.reset_textures(&creator)?;
            panel.gallery.set_art_pack(frame.art_pack());
            thumbnail_versions.fill(None);
            crt = crt::Crt::new(&video, &mut canvas);
            reset_renderer = false;
            upload = true;
            repaint = true;
        }
        if fit_frame_to_window(frame, &canvas, &settings)? {
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
            let raster_start = Instant::now();
            scene.draw(frame);
            performance.raster_ms = raster_start.elapsed().as_secs_f32() * 1000.0;
            upload = true;
        }
        if std::mem::take(&mut upload) {
            let transfer = transfer_buffers.upload(
                &mut canvas,
                &mut texture,
                &mut crt,
                frame,
                &settings,
                focused,
            )?;
            performance.resolve_ms = transfer.resolve_ms;
            performance.upload_ms = transfer.upload_ms;
            performance.upload_bytes = transfer.bytes;
            repaint = true;
        }
        // The panel redraws at the tick rate and on input.
        if (show_panel || performance.visible || toast.is_some()) && (real_ticks > 0 || had_event) {
            repaint = true;
        }
        if std::mem::take(&mut repaint) || pacing != Pacing::Clock {
            let submit_start = Instant::now();
            canvas.set_draw_color(sdl2::pixels::Color::BLACK);
            canvas.clear();
            let rect = game_rect(&canvas, &settings)?;
            if let Some(transfer) = transfer::draw(
                &mut canvas,
                &mut texture,
                &mut crt,
                frame,
                &settings,
                focused,
                &mut transfer_buffers,
            )? {
                performance.resolve_ms += transfer.resolve_ms;
                performance.upload_ms += transfer.upload_ms;
                performance.upload_bytes += transfer.bytes;
            }
            performance.submit_ms = submit_start.elapsed().as_secs_f32() * 1000.0;
            if std::mem::take(&mut screenshot_requested)
                && let Err(error) =
                    screenshots.request(&mut canvas, rect, screenshot_directory.clone())
            {
                let message = format!("Screenshot failed: {error}");
                eprintln!("{message}");
                toast = Some((message, Instant::now() + Duration::from_secs(5)));
            }
            let source_size = (frame.output_width(), frame.output_height());
            let viewport_size = (rect.width(), rect.height());
            let sprites = frame.sprite_count();
            let renderer = canvas.info().name;
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
                if audio_loading
                    .as_ref()
                    .is_some_and(audio_packs::Loading::finished)
                {
                    let (path, result) = audio_loading.take().unwrap().join();
                    match result {
                        Ok(pack) => {
                            audio_status = format!("Loaded {}. {}", pack.name, pack_notes(&pack));
                            if soundtrack.is_none() {
                                audio_status += "Sound is off in this mode; play mode uses it.";
                            }
                            settings.audio_pack =
                                selection_value(&audio_pack::library_directory(), &path);
                            if let Some(player) = soundtrack.as_mut() {
                                player.set_pack(Some(Arc::new(pack)));
                            }
                        }
                        Err(error) => audio_status = format!("Could not load pack: {error}"),
                    }
                    panel.set_audio_selection(
                        if soundtrack.as_ref().is_some_and(|s| s.pack().is_some()) {
                            &settings.audio_pack
                        } else {
                            ""
                        },
                    );
                }
                if audio_export
                    .as_ref()
                    .is_some_and(std::thread::JoinHandle::is_finished)
                {
                    let result = audio_export
                        .take()
                        .unwrap()
                        .join()
                        .unwrap_or_else(|_| Err("the export stopped unexpectedly".into()));
                    audio_status = match result {
                        Ok(folder) => {
                            panel.refresh_audio_packs();
                            format!(
                                "Template exported to {}. Replace its files, then select it from Audio.",
                                folder.display()
                            )
                        }
                        Err(error) => format!("Could not export template: {error}"),
                    };
                }
                let track = track_name(
                    rom,
                    soundtrack
                        .as_ref()
                        .and_then(ninja_master_native::sound::SoundPlayer::current_music),
                );
                let info = panel::PanelInfo {
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
                    audio_name: &soundtrack.as_ref().and_then(|s| s.pack()).map_or_else(
                        || "Original".to_owned(),
                        |p| {
                            let [music, effects, voices] = Section::ALL.map(|s| p.count(s));
                            format!(
                                "{} ({music} music, {effects} effects, {voices} voices)",
                                p.name
                            )
                        },
                    ),
                    audio_status: &audio_status,
                    audio_exporting: audio_export.is_some(),
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
                    performance.show(ui, source_size, viewport_size, sprites, renderer);
                    actions = panel.show(ui, scene, tools.as_mut(), &mut settings, &info);
                    performance::feedback(ui, &toast);
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
                        fit_frame_to_window(frame, &canvas, &settings)?;
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
                if let Some(action) = actions.audio {
                    match action {
                        audio_packs::Action::Load(path) => {
                            audio_status = "Loading the audio pack...".into();
                            let rom = soundtrack.is_some().then(|| rom.to_vec());
                            audio_loading = Some(audio_packs::Loading::start(path, rom));
                        }
                        audio_packs::Action::Original => {
                            audio_loading = None;
                            settings.audio_pack.clear();
                            audio_status = "Using the original soundtrack.".into();
                            if let Some(player) = soundtrack.as_mut() {
                                player.set_pack(None);
                            }
                        }
                        audio_packs::Action::Export(folder) => {
                            let rom = rom.to_vec();
                            audio_status = format!(
                                "Rendering the audio template into {}...",
                                folder.display()
                            );
                            audio_export = Some(std::thread::spawn(move || {
                                audio_pack::export_template(&rom, &folder).map(|()| folder)
                            }));
                        }
                    }
                    panel.set_audio_selection(&settings.audio_pack);
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
                    if before.game != settings.game
                        && let Scene::Play(modes) = &mut *scene
                    {
                        let mut game_options = modes.options().unwrap_or(settings.game);
                        game_options.control = settings.game.control;
                        modes.set_options(game_options);
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
            } else {
                if performance.visible || toast.is_some() {
                    gui.draw(&mut canvas, |ui| {
                        performance.show(ui, source_size, viewport_size, sprites, renderer);
                        performance::feedback(ui, &toast);
                    })?;
                }
                if text_input_active {
                    video.text_input().stop();
                    text_input_active = false;
                }
            }
            if ticked && pacing == Pacing::Clock {
                let due = (now + PRESENT_LEAD)
                    .checked_sub(clock.since_tick())
                    .unwrap_or(now);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
            }
            let present_start = Instant::now();
            canvas.present();
            performance.present_ms = present_start.elapsed().as_secs_f32() * 1000.0;
            let presented = Instant::now();
            if let Some(previous_present) = last_present {
                let period = presented.duration_since(previous_present);
                panel.observe_present(period.as_secs_f32() * 1000.0);
                performance.observe(period);
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
    screenshots.finish();
    if let Scene::Play(modes) = scene {
        modes.checkpoint();
    }
    Ok(total_ticks)
}
