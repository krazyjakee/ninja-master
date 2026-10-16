//! Capture the displayed game viewport before desktop overlays are drawn.
use sdl2::{pixels::PixelFormatEnum, rect::Rect, render::WindowCanvas};
use std::{
    fs::{self, OpenOptions},
    io::{self, BufWriter},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default)]
pub(super) struct Capture {
    pending: Option<Receiver<Result<PathBuf, String>>>,
}
impl Capture {
    pub fn request(
        &mut self,
        canvas: &mut WindowCanvas,
        rect: Rect,
        directory: PathBuf,
    ) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("A screenshot is still being saved".into());
        }
        let pixels = canvas.read_pixels(rect, PixelFormatEnum::RGB24)?;
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("screenshot".into())
            .spawn(move || {
                let result =
                    save_png(&directory, rect.width(), rect.height(), &pixels).map_err(|error| {
                        format!(
                            "Could not save screenshot in {}: {error}",
                            directory.display()
                        )
                    });
                let _ = sender.send(result);
            })
            .map_err(|error| error.to_string())?;
        self.pending = Some(receiver);
        Ok(())
    }
    pub fn poll(&mut self) -> Option<String> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("Screenshot writer stopped unexpectedly".into()),
        };
        self.pending = None;
        Some(match result {
            Ok(path) => format!("Screenshot saved: {}", path.display()),
            Err(error) => error,
        })
    }
    pub fn finish(&mut self) {
        if let Some(receiver) = self.pending.take() {
            match receiver.recv() {
                Ok(Ok(path)) => eprintln!("Screenshot saved: {}", path.display()),
                Ok(Err(error)) => eprintln!("{error}"),
                Err(error) => eprintln!("Screenshot writer: {error}"),
            }
        }
    }
}

fn save_png(directory: &Path, width: u32, height: u32, pixels: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for sequence in 0..1000 {
        let path = directory.join(format!(
            "ninja-master-{stamp}-{}-{sequence}.png",
            std::process::id()
        ));
        let file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(pixels)?;
            writer.finish()?;
            Ok(())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Screenshot filename space exhausted",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a desktop SDL display with a window manager"]
    fn capture_crops_the_viewport_before_overlays_in_windowed_and_fullscreen() {
        use ninja_master_native::settings::Settings;
        use sdl2::pixels::Color;
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("ninja-viewport-{}-{stamp}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let window = video
            .window("Screenshot viewport test", 640, 480)
            .resizable()
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().build().unwrap();
        let mut settings = Settings {
            widescreen: false,
            scale: 2,
            aspect: 1,
            integer_scale: false,
            ..Default::default()
        };
        let mut events = sdl.event_pump().unwrap();
        let creator = canvas.texture_creator();
        let mut gui = super::super::gui::Gui::new(&creator, None).unwrap();
        gui.draw(&mut canvas, |_| {}).unwrap();
        let mut capture = Capture::default();
        for (index, fullscreen) in [false, true, false].into_iter().enumerate() {
            settings.fullscreen = fullscreen;
            super::super::presentation::apply_window_settings(&mut canvas, &settings).unwrap();
            // Let the window system resize its backing buffer, as several
            // normal presentation frames do after a production mode change.
            for _ in 0..3 {
                canvas.present();
                std::thread::sleep(std::time::Duration::from_millis(16));
                events.pump_events();
            }
            let rect = super::super::presentation::game_rect(&canvas, &settings).unwrap();
            canvas.set_draw_color(Color::BLACK);
            canvas.clear();
            canvas.set_draw_color(Color::GREEN);
            canvas.fill_rect(rect).unwrap();
            capture
                .request(&mut canvas, rect, directory.clone())
                .unwrap();
            assert!(
                capture
                    .request(&mut canvas, rect, directory.clone())
                    .is_err()
            );
            // F1/F2 are drawn later in the production loop.
            canvas.set_draw_color(Color::RED);
            canvas.fill_rect(rect).unwrap();
            capture.finish();
            let paths: Vec<_> = std::fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(paths.len(), index + 1);
            let latest = paths.iter().max().unwrap();
            let mut reader = png::Decoder::new(std::fs::File::open(latest).unwrap())
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height), (rect.width(), rect.height()));
            assert!(
                pixels
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .all(|rgb| *rgb == [0, 255, 0])
            );
            canvas.present();
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn png_round_trip_unique_names_and_useful_errors() {
        let directory = std::env::temp_dir().join(format!("ninja-capture-{}", std::process::id()));
        let pixels = [255, 0, 0, 0, 255, 0];
        let first = save_png(&directory, 2, 1, &pixels).unwrap();
        let second = save_png(&directory, 2, 1, &pixels).unwrap();
        assert_ne!(first, second);
        let mut reader = png::Decoder::new(std::fs::File::open(&first).unwrap())
            .read_info()
            .unwrap();
        let mut actual = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut actual).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(actual, pixels);
        assert!(save_png(&first, 2, 1, &pixels).is_err());
        assert!(save_png(&directory, 2, 1, &[]).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        fs::remove_dir_all(directory).unwrap();
    }
}
