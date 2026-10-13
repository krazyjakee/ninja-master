//! Material 15 (#63): the splashes the player's collision leaves in water.
//! The game turns each into a list-1 type-`$24` object
//! (`crate::combat::Combat::splash`); only 1-2 has material 15.
use super::{Player, UNIT};
use crate::{game::Terrain, movement::Material};

/// Which of the three splash sites fired. The `Data_022DD8` frame group
/// the object shows (its `plr_ctrl`, +`$2A`) follows from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplashKind {
    /// `$21F50`: the feet probe of a falling (or level) update first reads
    /// 15. Effect `$27`, group 0.
    Enter,
    /// `Player_CollideCeiling` (`$22134..$22178`): rising, the feet are in 15
    /// after this update's motion and would be out of it after the next.
    /// Effect `$27`, group `$C`.
    Leave,
    /// `$223A4`: the first wall-probe material is 15 while a direction is
    /// held, on frames where `Frame_Counter & 7` is 0 (the caller tests
    /// that). Silent; group `$18` right, `$24` left.
    Wade { left: bool },
}
impl SplashKind {
    /// Index of the three-entry `Data_022DD8` group: `plr_ctrl` / `$C`.
    pub fn group(self) -> usize {
        match self {
            Self::Enter => 0,
            Self::Leave => 1,
            Self::Wade { left: false } => 2,
            Self::Wade { left: true } => 3,
        }
    }
    /// Entering and leaving request effect `$27` before the allocation.
    pub fn sound(self) -> bool {
        !matches!(self, Self::Wade { .. })
    }
}

/// A splash to allocate at a world pixel. The object snaps Y to its
/// 16-pixel row itself (`$22D86`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Splash {
    pub kind: SplashKind,
    pub x: i32,
    pub y: i32,
}

impl Player {
    /// `$21F00..$21F46`: the material the floor probes leave in D2. The
    /// centre probe, then (unless hanging, `plr_flags` bit 0) the left and
    /// right body edges; the first floor material (`$5DBE`) ends the search,
    /// otherwise the last probe's material remains.
    pub(super) fn feet_material(&self, terrain: &impl Terrain, x: i32, row: i32) -> Material {
        let probes = Self::probes(x);
        let probes = if self.hanging {
            &probes[..1]
        } else {
            &probes[..]
        };
        let mut material = Material::EMPTY;
        for &px in probes {
            material = terrain.material_at(px, row).for_player();
            if material.stands_on() {
                break;
            }
        }
        material
    }
    /// `Player_CollideCeiling` (`$22104..$22178`), run instead of the floor
    /// collision while rising outside the somersault: the probe at the feet
    /// after this update's motion reads 15 and the one a second step of the
    /// same velocity further up does not. The splash is at the feet before
    /// the motion.
    pub(super) fn leave_water(&self, terrain: &impl Terrain, x: i32) -> Option<Splash> {
        let at = |y: i32| terrain.material_at(x, y.div_euclid(UNIT));
        let after = self.y + self.velocity_y;
        (at(after) == Material::SPLASH && at(after + self.velocity_y) != Material::SPLASH).then(
            || Splash {
                kind: SplashKind::Leave,
                x,
                y: self.y.div_euclid(UNIT),
            },
        )
    }
    /// `$21F46..$21F86`: the feet probe read at the feet after the motion
    /// replaces `plr_floor`; a change to 15 splashes there.
    pub(super) fn enter_water(&mut self, terrain: &impl Terrain, x: i32) -> Option<Splash> {
        let (_, feet) = self.position();
        let material = self.feet_material(terrain, x, feet);
        let before = std::mem::replace(&mut self.feet_probe, material);
        (material != before && material == Material::SPLASH).then_some(Splash {
            kind: SplashKind::Enter,
            x,
            y: feet,
        })
    }
}
