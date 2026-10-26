//! Behaviour an expansion adds in code: new enemy types and bosses.
//!
//! Packs hold data only. Anything that needs new logic is Rust compiled
//! into the game and registered in [`CODE`] under a name; a manifest
//! selects it with `"code": "<name>"`. Handlers use the same API as the
//! original game's (`crate::enemies::Behavior` and `Ctx`), and draw with art
//! and poses the cartridge already provides.
//!
//! To add code for an expansion: write a module here implementing [`Code`],
//! add it to [`CODE`], and give new enemy types kinds from [`FIRST_KIND`]
//! up, in steps of 4.
use crate::{enemies::Behavior, rom::Result, stage::StageId};
use std::sync::Arc;

mod forest_trail;

/// The first expansion object type. The cartridge's types are the
/// `$12E9A` byte offsets `$00..$FC`, and `crate::bosses::BOSS_KIND` is
/// `$100`.
pub const FIRST_KIND: u16 = 0x200;

/// A new enemy type and the stats a cartridge type reads from `$68A8` and
/// `$68AA`.
pub(crate) struct EnemyType {
    pub(crate) kind: u16,
    /// The cartridge type whose art the new type draws with. A part can
    /// place it only where its base loads that art.
    pub(crate) art: u16,
    pub(crate) score_bcd: u16,
    pub(crate) contact_damage: u8,
    /// Hits it survives (`obj_health` counts down to 0).
    pub(crate) health_counter: u8,
    pub(crate) behavior: Arc<dyn Behavior>,
}

/// One expansion's compiled behaviour.
pub(crate) trait Code: Send + Sync {
    /// New enemy types its parts can place by kind.
    fn enemies(&self, _rom: &[u8]) -> Result<Vec<EnemyType>> {
        Ok(Vec::new())
    }
    /// A boss for the arena of expansion part `id`, replacing its base's.
    fn boss(&self, _rom: &[u8], _id: StageId) -> Result<Option<Arc<dyn Behavior>>> {
        Ok(None)
    }
}

/// Every expansion's code, by the name a manifest's `code` gives.
static CODE: &[(&str, &dyn Code)] = &[("forest-trail", &forest_trail::ForestTrail)];

pub(crate) fn find(name: &str) -> Option<&'static dyn Code> {
    CODE.iter()
        .find(|(entry, _)| *entry == name)
        .map(|&(_, code)| code)
}
