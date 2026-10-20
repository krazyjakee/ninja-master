//! The drop-in pack list shared by the art and audio pack tabs.
use imgui::Ui;
use ninja_master_native::pack::{Kind, Library, PackEntry, library_directory, resolve_selection};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

/// What a tab calls its packs and the original content.
pub(super) struct Labels {
    pub combo: &'static str,
    pub original: &'static str,
    pub reload: &'static str,
    pub missing: &'static str,
}

pub(super) struct PackPicker {
    pub library: Library,
    kind: &'static Kind,
    selection: Option<PackEntry>,
    last_refresh: Instant,
}

impl PackPicker {
    pub fn new(kind: &'static Kind) -> Self {
        Self {
            library: Library::new(library_directory(kind), kind),
            kind,
            selection: None,
            last_refresh: Instant::now(),
        }
    }

    pub fn set_selection(&mut self, value: &str) {
        self.selection = if value.is_empty() {
            None
        } else {
            PackEntry::read(&resolve_selection(&self.library.root, value), self.kind).ok()
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

    /// The folder, pack choice, refresh/reload buttons and discovery errors.
    /// `Some(None)` chooses the original content.
    pub fn show(&mut self, ui: &Ui, labels: &Labels) -> Option<Option<PathBuf>> {
        let mut choice = None;
        if self.last_refresh.elapsed() >= Duration::from_secs(2) {
            self.refresh();
        }
        let mut folder = self.library.root.to_string_lossy().into_owned();
        ui.input_text("Pack folder", &mut folder)
            .read_only(true)
            .build();
        ui.text_wrapped("Drop ZIP files or pack folders here. New packs appear automatically.");
        let mut choices = vec![(labels.original.to_owned(), None)];
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
        if ui.combo_simple_string(labels.combo, &mut selected, &names) {
            choice = Some(choices[selected].1.clone());
        }
        if ui.button("Refresh list") {
            self.rescan();
        }
        ui.same_line();
        if ui.button(labels.reload) {
            choice = Some(self.selection.as_ref().map(|pack| pack.path.clone()));
        }
        ui.text_wrapped(labels.missing);
        for error in &self.library.errors {
            ui.text_wrapped(error);
        }
        choice
    }
}
