//! One upload per active presentation path. RGB resolution is measured separately.
use super::{
    crt::Crt,
    presentation::{gray_out, scanlines},
};
use ninja_master_native::{render::Frame, settings::Settings};
use sdl2::render::{Texture, WindowCanvas};
use std::time::Instant;

#[derive(Clone, Copy, Default)]
pub(super) struct Transfer {
    pub resolve_ms: f32,
    pub upload_ms: f32,
    pub bytes: usize,
}

#[derive(Default)]
pub(super) struct Buffers {
    inactive: Vec<u8>,
    scanlines: Vec<u8>,
}

impl Buffers {
    pub fn upload(
        &mut self,
        canvas: &mut WindowCanvas,
        texture: &mut Texture<'_>,
        crt: &mut Result<Crt, String>,
        frame: &Frame,
        settings: &Settings,
        focused: bool,
    ) -> Result<Transfer, String> {
        let start = Instant::now();
        let pixels = if focused {
            frame.presentation_rgb()
        } else {
            gray_out(frame.presentation_rgb(), &mut self.inactive);
            &self.inactive
        };
        let mut result = Transfer {
            resolve_ms: start.elapsed().as_secs_f32() * 1000.0,
            ..Transfer::default()
        };
        if settings.crt
            && let Ok(effect) = crt
        {
            let start = Instant::now();
            let uploaded =
                effect.upload(canvas, pixels, frame.output_width(), frame.output_height());
            result.upload_ms += start.elapsed().as_secs_f32() * 1000.0;
            result.bytes += pixels.len();
            match uploaded {
                Ok(()) => return Ok(result),
                Err(error) => {
                    eprintln!("CRT disabled: {error}");
                    *crt = Err(error);
                }
            }
        }
        let start = Instant::now();
        let pixels = if settings.scanlines == 0 {
            pixels
        } else {
            scanlines(
                pixels,
                frame.output_width(),
                frame.output_scale(),
                settings.scanlines,
                &mut self.scanlines,
            );
            &self.scanlines
        };
        result.resolve_ms += start.elapsed().as_secs_f32() * 1000.0;
        let start = Instant::now();
        texture
            .update(None, pixels, frame.output_width() * 3)
            .map_err(|e| e.to_string())?;
        result.upload_ms += start.elapsed().as_secs_f32() * 1000.0;
        result.bytes += pixels.len();
        Ok(result)
    }
}

/// A failed CRT draw falls back in this same frame, including a fresh flat upload.
pub(super) fn draw(
    canvas: &mut WindowCanvas,
    texture: &mut Texture<'_>,
    crt: &mut Result<Crt, String>,
    frame: &Frame,
    settings: &Settings,
    focused: bool,
    buffers: &mut Buffers,
) -> Result<Option<Transfer>, String> {
    let rect = super::presentation::game_rect(canvas, settings)?;
    if settings.crt
        && let Ok(effect) = crt
    {
        match effect.draw(
            canvas,
            rect,
            frame.width(),
            settings.crt_curvature,
            settings.scanlines,
        ) {
            Ok(()) => return Ok(None),
            Err(error) => {
                eprintln!("CRT disabled: {error}");
                *crt = Err(error);
                let transfer = buffers.upload(canvas, texture, crt, frame, settings, focused)?;
                canvas.copy(texture, None, rect)?;
                return Ok(Some(transfer));
            }
        }
    }
    canvas.copy(texture, None, rect)?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl2::pixels::{Color, PixelFormatEnum};

    #[test]
    #[ignore = "requires a desktop OpenGL display"]
    fn rendering_uploads_only_the_active_texture_and_switches_without_ticks() {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let driver = sdl2::render::drivers()
            .position(|d| d.name == "opengl")
            .unwrap() as u32;
        let window = video
            .window("Upload routing regression", 640, 448)
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().index(driver).build().unwrap();
        let creator = canvas.texture_creator();
        let mut texture = creator
            .create_texture_streaming(PixelFormatEnum::RGB24, 320, 224)
            .unwrap();
        texture
            .update(None, &[255, 0, 255].repeat(320 * 224), 320 * 3)
            .unwrap();
        let mut crt = Crt::new(&video, &mut canvas);
        let mut buffers = Buffers::default();
        let mut frame = Frame::default();
        frame.clear([0, 255, 0]);
        let mut settings = Settings {
            widescreen: false,
            crt: true,
            crt_curvature: 0,
            scanlines: 0,
            ..Settings::default()
        };
        let transfer = buffers
            .upload(&mut canvas, &mut texture, &mut crt, &frame, &settings, true)
            .unwrap();
        assert_eq!(transfer.bytes, 320 * 224 * 3);
        // The inactive SDL texture must still contain its poison colour.
        canvas.copy(&texture, None, None).unwrap();
        assert_eq!(
            canvas
                .read_pixels(
                    sdl2::rect::Rect::new(100, 100, 1, 1),
                    PixelFormatEnum::RGB24
                )
                .unwrap(),
            [255, 0, 255]
        );
        // Toggle both ways without a simulation tick, including a changed frame.
        for (effect, color) in [
            (false, [255, 0, 0]),
            (true, [0, 255, 0]),
            (false, [0, 0, 255]),
        ] {
            settings.crt = effect;
            frame.clear(color);
            let transfer = buffers
                .upload(&mut canvas, &mut texture, &mut crt, &frame, &settings, true)
                .unwrap();
            assert_eq!(transfer.bytes, 320 * 224 * 3);
            canvas.set_draw_color(Color::BLACK);
            canvas.clear();
            assert!(
                draw(
                    &mut canvas,
                    &mut texture,
                    &mut crt,
                    &frame,
                    &settings,
                    true,
                    &mut buffers
                )
                .unwrap()
                .is_none()
            );
            let pixel = canvas
                .read_pixels(
                    sdl2::rect::Rect::new(100, 100, 1, 1),
                    PixelFormatEnum::RGB24,
                )
                .unwrap();
            let channel = color.iter().position(|&c| c == 255).unwrap();
            assert!(pixel[channel] > 100);
            assert_eq!(pixel[(channel + 1) % 3], 0);
            assert_eq!(pixel[(channel + 2) % 3], 0);
            canvas.present();
        }
        // A draw failure must upload the current frame to the previously inactive
        // SDL texture before displaying it, rather than showing its old blue frame.
        settings.crt = true;
        frame.clear([0, 255, 0]);
        buffers
            .upload(&mut canvas, &mut texture, &mut crt, &frame, &settings, true)
            .unwrap();
        crt.as_ref().unwrap().inject_error_for_test();
        let retry = draw(
            &mut canvas,
            &mut texture,
            &mut crt,
            &frame,
            &settings,
            true,
            &mut buffers,
        )
        .unwrap()
        .unwrap();
        assert_eq!(retry.bytes, 320 * 224 * 3);
        assert!(crt.is_err());
        assert_eq!(
            canvas
                .read_pixels(
                    sdl2::rect::Rect::new(100, 100, 1, 1),
                    PixelFormatEnum::RGB24
                )
                .unwrap(),
            [0, 255, 0]
        );
        // Shader unavailable: scanlines and focus dimming must still be applied.
        crt = Err("injected shader failure".into());
        settings.crt = true;
        settings.scanlines = 50;
        frame.clear([200; 3]);
        buffers
            .upload(
                &mut canvas,
                &mut texture,
                &mut crt,
                &frame,
                &settings,
                false,
            )
            .unwrap();
        canvas.clear();
        draw(
            &mut canvas,
            &mut texture,
            &mut crt,
            &frame,
            &settings,
            false,
            &mut buffers,
        )
        .unwrap();
        let pixel = canvas
            .read_pixels(
                sdl2::rect::Rect::new(100, 100, 1, 1),
                PixelFormatEnum::RGB24,
            )
            .unwrap();
        assert_eq!(pixel, [120; 3]);
        let pixel = canvas
            .read_pixels(
                sdl2::rect::Rect::new(100, 102, 1, 1),
                PixelFormatEnum::RGB24,
            )
            .unwrap();
        assert_eq!(pixel, [60; 3]);
    }

    #[test]
    #[ignore = "requires a desktop OpenGL display"]
    fn rendering_single_upload_matches_legacy_for_hd_and_native_frames() {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let driver = sdl2::render::drivers()
            .position(|d| d.name == "opengl")
            .unwrap() as u32;
        let window = video
            .window("Rendering fidelity regression", 1024, 768)
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().index(driver).build().unwrap();
        let creator = canvas.texture_creator();
        let directory =
            std::env::temp_dir().join(format!("ninja-render-transfer-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let pack = super::super::benchmark::hd_pack(&directory).unwrap();
        for hd in [false, true] {
            let mut frame = Frame::new(398);
            if hd {
                frame.set_art_pack(Some(pack.clone()));
                frame.set_output_scale(2);
            }
            let mut texture = creator
                .create_texture_streaming(
                    PixelFormatEnum::RGB24,
                    frame.output_width() as u32,
                    frame.output_height() as u32,
                )
                .unwrap();
            let mut crt = Crt::new(&video, &mut canvas);
            let mut buffers = Buffers::default();
            for tick in [0, 19] {
                for curvature in [0, 25] {
                    super::super::benchmark::fixture(398, tick).cpu_draw(&mut frame);
                    let settings = Settings {
                        widescreen: true,
                        integer_scale: false,
                        crt: true,
                        crt_curvature: curvature,
                        ..Settings::default()
                    };
                    texture
                        .update(None, frame.presentation_rgb(), frame.output_width() * 3)
                        .unwrap();
                    crt.as_mut()
                        .unwrap()
                        .upload(
                            &mut canvas,
                            frame.presentation_rgb(),
                            frame.output_width(),
                            frame.output_height(),
                        )
                        .unwrap();
                    canvas.set_draw_color(Color::BLACK);
                    canvas.clear();
                    let rect = super::super::presentation::game_rect(&canvas, &settings).unwrap();
                    crt.as_ref()
                        .unwrap()
                        .draw(&mut canvas, rect, 398, curvature, settings.scanlines)
                        .unwrap();
                    let expected = canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
                    buffers
                        .upload(&mut canvas, &mut texture, &mut crt, &frame, &settings, true)
                        .unwrap();
                    canvas.clear();
                    draw(
                        &mut canvas,
                        &mut texture,
                        &mut crt,
                        &frame,
                        &settings,
                        true,
                        &mut buffers,
                    )
                    .unwrap();
                    let actual = canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
                    assert_eq!(
                        actual, expected,
                        "HD={hd}, tick={tick}, curvature={curvature}"
                    );
                }
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
