//! Where a session's stages come from: the cartridge's seven rounds or an
//! expansion.
//!
//! [`Content`] names the catalog a [`StageId`] belongs to. Expansion stages
//! number their rounds from 1 like the original game, so the id alone never
//! says which game is being played; anything that must only see the
//! original game (achievements, practice unlocks, the Continue checkpoint,
//! profile statistics) checks the session's content as well.
//!
//! [`StageSource`] is the seam `Game` loads stage parts through. [`Cartridge`]
//! decodes everything from the ROM exactly as before; an expansion source
//! builds the same structures from its own data and may reference ROM
//! content (art, enemy types, music) by id.
use crate::{
    assets::StageAssets,
    combat::Combat,
    rom::Result,
    stage::{CameraBounds, ScrollModel, Section, StageId},
    stage_objects::StageObjects,
};
use std::sync::Arc;

/// The catalog a session's [`StageId`]s belong to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Content {
    /// The cartridge's seven rounds.
    Original,
    /// An installed expansion, by its manifest id.
    Expansion(Arc<str>),
}
impl Content {
    pub fn is_original(&self) -> bool {
        matches!(self, Self::Original)
    }
}

/// Everything `Game` loads for a substage and its parts. `rom` is the
/// verified cartridge; every source may read it.
pub trait StageSource: Send + Sync {
    fn content(&self) -> Content;
    /// The part records of `id`'s substage (`id.part` is ignored).
    fn sections(&self, rom: &[u8], id: StageId) -> Result<Vec<Section>>;
    fn scroll_model(&self, rom: &[u8], id: StageId) -> Result<ScrollModel>;
    fn camera_bounds(&self, rom: &[u8], section: &Section) -> Result<CameraBounds>;
    fn stage_assets(&self, rom: &[u8], id: StageId) -> Result<StageAssets>;
    fn combat(&self, rom: &[u8], id: StageId) -> Result<Combat>;
    fn objects(&self, rom: &[u8], id: StageId) -> Result<StageObjects>;
    /// Moves `objects` into part `id`; true when it kept its state.
    fn enter_part_objects(
        &self,
        objects: &mut StageObjects,
        rom: &[u8],
        id: StageId,
    ) -> Result<bool>;
    /// The part's music command, requested on every load of the part.
    fn music(&self, rom: &[u8], id: StageId) -> Result<u8>;
    /// The music command a fresh load of the substage starts with.
    fn start_music(&self, rom: &[u8], id: StageId) -> Result<u8> {
        self.music(rom, id)
    }
    /// The substage after `id`; `None` ends the game.
    fn next_substage(&self, rom: &[u8], id: StageId) -> Result<Option<StageId>>;
}

/// The original game, decoded from the cartridge.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cartridge;
impl Cartridge {
    pub fn source() -> Arc<dyn StageSource> {
        Arc::new(Self)
    }
}
impl StageSource for Cartridge {
    fn content(&self) -> Content {
        Content::Original
    }
    fn sections(&self, rom: &[u8], id: StageId) -> Result<Vec<Section>> {
        crate::stage::sections(rom, id.round_index(), id.substage_index())
    }
    fn scroll_model(&self, rom: &[u8], id: StageId) -> Result<ScrollModel> {
        crate::stage::scroll_model(rom, id)
    }
    fn camera_bounds(&self, rom: &[u8], section: &Section) -> Result<CameraBounds> {
        CameraBounds::for_section(rom, section)
    }
    fn stage_assets(&self, rom: &[u8], id: StageId) -> Result<StageAssets> {
        StageAssets::load_stage(rom, id)
    }
    fn combat(&self, rom: &[u8], id: StageId) -> Result<Combat> {
        Combat::load_stage(rom, id)
    }
    fn objects(&self, rom: &[u8], id: StageId) -> Result<StageObjects> {
        StageObjects::load(rom, id)
    }
    fn enter_part_objects(
        &self,
        objects: &mut StageObjects,
        rom: &[u8],
        id: StageId,
    ) -> Result<bool> {
        objects.enter_part(rom, id)
    }
    fn music(&self, rom: &[u8], id: StageId) -> Result<u8> {
        crate::stage::stage_music(rom, id)
    }
    /// `$119F2` selects the stage-table music first. On 2-1, `Player_Init`
    /// immediately replaces IDATEN with UMA at `$22CC4`; the driver later
    /// raises its communication byte to request IDATEN at `$205D6`.
    fn start_music(&self, rom: &[u8], id: StageId) -> Result<u8> {
        if id.round == 2 && id.substage == 1 {
            crate::sound_events::SoundEvent::HorseStart.command(rom)
        } else {
            self.music(rom, id)
        }
    }
    fn next_substage(&self, rom: &[u8], id: StageId) -> Result<Option<StageId>> {
        crate::stage::next_substage(rom, id)
    }
}
