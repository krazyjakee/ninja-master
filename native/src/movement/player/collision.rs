//! The player's terrain probes and wall and floor collision.
use super::{BODY_HEIGHT, HALF_WIDTH, Player, UNIT, WALL_HALF_WIDTH};
use crate::{
    game::Terrain,
    movement::{
        GRAVITY, Material,
        material::{CLING_PROBES, WALL_PROBE_ROWS, wall_probe_rows},
    },
};

/// What `Player_CollideWalls` leaves for the next update's
/// `Player_TryWallKick` (`$210CE`) besides `plr_ctrl` bit 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WallProbe {
    /// The side of the motion that was probed: -1 left, 1 right.
    pub side: i32,
    /// The first wall-probe material met (`plr_floor+2` or `+3`).
    pub material: Material,
    /// `plr_flags` bit 2: it was met 1 or 24 px above the feet.
    pub cling: bool,
}

impl Player {
    /// The body overlaps terrain that blocks the player from every side
    /// (materials 1, 3, 4 and 5). One-way materials never count.
    pub(crate) fn occupied(terrain: &impl Terrain, x: i32, feet: i32) -> bool {
        (x - HALF_WIDTH..x + HALF_WIDTH)
            .any(|px| (feet - BODY_HEIGHT..feet).any(|py| terrain.material_at(px, py).full_block()))
    }
    /// `$21F00..$21F46`: centre, then +/- (obj_hitbox_w - 1).
    /// Ordinary player mappings have half-width 15, independently of the
    /// smaller native body used for occupancy and the layout bounds.
    pub(super) fn probes(x: i32) -> [i32; 3] {
        [x, x - (WALL_HALF_WIDTH - 1), x + (WALL_HALF_WIDTH - 1)]
    }
    /// The floor material in pixel row `row`, the first row below the feet.
    /// The original records the first probe in `Player_SolidMaterialMask`;
    /// natively a hard floor wins over material 12, which the player sinks
    /// through, so the body never enters solid ground.
    pub(super) fn floor_below(terrain: &impl Terrain, x: i32, row: i32) -> Option<Material> {
        let mut sinking = None;
        for px in Self::probes(x) {
            let material = terrain.material_at(px, row).for_player();
            if material.stands_on() {
                if !material.slows() {
                    return Some(material);
                }
                sinking = Some(material);
            }
        }
        sinking
    }
    /// Whether [`Self::floor_below`]'s hard floor in `row` is a list-6
    /// object rather than the layout.
    fn floor_on_object(terrain: &impl Terrain, x: i32, row: i32) -> bool {
        Self::probes(x)
            .into_iter()
            .find(|&px| {
                let material = terrain.material_at(px, row).for_player();
                material.stands_on() && !material.slows()
            })
            .is_some_and(|px| terrain.object_at(px, row))
    }
    /// A 1, 3, 4 or 5 in row `row` at any of the three body columns. Only
    /// the native somersault and air-stab lifts use it; the original's rise
    /// stops at the head probe alone (`Player::bonk`).
    pub(super) fn ceiling_above(terrain: &impl Terrain, x: i32, row: i32) -> Option<Material> {
        [x, x - HALF_WIDTH, x + HALF_WIDTH - 1]
            .into_iter()
            .map(|px| terrain.material_at(px, row).for_player())
            .find(|material| material.blocks_ceiling())
    }
    /// The body column at pixel `column`, feet first as `$22264..$22336`
    /// probes it, up to `height` px above the feet
    /// (`movement::wall_probe_height`): the first material that stops
    /// motion in `direction`.
    fn wall_at(
        terrain: &impl Terrain,
        column: i32,
        feet: i32,
        height: i32,
        direction: i32,
    ) -> Option<Material> {
        (feet - height..feet)
            .rev()
            .map(|py| terrain.wall_material_at(column, py).for_player())
            .find(|material| material.blocks_horizontal(direction))
    }
    /// `$22264..$22366`: the probes at the leading column of the position
    /// this update's velocity `dx` heads for, from the ground up. The first
    /// wall-probe material (`Material::wall_probe`) is recorded on the side
    /// of the motion (`plr_floor+2` right, `+3` left), and `plr_flags` bit 2
    /// says whether it came from the probe 1 or 24 px above the feet. The
    /// motion is tested at the destination, not at the current position, so
    /// a velocity the wall rejects still records it from a pixel or two
    /// away.
    ///
    /// Also returns the probed pixel, where `$223A4` places a wading splash.
    fn wall_probe(
        &self,
        terrain: &impl Terrain,
        dx: i32,
        height: i32,
    ) -> Option<(WallProbe, (i32, i32))> {
        let side = dx.signum();
        if side == 0 {
            return None;
        }
        let (_, feet) = self.position();
        let column = Self::leading_column((self.x + dx).div_euclid(UNIT), side);
        wall_probe_rows(height).iter().find_map(|&rise| {
            let material = terrain.wall_material_at(column, feet - rise).for_player();
            material.wall_probe().then(|| {
                let probe = WallProbe {
                    side,
                    material,
                    cling: WALL_PROBE_ROWS[..CLING_PROBES].contains(&rise),
                };
                (probe, (column, feet - rise))
            })
        })
    }
    /// The body's leading pixel column at `x` when moving in `direction`.
    pub(super) fn leading_column(x: i32, direction: i32) -> i32 {
        if direction > 0 {
            x + WALL_HALF_WIDTH
        } else {
            x - WALL_HALF_WIDTH
        }
    }
    /// Whether a pixel in `column` up to `height` px above the feet has
    /// material satisfying `test`.
    pub(super) fn column_has(
        terrain: &impl Terrain,
        column: i32,
        feet: i32,
        height: i32,
        test: impl Fn(Material) -> bool,
    ) -> bool {
        (feet - height..feet).any(|py| test(terrain.wall_material_at(column, py).for_player()))
    }
    /// `Player_CollideWalls` for this update's X velocity `dx`: material 12
    /// ahead halves it, and the body moves until its leading column meets a
    /// wall `wall_height` px high. Returns the velocity after the halving.
    /// A `forced_exit` (`plr_event` 3) can leave the layout's right edge,
    /// but still meets walls (`$22264`), including 7-2's closed door.
    pub(super) fn collide_walls(
        &mut self,
        terrain: &impl Terrain,
        mut dx: i32,
        wall_height: i32,
        forced_exit: bool,
    ) -> i32 {
        let (mut x, feet) = self.position();
        let side = dx.signum();
        let probe = self.wall_probe(terrain, dx, wall_height);
        self.wall_probe = probe.map(|(probe, _)| probe);
        self.water_probe = probe
            .filter(|(probe, _)| probe.material == Material::SPLASH)
            .map(|(_, at)| at);
        if side != 0
            && Self::column_has(
                terrain,
                Self::leading_column((self.x + dx).div_euclid(UNIT), side),
                feet,
                wall_height,
                Material::slows,
            )
        {
            dx >>= 1; // $22370..$2237C: material 12 ahead halves X velocity.
        }
        let old_x = self.x;
        // Ordinary play stays inside the recovered layout bounds.
        let right_bound = if forced_exit {
            i32::MAX / 2
        } else {
            (terrain.width() - HALF_WIDTH) * UNIT
        };
        let target_x = (self.x + dx).clamp(HALF_WIDTH * UNIT, right_bound);
        let destination = target_x.div_euclid(UNIT);
        let step = (destination - x).signum();
        let mut blocked = false;
        // $22366..$223A2: only the leading body column meets walls; one-way
        // floors (2, 7) and hang bars (6) never block, 9 and 13 in one direction.
        while x != destination {
            let column = Self::leading_column(x + step, step);
            if Self::wall_at(terrain, column, feet, wall_height, step).is_some() {
                blocked = true;
                break;
            }
            x += step;
        }
        // $22414..$2242E negates the complete velocity and adds it back to
        // obj_x, preserving the old 16.16 fraction rather than moving as far
        // as the first free pixel.
        self.x = if blocked { old_x } else { target_x };
        self.against_wall = blocked;
        if blocked {
            self.run_control.blocked(); // $22428..$22436
        }
        self.moving = self.x != old_x;
        dx
    }
    /// `$19D4` integrates Y after the handlers; falling lands on the first
    /// floor in the row below the feet at column `x` unless `passing`.
    /// Sets the new floor material.
    pub(super) fn move_vertically(
        &mut self,
        terrain: &impl Terrain,
        x: i32,
        floor: Option<Material>,
        passing: bool,
        airborne_before: bool,
        probe_velocity_y: i32,
    ) {
        // $21FE0/$2207E: material 12 neither snaps nor cancels gravity, so
        // the standing player sinks by one tick of gravity until a hard floor.
        let sinking = self.grounded && floor.is_some_and(Material::slows);
        let target_y = self.y + self.velocity_y + if sinking { GRAVITY } else { 0 };
        let (_, feet) = self.position();
        let mut y = feet;
        let destination = target_y.div_euclid(UNIT);
        let step = (destination - y).signum();
        // `$21F00`: probe obj_y + obj_vel_y before Object_Integrate adds
        // gravity. Include that row when the move ends exactly on it.
        let probe_row = (self.y + probe_velocity_y).div_euclid(UNIT);
        let last = if step > 0 && probe_row == destination {
            destination + 1
        } else {
            destination
        };
        let mut blocked = false;
        while y != last {
            // Falling meets floors in the row below the feet. Rising meets
            // nothing: Player_CollideCeiling only splashes, and the head
            // probe's bonk above is what stops a rise.
            let stop = if passing || step < 0 {
                None
            } else {
                Self::floor_below(terrain, x, y).filter(|floor| !(sinking && floor.slows()))
            };
            if stop.is_some() {
                self.grounded = true;
                self.velocity_y = 0;
                blocked = true;
                break;
            }
            y += step;
        }
        self.y = if blocked { y * UNIT } else { target_y };
        // $2202A..$2204A checks one pixel above the floor at the column
        // that found support, not all three columns again. Probing the
        // other body edges here arms adjacent list-6 mines too early.
        let embedded = |feet: i32| {
            let column = Self::probes(x)
                .into_iter()
                .find(|&px| {
                    let material = terrain.material_at(px, y).for_player();
                    material.stands_on() && !material.slows()
                })
                .unwrap_or(x);
            let material = terrain.material_at(column, feet - 1).for_player();
            material.stands_on() && !material.slows()
        };
        // $22004: a list-6 object floor (`d2` >= `$8200`) leaves a standing
        // rider's Y alone, fraction included; a landing takes the whole
        // pixel of y + vy plus one ($22010..$22026). Its carry then writes
        // the platform's Y.
        let object_floor = self.grounded && !sinking && Self::floor_on_object(terrain, x, y);
        if object_floor {
            if airborne_before {
                self.y = (y + 1) * UNIT;
            }
        } else if self.grounded && !sinking && embedded(y) {
            // $22020..$2204A: feet inside a floor (landing within a one-way
            // ledge) snap to its 8-pixel row, then rise 2 px per update.
            let mut feet = y & !7;
            if embedded(feet) {
                feet -= 2;
            }
            self.y = feet * UNIT;
        }
        if self.grounded {
            self.step_down = false; // $21FD2
            if !sinking && !object_floor {
                // $22020: the contact writes the row, clearing the fraction.
                self.y = self.y.div_euclid(UNIT) * UNIT;
            }
        }
        let (_, feet) = self.position();
        self.floor = if self.grounded {
            Self::floor_below(terrain, x, feet).unwrap_or_default()
        } else {
            Material::EMPTY
        };
    }
}
