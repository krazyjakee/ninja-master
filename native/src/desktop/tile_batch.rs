//! Experimental indexed tile/sprite atlas compositor. Used by the benchmark,
//! not selected for gameplay until a complete command encoder proves fidelity.
use glow::HasContext;
use ninja_master_native::{
    HEIGHT,
    render::{Color, CramChange, Flip, Frame, Palette, PaletteRef, Tile},
};
use sdl2::{VideoSubsystem, rect::Rect, render::WindowCanvas};

#[derive(Clone, Copy)]
pub(super) struct Command {
    pub tile: usize,
    pub palette: usize,
    pub x: i32,
    pub y: i32,
    pub flip: Flip,
    pub rows: u8,
    pub end: usize,
}

pub(super) struct Batch {
    pub width: usize,
    pub tiles: Vec<Tile>,
    pub palettes: Vec<Palette>,
    pub changes: Vec<CramChange>,
    /// Painter's order: low planes, ordered sprites, high planes/sprites.
    pub commands: Vec<Command>,
    pub backdrop: Color,
}

impl Batch {
    pub fn cpu_draw(&self, frame: &mut Frame) {
        frame.clear(self.backdrop);
        for change in &self.changes {
            frame.change_cram_line(*change);
        }
        for command in &self.commands {
            let palette = PaletteRef::Cram(command.palette as u8, &self.palettes[command.palette]);
            frame.clip_lines(Some(command.end));
            if command.rows == 255 {
                frame.tile(
                    &self.tiles[command.tile],
                    palette,
                    command.x,
                    command.y,
                    command.flip,
                );
            } else {
                let slot = frame.slot(palette);
                let tile = &self.tiles[command.tile].0;
                for y in 0..8 {
                    if command.rows & (1 << y) == 0 {
                        continue;
                    }
                    for x in 0..8 {
                        let sx = if command.flip.x { 7 - x } else { x };
                        let sy = if command.flip.y { 7 - y } else { y };
                        let byte = tile[sy * 4 + sx / 2];
                        let index = if sx % 2 == 0 { byte >> 4 } else { byte & 15 };
                        if index != 0 {
                            frame.index_pixel(
                                command.x + x as i32,
                                command.y + y as i32,
                                slot,
                                index,
                            );
                        }
                    }
                }
            }
        }
        frame.clip_lines(None);
    }

    fn palette_rgb(&self) -> Vec<u8> {
        let mut palettes = self.palettes.clone();
        let mut changes = self.changes.iter().peekable();
        let mut pixels = Vec::with_capacity(self.palettes.len() * 16 * HEIGHT * 3);
        for y in 0..HEIGHT {
            while let Some(change) = changes.next_if(|c| c.line <= y) {
                if let Some(palette) = palettes.get_mut(usize::from(change.cram_line)) {
                    *palette = change.colors;
                }
            }
            for palette in &palettes {
                for color in palette {
                    pixels.extend(color);
                }
            }
        }
        pixels
    }
}

const VERTEX: &str = include_str!("tile_batch.vert");
const FRAGMENT: &str = include_str!("tile_batch.frag");

pub(super) struct GpuBatch {
    gl: glow::Context,
    program: glow::NativeProgram,
    atlas: glow::NativeTexture,
    palettes: glow::NativeTexture,
    atlas_height: usize,
    tile_count: usize,
    palette_count: usize,
    begin: unsafe extern "system" fn(u32),
    vertex: unsafe extern "system" fn(f32, f32),
    coord: unsafe extern "system" fn(f32, f32, f32, f32),
    end: unsafe extern "system" fn(),
}

/// Preserve the SDL context's state on success and on every error path.
struct State<'a> {
    gl: &'a glow::Context,
    program: Option<glow::NativeProgram>,
    viewport: [i32; 4],
    active: u32,
    textures: [Option<glow::NativeTexture>; 2],
    flags: [(u32, bool); 5],
    coord: [f32; 4],
    set_coord: unsafe extern "system" fn(f32, f32, f32, f32),
}
impl<'a> State<'a> {
    unsafe fn save(
        gl: &'a glow::Context,
        set_coord: unsafe extern "system" fn(f32, f32, f32, f32),
    ) -> Self {
        // SAFETY: the benchmark owns the current SDL compatibility GL context.
        unsafe {
            let program = gl.get_parameter_program(glow::CURRENT_PROGRAM);
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let active = gl.get_parameter_i32(glow::ACTIVE_TEXTURE) as u32;
            let textures = std::array::from_fn(|i| {
                gl.active_texture(glow::TEXTURE0 + i as u32);
                gl.get_parameter_texture(glow::TEXTURE_BINDING_2D)
            });
            let flags = [
                glow::BLEND,
                glow::SCISSOR_TEST,
                glow::DEPTH_TEST,
                glow::CULL_FACE,
                glow::DITHER,
            ]
            .map(|f| (f, gl.is_enabled(f)));
            let mut coord = [0.0; 4];
            gl.active_texture(glow::TEXTURE0);
            gl.get_parameter_f32_slice(0x0B03, &mut coord);
            Self {
                gl,
                program,
                viewport,
                active,
                textures,
                flags,
                coord,
                set_coord,
            }
        }
    }
}
impl Drop for State<'_> {
    fn drop(&mut self) {
        // SAFETY: state guards never outlive the current canvas/context.
        unsafe {
            for (i, texture) in self.textures.iter().enumerate() {
                self.gl.active_texture(glow::TEXTURE0 + i as u32);
                self.gl.bind_texture(glow::TEXTURE_2D, *texture);
            }
            self.gl.active_texture(self.active);
            self.gl.use_program(self.program);
            self.gl.viewport(
                self.viewport[0],
                self.viewport[1],
                self.viewport[2],
                self.viewport[3],
            );
            for (flag, on) in self.flags {
                if on {
                    self.gl.enable(flag);
                } else {
                    self.gl.disable(flag);
                }
            }
            (self.set_coord)(self.coord[0], self.coord[1], self.coord[2], self.coord[3]);
        }
    }
}

impl GpuBatch {
    pub fn new(
        video: &VideoSubsystem,
        canvas: &mut WindowCanvas,
        batch: &Batch,
    ) -> Result<Self, String> {
        if canvas.info().name != "opengl" {
            return Err("Tile batching requires desktop OpenGL".into());
        }
        // SAFETY: flush SDL before mixing GL calls; SDL owns a current context.
        unsafe {
            if sdl2::sys::SDL_RenderFlush(canvas.raw()) < 0 {
                return Err(sdl2::get_error());
            }
        }
        let begin = video.gl_get_proc_address("glBegin");
        let vertex = video.gl_get_proc_address("glVertex2f");
        let coord = video.gl_get_proc_address("glTexCoord4f");
        let end = video.gl_get_proc_address("glEnd");
        if [begin, vertex, coord, end].iter().any(|p| p.is_null()) {
            return Err("Compatibility drawing unavailable".into());
        }
        // SAFETY: checked function pointers belong to the current SDL context.
        unsafe {
            let gl = glow::Context::from_loader_function(|n| video.gl_get_proc_address(n).cast());
            let coord = std::mem::transmute::<
                *const (),
                unsafe extern "system" fn(f32, f32, f32, f32),
            >(coord);
            let max = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as usize;
            let height = batch.tiles.len().div_ceil(16).max(1) * 8;
            if height > max
                || batch.palettes.is_empty()
                || batch.palettes.len() * 16 > max
                || HEIGHT > max
            {
                return Err("Batch exceeds texture limits".into());
            }
            let program = super::crt::compile_sources(&gl, VERTEX, FRAGMENT)?;
            let atlas = match gl.create_texture() {
                Ok(t) => t,
                Err(e) => {
                    gl.delete_program(program);
                    return Err(e);
                }
            };
            let palettes = match gl.create_texture() {
                Ok(t) => t,
                Err(e) => {
                    gl.delete_texture(atlas);
                    gl.delete_program(program);
                    return Err(e);
                }
            };
            let result = Self {
                gl,
                program,
                atlas,
                palettes,
                atlas_height: height,
                tile_count: batch.tiles.len(),
                palette_count: batch.palettes.len(),
                begin: std::mem::transmute::<*const (), unsafe extern "system" fn(u32)>(begin),
                vertex: std::mem::transmute::<*const (), unsafe extern "system" fn(f32, f32)>(
                    vertex,
                ),
                coord,
                end: std::mem::transmute::<*const (), unsafe extern "system" fn()>(end),
            };
            {
                let _state = State::save(&result.gl, coord);
                let gl = &result.gl;
                let _unpack = super::crt::Unpack::new(gl);
                let mut indices = vec![0u8; 128 * height];
                for (id, tile) in batch.tiles.iter().enumerate() {
                    for y in 0..8 {
                        for x in 0..8 {
                            let byte = tile.0[y * 4 + x / 2];
                            indices[((id / 16) * 8 + y) * 128 + (id % 16) * 8 + x] =
                                if x % 2 == 0 { byte >> 4 } else { byte & 15 };
                        }
                    }
                }
                for (unit, texture, width, height, format, pixels) in [
                    (0, atlas, 128, height, glow::LUMINANCE, indices),
                    (
                        1,
                        palettes,
                        batch.palettes.len() * 16,
                        HEIGHT,
                        glow::RGB,
                        batch.palette_rgb(),
                    ),
                ] {
                    gl.active_texture(glow::TEXTURE0 + unit);
                    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                    gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        format as i32,
                        width as i32,
                        height as i32,
                        0,
                        format,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(Some(&pixels)),
                    );
                    for param in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                        gl.tex_parameter_i32(glow::TEXTURE_2D, param, glow::NEAREST as i32);
                    }
                    for param in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                        gl.tex_parameter_i32(glow::TEXTURE_2D, param, glow::CLAMP_TO_EDGE as i32);
                    }
                }
            }
            result.check()?;
            Ok(result)
        }
    }

    pub fn palette_upload_bytes(&self) -> usize {
        self.palette_count * 16 * HEIGHT * 3
    }
    pub fn atlas_upload_bytes(&self) -> usize {
        128 * self.atlas_height
    }

    pub fn upload_palettes(
        &self,
        canvas: &mut WindowCanvas,
        batch: &Batch,
    ) -> Result<super::transfer::Transfer, String> {
        if batch.palettes.len() != self.palette_count {
            return Err("Palette atlas dimensions changed".into());
        }
        let start = std::time::Instant::now();
        let pixels = batch.palette_rgb();
        let resolve_ms = start.elapsed().as_secs_f32() * 1000.0;
        let start = std::time::Instant::now();
        // SAFETY: the owning SDL context is current; all client upload state is restored.
        unsafe {
            if sdl2::sys::SDL_RenderFlush(canvas.raw()) < 0 {
                return Err(sdl2::get_error());
            }
            let _state = State::save(&self.gl, self.coord);
            let gl = &self.gl;
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.palettes));
            let _unpack = super::crt::Unpack::new(gl);
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                (self.palette_count * 16) as i32,
                HEIGHT as i32,
                glow::RGB,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&pixels)),
            );
        }
        self.check()?;
        Ok(super::transfer::Transfer {
            resolve_ms,
            upload_ms: start.elapsed().as_secs_f32() * 1000.0,
            bytes: self.palette_upload_bytes(),
        })
    }

    pub fn draw(&self, canvas: &mut WindowCanvas, rect: Rect, batch: &Batch) -> Result<(), String> {
        if batch.width < 320
            || batch.tiles.len() != self.tile_count
            || batch.palettes.len() != self.palette_count
            || batch
                .commands
                .iter()
                .any(|c| c.tile >= self.tile_count || c.palette >= self.palette_count)
        {
            return Err("Invalid batch command or changed atlas".into());
        }
        let (_, height) = canvas.output_size()?;
        // SAFETY: all indices are checked; draw and guard use the owning SDL context.
        unsafe {
            if sdl2::sys::SDL_RenderFlush(canvas.raw()) < 0 {
                return Err(sdl2::get_error());
            }
            let _state = State::save(&self.gl, self.coord);
            let gl = &self.gl;
            for (flag, _) in _state.flags {
                gl.disable(flag);
            }
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.palettes));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
            gl.viewport(
                rect.x(),
                height as i32 - rect.y() - rect.height() as i32,
                rect.width() as i32,
                rect.height() as i32,
            );
            gl.use_program(Some(self.program));
            gl.uniform_1_i32(gl.get_uniform_location(self.program, "atlas").as_ref(), 0);
            gl.uniform_1_i32(
                gl.get_uniform_location(self.program, "palettes").as_ref(),
                1,
            );
            gl.uniform_1_f32(
                gl.get_uniform_location(self.program, "palette_width")
                    .as_ref(),
                (self.palette_count * 16) as f32,
            );
            (self.begin)(glow::TRIANGLES);
            for c in &batch.commands {
                let mut y = 0;
                while y < 8 {
                    if c.rows & (1 << y) == 0 || c.y + y >= c.end as i32 {
                        y += 1;
                        continue;
                    }
                    let top = y;
                    y += 1;
                    while y < 8 && c.rows & (1 << y) != 0 && c.y + y < c.end as i32 {
                        y += 1;
                    }
                    for (dx, dy) in [(0, top), (0, y), (8, top), (8, top), (0, y), (8, y)] {
                        let u = (c.tile % 16) * 8 + if c.flip.x { 8 - dx } else { dx };
                        let v = (c.tile / 16) * 8
                            + if c.flip.y {
                                8 - dy as usize
                            } else {
                                dy as usize
                            };
                        (self.coord)(
                            u as f32 / 128.0,
                            v as f32 / self.atlas_height as f32,
                            c.palette as f32,
                            1.0,
                        );
                        (self.vertex)(
                            (c.x + dx as i32 + (batch.width as i32 - 320) / 2) as f32
                                / batch.width as f32
                                * 2.0
                                - 1.0,
                            1.0 - (c.y + dy) as f32 / HEIGHT as f32 * 2.0,
                        );
                    }
                }
            }
            (self.end)();
        }
        self.check()
    }

    fn check(&self) -> Result<(), String> {
        let e = unsafe { self.gl.get_error() };
        if e == glow::NO_ERROR {
            Ok(())
        } else {
            Err(format!("Tile compositor OpenGL error 0x{e:04x}"))
        }
    }
    pub fn description(&self) -> String {
        unsafe {
            format!(
                "{} / {} / {}",
                self.gl.get_parameter_string(glow::VENDOR),
                self.gl.get_parameter_string(glow::RENDERER),
                self.gl.get_parameter_string(glow::VERSION)
            )
        }
    }
}

impl Drop for GpuBatch {
    fn drop(&mut self) {
        // SAFETY: the batch is dropped before its SDL canvas/context.
        unsafe {
            self.gl.delete_texture(self.atlas);
            self.gl.delete_texture(self.palettes);
            self.gl.delete_program(self.program);
        }
    }
}

#[cfg(test)]
#[path = "tile_batch_tests.rs"]
mod tests;
