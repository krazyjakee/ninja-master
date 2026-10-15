//! The F1 companion panel: display, controls, audio, saves, gallery, stages and tools.

use super::gallery::Gallery;
use crate::Scene;
use imgui::{Condition, Image, TextureId, TreeNodeFlags, Ui};
use ninja_master_native::{
    devtools::{DevTools, SLOTS, SPEEDS},
    input::Button,
    modes::Modes,
    settings::{self, Bindings, Device, Settings},
};

/// Panel selections that are not game state.
#[derive(Default)]
pub(super) struct Panel {
    art: super::art::ArtPanel,
    /// Index into `DevTools::stages`.
    stage: usize,
    part: u8,
    pub gallery: Gallery,
    save_name: String,
    rename_slot: Option<usize>,
    periods: Vec<f32>,
    period_cursor: usize,
}

impl Panel {
    pub(super) fn set_art_selection(&mut self, selection: &str) {
        self.art.set_selection(selection);
    }
    pub(super) fn refresh_art_packs(&mut self) {
        self.art.rescan();
    }
    pub(super) fn show(
        &mut self,
        ui: &Ui,
        scene: &mut Scene,
        tools: Option<&mut DevTools>,
        settings: &mut Settings,
        info: &PanelInfo<'_>,
    ) -> Actions {
        let mut actions = Actions::default();
        self.gallery.visible = false;
        ui.window("Ninja Master | F1")
            .position([24.0, 24.0], Condition::FirstUseEver)
            .size([720.0, 620.0], Condition::FirstUseEver)
            .size_constraints([480.0, 320.0], [1600.0, 1200.0])
            .build(|| {
                ui.text_colored([0.92, 0.43, 0.47, 1.0], "NINJA MASTER");
                ui.same_line();
                ui.text_disabled("Companion panel");
                ui.text_disabled("F1 hides this panel. Opening it leaves the game running.");
                ui.separator();
                let Some(_tabs) = ui.tab_bar("Companion tabs") else {
                    return;
                };
                if let Some(_tab) = ui.tab_item("Display") {
                    self.display(ui, settings, info.crt_error);
                }
                if let Some(_tab) = ui.tab_item("Controls") {
                    self.controls(ui, settings, info, &mut actions);
                }
                if let Some(_tab) = ui.tab_item("Audio") {
                    let mut volume = i32::from(settings.volume);
                    if ui.slider("Master volume", 0, 100, &mut volume) {
                        settings.volume = volume as u8;
                    }
                    ui.checkbox("Mute", &mut settings.muted);
                    help(
                        ui,
                        "Mute preserves your chosen volume and keeps sound synthesis running.",
                    );
                    ui.separator();
                    ui.text(format!("Current track: {}", info.track));
                    ui.text_disabled(if info.audio_available {
                        "Output: stereo PCM"
                    } else {
                        "Audio output unavailable or disabled at launch"
                    });
                    ui.text(format!(
                        "Audio buffer: {} frames | underruns: {} frames",
                        info.queued_audio, info.underruns
                    ));
                }
                if let Some(_tab) = ui.tab_item("Art packs") {
                    actions.art = self.art.show(
                        ui,
                        tools.as_deref().map_or(&[], |t| t.stages.as_slice()),
                        info.art_name,
                        info.art_status,
                    );
                }
                match (scene, tools) {
                    (Scene::Play(modes), Some(tools)) => {
                        if let Some(_tab) = ui.tab_item("Saves") {
                            actions.scene_changed |= self.saves(ui, modes, tools, info.ticks);
                        }
                        if let Some(_tab) = ui.tab_item("Gallery") {
                            self.gallery.show(ui, &tools.stages);
                        }
                        if let Some(_tab) = ui.tab_item("Stages") {
                            actions.scene_changed |= self.stages(ui, modes, tools);
                        }
                        if let Some(_tab) = ui.tab_item("Tools") {
                            actions.scene_changed |= self.play(ui, modes, tools, info.ticks);
                        }
                        if !tools.status.is_empty() {
                            ui.separator();
                            ui.text_wrapped(&tools.status);
                        }
                    }
                    _ => {
                        if let Some(_tab) = ui.tab_item("Gallery") {
                            ui.text_wrapped(
                                "Use --play or --boot to browse all stage assets here.",
                            );
                        }
                    }
                }
            });
        actions
    }

    fn display(&self, ui: &Ui, settings: &mut Settings, crt_error: Option<&str>) {
        ui.checkbox("True widescreen", &mut settings.widescreen);
        help(
            ui,
            "Shows more of the stage to fit your window or monitor, including ultrawide displays, with unchanged sprite proportions. Turn off for the original view.",
        );
        ui.checkbox("Fullscreen (F11)", &mut settings.fullscreen);
        let mut scale = settings.scale as i32;
        if ui.slider(
            "Window scale",
            settings::MIN_SCALE as i32,
            settings::MAX_SCALE as i32,
            &mut scale,
        ) {
            settings.scale = scale as u32;
        }
        help(
            ui,
            "Sets the window size. Fullscreen uses the desktop resolution.",
        );
        if settings.widescreen {
            ui.text("Aspect ratio: Matches window / monitor");
        } else {
            ui.combo_simple_string(
                "Aspect ratio",
                &mut settings.aspect,
                &[
                    "Native pixels (10:7)",
                    "Mega Drive display (4:3)",
                    "Fill window",
                ],
            );
        }
        ui.checkbox("Integer scaling", &mut settings.integer_scale);
        help(
            ui,
            "Uses whole pixel multiples when they fit. This can leave borders; turn off to fill more of the display.",
        );
        ui.checkbox("Smooth wave effects", &mut settings.smooth_wave_effects);
        help(
            ui,
            "Smooths scenery distortion. Turn off for the original stepped waves.",
        );
        ui.separator();
        {
            let _disabled = ui.begin_disabled(crt_error.is_some());
            ui.checkbox("CRT shader", &mut settings.crt);
        }
        help(
            ui,
            "Simulates phosphor beams, an RGB grille, soft glow and curved CRT glass.",
        );
        if let Some(error) = crt_error {
            ui.text_wrapped(error);
        }
        if settings.crt && crt_error.is_none() {
            let mut curvature = i32::from(settings.crt_curvature);
            if ui.slider("CRT fisheye", 0, 100, &mut curvature) {
                settings.crt_curvature = curvature as u8;
            }
            help(ui, "25 gives a slight curve. Set to 0 for flat glass.");
        }
        let mut strength = i32::from(settings.scanlines);
        if ui.slider("Scanline strength", 0, 100, &mut strength) {
            settings.scanlines = strength as u8;
        }
        help(
            ui,
            "Adds darker scanlines. CRT mode uses soft phosphor beams; flat mode darkens alternating rows.",
        );
        if ui.button("Restore display defaults") {
            let defaults = Settings::default();
            settings.fullscreen = defaults.fullscreen;
            settings.widescreen = defaults.widescreen;
            settings.scale = defaults.scale;
            settings.aspect = defaults.aspect;
            settings.integer_scale = defaults.integer_scale;
            settings.smooth_wave_effects = defaults.smooth_wave_effects;
            settings.scanlines = defaults.scanlines;
            settings.crt = defaults.crt;
            settings.crt_curvature = defaults.crt_curvature;
        }
    }

    fn controls(
        &self,
        ui: &Ui,
        settings: &mut Settings,
        info: &PanelInfo<'_>,
        actions: &mut Actions,
    ) {
        let mut names = vec!["All connected controllers".to_string()];
        names.extend(info.controllers.iter().cloned());
        if !settings.controller.is_empty() && !info.controllers.contains(&settings.controller) {
            names.push(settings.controller.clone());
        }
        let mut selected = if settings.controller.is_empty() {
            0
        } else {
            names
                .iter()
                .position(|s| s == &settings.controller)
                .unwrap_or(0)
        };
        if ui.combo_simple_string("Controller", &mut selected, &names) {
            settings.controller = if selected == 0 {
                String::new()
            } else {
                names[selected].clone()
            };
        }
        ui.text_disabled("Left stick mirrors the D-pad. Click a binding to replace it.");
        if let Some((_, _, label)) = info
            .capture
            .and_then(|(button, _)| settings::PAD_CONTROLS.iter().find(|(b, _, _)| *b == button))
        {
            ui.text_colored(
                [0.92, 0.43, 0.47, 1.0],
                format!("Bind {label}: press an input. Esc cancels; Backspace unbinds."),
            );
        }
        ui.columns(3, "Bindings", true);
        ui.text("Action");
        ui.next_column();
        ui.text("Keyboard");
        ui.next_column();
        ui.text("Controller");
        ui.next_column();
        for (button, _, label) in settings::PAD_CONTROLS {
            ui.text(label);
            ui.next_column();
            for (device, bindings) in [
                (Device::Keyboard, &settings.keyboard),
                (Device::Gamepad, &settings.gamepad),
            ] {
                let names = bindings.get(button);
                let label = if names.is_empty() {
                    "Unbound".to_string()
                } else {
                    names.join(" / ")
                };
                if ui.button(format!("{label}##{device:?}-{button:?}")) {
                    actions.capture = Some((button, device));
                }
                ui.next_column();
            }
        }
        ui.columns(1, "", false);
        if ui.button("Restore control defaults") {
            settings.keyboard = Bindings::default_keyboard();
            settings.gamepad = Bindings::default_gamepad();
        }
        ui.separator();
        ui.text("Input tester");
        ui.columns(4, "Live inputs", false);
        for (button, _, label) in settings::PAD_CONTROLS {
            let down = info.held & button.mask() != 0;
            ui.text_colored(
                if down {
                    [0.95, 0.45, 0.48, 1.0]
                } else {
                    [0.45, 0.47, 0.52, 1.0]
                },
                format!("{} {label}", if down { "[ON]" } else { "[--]" }),
            );
            ui.next_column();
        }
        ui.columns(1, "", false);
        ui.text(format!(
            "Left stick: X {} / Y {}",
            info.axes[0], info.axes[1]
        ));
    }

    fn saves(&mut self, ui: &Ui, modes: &mut Modes, tools: &mut DevTools, ticks: u64) -> bool {
        let mut changed = false;
        ui.text_wrapped(if tools.disk_saves_enabled() {
            "Eight slots, saved beside settings. F5 saves; F6 loads the selected slot."
        } else {
            "Eight memory slots. Set NINJA_MASTER_CONFIG to enable disk saves."
        });
        ui.columns(2, "Save browser", false);
        ui.set_column_width(0, 280.0);
        for slot in 0..SLOTS {
            let label = tools.slots[slot].as_ref().map_or_else(
                || format!("{}: Empty", slot + 1),
                |s| format!("{}: {} | {}", slot + 1, s.name, s.stage()),
            );
            if ui
                .selectable_config(format!("{label}##slot{slot}"))
                .selected(tools.slot == slot)
                .build()
            {
                tools.slot = slot;
                self.rename_slot = None;
            }
        }
        ui.next_column();
        if let Some(state) = &tools.slots[tools.slot] {
            Image::new(TextureId::new(tools.slot + 2), [280.0, 196.0]).build(ui);
            ui.text(format!("Stage {} | tick {}", state.stage(), state.tick));
            if self.rename_slot != Some(tools.slot) {
                self.save_name = state.name.clone();
                self.rename_slot = Some(tools.slot);
            }
            ui.input_text("Name", &mut self.save_name).build();
            if ui.button("Rename") {
                tools.rename(&self.save_name);
            }
        } else {
            ui.text_disabled("Save gameplay to see a preview here.");
        }
        ui.columns(1, "", false);
        if ui.button("Save (F5)") {
            tools.save(modes, ticks);
            self.rename_slot = None;
        }
        ui.same_line();
        if ui.button("Load (F6)") {
            tools.load(modes);
            changed = true;
        }
        help(
            ui,
            "Disk saves replay the recorded session when loaded. Long sessions take longer to restore; incompatible saves leave the game unchanged.",
        );
        ui.same_line();
        if ui.button("Clear slot") {
            tools.clear();
            self.rename_slot = None;
        }
        changed
    }

    pub fn observe_present(&mut self, millis: f32) {
        if self.periods.len() < 180 {
            self.periods.push(millis);
        } else {
            self.periods[self.period_cursor] = millis;
            self.period_cursor = (self.period_cursor + 1) % self.periods.len();
        }
    }

    fn play(&mut self, ui: &Ui, modes: &mut Modes, tools: &mut DevTools, total_ticks: u64) -> bool {
        let mut changed = false;
        ui.text(format!("Mode {}  |  tick {total_ticks}", modes.name()));
        match modes.game() {
            Some(game) => {
                let (x, y) = game.player.position();
                let (health, max_health) = game.health();
                ui.text(format!("Stage {}", game.stage_id));
                ui.text(format!(
                    "Player {x}, {y}  camera {}, {}",
                    game.player.camera_x, game.player.camera_y
                ));
                ui.text(format!(
                    "Health {health}/{max_health}  lives {}  ammo {}",
                    game.lives(),
                    game.ammo()
                ));
                ui.text(format!("Score {}", game.score()));
            }
            None => ui.text_disabled("No game running: use Stages to warp to one."),
        }

        if !self.periods.is_empty() {
            let average = self.periods.iter().sum::<f32>() / self.periods.len() as f32;
            ui.text(format!(
                "Presentation: {:.1} FPS | {average:.2} ms average",
                1000.0 / average.max(0.01)
            ));
            ui.plot_lines("Frame time (ms)", &self.periods)
                .values_offset(self.period_cursor)
                .scale_min(0.0)
                .scale_max(50.0)
                .graph_size([0.0, 80.0])
                .build();
        }

        if ui.collapsing_header("Frame", TreeNodeFlags::DEFAULT_OPEN) {
            changed |= ui.checkbox("Pause", &mut tools.paused);
            ui.same_line();
            if ui.button("Step") {
                tools.step();
                changed = true;
            }
            let labels = SPEEDS.map(|speed| format!("{speed}x"));
            ui.combo_simple_string("Speed", &mut tools.speed, &labels);
        }

        if ui.collapsing_header("Cheats", TreeNodeFlags::DEFAULT_OPEN) {
            if let Some(game) = modes.game() {
                let mut invincible = game.options.invincible;
                let mut infinite = game.options.infinite;
                if ui.checkbox("God mode (invincibility)", &mut invincible) {
                    tools.set_invincible(modes, invincible);
                    changed = true;
                }
                if ui.checkbox("Infinite shurikens", &mut infinite) {
                    tools.set_infinite(modes, infinite);
                    changed = true;
                }
            }
            // The POW item's power-up (#70); a hit clears it again.
            if let Some(mut powered) = tools.powered(modes)
                && ui.checkbox("Powered up (POW)", &mut powered)
            {
                tools.set_powered(modes, powered);
                changed = true;
            }
            changed |= ui.checkbox("Infinite lives", &mut tools.infinite_lives);
            changed |= ui.checkbox("Lock health", &mut tools.lock_health);
            if ui.button("Refill health + ninjutsu") {
                tools.refill(modes);
                changed = true;
            }
        }

        changed
    }

    fn stages(&mut self, ui: &Ui, modes: &mut Modes, tools: &mut DevTools) -> bool {
        let mut changed = false;
        let current = modes.game().map(|game| game.stage_id);
        if let Some(id) = current {
            ui.text(format!("Current stage: {id}"));
        } else {
            ui.text_disabled("No game running: warp to start one.");
        }
        if ui.button("Skip substage") {
            tools.skip_substage(modes);
            changed = true;
        }
        if let Some(id) = current {
            ui.same_line();
            if ui.button("Restart part") {
                tools.warp(modes, id);
                changed = true;
            }
        }
        ui.separator();
        if !tools.stages.is_empty() {
            let labels: Vec<String> = tools
                .stages
                .iter()
                .map(|s| format!("{}-{} ({} parts)", s.id.round, s.id.substage, s.parts))
                .collect();
            self.stage = self.stage.min(labels.len() - 1);
            ui.combo_simple_string("Substage", &mut self.stage, &labels);
            let target = tools.stages[self.stage];
            let mut part = i32::from(self.part.min(target.parts - 1));
            ui.slider("Part", 0, i32::from(target.parts) - 1, &mut part);
            self.part = part as u8;
            if ui.button("Warp") {
                tools.warp(modes, target.id.with_part(self.part));
                changed = true;
            }
        }

        changed
    }
}

#[derive(Default)]
pub(super) struct Actions {
    pub art: Option<super::art::Action>,
    pub scene_changed: bool,
    pub capture: Option<(Button, Device)>,
}

pub(super) struct PanelInfo<'a> {
    pub crt_error: Option<&'a str>,
    pub art_name: &'a str,
    pub art_status: &'a str,
    pub ticks: u64,
    pub controllers: &'a [String],
    pub capture: Option<(Button, Device)>,
    pub held: u16,
    pub axes: [i16; 2],
    pub track: &'a str,
    pub audio_available: bool,
    pub queued_audio: usize,
    pub underruns: usize,
}

fn help(ui: &Ui, text: &str) {
    if ui.is_item_hovered() {
        ui.tooltip_text(text);
    }
}
