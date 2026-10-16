//! CRT presentation in SDL's compatibility OpenGL context. SDL still owns
//! the window, pacing and ImGui drawing; every GL state we change is restored.

use glow::HasContext;
use sdl2::{VideoSubsystem, rect::Rect, render::WindowCanvas};
use std::num::NonZeroU32;

const VERTEX: &str = "#version 120\n\
    varying vec2 screen_uv;\n\
    void main() {\n\
        gl_Position = gl_Vertex;\n\
        screen_uv = vec2(gl_Vertex.x + 1.0, 1.0 - gl_Vertex.y) * 0.5;\n\
    }";

pub(super) struct Crt {
    gl: glow::Context,
    program: glow::NativeProgram,
    texture: glow::NativeTexture,
    size: (usize, usize),
    source_size: Option<glow::NativeUniformLocation>,
    beam_size: Option<glow::NativeUniformLocation>,
    viewport_size: Option<glow::NativeUniformLocation>,
    curvature: Option<glow::NativeUniformLocation>,
    scanlines: Option<glow::NativeUniformLocation>,
    // SDL's desktop OpenGL backend requires the compatibility API already.
    begin: unsafe extern "system" fn(u32),
    vertex: unsafe extern "system" fn(f32, f32),
    end: unsafe extern "system" fn(),
}

fn flush(canvas: &mut WindowCanvas) -> Result<(), String> {
    // SAFETY: the canvas owns a live SDL renderer on this thread.
    if unsafe { sdl2::sys::SDL_RenderFlush(canvas.raw()) } < 0 {
        return Err(sdl2::get_error());
    }
    Ok(())
}

fn gl_result(gl: &glow::Context) -> Result<(), String> {
    // SAFETY: called only while the owning canvas has its GL context current.
    let error = unsafe { gl.get_error() };
    if error == glow::NO_ERROR {
        Ok(())
    } else {
        Err(format!("OpenGL error 0x{error:04x}"))
    }
}

/// SDL may leave row strides or a pixel buffer bound after texture updates.
/// Raw slice uploads must use tightly packed client memory and restore that state.
pub(super) struct Unpack<'a> {
    gl: &'a glow::Context,
    stores: [(u32, i32); 4],
    buffer: Option<glow::NativeBuffer>,
}

impl<'a> Unpack<'a> {
    pub(super) unsafe fn new(gl: &'a glow::Context) -> Self {
        // SAFETY: caller owns the current compatibility GL context.
        unsafe {
            let stores = [
                glow::UNPACK_ALIGNMENT,
                glow::UNPACK_ROW_LENGTH,
                glow::UNPACK_SKIP_ROWS,
                glow::UNPACK_SKIP_PIXELS,
            ]
            .map(|p| (p, gl.get_parameter_i32(p)));
            let buffer = gl.get_parameter_buffer(glow::PIXEL_UNPACK_BUFFER_BINDING);
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            for (parameter, _) in stores {
                gl.pixel_store_i32(
                    parameter,
                    if parameter == glow::UNPACK_ALIGNMENT {
                        1
                    } else {
                        0
                    },
                );
            }
            Self { gl, stores, buffer }
        }
    }
}
impl Drop for Unpack<'_> {
    fn drop(&mut self) {
        // SAFETY: guard never outlives the owning SDL context.
        unsafe {
            for (p, v) in self.stores {
                self.gl.pixel_store_i32(p, v);
            }
            self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, self.buffer);
        }
    }
}

impl Crt {
    #[cfg(test)]
    pub(super) fn inject_error_for_test(&self) {
        // SAFETY: fault injection on the test's current context; an invalid enum
        // produces a recoverable GL error without deleting or corrupting resources.
        unsafe {
            self.gl.enable(u32::MAX);
        }
    }

    pub(super) fn finish(&self, canvas: &mut WindowCanvas) -> Result<(), String> {
        flush(canvas)?;
        // SAFETY: the owning SDL canvas has its context current. Benchmark only.
        unsafe {
            self.gl.finish();
        }
        gl_result(&self.gl)
    }
    pub(super) fn new(video: &VideoSubsystem, canvas: &mut WindowCanvas) -> Result<Self, String> {
        if canvas.info().name != "opengl" {
            return Err("CRT requires the desktop OpenGL renderer.".into());
        }
        flush(canvas)?;
        let begin = video.gl_get_proc_address("glBegin");
        let vertex = video.gl_get_proc_address("glVertex2f");
        let end = video.gl_get_proc_address("glEnd");
        if begin.is_null() || vertex.is_null() || end.is_null() {
            return Err("OpenGL compatibility drawing is unavailable.".into());
        }
        // SAFETY: SDL's OpenGL renderer has made its compatibility context
        // current. Entry points use the platform GL calling convention. The
        // Crt is created after the canvas, so it is dropped before the canvas.
        unsafe {
            let gl =
                glow::Context::from_loader_function(|name| video.gl_get_proc_address(name).cast());
            let program = compile_program(&gl)?;
            let texture = match gl.create_texture() {
                Ok(texture) => texture,
                Err(error) => {
                    gl.delete_program(program);
                    return Err(error);
                }
            };
            Ok(Self {
                source_size: gl.get_uniform_location(program, "source_size"),
                beam_size: gl.get_uniform_location(program, "beam_size"),
                viewport_size: gl.get_uniform_location(program, "viewport_size"),
                curvature: gl.get_uniform_location(program, "curvature"),
                scanlines: gl.get_uniform_location(program, "scanlines"),
                gl,
                program,
                texture,
                size: (0, 0),
                begin: std::mem::transmute::<*const (), unsafe extern "system" fn(u32)>(begin),
                vertex: std::mem::transmute::<*const (), unsafe extern "system" fn(f32, f32)>(
                    vertex,
                ),
                end: std::mem::transmute::<*const (), unsafe extern "system" fn()>(end),
            })
        }
    }

    pub(super) fn upload(
        &mut self,
        canvas: &mut WindowCanvas,
        pixels: &[u8],
        width: usize,
        height: usize,
    ) -> Result<(), String> {
        flush(canvas)?;
        if width == 0
            || height == 0
            || width.checked_mul(height).and_then(|n| n.checked_mul(3)) != Some(pixels.len())
        {
            return Err("Invalid CRT RGB frame dimensions".into());
        }
        // SAFETY: this context belongs to the live canvas. RGB24 input is
        // tightly packed; unpack alignment and texture binding are restored.
        unsafe {
            let gl = &self.gl;
            let previous = gl.get_parameter_texture(glow::TEXTURE_BINDING_2D);
            let _unpack = Unpack::new(gl);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            if self.size != (width, height) {
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGB as i32,
                    width as i32,
                    height as i32,
                    0,
                    glow::RGB,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(pixels)),
                );
                for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::LINEAR as i32);
                }
                for name in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, name, glow::CLAMP_TO_EDGE as i32);
                }
                self.size = (width, height);
            } else {
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    glow::RGB,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(pixels)),
                );
            }
            gl.bind_texture(glow::TEXTURE_2D, previous);
        }
        gl_result(&self.gl)
    }

    pub(super) fn draw(
        &self,
        canvas: &mut WindowCanvas,
        rect: Rect,
        native_width: usize,
        curvature: u8,
        scanlines: u8,
    ) -> Result<(), String> {
        let (_, output_height) = canvas.output_size()?;
        flush(canvas)?;
        // SAFETY: SDL's queued rendering is complete. We restore the program,
        // viewport, texture unit/binding and capability flags before ImGui or
        // any other SDL drawing can run. Immediate vertices leave SDL's
        // vertex arrays, buffers and fixed-function matrices untouched.
        unsafe {
            let gl = &self.gl;
            let program = NonZeroU32::new(gl.get_parameter_i32(glow::CURRENT_PROGRAM) as u32)
                .map(glow::NativeProgram);
            let mut viewport = [0; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);
            let active = gl.get_parameter_i32(glow::ACTIVE_TEXTURE) as u32;
            gl.active_texture(glow::TEXTURE0);
            let texture = gl.get_parameter_texture(glow::TEXTURE_BINDING_2D);
            let flags = [
                glow::BLEND,
                glow::SCISSOR_TEST,
                glow::DEPTH_TEST,
                glow::CULL_FACE,
            ]
            .map(|flag| (flag, gl.is_enabled(flag)));
            for (flag, _) in flags {
                gl.disable(flag);
            }
            gl.viewport(
                rect.x(),
                output_height as i32 - rect.y() - rect.height() as i32,
                rect.width() as i32,
                rect.height() as i32,
            );
            gl.use_program(Some(self.program));
            gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            gl.uniform_2_f32(
                self.source_size.as_ref(),
                self.size.0 as f32,
                self.size.1 as f32,
            );
            gl.uniform_2_f32(
                self.beam_size.as_ref(),
                native_width as f32,
                ninja_master_native::HEIGHT as f32,
            );
            gl.uniform_2_f32(
                self.viewport_size.as_ref(),
                rect.width() as f32,
                rect.height() as f32,
            );
            gl.uniform_1_f32(
                self.curvature.as_ref(),
                f32::from(curvature.min(100)) * 0.0018,
            );
            gl.uniform_1_f32(
                self.scanlines.as_ref(),
                f32::from(scanlines.min(100)) / 100.0,
            );
            // Samplers default to texture unit zero after program linking.
            (self.begin)(glow::TRIANGLE_STRIP);
            for (x, y) in [(-1.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (1.0, -1.0)] {
                (self.vertex)(x, y);
            }
            (self.end)();
            gl.bind_texture(glow::TEXTURE_2D, texture);
            gl.active_texture(active);
            gl.use_program(program);
            gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            for (flag, enabled) in flags {
                if enabled {
                    gl.enable(flag);
                }
            }
        }
        gl_result(&self.gl)
    }
}

impl Drop for Crt {
    fn drop(&mut self) {
        // SAFETY: Crt is dropped while its SDL canvas/context is still alive.
        unsafe {
            self.gl.delete_texture(self.texture);
            self.gl.delete_program(self.program);
        }
    }
}

unsafe fn compile_program(gl: &glow::Context) -> Result<glow::NativeProgram, String> {
    // SAFETY: caller owns the current GL context.
    unsafe { compile_sources(gl, VERTEX, include_str!("crt.frag")) }
}

pub(super) unsafe fn compile_sources(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
) -> Result<glow::NativeProgram, String> {
    // SAFETY: called only with the canvas's current OpenGL context.
    unsafe {
        let program = gl.create_program()?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = match gl.create_shader(kind) {
                Ok(shader) => shader,
                Err(error) => {
                    gl.delete_program(program);
                    return Err(error);
                }
            };
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let error = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(format!("CRT shader: {error}"));
            }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            let error = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(format!("CRT shader link: {error}"));
        }
        Ok(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl2::pixels::{Color, PixelFormatEnum};

    #[test]
    #[ignore = "requires a desktop OpenGL display"]
    fn rendering_upload_restores_stride_offsets_and_pixel_buffer_and_handles_bad_shaders() {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let driver = sdl2::render::drivers()
            .position(|d| d.name == "opengl")
            .unwrap() as u32;
        let window = video
            .window("GL upload state regression", 400, 300)
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().index(driver).build().unwrap();
        let mut crt = Crt::new(&video, &mut canvas).unwrap();
        // SAFETY: test owns this canvas's current GL context.
        let buffer = unsafe {
            let gl = &crt.gl;
            assert!(compile_sources(gl, VERTEX, "#version 120\n invalid shader").is_err());
            let buffer = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(buffer));
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 509);
            gl.pixel_store_i32(glow::UNPACK_SKIP_PIXELS, 7);
            gl.pixel_store_i32(glow::UNPACK_SKIP_ROWS, 3);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 8);
            buffer
        };
        crt.upload(&mut canvas, &[255, 0, 0].repeat(398 * 224), 398, 224)
            .unwrap();
        // SAFETY: same current context; verify restoration and reset for SDL.
        unsafe {
            assert_eq!(crt.gl.get_parameter_i32(glow::UNPACK_ROW_LENGTH), 509);
            assert_eq!(crt.gl.get_parameter_i32(glow::UNPACK_SKIP_PIXELS), 7);
            assert_eq!(crt.gl.get_parameter_i32(glow::UNPACK_SKIP_ROWS), 3);
            assert_eq!(crt.gl.get_parameter_i32(glow::UNPACK_ALIGNMENT), 8);
            assert_eq!(
                crt.gl
                    .get_parameter_buffer(glow::PIXEL_UNPACK_BUFFER_BINDING),
                Some(buffer)
            );
            crt.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            crt.gl.delete_buffer(buffer);
            for parameter in [
                glow::UNPACK_ROW_LENGTH,
                glow::UNPACK_SKIP_PIXELS,
                glow::UNPACK_SKIP_ROWS,
            ] {
                crt.gl.pixel_store_i32(parameter, 0);
            }
            crt.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        }
        assert!(crt.upload(&mut canvas, &[0; 3], 398, 224).is_err());
        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        crt.draw(&mut canvas, Rect::new(0, 0, 398, 224), 398, 0, 0)
            .unwrap();
        let pixel = canvas
            .read_pixels(Rect::new(100, 100, 1, 1), PixelFormatEnum::RGB24)
            .unwrap();
        assert!(pixel[0] > 100);
        assert_eq!(&pixel[1..], &[0, 0]);
    }

    #[test]
    #[ignore = "requires a desktop OpenGL display"]
    fn crt_shader_preserves_orientation_letterboxing_and_sdl_overlays() {
        let sdl = sdl2::init().unwrap();
        let video = sdl.video().unwrap();
        let driver = sdl2::render::drivers()
            .position(|d| d.name == "opengl")
            .unwrap() as u32;
        let window = video
            .window("CRT render test", 400, 300)
            .hidden()
            .build()
            .unwrap();
        let mut canvas = window.into_canvas().index(driver).build().unwrap();
        let mut crt = Crt::new(&video, &mut canvas).unwrap();
        let width = 320;
        let height = 224;
        let mut pixels = vec![0; width * height * 3];
        for y in 0..height {
            for x in 0..width {
                let at = (y * width + x) * 3;
                pixels[at..at + 3].copy_from_slice(match (x < width / 2, y < height / 2) {
                    (true, true) => &[255, 0, 0],
                    (false, true) => &[0, 255, 0],
                    (true, false) => &[0, 0, 255],
                    (false, false) => &[255, 255, 255],
                });
            }
        }
        crt.upload(&mut canvas, &pixels, width, height).unwrap();
        // Repeat uploads exercise texture reuse and changed source resolution.
        crt.upload(&mut canvas, &vec![0; 24 * 24 * 3], 24, 24)
            .unwrap();
        crt.upload(&mut canvas, &pixels, width, height).unwrap();
        let rect = Rect::new(40, 38, 320, 224);
        for curvature in [0, 25, 100] {
            canvas.set_draw_color(Color::BLACK);
            canvas.clear();
            crt.draw(&mut canvas, rect, width, curvature, 35).unwrap();
            let image = canvas.read_pixels(None, PixelFormatEnum::RGB24).unwrap();
            let pixel = |x: usize, y: usize| &image[(y * 400 + x) * 3..(y * 400 + x) * 3 + 3];
            assert_eq!(pixel(10, 10), &[0, 0, 0]);
            assert_eq!(pixel(40, 38), &[0, 0, 0]);
            for (x, y, channel) in [(120, 95, 0), (280, 95, 1), (120, 205, 2)] {
                let color = pixel(x, y);
                assert!(color[channel] > 150, "curvature {curvature}: {color:?}");
                assert_eq!(color[(channel + 1) % 3], 0);
                assert_eq!(color[(channel + 2) % 3], 0);
            }
            // Exercise SDL's textured geometry path, as used by ImGui, after
            // the shader. This catches a leaked program, viewport or binding.
            let creator = canvas.texture_creator();
            let mut texture = creator
                .create_texture_streaming(PixelFormatEnum::RGB24, 1, 1)
                .unwrap();
            texture.update(None, &[255, 0, 255], 3).unwrap();
            use sdl2::{rect::FPoint, render::Vertex};
            let vertices = [(180.0, 140.0), (220.0, 140.0), (200.0, 180.0)].map(|(x, y)| Vertex {
                position: FPoint::new(x, y),
                color: Color::WHITE,
                tex_coord: FPoint::new(0.5, 0.5),
            });
            canvas
                .render_geometry(&vertices, Some(&texture), &[0, 1, 2][..])
                .unwrap();
            let overlay = canvas
                .read_pixels(Some(Rect::new(200, 150, 1, 1)), PixelFormatEnum::RGB24)
                .unwrap();
            assert_eq!(overlay, [255, 0, 255]);
            // A flat SDL copy must also work after switching the effect off.
            canvas
                .copy(&texture, None, Rect::new(0, 0, 10, 10))
                .unwrap();
            let flat = canvas
                .read_pixels(Some(Rect::new(0, 0, 1, 1)), PixelFormatEnum::RGB24)
                .unwrap();
            assert_eq!(flat, [255, 0, 255]);
            // SAFETY: the test canvas still owns the current GL context.
            assert_eq!(unsafe { crt.gl.get_error() }, glow::NO_ERROR);
        }
    }
}
