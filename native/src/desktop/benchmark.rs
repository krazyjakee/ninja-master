//! Reproducible renderer experiments; all default artwork is procedural.
use super::{
    crt::Crt,
    presentation,
    tile_batch::{Batch, Command, GpuBatch},
    transfer,
};
use ninja_master_native::{
    HEIGHT,
    art_pack::{ArtPack, Cell, Manifest, Sheet},
    game::Game,
    input::Input,
    render::{CramChange, Flip, Frame, Palette, Tile},
    settings::Settings,
    stage::StageId,
};
use sdl2::{
    pixels::{Color, PixelFormatEnum},
    rect::Rect,
};
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
    rc::Rc,
    time::Instant,
};

fn ms(start: Instant) -> f32 {
    start.elapsed().as_secs_f32() * 1000.0
}

pub(super) fn fixture(width: usize, tick: usize) -> Batch {
    let tiles: Vec<_> = (0..32)
        .map(|id| {
            Tile(std::array::from_fn(|i| {
                let x = i % 4 * 2;
                let y = i / 4;
                let pixel = |x| {
                    if (x + y + id) % 7 == 0 {
                        0
                    } else {
                        ((x * 3 + y + id) % 15 + 1) as u8
                    }
                };
                pixel(x) << 4 | pixel(x + 1)
            }))
        })
        .collect();
    let palettes: Vec<Palette> = (0..4)
        .map(|p| {
            std::array::from_fn(|i| {
                [
                    ((i * 17 + p * 19 + tick) % 256) as u8,
                    ((i * 11 + p * 53) % 256) as u8,
                    ((i * 7 + p * 31) % 256) as u8,
                ]
            })
        })
        .collect();
    let mut commands = Vec::new();
    let origin = (width as i32 - 320) / 2;
    for y in 0..HEIGHT.div_ceil(8) {
        for x in 0..width.div_ceil(8) {
            commands.push(Command {
                tile: (x + y) % 32,
                palette: y % 4,
                x: x as i32 * 8 - origin,
                y: y as i32 * 8,
                flip: Flip {
                    x: x % 2 == 0,
                    y: y % 3 == 0,
                },
                rows: 255,
                end: HEIGHT,
            });
        }
    }
    // Ordered sprite overlaps, transparent texels, flips, scanline masks and clips.
    for i in 0..160 {
        commands.push(Command {
            tile: i % 32,
            palette: i % 4,
            x: ((i * 29 + tick) % (width + 16)) as i32 - origin - 8,
            y: ((i * 17) % 240) as i32 - 8,
            flip: Flip {
                x: i % 2 == 0,
                y: i % 3 == 0,
            },
            rows: if i % 5 == 0 { 0b10110101 } else { 255 },
            end: if i % 7 == 0 { 200 } else { HEIGHT },
        });
    }
    let mut changed = palettes[1];
    changed.rotate_left(3);
    Batch {
        width,
        tiles,
        palettes,
        commands,
        backdrop: [17, 23, 31],
        changes: vec![CramChange {
            line: 113,
            cram_line: 1,
            colors: changed,
        }],
    }
}

pub(super) fn hd_pack(directory: &Path) -> Result<Rc<ArtPack>, String> {
    let directory = directory.join("procedural-hd10");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let batch = fixture(320, 0);
    let scale = 10;
    let width = 128;
    let height = 64;
    let mut pixels = vec![0u8; width * height * scale * scale * 4];
    let mut cells = Vec::new();
    for line in 0..4 {
        for (id, tile) in batch.tiles.iter().enumerate() {
            let x = id % 16 * 8;
            let y = (line * 2 + id / 16) * 8;
            cells.push(Cell {
                tile: tile.0.iter().map(|b| format!("{b:02x}")).collect(),
                line: line as u8,
                x,
                y,
                flip_x: false,
                flip_y: false,
                baseline: None,
            });
            for py in 0..8 * scale {
                for px in 0..8 * scale {
                    let byte = tile.0[(py / scale) * 4 + px / scale / 2];
                    let index = if (px / scale) % 2 == 0 {
                        byte >> 4
                    } else {
                        byte & 15
                    };
                    let color = batch.palettes[line][index as usize];
                    let at = ((y * scale + py) * width * scale + x * scale + px) * 4;
                    pixels[at..at + 4].copy_from_slice(&[
                        color[0].saturating_add(1 + (px % 3) as u8),
                        color[1],
                        color[2],
                        if index == 0 {
                            0
                        } else if (px + py) % 11 == 0 {
                            128
                        } else {
                            255
                        },
                    ]);
                }
            }
        }
    }
    let file = fs::File::create(directory.join("tiles.png")).map_err(|e| e.to_string())?;
    let mut png = png::Encoder::new(file, (width * scale) as u32, (height * scale) as u32);
    png.set_color(png::ColorType::Rgba);
    png.set_depth(png::BitDepth::Eight);
    png.write_header()
        .map_err(|e| e.to_string())?
        .write_image_data(&pixels)
        .map_err(|e| e.to_string())?;
    let manifest = Manifest {
        version: 1,
        name: "Procedural renderer benchmark".into(),
        sheets: vec![Sheet {
            file: "tiles.png".into(),
            width,
            height,
            palettes: batch.palettes.try_into().unwrap(),
            cells,
        }],
        player_frames: vec![],
    };
    fs::write(
        directory.join("pack.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    ArtPack::load(&directory).map(Rc::new)
}

/// CSV times are CPU wall times; explicit GL completion is a separate column.
pub(crate) fn run(directory: &Path, samples: u64, rom_path: Option<&Path>) -> Result<(), String> {
    if samples == 0 || samples > 10_000 {
        return Err("Rendering benchmark requires 1..10000 samples".into());
    }
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let pack = hd_pack(directory)?;
    let rom = rom_path.map(crate::rom_source::read).transpose()?;
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let driver = sdl2::render::drivers()
        .position(|d| d.name == "opengl")
        .ok_or("Benchmark requires SDL's OpenGL renderer")? as u32;
    let window = video
        .window("Ninja Master rendering benchmark", 3840, 2160)
        .position(0, 0)
        .build()
        .map_err(|e| e.to_string())?;
    let mut canvas = window
        .into_canvas()
        .index(driver)
        .build()
        .map_err(|e| e.to_string())?;
    let mut events = sdl.event_pump()?;
    let creator = canvas.texture_creator();
    let gpu = GpuBatch::new(&video, &mut canvas, &fixture(320, 0))?;
    let source = concat!(
        include_str!("../desktop.rs"),
        include_str!("crt.rs"),
        include_str!("crt.frag"),
        include_str!("transfer.rs"),
        include_str!("tile_batch.rs"),
        include_str!("tile_batch.vert"),
        include_str!("tile_batch.frag"),
        include_str!("benchmark.rs"),
        include_str!("../render.rs"),
        include_str!("../render/art.rs"),
        include_str!("../render/sat.rs")
    );
    let metadata = serde_json::json!({"source_crc32":format!("{:08x}",ninja_master_native::assets::crc32(source.as_bytes())),"started_unix_seconds":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e|e.to_string())?.as_secs(),"version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,
        "gpu":gpu.description(),"sdl":sdl2::version::version().to_string(),"renderer":canvas.info().name,
        "samples":samples,"warmup":5,"vsync_requested":false,"gpu_completion":"glFinish after submission; separate gpu_wait_ms",
        "artwork":"procedural indexed tiles and a generated 10x RGBA replacement pack; no original assets",
        "rom_crc32":rom.as_ref().map(|r|format!("{:08x}",ninja_master_native::assets::crc32(r))),
        "prototype_scope":"native indexed atlas only; CPU remains authoritative for gameplay, HD tint/alpha and raster waves",
        "command":std::env::args().collect::<Vec<_>>(),
        "cpu":std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|s|s.lines().find(|l|l.starts_with("model name")).map(str::to_owned)),
        "revision":std::process::Command::new("git").args(["rev-parse","HEAD"]).output().ok().filter(|o|o.status.success()).map(|o|String::from_utf8_lossy(&o.stdout).trim().to_owned())});
    fs::write(
        directory.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    drop(gpu);
    let mut csv =
        BufWriter::new(fs::File::create(directory.join("frames.csv")).map_err(|e| e.to_string())?);
    writeln!(csv,"scene,viewport_width,viewport_height,view,art,path,sample,compose_ms,resolve_ms,upload_ms,submit_ms,gpu_wait_ms,present_ms,frame_ms,upload_bytes,atlas_init_bytes,draw_calls").map_err(|e|e.to_string())?;
    let mut comparisons = Vec::new();
    for (vw, vh) in [(1920, 1080), (2560, 1440), (3840, 2160), (3440, 1440)] {
        for wide in [false, true] {
            for hd in [false, true] {
                let settings = Settings {
                    widescreen: wide,
                    integer_scale: false,
                    ..Settings::default()
                };
                let mut frame = Frame::default();
                if hd {
                    frame.set_art_pack(Some(pack.clone()));
                }
                presentation::fit_frame_to_output(&mut frame, vw, vh, &settings);
                let rect = presentation::presentation_rect(vw, vh, &settings);
                let width = frame.width();
                let mut texture = creator
                    .create_texture_streaming(
                        PixelFormatEnum::RGB24,
                        frame.output_width() as u32,
                        frame.output_height() as u32,
                    )
                    .map_err(|e| e.to_string())?;
                let mut crt = Crt::new(&video, &mut canvas);
                let mut buffers = transfer::Buffers::default();
                let mut gpu = if hd {
                    None
                } else {
                    Some(GpuBatch::new(&video, &mut canvas, &fixture(width, 0))?)
                };
                if let Some(gpu) = &gpu {
                    // Compare at native resolution to avoid fractional sampling ties.
                    let batch = fixture(width, 0);
                    batch.cpu_draw(&mut frame);
                    canvas.set_draw_color(Color::RGB(17, 23, 31));
                    canvas.clear();
                    let exact = Rect::new(0, 0, width as u32, HEIGHT as u32);
                    gpu.upload_palettes(&mut canvas, &batch)?;
                    gpu.draw(&mut canvas, exact, &batch)?;
                    let actual = canvas.read_pixels(exact, PixelFormatEnum::RGB24)?;
                    let expected = frame.rgb();
                    let differences = actual.iter().zip(expected).filter(|(a, b)| a != b).count();
                    comparisons.push(serde_json::json!({"width":width,"different_channels":differences,"channels":expected.len()}));
                    if differences != 0 {
                        for (name, pixels) in [("gpu", actual.as_slice()), ("cpu", expected)] {
                            let mut image = png::Encoder::new(
                                fs::File::create(directory.join(format!("{name}-mismatch.png")))
                                    .map_err(|e| e.to_string())?,
                                width as u32,
                                HEIGHT as u32,
                            );
                            image.set_color(png::ColorType::Rgb);
                            image
                                .write_header()
                                .map_err(|e| e.to_string())?
                                .write_image_data(pixels)
                                .map_err(|e| e.to_string())?;
                        }
                        return Err(format!(
                            "GPU batch differs from CPU at width {width}: {differences} channels"
                        ));
                    }
                }
                for path in ["flat", "legacy-crt", "single-crt", "indexed-batch"] {
                    if hd && path == "indexed-batch" {
                        continue;
                    }
                    let settings = Settings {
                        crt: path.contains("crt"),
                        ..settings.clone()
                    };
                    for sample in 0..samples + 5 {
                        if events
                            .poll_iter()
                            .any(|e| matches!(e, sdl2::event::Event::Quit { .. }))
                        {
                            return Err("Rendering benchmark cancelled".into());
                        }
                        let batch = fixture(width, sample as usize);
                        let total = Instant::now();
                        let start = Instant::now();
                        if path != "indexed-batch" {
                            batch.cpu_draw(&mut frame);
                        }
                        let compose = ms(start);
                        let mut transfer = transfer::Transfer::default();
                        if path != "indexed-batch" {
                            if path == "legacy-crt" {
                                let start = Instant::now();
                                let pixels = frame.presentation_rgb();
                                transfer.resolve_ms = ms(start);
                                let start = Instant::now();
                                texture
                                    .update(None, pixels, frame.output_width() * 3)
                                    .map_err(|e| e.to_string())?;
                                transfer.upload_ms = ms(start);
                                transfer.bytes = pixels.len();
                            }
                            let next = buffers.upload(
                                &mut canvas,
                                &mut texture,
                                &mut crt,
                                &frame,
                                &settings,
                                true,
                            )?;
                            transfer.resolve_ms += next.resolve_ms;
                            transfer.upload_ms += next.upload_ms;
                            transfer.bytes += next.bytes;
                            if settings.crt && crt.is_err() {
                                return Err(format!(
                                    "Benchmark CRT unavailable: {}",
                                    crt.as_ref().err().unwrap()
                                ));
                            }
                        }
                        if path == "indexed-batch" {
                            transfer =
                                gpu.as_ref().unwrap().upload_palettes(&mut canvas, &batch)?;
                        }
                        canvas.set_draw_color(Color::RGB(17, 23, 31));
                        canvas.clear();
                        let start = Instant::now();
                        if path == "indexed-batch" {
                            let gpu = gpu.as_mut().unwrap();
                            gpu.draw(&mut canvas, rect, &batch)?;
                        } else if settings.crt {
                            crt.as_ref().unwrap().draw(
                                &mut canvas,
                                rect,
                                width,
                                settings.crt_curvature,
                                settings.scanlines,
                            )?;
                        } else {
                            canvas.copy(&texture, None, rect)?;
                        }
                        let submit = ms(start);
                        let start = Instant::now();
                        // Complete both SDL's queue and GPU work before measuring the swap.
                        crt.as_ref().map_err(Clone::clone)?.finish(&mut canvas)?;
                        let wait = ms(start);
                        let start = Instant::now();
                        canvas.present();
                        let present = ms(start);
                        let total = ms(total);
                        if sample >= 5 {
                            writeln!(csv,"procedural,{vw},{vh},{},{},{path},{},{compose:.6},{:.6},{:.6},{submit:.6},{wait:.6},{present:.6},{total:.6},{},{},1",
                        if wide {"wide"} else {"original"},if hd {"hd10"} else {"native"},sample-5,transfer.resolve_ms,transfer.upload_ms,transfer.bytes,
                        gpu.as_ref().filter(|_|path=="indexed-batch").map_or(0,GpuBatch::atlas_upload_bytes)).map_err(|e|e.to_string())?;
                        }
                    }
                }
                eprintln!("Rendering benchmark: {vw}x{vh}, wide={wide}, HD={hd}");
            }
        }
    }
    // Optional real cartridge scenes exercise stage-specific CPU composition.
    if let Some(rom) = rom {
        for stage in [
            StageId::new(1, 1, 0),
            StageId::new(3, 2, 0),
            StageId::new(4, 3, 0),
        ] {
            for (vw, vh) in [(1920, 1080), (3840, 2160)] {
                for path in ["flat", "legacy-crt", "single-crt"] {
                    let mut game = Game::load_stage(&rom, stage)?;
                    let settings = Settings {
                        widescreen: true,
                        integer_scale: false,
                        crt: path.contains("crt"),
                        ..Settings::default()
                    };
                    let mut frame = Frame::default();
                    presentation::fit_frame_to_output(&mut frame, vw, vh, &settings);
                    let mut texture = creator
                        .create_texture_streaming(
                            PixelFormatEnum::RGB24,
                            frame.output_width() as u32,
                            frame.output_height() as u32,
                        )
                        .map_err(|e| e.to_string())?;
                    let mut crt = Crt::new(&video, &mut canvas);
                    let mut buffers = transfer::Buffers::default();
                    for sample in 0..samples + 5 {
                        if events
                            .poll_iter()
                            .any(|e| matches!(e, sdl2::event::Event::Quit { .. }))
                        {
                            return Err("Rendering benchmark cancelled".into());
                        }
                        // Simulation and sound are deliberately outside rendering timings.
                        game.tick(Input::default());
                        let total = Instant::now();
                        let start = Instant::now();
                        game.draw(&mut frame);
                        let compose = ms(start);
                        let mut extra = transfer::Transfer::default();
                        if path == "legacy-crt" {
                            let start = Instant::now();
                            let pixels = frame.presentation_rgb();
                            extra.resolve_ms = ms(start);
                            let start = Instant::now();
                            texture
                                .update(None, pixels, frame.output_width() * 3)
                                .map_err(|e| e.to_string())?;
                            extra.upload_ms = ms(start);
                            extra.bytes = pixels.len();
                        }
                        let t = buffers.upload(
                            &mut canvas,
                            &mut texture,
                            &mut crt,
                            &frame,
                            &settings,
                            true,
                        )?;
                        canvas.set_draw_color(Color::BLACK);
                        canvas.clear();
                        let start = Instant::now();
                        // Draw at the requested virtual viewport within the larger drawable.
                        let rect = presentation::presentation_rect(vw, vh, &settings);
                        if settings.crt {
                            crt.as_ref().map_err(Clone::clone)?.draw(
                                &mut canvas,
                                rect,
                                frame.width(),
                                settings.crt_curvature,
                                settings.scanlines,
                            )?;
                        } else {
                            canvas.copy(&texture, None, rect)?;
                        }
                        let submit = ms(start);
                        let start = Instant::now();
                        crt.as_ref().map_err(Clone::clone)?.finish(&mut canvas)?;
                        let wait = ms(start);
                        let start = Instant::now();
                        canvas.present();
                        let present = ms(start);
                        let total = ms(total);
                        if sample >= 5 {
                            writeln!(csv,"stage-{stage},{vw},{vh},wide,native,{path},{},{compose:.6},{:.6},{:.6},{submit:.6},{wait:.6},{present:.6},{total:.6},{},0,1",sample-5,t.resolve_ms+extra.resolve_ms,t.upload_ms+extra.upload_ms,t.bytes+extra.bytes).map_err(|e|e.to_string())?;
                        }
                    }
                }
            }
        }
    }
    csv.flush().map_err(|e| e.to_string())?;
    fs::write(
        directory.join("comparisons.json"),
        serde_json::to_vec_pretty(&comparisons).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    eprintln!("Rendering benchmark written to {}", directory.display());
    Ok(())
}
