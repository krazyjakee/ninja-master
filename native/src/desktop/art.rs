//! Pack creation and selection in the companion panel.
use imgui::Ui;
use ninja_master_native::{
    art_pack::{Library, PackEntry, library_directory, resolve_selection},
    devtools::Substage,
    stage::StageId,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
    library: Library,
    selection: Option<PackEntry>,
    last_refresh: Instant,
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
            library: Library::new(library_directory()),
            selection: None,
            last_refresh: Instant::now(),
            export_name: format!("template-{stamp}"),
            export_error: String::new(),
            scale: 2,
            stage: 0,
            part: 0,
        }
    }
}

impl ArtPanel {
    pub fn set_selection(&mut self, value: &str) {
        self.selection = if value.is_empty() {
            None
        } else {
            PackEntry::read(&resolve_selection(&self.library.root, value)).ok()
        };
    }

    pub fn refresh(&mut self) {
        self.library.refresh();
        self.last_refresh = Instant::now();
    }
    pub fn rescan(&mut self) {
        self.library.rescan();
        self.last_refresh = Instant::now();
    }
    pub fn show(
        &mut self,
        ui: &Ui,
        stages: &[Substage],
        current: &str,
        status: &str,
    ) -> Option<Action> {
        let mut action = None;
        ui.text(format!("Active artwork: {current}"));
        if self.last_refresh.elapsed() >= Duration::from_secs(2) {
            self.refresh();
        }
        let mut folder = self.library.root.to_string_lossy().into_owned();
        ui.input_text("Pack folder", &mut folder)
            .read_only(true)
            .build();
        ui.text_wrapped("Drop ZIP files or pack folders here. New packs appear automatically.");
        let mut choices = vec![("Original artwork".to_owned(), None)];
        choices.extend(
            self.library
                .packs
                .iter()
                .map(|pack| (pack.label(), Some(pack.path.clone()))),
        );
        if let Some(pack) = &self.selection
            && !choices
                .iter()
                .any(|(_, path)| path.as_ref() == Some(&pack.path))
        {
            choices.push((pack.label(), Some(pack.path.clone())));
        }
        let active = self.selection.as_ref().map(|pack| &pack.path);
        let mut selected = choices
            .iter()
            .position(|(_, path)| path.as_ref() == active)
            .unwrap_or(0);
        let names: Vec<_> = choices.iter().map(|(name, _)| name.as_str()).collect();
        if ui.combo_simple_string("Artwork", &mut selected, &names) {
            action = Some(
                choices[selected]
                    .1
                    .clone()
                    .map_or(Action::Original, Action::Load),
            );
        }
        if ui.button("Refresh list") {
            self.rescan();
        }
        ui.same_line();
        if ui.button("Reload artwork") {
            action = Some(
                self.selection
                    .as_ref()
                    .map_or(Action::Original, |pack| Action::Load(pack.path.clone())),
            );
        }
        ui.text_wrapped("Edit folder packs and reload to see changes. Missing artwork uses the original graphics.");
        for error in &self.library.errors {
            ui.text_wrapped(error);
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
                match self.library.template_path(&self.export_name) {
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
