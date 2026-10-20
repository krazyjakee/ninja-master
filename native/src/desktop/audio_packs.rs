//! Audio pack selection and template export in the companion panel.
use super::packs::{Labels, PackPicker};
use imgui::Ui;
use ninja_master_native::audio_pack::{AudioPack, KIND};
use std::{
    path::PathBuf,
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

/// A pack loading beside the game: matching its loudness renders the
/// originals it replaces, which takes a few seconds.
pub(super) struct Loading {
    pub path: PathBuf,
    handle: JoinHandle<Result<AudioPack, String>>,
}

impl Loading {
    /// Without a ROM the pack loads unmatched (for a mode without sound).
    pub fn start(path: PathBuf, rom: Option<Vec<u8>>) -> Self {
        let file = path.clone();
        Self {
            path,
            handle: std::thread::spawn(move || AudioPack::load(&file, rom.as_deref())),
        }
    }
    pub fn finished(&self) -> bool {
        self.handle.is_finished()
    }
    pub fn join(self) -> (PathBuf, Result<AudioPack, String>) {
        let result = self
            .handle
            .join()
            .unwrap_or_else(|_| Err("loading stopped unexpectedly".into()));
        (self.path, result)
    }
}

pub(super) enum Action {
    Load(PathBuf),
    Original,
    Export(PathBuf),
}

pub(super) struct AudioPanel {
    pub packs: PackPicker,
    export_name: String,
    export_error: String,
}

impl Default for AudioPanel {
    fn default() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            packs: PackPicker::new(&KIND),
            export_name: format!("audio-template-{stamp}"),
            export_error: String::new(),
        }
    }
}

impl AudioPanel {
    pub fn show(
        &mut self,
        ui: &Ui,
        current: &str,
        status: &str,
        exporting: bool,
    ) -> Option<Action> {
        let mut action = None;
        ui.text(format!("Active audio: {current}"));
        let labels = Labels {
            combo: "Audio",
            original: "Original soundtrack",
            reload: "Reload audio",
            missing: "Edit folder packs and reload to hear changes. Music, effects and voices a pack leaves out keep the original sound.",
        };
        if let Some(choice) = self.packs.show(ui, &labels) {
            action = Some(choice.map_or(Action::Original, Action::Load));
        }
        ui.separator();
        ui.text("Create a pack");
        ui.text_wrapped("Render every music track, effect and voice to WAV with its loop points, ready to replace one file at a time with your own WAV or Ogg Vorbis audio.");
        ui.input_text("New template name##audio", &mut self.export_name)
            .build();
        ui.text_disabled(
            "Exports into the pack folder (about 400 MB). Existing files are never overwritten.",
        );
        {
            let _disabled = ui.begin_disabled(exporting);
            if ui.button("Export audio template") {
                match self.packs.library.template_path(&self.export_name) {
                    Ok(folder) => {
                        self.export_error.clear();
                        action = Some(Action::Export(folder));
                    }
                    Err(error) => self.export_error = error,
                }
            }
        }
        if !self.export_error.is_empty() {
            ui.text_wrapped(&self.export_error);
        }
        if !status.is_empty() {
            ui.separator();
            ui.text_wrapped(status);
        }
        action
    }
}
