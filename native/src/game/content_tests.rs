use super::*;
use crate::{
    assets::StageAssets,
    combat::Combat,
    content::{Cartridge, Content, StageSource},
    rom::Result,
    stage::{Section, StageId},
    stage_objects::StageObjects,
};
use std::sync::Arc;

fn cartridge() -> Vec<u8> {
    let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
    crate::rom::regions::prepare(std::fs::read(directory.join("shinobi3_us.bin")).unwrap()).unwrap()
}

/// The cartridge's stages under an expansion's name, playing 1-1 then 1-3.
struct Shortcut;
impl StageSource for Shortcut {
    fn content(&self) -> Content {
        Content::Expansion("shortcut".into())
    }
    fn sections(&self, rom: &[u8], id: StageId) -> Result<Vec<Section>> {
        Cartridge.sections(rom, id)
    }
    fn scroll_model(&self, rom: &[u8], id: StageId) -> Result<ScrollModel> {
        Cartridge.scroll_model(rom, id)
    }
    fn camera_bounds(&self, rom: &[u8], section: &Section) -> Result<CameraBounds> {
        Cartridge.camera_bounds(rom, section)
    }
    fn stage_assets(&self, rom: &[u8], id: StageId) -> Result<StageAssets> {
        Cartridge.stage_assets(rom, id)
    }
    fn combat(&self, rom: &[u8], id: StageId) -> Result<Combat> {
        Cartridge.combat(rom, id)
    }
    fn objects(&self, rom: &[u8], id: StageId) -> Result<StageObjects> {
        Cartridge.objects(rom, id)
    }
    fn enter_part_objects(
        &self,
        objects: &mut StageObjects,
        rom: &[u8],
        id: StageId,
    ) -> Result<bool> {
        Cartridge.enter_part_objects(objects, rom, id)
    }
    fn music(&self, rom: &[u8], id: StageId) -> Result<u8> {
        Cartridge.music(rom, id)
    }
    fn next_substage(&self, _: &[u8], id: StageId) -> Result<Option<StageId>> {
        Ok(((id.round, id.substage) == (1, 1)).then(|| StageId::new(1, 3, 0)))
    }
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn original_sessions_load_from_the_cartridge() {
    let rom = cartridge();
    let game = Game::load(&rom).unwrap();
    assert_eq!(game.content(), Content::Original);
    assert_eq!(game.next_stage(), Some(StageId::new(1, 2, 0)));
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn sessions_follow_their_source_through_substage_transitions() {
    let rom = cartridge();
    let mut game = Game::load_from(&rom, Arc::new(Shortcut), StageId::FOREST).unwrap();
    assert_eq!(game.content(), Content::Expansion("shortcut".into()));
    assert_eq!(game.next_stage(), Some(StageId::new(1, 3, 0)));

    game.request_next_substage();
    game.enter_next_substage();
    assert_eq!(game.stage_id, StageId::new(1, 3, 0));
    assert_eq!(game.content(), Content::Expansion("shortcut".into()));
    assert!(!game.complete);

    game.request_next_substage();
    game.enter_next_substage();
    assert!(game.complete);
}
