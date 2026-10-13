//! A cross-platform file picker built with the existing desktop UI.
use super::{gui::Gui, set_icon};
use crate::rom_source;
use imgui::Condition;
use sdl2::{event::Event, keyboard::Scancode, pixels::Color};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn entries(dir: &Path) -> Result<Vec<(PathBuf, bool)>, String> {
    let mut files = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let directory = path.is_dir();
            let bin = path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("bin"));
            (directory || bin).then_some((path, directory))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(files)
}

/// None means the user cancelled; rejected files leave the picker open.
pub(crate) fn pick_rom(initial: &Path, warning: &str) -> Result<Option<Vec<u8>>, String> {
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let mut window = video
        .window("Ninja Master — Locate ROM", 760, 560)
        .position_centered()
        .resizable()
        .allow_highdpi()
        .build()
        .map_err(|e| e.to_string())?;
    set_icon(&mut window)?;
    let mut canvas = window.into_canvas().build().map_err(|e| e.to_string())?;
    let creator = canvas.texture_creator();
    let mut gui = Gui::new(&creator, None)?;
    let mut events = sdl.event_pump()?;
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut dir = initial
        .parent()
        .filter(|p| p.is_dir())
        .map(Path::to_owned)
        .or_else(|| home.clone())
        .unwrap_or_else(|| PathBuf::from("."));
    dir = fs::canonicalize(&dir).unwrap_or(dir);
    let mut listing = entries(&dir);
    let mut location = dir.to_string_lossy().into_owned();
    let mut selected = String::new();
    let mut message = warning.to_owned();
    loop {
        for event in events.poll_iter() {
            if matches!(
                event,
                Event::Quit { .. }
                    | Event::KeyDown {
                        scancode: Some(Scancode::Escape),
                        ..
                    }
            ) {
                return Ok(None);
            }
            gui.handle_event(&event);
        }
        if gui.wants_text() {
            video.text_input().start();
        } else {
            video.text_input().stop();
        }
        canvas.set_draw_color(Color::RGB(19, 20, 24));
        canvas.clear();
        let mut navigate = None;
        let mut choose = None;
        let mut cancel = false;
        let (width, height) = canvas.window().size();
        gui.draw(&mut canvas, |ui| {
            ui.window("Locate your Shinobi III US ROM")
                .position([0.0, 0.0], Condition::Always)
                .size([width as f32, height as f32], Condition::Always)
                .title_bar(false).resizable(false).movable(false).build(|| {
                    ui.text("A user-supplied ROM is required");
                    ui.text_wrapped("This release contains no game data. Select your own plain US cartridge dump (.bin, 1 MiB). No download is provided.");
                    let warning_color = ui.push_style_color(imgui::StyleColor::Text, [1.0, 0.55, 0.48, 1.0]);
                    ui.text_wrapped(&message);
                    drop(warning_color);
                    ui.separator();
                    ui.input_text("Folder", &mut location).build();
                    if ui.button("Go") { navigate = Some(PathBuf::from(&location)); }
                    ui.same_line();
                    if ui.button("Parent") { navigate = dir.parent().map(Path::to_owned); }
                    if let Some(home) = &home {
                        ui.same_line();
                        if ui.button("Home") { navigate = Some(home.clone()); }
                    }
                    #[cfg(target_os = "windows")]
                    {
                        ui.same_line();
                        if ui.button("C:\\") { navigate = Some(PathBuf::from("C:\\")); }
                    }
                    ui.child_window("files").size([0.0, (height as f32 - 290.0).max(80.0)]).build(|| {
                        match &listing {
                            Ok(files) => {
                                for (index, (path, directory)) in files.iter().enumerate() {
                                    let _id = ui.push_id_usize(index);
                                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                                    let label = if *directory { format!("[Folder] {name}") } else { name.into_owned() };
                                    if ui.selectable(&label) {
                                        if *directory { navigate = Some(path.clone()); }
                                        else { selected = path.to_string_lossy().into_owned(); }
                                    }
                                }
                            }
                            Err(error) => ui.text_wrapped(error),
                        }
                    });
                    ui.input_text("ROM file", &mut selected).build();
                    if ui.button("Open ROM") && !selected.is_empty() { choose = Some(PathBuf::from(&selected)); }
                    ui.same_line();
                    if ui.button("Cancel") { cancel = true; }
                });
        })?;
        canvas.present();
        if cancel {
            return Ok(None);
        }
        if let Some(path) = navigate {
            dir = fs::canonicalize(&path).unwrap_or(path);
            location = dir.to_string_lossy().into_owned();
            listing = entries(&dir);
        }
        if let Some(path) = choose {
            let path = if path.is_absolute() {
                path
            } else {
                dir.join(path)
            };
            match rom_source::read(&path) {
                Ok(data) => {
                    if let Err(error) = rom_source::remember(&path) {
                        eprintln!("warning: could not remember ROM path: {error}");
                    }
                    return Ok(Some(data));
                }
                Err(error) => message = error,
            }
        }
        std::thread::sleep(Duration::from_millis(16));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_picker_lists_folders_and_bin_files() {
        let dir = std::env::temp_dir().join(format!("ninja-picker-{}", std::process::id()));
        fs::create_dir_all(dir.join("folder")).unwrap();
        fs::write(dir.join("game.BIN"), b"test").unwrap();
        fs::write(dir.join("notes.txt"), b"test").unwrap();
        let files = entries(&dir).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].1);
        assert_eq!(files[1].0.file_name().unwrap(), "game.BIN");
        assert!(entries(&dir.join("missing")).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    // Run with SDL_VIDEODRIVER=dummy SDL_RENDER_DRIVER=software.
    // Ignored in the normal suite, which has no display requirement.
    #[test]
    #[ignore = "requires an SDL video driver"]
    fn rom_picker_renders_warning_and_cancels() {
        let sdl = sdl2::init().unwrap();
        let events = sdl.event().unwrap();
        let sender = events.event_sender();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            sender.push_event(Event::Quit { timestamp: 0 }).unwrap();
        });
        assert!(
            pick_rom(
                Path::new("missing.bin"),
                "ROM not found. Choose your US cartridge dump."
            )
            .unwrap()
            .is_none()
        );
        cancel.join().unwrap();
    }
}
