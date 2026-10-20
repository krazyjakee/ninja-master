//! Pack creation and selection in the companion panel.
use super::packs::{Labels, PackPicker};
use imgui::Ui;
use ninja_master_native::{art_pack::KIND, devtools::Substage, stage::StageId};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) enum Action {
    Load(PathBuf),
    Original,
    Export {
        folder: PathBuf,
        stage: StageId,
        scale: usize,
    },
}

pub(super) struct ArtPanel {
    pub packs: PackPicker,
    export_name: String,
    export_error: String,
    scale: usize,
    stage: usize,
    part: i32,
}

impl Default for ArtPanel {
    fn default() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            packs: PackPicker::new(&KIND),
            export_name: format!("template-{stamp}"),
            export_error: String::new(),
            scale: 2,
            stage: 0,
            part: 0,
        }
    }
}

impl ArtPanel {
    pub fn show(
        &mut self,
        ui: &Ui,
        stages: &[Substage],
        current: &str,
        status: &str,
    ) -> Option<Action> {
        let mut action = None;
        ui.text(format!("Active artwork: {current}"));
        let labels = Labels {
            combo: "Artwork",
            original: "Original artwork",
            reload: "Reload artwork",
            missing: "Edit folder packs and reload to see changes. Missing artwork uses the original graphics.",
        };
        if let Some(choice) = self.packs.show(ui, &labels) {
            action = Some(choice.map_or(Action::Original, Action::Load));
        }
        ui.separator();
        ui.text("Create a pack: easy mode");
        ui.text_wrapped("Export assembled player frames and a stage's tile sheets, ready to paint in any PNG editor. Animation and alignment are already mapped.");
        if !stages.is_empty() {
            self.stage = self.stage.min(stages.len() - 1);
            let names: Vec<_> = stages
                .iter()
                .map(|s| format!("{}-{}", s.id.round, s.id.substage))
                .collect();
            ui.combo_simple_string("Stage##art", &mut self.stage, &names);
            let stage = stages[self.stage];
            self.part = self.part.min(i32::from(stage.parts) - 1);
            ui.slider("Part##art", 0, i32::from(stage.parts) - 1, &mut self.part);
            ui.combo_simple_string(
                "Artwork size",
                &mut self.scale,
                &["1x (original)", "2x", "4x", "8x", "10x (4K)"],
            );
            ui.input_text("New template name", &mut self.export_name)
                .build();
            ui.text_disabled(
                "Exports into the pack folder. Existing artwork is never overwritten.",
            );
            if ui.button("Export easy mode template") {
                match self.packs.library.template_path(&self.export_name) {
                    Ok(folder) => {
                        self.export_error.clear();
                        action = Some(Action::Export {
                            folder,
                            stage: stage.id.with_part(self.part as u8),
                            scale: [1, 2, 4, 8, 10][self.scale],
                        });
                    }
                    Err(error) => self.export_error = error,
                }
            }
            if !self.export_error.is_empty() {
                ui.text_wrapped(&self.export_error);
            }
        } else {
            ui.text_wrapped("Start play mode to select a stage and export its artwork.");
        }
        if !status.is_empty() {
            ui.separator();
            ui.text_wrapped(status);
        }
        action
    }
}
