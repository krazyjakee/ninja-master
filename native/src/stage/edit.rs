//! Runtime writes to `Layout_Buffer` (`$FF177C`), the decoded foreground
//! layout. Collision (`$3854`) and plane A art are both read through the
//! layout, so one metatile write changes what the player stands on and what
//! is drawn. The ROM pairs each layout write with a VRAM name-table write:
//! `Func_015054` (`Func_015074` + `Func_0150BA`, a metatile's four cells) or
//! `Func_015134` (four blank cells); native draws plane A from the layout
//! every frame, so the layout write is all it needs.
use super::{Orientation, StageData};

/// Replace the 16x16 metatile containing the world pixel (`x`, `y`) with
/// `metatile`, as `Func_015074` does. Requested by stage objects
/// (`ObjectEvents::terrain_edits`) and enemies (`Ctx::edit_terrain`) and
/// applied by the game after their update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainEdit {
    pub x: i32,
    pub y: i32,
    pub metatile: u8,
}

impl StageData {
    /// Index into `layout` of a metatile column and row (`Func_037606`,
    /// `Func_015074`): 32 bytes per column, or per row when vertical.
    pub(super) fn layout_index(&self, col: i32, row: i32) -> Option<usize> {
        if col < 0 || row < 0 {
            return None;
        }
        let (col, row) = (col as usize, row as usize);
        let index = match self.orientation {
            Orientation::Horizontal if row < 32 => col * 32 + row,
            Orientation::Vertical if col < 32 => row * 32 + col,
            _ => return None,
        };
        (index < self.layout.len()).then_some(index)
    }
    /// The layout byte under a world pixel (`Func_037606`).
    pub fn metatile_at_pixel(&self, x: i32, y: i32) -> Option<u8> {
        self.metatile_at(x.div_euclid(16), y.div_euclid(16))
    }
    /// Write one layout byte; returns false outside the layout. The byte the
    /// part loaded with is kept for [`StageData::revert_edits`].
    pub fn apply_edit(&mut self, edit: TerrainEdit) -> bool {
        let Some(index) = self.layout_index(edit.x.div_euclid(16), edit.y.div_euclid(16)) else {
            return false;
        };
        if !self.edits.iter().any(|&(at, _)| at == index) {
            self.edits.push((index, self.layout[index]));
        }
        self.layout[index] = edit.metatile;
        true
    }
    /// `Mode_StageLoad` decodes the layout again when a life is lost: undo
    /// every runtime write.
    pub fn revert_edits(&mut self) {
        for (index, original) in self.edits.drain(..) {
            self.layout[index] = original;
        }
    }
}
