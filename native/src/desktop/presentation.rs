use ninja_master_native::{HEIGHT, WIDTH, render::Frame, settings::Settings};
use sdl2::{rect::Rect, render::WindowCanvas};

/// The largest whole multiple of the frame that fits the window, centred.
pub(super) fn game_rect(canvas: &WindowCanvas, settings: &Settings) -> Result<Rect, String> {
    let (width, height) = canvas.output_size()?;
    Ok(presentation_rect(width, height, settings))
}

fn art_output_scale(rect: Rect, width: usize) -> usize {
    (rect.width() as usize)
        .div_ceil(width)
        .max((rect.height() as usize).div_ceil(HEIGHT))
        .max(1)
}

pub(super) fn fit_art_to_window(
    frame: &mut Frame,
    canvas: &WindowCanvas,
    settings: &Settings,
) -> Result<bool, String> {
    Ok(frame.set_output_scale(art_output_scale(
        game_rect(canvas, settings)?,
        frame.width(),
    )))
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

pub(super) fn set_icon(window: &mut sdl2::video::Window) -> Result<(), String> {
    let mut data =
        sdl2::rwops::RWops::from_bytes(include_bytes!("../../resources/ninja-master.bmp"))?;
    let mut icon = sdl2::surface::Surface::load_bmp_rw(&mut data)?;
    icon.set_color_key(true, sdl2::pixels::Color::BLACK)?;
    window.set_icon(icon);
    Ok(())
}

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

/// Dim the unfocused presentation without changing the game frame.
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
    use super::*;

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
