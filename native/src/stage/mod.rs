//! Stage loader for every round, substage and part (issue #35) and the
//! dynamic presentation of each part: palette routines, tile animation,
//! scenery sprites and the round 1-1 event.
//!
//! Stage setup, artwork and other content are decoded from the supplied US
//! cartridge through bounded readers, independently of SDL. `StageEffects`
//! updates the viewer's and the game's presentation state.
mod content;
mod data;
mod edit;
mod effects;
mod flags;
mod id;
mod palette_fx;
mod planes;
mod scenery;
mod scroll;
mod sections;
mod tile_anim;

pub use data::{Orientation, StageData, UploadKind};
pub use edit::TerrainEdit;
pub use effects::{
    EVENT_CAMERA_FLOOR, EVENT_CAMERA_X, EVENT_FLAG_TICK, EVENT_PALETTE_TICK, EVENT_PART,
    StageEffects,
};
pub use flags::{AutoScroll, ModeFlags, RuntimeFlags};
pub use id::{StageId, all_substages, camera_lock_music, next_substage, part_count, stage_music};
pub use palette_fx::PaletteContext;
pub use planes::{
    BossPlane, BossPlanes, BossRaster, LineWave, LzWordMapCall, RASTER_LINES, lz_word_map_call,
};
pub(crate) use planes::{FLYER_MAP, GUNBOAT_MAP};

pub use scroll::{
    COLUMNS, CameraBounds, Column, ColumnSource, RasterState, ScrollInputs, ScrollModel, VScroll,
    Vsram, forest_background_x, scroll_model,
};
pub use sections::{ExitKind, Section, sections};

/// Output bound of the Musashi and Kosinski streams stage loading decodes.
const LIMIT: usize = 65536;
