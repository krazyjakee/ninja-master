use super::*;

/// Minimal terrain interface keeps simulation usable without SDL or a ROM.
pub trait Terrain {
    fn width(&self) -> i32;
    /// Layout height in pixels: 512 for horizontal layouts.
    fn height(&self) -> i32 {
        512
    }
    /// Compatibility query for ordinary objects: `$3854`'s D2 bit 15 for a
    /// sprite with the priority bit clear (`$3906` high word `$55BE`:
    /// materials 1-5, 7, 8, 10, 12 and 14), the test most enemy handlers
    /// make with `BMI`. It is not the player's collision; the player uses
    /// the predicates below. Terrains that only implement this read as
    /// full-solid material 1 through the default `material_at`.
    fn solid_at(&self, x: i32, y: i32) -> bool;
    /// `$3854`'s signed result for an ordinary object on one sprite-priority
    /// layer. The mask at `$3906` selects `$55BE` for low-priority sprites
    /// and `$5ABE` for high-priority sprites.
    fn object_solid_at(&self, x: i32, y: i32, high_priority: bool) -> bool {
        self.material_at(x, y).object_solid(high_priority)
    }
    /// `$3854`: the material code at a pixel after quadrant exclusion.
    fn material_at(&self, x: i32, y: i32) -> Material {
        if self.solid_at(x, y) {
            Material::SOLID
        } else {
            Material::EMPTY
        }
    }
    /// `$22456`: wall scans query the layout directly, without probing or
    /// activating list-6 platforms (`Player_ProbeTerrain`, `$21B20`).
    fn wall_material_at(&self, x: i32, y: i32) -> Material {
        self.material_at(x, y)
    }
    /// `$21B98`: the player probe found a list-6 object at the pixel
    /// (`d2` = `$8200` + its material), not the layout.
    fn object_at(&self, _x: i32, _y: i32) -> bool {
        false
    }
    /// Player floor: lands on and stands on the pixel (`$220EC` mask).
    fn stands_on(&self, x: i32, y: i32) -> bool {
        self.material_at(x, y).stands_on()
    }
    /// Player wall moving in `direction` (`$22490` mask, one-way 9 and 13).
    fn blocks_horizontal(&self, x: i32, y: i32, direction: i32) -> bool {
        self.material_at(x, y).blocks_horizontal(direction)
    }
    /// Stops a rising player's head (`$2259E` mask without 2, 6 and 14).
    fn blocks_ceiling(&self, x: i32, y: i32) -> bool {
        self.material_at(x, y).blocks_ceiling()
    }
    /// A wall the player can cling to while moving in `direction` (`$210F2`).
    fn clingable(&self, x: i32, y: i32, direction: i32) -> bool {
        self.material_at(x, y).clingable(direction)
    }
    /// Spikes (`$21C44..$21C94`).
    fn hurts(&self, x: i32, y: i32) -> bool {
        self.material_at(x, y).hurts()
    }
    /// The `Layout_Buffer` metatile under a pixel (`Func_037606`), for
    /// handlers that test the layout itself; `None` without a layout.
    fn metatile_at(&self, _x: i32, _y: i32) -> Option<u8> {
        None
    }
}
impl Terrain for StageAssets {
    fn width(&self) -> i32 {
        self.data.pixel_size().0
    }
    fn height(&self) -> i32 {
        self.data.pixel_size().1
    }
    fn solid_at(&self, x: i32, y: i32) -> bool {
        StageAssets::solid_at(self, x, y)
    }
    fn material_at(&self, x: i32, y: i32) -> Material {
        Material(self.collision_at(x, y))
    }
    fn metatile_at(&self, x: i32, y: i32) -> Option<u8> {
        self.data.metatile_at_pixel(x, y)
    }
}
