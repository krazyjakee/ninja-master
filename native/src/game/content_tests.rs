use super::*;
use crate::{
    assets::StageAssets,
    combat::Combat,
    content::{Cartridge, Content, StageRules, StageSource},
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

/// Cartridge stages under an expansion's name: substage `id` plays the
/// cartridge's `data(id)` with the routines of `routines(id)`, and
/// `next(id)` follows.
struct Remapped {
    data: fn(StageId) -> StageId,
    routines: fn(StageId) -> StageId,
    next: fn(StageId) -> Option<StageId>,
}
impl Remapped {
    fn source(self) -> Arc<dyn StageSource> {
        Arc::new(self)
    }
    fn at(&self, id: StageId) -> StageId {
        (self.data)(id.with_part(0)).with_part(id.part)
    }
}
impl StageSource for Remapped {
    fn content(&self) -> Content {
        Content::Expansion("remapped".into())
    }
    fn rules(&self, id: StageId) -> StageRules {
        StageRules::original((self.routines)(id.with_part(0)).with_part(id.part))
    }
    fn sections(&self, rom: &[u8], id: StageId) -> Result<Vec<Section>> {
        Cartridge.sections(rom, self.at(id))
    }
    fn scroll_model(&self, rom: &[u8], id: StageId) -> Result<ScrollModel> {
        Cartridge.scroll_model(rom, self.at(id))
    }
    fn camera_bounds(&self, rom: &[u8], section: &Section) -> Result<CameraBounds> {
        Cartridge.camera_bounds(rom, section)
    }
    fn stage_assets(&self, rom: &[u8], id: StageId) -> Result<StageAssets> {
        Cartridge.stage_assets(rom, self.at(id))
    }
    fn combat(&self, rom: &[u8], id: StageId) -> Result<Combat> {
        Cartridge.combat(rom, self.at(id))
    }
    fn objects(&self, rom: &[u8], id: StageId) -> Result<StageObjects> {
        Cartridge.objects(rom, self.at(id))
    }
    fn enter_part_objects(
        &self,
        objects: &mut StageObjects,
        rom: &[u8],
        id: StageId,
    ) -> Result<bool> {
        Cartridge.enter_part_objects(objects, rom, self.at(id))
    }
    fn music(&self, rom: &[u8], id: StageId) -> Result<u8> {
        Cartridge.music(rom, self.at(id))
    }
    fn next_substage(&self, _: &[u8], id: StageId) -> Result<Option<StageId>> {
        Ok((self.next)(id))
    }
}

/// 1-1 then 1-3, as themselves.
fn shortcut() -> Arc<dyn StageSource> {
    Remapped {
        data: |id| id,
        routines: |id| id,
        next: |id| ((id.round, id.substage) == (1, 1)).then(|| StageId::new(1, 3, 0)),
    }
    .source()
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
    let mut game = Game::load_from(&rom, shortcut(), StageId::FOREST).unwrap();
    assert_eq!(game.content(), Content::Expansion("remapped".into()));
    assert_eq!(game.next_stage(), Some(StageId::new(1, 3, 0)));

    game.request_next_substage();
    game.enter_next_substage();
    assert_eq!(game.stage_id, StageId::new(1, 3, 0));
    assert_eq!(game.content(), Content::Expansion("remapped".into()));
    assert!(!game.complete);

    game.request_next_substage();
    game.enter_next_substage();
    assert!(game.complete);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn stage_routines_follow_the_rules_not_the_stage_number() {
    let rom = cartridge();
    // The horse is 2-1's routine: 2-1's data numbered 1-1 still rides it,
    // and 2-1's number with 1-1's routines does not.
    let ridden = Remapped {
        data: |_| StageId::new(2, 1, 0),
        routines: |_| StageId::new(2, 1, 0),
        next: |_| None,
    };
    let game = Game::load_from(&rom, ridden.source(), StageId::FOREST).unwrap();
    assert!(game.special().is_some());
    let walked = Remapped {
        data: |_| StageId::new(1, 1, 0),
        routines: |_| StageId::new(1, 1, 0),
        next: |_| None,
    };
    let game = Game::load_from(&rom, walked.source(), StageId::new(2, 1, 0)).unwrap();
    assert!(game.special().is_none());
    assert_eq!(game.rules().routines, StageId::FOREST);
}

#[test]
#[ignore = "requires a user-supplied cartridge"]
fn continuing_after_game_over_keeps_the_source() {
    let rom = cartridge();
    let mut game = Game::load_from(&rom, shortcut(), StageId::FOREST).unwrap();
    game.request_next_substage();
    game.enter_next_substage();
    let resumed = game.continue_game().unwrap();
    assert_eq!(resumed.content(), Content::Expansion("remapped".into()));
    assert_eq!(resumed.stage_id, StageId::new(1, 3, 0));
    assert_eq!(resumed.next_stage(), None);
}
