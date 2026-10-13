//! Asset previews have their own viewer and clock; they never mutate Scene.
use imgui::{Image, TextureId, Ui};
use ninja_master_native::{
    assets::{PlayerAssets, StageAssets},
    devtools::Substage,
    render::{Frame, cram_lines},
    stage::StageId,
    viewer::{View, Viewer},
};
use std::time::Instant;

pub(super) struct Gallery {
    stage: usize,
    part: i32,
    view: usize,
    animation: usize,
    animate: bool,
    flip: bool,
    cursor: i32,
    start: Instant,
    viewer: Option<Viewer>,
    player: Option<PlayerAssets>,
    loaded: Option<StageId>,
    prepared: Option<(StageId, usize, usize, bool, i32, i32)>,
    pub frame: Frame,
    pub visible: bool,
    error: String,
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            stage: 0,
            part: 0,
            view: 0,
            animation: 0,
            animate: true,
            flip: false,
            cursor: 0,
            start: Instant::now(),
            viewer: None,
            player: None,
            loaded: None,
            prepared: None,
            frame: Frame::default(),
            visible: false,
            error: String::new(),
        }
    }
}

impl Gallery {
    pub fn set_art_pack(
        &mut self,
        pack: Option<std::rc::Rc<ninja_master_native::art_pack::ArtPack>>,
    ) {
        self.frame.set_art_pack(pack);
        self.frame.set_output_scale(2);
        self.prepared = None;
    }
    pub fn prepare(&mut self, rom: &[u8], stages: &[Substage]) -> bool {
        if !self.visible || stages.is_empty() {
            return false;
        }
        let stage = stages[self.stage.min(stages.len() - 1)];
        let id = stage
            .id
            .with_part(self.part.clamp(0, i32::from(stage.parts) - 1) as u8);
        if self.loaded != Some(id) {
            self.loaded = Some(id);
            self.viewer = None;
            self.prepared = None;
            self.error.clear();
            match StageAssets::load_stage(rom, id) {
                Ok(assets) => self.viewer = Some(Viewer::new(assets, View::Scene)),
                Err(error) => self.error = error,
            }
        }
        let Some(viewer) = &mut self.viewer else {
            return false;
        };
        if self.view == 1 {
            if self.player.is_none() {
                match PlayerAssets::load(rom) {
                    Ok(player) => self.player = Some(player),
                    Err(error) => {
                        self.error = error;
                        return false;
                    }
                }
            }
            let player = self.player.as_ref().unwrap();
            let sequence = &player.animations[self.animation];
            let tick = if self.animate {
                (self.start.elapsed().as_secs_f64() * 60.0) as u64
            } else {
                0
            };
            let index = if self.animate {
                sequence.frame_at(tick)
            } else {
                sequence.frames[self.cursor.clamp(0, sequence.frames.len() as i32 - 1) as usize]
            };
            let key = (id, self.view, index, self.flip, 0, 0);
            if self.prepared == Some(key) {
                return false;
            }
            self.prepared = Some(key);
            self.frame.clear([24, 26, 31]);
            if let Some(sprite) = player.frames.get(index) {
                sprite.draw(
                    &mut self.frame,
                    &cram_lines(&viewer.assets.palettes),
                    160,
                    156,
                    self.flip,
                );
            }
        } else {
            let key = (id, self.view, 0, false, viewer.x, viewer.y);
            if self.prepared == Some(key) {
                return false;
            }
            self.prepared = Some(key);
            viewer.view = if self.view == 2 {
                View::Palettes
            } else {
                View::Scene
            };
            viewer.background_y = i64::from(viewer.y) << 16;
            viewer.draw(&mut self.frame);
        }
        true
    }

    pub fn show(&mut self, ui: &Ui, stages: &[Substage]) {
        self.visible = true;
        ui.text_wrapped(
            "Browse stage art and animations with the active art pack. This preview has its own camera and animation clock.",
        );
        if stages.is_empty() {
            return;
        }
        let names: Vec<_> = stages
            .iter()
            .map(|s| format!("{}-{}", s.id.round, s.id.substage))
            .collect();
        self.stage = self.stage.min(names.len() - 1);
        ui.combo_simple_string("Stage##gallery", &mut self.stage, &names);
        self.part = self.part.min(i32::from(stages[self.stage].parts) - 1);
        ui.slider(
            "Part##gallery",
            0,
            i32::from(stages[self.stage].parts) - 1,
            &mut self.part,
        );
        ui.combo_simple_string(
            "Preview",
            &mut self.view,
            &["Stage", "Sprite animations", "Palettes"],
        );
        if self.view == 1 {
            let Some(player) = &self.player else {
                ui.text("Loading player animations...");
                return;
            };
            let names: Vec<_> = player
                .animations
                .iter()
                .map(|s| format!("State {} ({} frames)", s.state, s.frames.len()))
                .collect();
            if ui.combo_simple_string("Animation", &mut self.animation, &names) {
                self.cursor = 0;
                self.start = Instant::now();
            }
            ui.checkbox("Animate", &mut self.animate);
            ui.same_line();
            ui.checkbox("Face left", &mut self.flip);
            if !self.animate {
                ui.slider(
                    "Frame",
                    0,
                    player.animations[self.animation].frames.len() as i32 - 1,
                    &mut self.cursor,
                );
            }
        } else if self.view == 0
            && let Some(viewer) = &mut self.viewer
        {
            let (w, h) = viewer.assets.data.pixel_size();
            ui.slider("Camera X", 0, (w - 320).max(0), &mut viewer.x);
            ui.slider("Camera Y", 0, (h - 192).max(0), &mut viewer.y);
        }
        if self.viewer.is_some() {
            let available = ui.content_region_avail();
            let width = available[0]
                .min((available[1] - 40.0).max(112.0) * 320.0 / 224.0)
                .clamp(160.0, 480.0);
            let backing_scale =
                ui.io().display_framebuffer_scale[0].max(ui.io().display_framebuffer_scale[1]);
            let scale = (width * backing_scale / 320.0).ceil().max(1.0) as usize;
            if self.frame.set_output_scale(scale) {
                self.prepared = None;
            }
            Image::new(TextureId::new(1), [width, width * 224.0 / 320.0]).build(ui);
        }
        if self.view == 2
            && let Some(viewer) = &self.viewer
        {
            for (bank, colors) in viewer.assets.palettes.iter().enumerate() {
                ui.text(format!("Palette {bank}"));
                for (index, color) in colors.iter().enumerate() {
                    if index > 0 {
                        ui.same_line();
                    }
                    ui.color_button_config(
                        format!("##color{bank}-{index}"),
                        [
                            f32::from(color[0]) / 255.0,
                            f32::from(color[1]) / 255.0,
                            f32::from(color[2]) / 255.0,
                            1.0,
                        ],
                    )
                    .size([20.0, 20.0])
                    .build();
                    if ui.is_item_hovered() {
                        ui.tooltip_text(format!(
                            "CRAM {bank}:{index} = ${:04X}",
                            viewer.assets.palette_words[bank][index]
                        ));
                    }
                }
            }
        }
        if !self.error.is_empty() {
            ui.text_wrapped(&self.error);
        }
    }
}
