//! The list-6 objects as seen by the player's terrain probe.
use super::{LIFT, MAX_RECORDS, Object, SWITCH_DEVICE, StageObjects};
use crate::{game::Terrain, movement::Material};
use std::cell::Cell;

/// The player's terrain as `Player_ProbeTerrain` (`$21B20`) reads it: a
/// pixel that is not solid for objects falls through to the list-6
/// objects, and inside one the object's `+$29` material replaces it.
/// Found objects are marked for their update (`obj_flags` bit 14).
/// Only objects on the player's sprite-priority layer count (`$21B52`).
pub struct ObjectTerrain<'a, T: Terrain> {
    terrain: &'a T,
    objects: &'a [Object],
    player_high_priority: bool,
    touched: Cell<u32>,
}
impl<'a, T: Terrain> ObjectTerrain<'a, T> {
    pub fn new(terrain: &'a T, objects: &'a StageObjects, player_high_priority: bool) -> Self {
        Self {
            terrain,
            objects: &objects.objects,
            player_high_priority,
            touched: Cell::new(0),
        }
    }
    /// Records whose object the probes found, as bits.
    pub fn touched(&self) -> u32 {
        self.touched.get()
    }
}
impl<T: Terrain> Terrain for ObjectTerrain<'_, T> {
    fn width(&self) -> i32 {
        self.terrain.width()
    }
    fn height(&self) -> i32 {
        self.terrain.height()
    }
    fn solid_at(&self, x: i32, y: i32) -> bool {
        self.terrain.solid_at(x, y)
    }
    fn metatile_at(&self, x: i32, y: i32) -> Option<u8> {
        self.terrain.metatile_at(x, y)
    }
    fn material_at(&self, x: i32, y: i32) -> Material {
        match self.object(x, y) {
            Some(object) => {
                let mut touched = self.touched.get() | 1 << (object.record % MAX_RECORDS);
                // A door's bottom edge can overlap an item box's standing
                // slab. Crates allocated earlier must not hide the sensor:
                // the ROM's subsequent probes skip already touched objects
                // ($21B48), whereas native support queries reuse the slab.
                for sensor in self.objects.iter().filter(|sensor| {
                    sensor.kind == SWITCH_DEVICE
                        && sensor.collidable
                        && sensor.priority == self.player_high_priority
                        && sensor.contains(x, y)
                }) {
                    touched |= 1 << (sensor.record % MAX_RECORDS);
                }
                self.touched.set(touched);
                Material(object.material)
            }
            None => self.terrain.material_at(x, y),
        }
    }
    fn object_at(&self, x: i32, y: i32) -> bool {
        self.object(x, y).is_some()
    }
    fn wall_material_at(&self, x: i32, y: i32) -> Material {
        self.terrain.wall_material_at(x, y)
    }
}
impl<T: Terrain> ObjectTerrain<'_, T> {
    /// The object the probe finds: `$3854`'s bit 15 keeps the terrain code
    /// (`$21B2E`); otherwise the first object on the player's layer.
    fn object(&self, x: i32, y: i32) -> Option<&Object> {
        if self
            .terrain
            .material_at(x, y)
            .object_solid(self.player_high_priority)
        {
            return None;
        }
        self.objects.iter().find(|object| {
            object.collidable
                // $21B4A masks $E800: the release's retained touch bit
                // excludes a lift until its next handler consumes it.
                && !(object.kind == LIFT && object.touched)
                && object.priority == self.player_high_priority
                && object.contains(x, y)
        })
    }
}

impl Object {
    /// `$21B5E..$21B90`: a probe pixel within the hitbox, edges included.
    fn contains(&self, x: i32, y: i32) -> bool {
        (self.x + self.hitbox.x - x).abs() <= self.hitbox.half_width
            && (self.y + self.hitbox.y - y).abs() <= self.hitbox.half_height
    }
}
