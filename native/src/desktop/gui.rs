//! Dear ImGui on the SDL canvas: event translation and a renderer that
//! draws ImGui's triangle lists with `SDL_RenderGeometry`. It exists for
//! the F1 developer panel (`super::panel`), not for the game itself.

use imgui::{Context, DrawCmd, Key, MouseButton as GuiButton, StyleColor, TextureId, Ui};
use sdl2::{
    event::Event,
    keyboard::{Mod, Scancode},
    mouse::MouseButton,
    pixels::{Color, PixelFormatEnum},
    rect::{FPoint, Rect},
    render::{BlendMode, Texture, TextureCreator, Vertex, WindowCanvas},
    video::{Window, WindowContext},
};
use std::{path::PathBuf, time::Instant};

pub(super) struct Gui<'t> {
    context: Context,
    font: Texture<'t>,
    images: Vec<Option<Texture<'t>>>,
    last_frame: Instant,
    vertices: Vec<Vertex>,
    indices: Vec<i32>,
}

impl<'t> Gui<'t> {
    pub(super) fn new(
        creator: &'t TextureCreator<WindowContext>,
        ini: Option<PathBuf>,
    ) -> Result<Self, String> {
        let mut context = Context::create();
        context.set_ini_filename(ini);
        context.io_mut().config_flags |= imgui::ConfigFlags::NAV_ENABLE_KEYBOARD;
        let style = context.style_mut();
        style.window_padding = [18.0, 16.0];
        style.frame_padding = [9.0, 6.0];
        style.item_spacing = [10.0, 9.0];
        style.window_rounding = 8.0;
        style.frame_rounding = 4.0;
        style.tab_rounding = 4.0;
        for (color, value) in [
            (StyleColor::WindowBg, [0.075, 0.080, 0.095, 0.98]),
            (StyleColor::TitleBg, [0.11, 0.12, 0.14, 1.0]),
            (StyleColor::TitleBgActive, [0.19, 0.10, 0.12, 1.0]),
            (StyleColor::FrameBg, [0.15, 0.16, 0.19, 1.0]),
            (StyleColor::Button, [0.28, 0.13, 0.16, 1.0]),
            (StyleColor::ButtonHovered, [0.46, 0.19, 0.23, 1.0]),
            (StyleColor::ButtonActive, [0.58, 0.22, 0.27, 1.0]),
            (StyleColor::Header, [0.30, 0.14, 0.18, 1.0]),
            (StyleColor::HeaderHovered, [0.45, 0.18, 0.23, 1.0]),
            (StyleColor::Tab, [0.16, 0.17, 0.20, 1.0]),
            (StyleColor::TabActive, [0.40, 0.16, 0.21, 1.0]),
            (StyleColor::TabHovered, [0.50, 0.19, 0.25, 1.0]),
            (StyleColor::CheckMark, [0.91, 0.37, 0.42, 1.0]),
            (StyleColor::SliderGrab, [0.75, 0.29, 0.35, 1.0]),
        ] {
            style.colors[color as usize] = value;
        }
        let fonts = context.fonts();
        fonts.add_font(&[imgui::FontSource::DefaultFontData {
            config: Some(imgui::FontConfig {
                size_pixels: 17.0,
                ..Default::default()
            }),
        }]);
        let atlas = fonts.build_rgba32_texture();
        let mut font = creator
            .create_texture_static(PixelFormatEnum::RGBA32, atlas.width, atlas.height)
            .map_err(|error| error.to_string())?;
        font.update(None, atlas.data, atlas.width as usize * 4)
            .map_err(|error| error.to_string())?;
        font.set_blend_mode(BlendMode::Blend);
        fonts.tex_id = TextureId::new(0);
        Ok(Self {
            context,
            font,
            images: Vec::new(),
            last_frame: Instant::now(),
            vertices: Vec::new(),
            indices: Vec::new(),
        })
    }

    /// Texture zero is the font; positive IDs are RGB previews owned here.
    pub(super) fn reset_textures(
        &mut self,
        creator: &'t TextureCreator<WindowContext>,
    ) -> Result<(), String> {
        let fonts = self.context.fonts();
        let atlas = fonts.build_rgba32_texture();
        let mut font = creator
            .create_texture_static(PixelFormatEnum::RGBA32, atlas.width, atlas.height)
            .map_err(|e| e.to_string())?;
        font.update(None, atlas.data, atlas.width as usize * 4)
            .map_err(|e| e.to_string())?;
        font.set_blend_mode(BlendMode::Blend);
        self.font = font;
        self.images.clear();
        Ok(())
    }

    /// Texture zero is the font; positive IDs are RGB previews owned here.
    pub(super) fn set_image(
        &mut self,
        creator: &'t TextureCreator<WindowContext>,
        id: usize,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        if self.images.len() < id {
            self.images.resize_with(id, || None);
        }
        let image = &mut self.images[id - 1];
        if image.as_ref().is_none_or(|t| {
            let q = t.query();
            q.width != width || q.height != height
        }) {
            *image = Some(
                creator
                    .create_texture_streaming(PixelFormatEnum::RGB24, width, height)
                    .map_err(|e| e.to_string())?,
            );
        }
        image
            .as_mut()
            .unwrap()
            .update(None, pixels, width as usize * 3)
            .map_err(|e| e.to_string())
    }

    /// ImGui has a text field or other keyboard widget active: game keys
    /// should not see the press.
    pub(super) fn wants_keyboard(&self) -> bool {
        self.context.io().want_capture_keyboard
    }
    pub(super) fn wants_text(&self) -> bool {
        self.context.io().want_text_input
    }

    pub(super) fn handle_event(&mut self, event: &Event) {
        feed(self.context.io_mut(), event);
    }

    /// Build one ImGui frame with `build` and draw it over the canvas.
    pub(super) fn draw(
        &mut self,
        canvas: &mut WindowCanvas,
        build: impl FnOnce(&Ui),
    ) -> Result<(), String> {
        prepare(self.context.io_mut(), canvas.window(), &mut self.last_frame);
        build(self.context.new_frame());
        let data = self.context.render();
        // New auto-sized windows can produce an empty first frame. imgui
        // 0.12's iterator constructs a slice from a null pointer in that
        // case, which current Rust debug builds correctly reject.
        if data.draw_lists_count() == 0 {
            canvas.set_clip_rect(None);
            return Ok(());
        }
        let [scale_x, scale_y] = data.framebuffer_scale;
        let [origin_x, origin_y] = data.display_pos;
        for list in data.draw_lists() {
            self.vertices.clear();
            self.vertices
                .extend(list.vtx_buffer().iter().map(|vertex| Vertex {
                    position: FPoint::new(
                        (vertex.pos[0] - origin_x) * scale_x,
                        (vertex.pos[1] - origin_y) * scale_y,
                    ),
                    color: Color::RGBA(vertex.col[0], vertex.col[1], vertex.col[2], vertex.col[3]),
                    tex_coord: FPoint::new(vertex.uv[0], vertex.uv[1]),
                }));
            let indices = list.idx_buffer();
            for command in list.commands() {
                let DrawCmd::Elements { count, cmd_params } = command else {
                    continue;
                };
                let [x1, y1, x2, y2] = cmd_params.clip_rect;
                let (left, top) = ((x1 - origin_x) * scale_x, (y1 - origin_y) * scale_y);
                let (width, height) = ((x2 - x1) * scale_x, (y2 - y1) * scale_y);
                if width < 1.0 || height < 1.0 {
                    continue;
                }
                canvas.set_clip_rect(Rect::new(
                    left as i32,
                    top as i32,
                    width as u32,
                    height as u32,
                ));
                let first = cmd_params.idx_offset;
                let base = cmd_params.vtx_offset as i32;
                self.indices.clear();
                self.indices.extend(
                    indices[first..first + count]
                        .iter()
                        .map(|&index| i32::from(index) + base),
                );
                canvas.render_geometry(
                    &self.vertices,
                    if cmd_params.texture_id.id() == 0 {
                        Some(&self.font)
                    } else {
                        self.images
                            .get(cmd_params.texture_id.id() - 1)
                            .and_then(Option::as_ref)
                    },
                    self.indices.as_slice(),
                )?;
            }
        }
        canvas.set_clip_rect(None);
        Ok(())
    }
}

/// Pass an SDL event to ImGui's input queue.
fn feed(io: &mut imgui::Io, event: &Event) {
    match *event {
        Event::MouseMotion { x, y, .. } => io.add_mouse_pos_event([x as f32, y as f32]),
        Event::MouseButtonDown { mouse_btn, .. } | Event::MouseButtonUp { mouse_btn, .. } => {
            if let Some(button) = mouse_button(mouse_btn) {
                let down = matches!(event, Event::MouseButtonDown { .. });
                io.add_mouse_button_event(button, down);
            }
        }
        Event::MouseWheel { x, y, .. } => io.add_mouse_wheel_event([x as f32, y as f32]),
        Event::TextInput { ref text, .. } => text.chars().for_each(|c| io.add_input_character(c)),
        Event::KeyDown {
            scancode: Some(key),
            keymod,
            ..
        }
        | Event::KeyUp {
            scancode: Some(key),
            keymod,
            ..
        } => {
            let down = matches!(event, Event::KeyDown { .. });
            for (modifier, mask) in [
                (Key::ModCtrl, Mod::LCTRLMOD | Mod::RCTRLMOD),
                (Key::ModShift, Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                (Key::ModAlt, Mod::LALTMOD | Mod::RALTMOD),
                (Key::ModSuper, Mod::LGUIMOD | Mod::RGUIMOD),
            ] {
                io.add_key_event(modifier, keymod.intersects(mask));
            }
            if let Some(key) = gui_key(key) {
                io.add_key_event(key, down);
            }
        }
        _ => {}
    }
}

/// Frame timing and the window's point and pixel sizes.
fn prepare(io: &mut imgui::Io, window: &Window, last_frame: &mut Instant) {
    let now = Instant::now();
    io.update_delta_time(now - *last_frame);
    *last_frame = now;
    let (width, height) = window.size();
    let (pixels_x, pixels_y) = window.drawable_size();
    io.display_size = [width as f32, height as f32];
    if width > 0 && height > 0 {
        io.display_framebuffer_scale = [
            pixels_x as f32 / width as f32,
            pixels_y as f32 / height as f32,
        ];
    }
}

fn mouse_button(button: MouseButton) -> Option<GuiButton> {
    Some(match button {
        MouseButton::Left => GuiButton::Left,
        MouseButton::Right => GuiButton::Right,
        MouseButton::Middle => GuiButton::Middle,
        MouseButton::X1 => GuiButton::Extra1,
        MouseButton::X2 => GuiButton::Extra2,
        MouseButton::Unknown => return None,
    })
}

/// The keys ImGui's text fields and shortcuts use.
fn gui_key(key: Scancode) -> Option<Key> {
    const LETTERS: [Key; 26] = [
        Key::A,
        Key::B,
        Key::C,
        Key::D,
        Key::E,
        Key::F,
        Key::G,
        Key::H,
        Key::I,
        Key::J,
        Key::K,
        Key::L,
        Key::M,
        Key::N,
        Key::O,
        Key::P,
        Key::Q,
        Key::R,
        Key::S,
        Key::T,
        Key::U,
        Key::V,
        Key::W,
        Key::X,
        Key::Y,
        Key::Z,
    ];
    const DIGITS: [Key; 10] = [
        Key::Alpha1,
        Key::Alpha2,
        Key::Alpha3,
        Key::Alpha4,
        Key::Alpha5,
        Key::Alpha6,
        Key::Alpha7,
        Key::Alpha8,
        Key::Alpha9,
        Key::Alpha0,
    ];
    let code = key as i32;
    if (Scancode::A as i32..=Scancode::Z as i32).contains(&code) {
        return Some(LETTERS[(code - Scancode::A as i32) as usize]);
    }
    if (Scancode::Num1 as i32..=Scancode::Num0 as i32).contains(&code) {
        return Some(DIGITS[(code - Scancode::Num1 as i32) as usize]);
    }
    Some(match key {
        Scancode::Tab => Key::Tab,
        Scancode::Left => Key::LeftArrow,
        Scancode::Right => Key::RightArrow,
        Scancode::Up => Key::UpArrow,
        Scancode::Down => Key::DownArrow,
        Scancode::PageUp => Key::PageUp,
        Scancode::PageDown => Key::PageDown,
        Scancode::Home => Key::Home,
        Scancode::End => Key::End,
        Scancode::Insert => Key::Insert,
        Scancode::Delete => Key::Delete,
        Scancode::Backspace => Key::Backspace,
        Scancode::Space => Key::Space,
        Scancode::Return => Key::Enter,
        Scancode::KpEnter => Key::KeypadEnter,
        Scancode::Escape => Key::Escape,
        Scancode::Minus | Scancode::KpMinus => Key::Minus,
        Scancode::Period | Scancode::KpPeriod => Key::Period,
        Scancode::LCtrl => Key::LeftCtrl,
        Scancode::RCtrl => Key::RightCtrl,
        Scancode::LShift => Key::LeftShift,
        Scancode::RShift => Key::RightShift,
        Scancode::LAlt => Key::LeftAlt,
        Scancode::RAlt => Key::RightAlt,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(context: &mut Context, clicked: &mut bool) {
        context
            .io_mut()
            .update_delta_time(std::time::Duration::from_millis(16));
        let ui = context.new_frame();
        ui.window("test")
            .position([0.0, 0.0], imgui::Condition::Always)
            .build(|| *clicked |= ui.button_with_size("button", [100.0, 40.0]));
        context.render();
    }

    #[test]
    fn sdl_mouse_events_click_imgui_buttons() {
        let mut context = Context::create();
        context.set_ini_filename(None);
        context.io_mut().display_size = [640.0, 480.0];
        context.fonts().build_rgba32_texture();
        let mut clicked = false;
        frame(&mut context, &mut clicked);
        let motion = Event::MouseMotion {
            timestamp: 0,
            window_id: 0,
            which: 0,
            mousestate: sdl2::mouse::MouseState::from_sdl_state(0),
            x: 40,
            y: 40,
            xrel: 0,
            yrel: 0,
        };
        let button = |down| {
            let (timestamp, window_id, which, mouse_btn, clicks, x, y) =
                (0, 0, 0, MouseButton::Left, 1, 40, 40);
            if down {
                Event::MouseButtonDown {
                    timestamp,
                    window_id,
                    which,
                    mouse_btn,
                    clicks,
                    x,
                    y,
                }
            } else {
                Event::MouseButtonUp {
                    timestamp,
                    window_id,
                    which,
                    mouse_btn,
                    clicks,
                    x,
                    y,
                }
            }
        };
        for event in [motion, button(true), button(false)] {
            feed(context.io_mut(), &event);
            frame(&mut context, &mut clicked);
        }
        assert!(clicked);
    }

    #[test]
    fn text_keys_map_to_imgui() {
        assert_eq!(gui_key(Scancode::A), Some(Key::A));
        assert_eq!(gui_key(Scancode::Z), Some(Key::Z));
        assert_eq!(gui_key(Scancode::Num1), Some(Key::Alpha1));
        assert_eq!(gui_key(Scancode::Num0), Some(Key::Alpha0));
        assert_eq!(gui_key(Scancode::Backspace), Some(Key::Backspace));
        assert_eq!(gui_key(Scancode::F3), None);
    }
}
