use ninja_master_native::{HEIGHT, WIDTH, render::Frame, settings::Settings};
use sdl2::{rect::Rect, render::WindowCanvas, video::FullscreenType};

pub(super) fn set_title(canvas: &mut WindowCanvas, title: &str) -> Result<(), String> {
    canvas
        .window_mut()
        .set_title(title)
        .map_err(|error| error.to_string())
}

/// The largest whole multiple of the frame that fits the window, centred.
pub(super) fn game_rect(canvas: &WindowCanvas, settings: &Settings) -> Result<Rect, String> {
    let (width, height) = canvas.output_size()?;
    Ok(presentation_rect(width, height, settings))
}

/// Invert the actual presentation, including DPI, letterboxing and CRT glass.
pub(super) fn menu_point(
    x: i32,
    y: i32,
    window: (u32, u32),
    output: (u32, u32),
    rect: Rect,
    frame_width: usize,
    curvature: f64,
) -> Option<(i32, i32)> {
    if window.0 == 0 || window.1 == 0 || rect.width() == 0 || rect.height() == 0 {
        return None;
    }
    let x = f64::from(x) * f64::from(output.0) / f64::from(window.0);
    let y = f64::from(y) * f64::from(output.1) / f64::from(window.1);
    let px = (x - f64::from(rect.x())) / f64::from(rect.width()) * 2.0 - 1.0;
    let py = (y - f64::from(rect.y())) / f64::from(rect.height()) * 2.0 - 1.0;
    let u = (px * (1.0 + curvature * py * py) + 1.0) * 0.5;
    let v = (py * (1.0 + curvature * px * px) + 1.0) * 0.5;
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return None;
    }
    Some((
        (u * frame_width as f64).floor() as i32 - ((frame_width - WIDTH) / 2) as i32,
        (v * HEIGHT as f64).floor() as i32,
    ))
}

fn art_output_scale(rect: Rect, width: usize) -> usize {
    (rect.width() as usize)
        .div_ceil(width)
        .max((rect.height() as usize).div_ceil(HEIGHT))
        .max(1)
}

fn view_width(width: u32, height: u32, settings: &Settings) -> usize {
    if !settings.widescreen || width == 0 || height == 0 {
        return WIDTH;
    }
    let integer_scale = height / HEIGHT as u32;
    let width = if settings.integer_scale && integer_scale > 0 {
        u64::from(width / integer_scale)
    } else {
        (u64::from(width) * HEIGHT as u64 + u64::from(height) / 2) / u64::from(height)
    };
    // A nearly collapsed window must not allocate an enormous world view.
    width.clamp(WIDTH as u64, 4096) as usize
}

pub(super) fn fit_frame_to_output(
    frame: &mut Frame,
    width: u32,
    height: u32,
    settings: &Settings,
) -> bool {
    let view_width = view_width(width, height, settings);
    let resized = frame.width() != view_width;
    if resized {
        let pack = frame.art_pack();
        *frame = Frame::new(view_width);
        frame.set_art_pack(pack);
    }
    frame.smooth_wave_effects = settings.smooth_wave_effects;
    frame.set_output_scale(art_output_scale(
        presentation_rect(width, height, settings),
        view_width,
    )) || resized
}

pub(super) fn fit_frame_to_window(
    frame: &mut Frame,
    canvas: &WindowCanvas,
    settings: &Settings,
) -> Result<bool, String> {
    let (width, height) = canvas.output_size()?;
    Ok(fit_frame_to_output(frame, width, height, settings))
}

/// Change window modes only after presenting the frame, when SDL has no queued
/// game or panel draws using the previous drawable dimensions.
pub(super) fn apply_window_settings(
    canvas: &mut WindowCanvas,
    settings: &Settings,
) -> Result<(), String> {
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

pub(super) fn presentation_rect(width: u32, height: u32, settings: &Settings) -> Rect {
    if !settings.widescreen && settings.aspect == 2 {
        return Rect::new(0, 0, width.max(1), height.max(1));
    }
    let base_width = if settings.widescreen {
        view_width(width, height, settings) as f64
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

pub(super) fn set_icon(window: &mut sdl2::video::Window) -> Result<(), String> {
    let mut data =
        sdl2::rwops::RWops::from_bytes(include_bytes!("../../resources/ninja-master.bmp"))?;
    let mut icon = sdl2::surface::Surface::load_bmp_rw(&mut data)?;
    icon.set_color_key(true, sdl2::pixels::Color::BLACK)?;
    window.set_icon(icon);
    Ok(())
}

/// Desaturate and dim the presentation copy; the paused game frame keeps its colour.
pub(super) fn scanlines(
    pixels: &[u8],
    width: usize,
    scale: usize,
    strength: u8,
    output: &mut Vec<u8>,
) {
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

pub(super) fn gray_out(pixels: &[u8], output: &mut Vec<u8>) {
    output.clear();
    for pixel in pixels.as_chunks::<3>().0 {
        let luminance =
            (u32::from(pixel[0]) * 77 + u32::from(pixel[1]) * 150 + u32::from(pixel[2]) * 29) >> 8;
        output.extend_from_slice(&[(luminance * 3 / 5) as u8; 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{crt, gui};
    use super::*;
    use sdl2::pixels::PixelFormatEnum;
    use std::time::Duration;

    #[test]
    fn pointer_mapping_uses_dpi_letterboxing_widescreen_and_crt() {
        for width in [320, 398, 800] {
            let rect = Rect::new(100, 50, 800, 448);
            assert_eq!(
                menu_point(500, 274, (1000, 600), (1000, 600), rect, width, 0.0),
                Some((160, 112))
            );
            assert_eq!(
                menu_point(250, 137, (500, 300), (1000, 600), rect, width, 0.0),
                Some((160, 112))
            );
            assert_eq!(
                menu_point(500, 274, (1000, 600), (1000, 600), rect, width, 0.18),
                Some((160, 112))
            );
            assert!(menu_point(0, 0, (1000, 600), (1000, 600), rect, width, 0.0).is_none());
        }
        let rect = Rect::new(0, 0, 320, 224);
        // Same inverse barrel mapping as crt.frag, away from the centre.
        let mapped = menu_point(240, 168, (320, 224), (320, 224), rect, 320, 0.18).unwrap();
        assert_eq!(mapped, (243, 170));
    }

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
                // Device-reset recovery rebuilds GPU resources in the existing ImGui context.
                gui.reset_textures(&creator).unwrap();
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
                Rect::new(0, 92, 1920, 896)
            );
        }
        settings.integer_scale = false;
        assert_eq!(
            presentation_rect(1920, 1080, &settings),
            Rect::new(0, 0, 1919, 1080)
        );
    }

    #[test]
    fn widescreen_expands_the_frame_to_fit_ultrawide_displays() {
        let mut settings = Settings::default();
        let mut frame = Frame::default();
        for (width, height, native_width) in [
            (1920, 1080, 480),
            (2560, 1080, 640),
            (3440, 1440, 573),
            (3840, 1600, 548),
            (5120, 1440, 853),
        ] {
            for aspect in 0..=2 {
                settings.aspect = aspect;
                fit_frame_to_output(&mut frame, width, height, &settings);
                assert_eq!(frame.width(), native_width);
                assert_eq!(frame.rgb().len(), native_width * HEIGHT * 3);
                let rect = presentation_rect(width, height, &settings);
                let scale = height / HEIGHT as u32;
                assert_eq!(rect.width(), native_width as u32 * scale);
                assert_eq!(rect.height(), HEIGHT as u32 * scale);
                assert!(width - rect.width() < scale);
                assert_eq!(rect.x(), (width - rect.width()) as i32 / 2);
                assert_eq!(rect.y(), (height - rect.height()) as i32 / 2);
            }
        }
        assert!(!fit_frame_to_output(&mut frame, 5120, 1440, &settings));
        settings.widescreen = false;
        assert!(fit_frame_to_output(&mut frame, 5120, 1440, &settings));
        assert_eq!(frame.width(), WIDTH);
        assert_eq!(frame.rgb().len(), WIDTH * HEIGHT * 3);
    }

    #[test]
    fn fractional_widescreen_matches_display_aspect_without_stretching_sprites() {
        let settings = Settings {
            integer_scale: false,
            ..Settings::default()
        };
        for (width, height, native_width) in [
            (1920, 1080, 398),
            (2560, 1080, 531),
            (3440, 1440, 535),
            (3840, 1600, 538),
            (5120, 1440, 796),
            (1920, 1200, 358),
        ] {
            assert_eq!(view_width(width, height, &settings), native_width);
            let rect = presentation_rect(width, height, &settings);
            assert!(width - rect.width() <= 4);
            assert!(height - rect.height() <= 4);
            let horizontal_scale = rect.width() as f64 / native_width as f64;
            let vertical_scale = rect.height() as f64 / HEIGHT as f64;
            assert!((horizontal_scale - vertical_scale).abs() < 0.005);
        }
    }

    #[test]
    fn resized_widescreen_preserves_the_original_view_and_render_preferences() {
        let mut settings = Settings {
            smooth_wave_effects: false,
            ..Settings::default()
        };
        let mut frame = Frame::default();
        assert!(fit_frame_to_output(&mut frame, 3440, 1440, &settings));
        assert!(!frame.smooth_wave_effects);
        settings.smooth_wave_effects = true;
        assert!(fit_frame_to_output(&mut frame, 1194, 672, &settings));
        assert_eq!(frame.width(), ninja_master_native::WIDE_WIDTH);
        assert!(frame.smooth_wave_effects);
        assert_eq!(view_width(199, 112, &settings), 398);
        assert_eq!(view_width(1080, 1920, &settings), WIDTH);
        assert_eq!(view_width(3440, 0, &settings), WIDTH);
        assert_eq!(view_width(0, 1440, &settings), WIDTH);
        assert_eq!(view_width(3440, 1, &settings), 4096);
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
