//! The automatic resume file is independent of the eight manual save slots.
use crate::{
    devtools::{SaveState, replay::History},
    game::Game,
};
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct Progress {
    path: Option<PathBuf>,
    state: Option<SaveState>,
    pub status: String,
}
impl Progress {
    pub fn load(path: Option<PathBuf>, rom: &[u8]) -> Self {
        let mut progress = Self {
            path,
            ..Self::default()
        };
        if let Some(path) = &progress.path {
            match std::fs::read(path) {
                Ok(bytes) => match SaveState::decode(&bytes, crate::rom::regions::checksum(rom)) {
                    Ok(state) => progress.state = Some(state),
                    Err(error) => progress.status = format!("Continue unavailable: {error}"),
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => progress.status = format!("Cannot read saved run: {error}"),
            }
        }
        progress
    }
    pub fn invalidate(&mut self) {
        self.state = None;
    }
    pub fn available(&self) -> bool {
        self.state.is_some()
    }
    pub fn restore(&self, rom: &[u8]) -> Result<(Box<Game>, History), String> {
        self.state.as_ref().ok_or("No saved run")?.restore(rom)
    }
    pub fn save(&mut self, state: SaveState, rom_crc: u32) {
        if let Some(path) = &self.path {
            if let Err(error) = state.write_path(path, rom_crc) {
                self.status = format!("Resume kept in memory; cannot save: {error}");
            } else {
                self.status.clear();
            }
        }
        self.state = Some(state);
    }
    pub fn clear(&mut self) -> Result<(), String> {
        if let Some(path) = &self.path
            && let Err(error) = std::fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error.to_string());
        }
        self.state = None;
        self.status.clear();
        Ok(())
    }
}
