//! The state-handler routines `Player::tick_with` calls in the original
//! order: the air C press, the hang entry and hang steps, and the ceiling
//! bonk and lifts.
use super::{BODY_HEIGHT, Player, UNIT};
use crate::{
    game::Terrain,
    movement::{
        Action, HANG_BELOW_CEILING, HANG_HEAD, HANG_WALL_AHEAD, HANG_WALL_DROP, Material,
        SOMERSAULT_LIFT, SOMERSAULT_SPEED, SOMERSAULT_WINDOW, hang_steps, state,
    },
};

impl Player {
    /// Raise the feet by up to `pixels` without entering a ceiling.
    pub(super) fn lift(&mut self, terrain: &impl Terrain, pixels: i32) {
        let (x, mut feet) = self.position();
        let fraction = self.y.rem_euclid(UNIT);
        for _ in 0..pixels {
            if Self::ceiling_above(terrain, x, feet - 1 - BODY_HEIGHT).is_some() {
                break;
            }
            feet -= 1;
        }
        self.y = feet * UNIT + fraction;
    }
    /// `Func_022494` (`$224F2..$2259C`) after the head probe touched 1, 3,
    /// 4 or 5: the somersault is spent. A rising somersault holds its
    /// place (`y -= vy`, so only this update's gravity moves it). Other
    /// states outside 32-47, below 2 px/tick down, snap the feet to
    /// `((y + vy) & ~7) + 9` and bounce down at |vy| / 4, unless Up has just
    /// been pressed under it (no contact on the last update), which leaves
    /// the rise to `Player_TryHang`. Returns whether it acted.
    pub(super) fn bonk(&mut self, state: usize, touching_before: bool) -> bool {
        use state::SOMERSAULT;
        self.somersault_spent = true; // $224F2: +$2A bit 12
        if state == SOMERSAULT {
            if self.velocity_y < 0 {
                self.y -= self.velocity_y; // $22522..$2252C
            }
            return true;
        }
        if state & !15 == 32 || self.velocity_y.div_euclid(UNIT) > 1 {
            return false; // $22540..$22556
        }
        let feet = ((self.y + self.velocity_y).div_euclid(UNIT) & !7) + 9;
        self.y = feet * UNIT; // $22562..$2257E
        if self.up_held && !touching_before {
            return true; // $22582..$2258C
        }
        if self.velocity_y < 0 {
            self.velocity_y = -self.velocity_y >> 2; // $2258E..$2259A
        }
        true
    }
    /// C in the air inside the state-16/25 handlers: `$210CE` wall cling,
    /// then `$21078` somersault. A refused somersault is spent (+$2A bit 12).
    /// The cling reads what the last update's wall probes left behind, not
    /// the terrain beside the body: a velocity the wall rejected leaves Joe
    /// where he was, up to its whole length short of the face, and the
    /// cling still takes.
    pub(super) fn air_press(&mut self, terrain: &impl Terrain, direction: i32) -> Option<Action> {
        let (x, _) = self.position();
        // $210CE..$21112: a wall stopped the last update's motion (+$2B bit
        // 1), its probes met it 1 or 24 px above the feet (plr_flags bit 2),
        // a direction is held and the material recorded on that side
        // (plr_floor+2 right, +3 left) is not 0 or 8, and X > 16.
        if self.against_wall
            && x > 16
            && self.wall_probe.is_some_and(|wall| {
                wall.cling && wall.side == direction && wall.material.clingable(direction)
            })
        {
            return Some(Action::WallCling);
        }
        // $21078..$210C0: once per airborne phase, only while |vy| < 1.4375.
        if !self.somersault_spent && self.velocity_y.abs() < SOMERSAULT_WINDOW {
            self.somersault_spent = true;
            self.lift(terrain, SOMERSAULT_LIFT);
            self.velocity_y = SOMERSAULT_SPEED;
            self.jump_control.cancel_hold();
            return Some(Action::Somersault);
        }
        self.somersault_spent = true;
        None
    }
    /// `Player_TryHang` (`$21118`): touching a ceiling that can be hung from
    /// (not 5 or 7) with Up held, and the head row under it empty or 12.
    /// Natively the hang Y comes from the touched 8-px row, which is what
    /// the original's `(y & ~7) + 8` gives on its successful path and also
    /// holds where the native body stopped under a solid ceiling.
    pub(super) fn try_hang(&mut self, terrain: &impl Terrain) -> bool {
        if !(self.ceiling_contact && self.up_held && self.ceiling.hangable()) {
            return false;
        }
        let (x, _) = self.position();
        let feet = (self.ceiling_row & !7) + HANG_BELOW_CEILING;
        let head = terrain.material_at(x, feet - HANG_HEAD).for_player();
        if head != Material::EMPTY && !head.slows() {
            return false;
        }
        self.y = feet * UNIT; // $21176..$21184, fraction cleared
        self.velocity_y = 0; // $2118A
        true
    }
    /// `PlayerState_Hang` (`$201C0`) while touching the ceiling: snap Y with
    /// `(y + 4) & ~7`, clear the velocity and return this update's X motion.
    /// `Player_HangSteps` moves only on a phase's first update; a wall 24 px
    /// ahead, or no direction, runs `Player_HangStop` instead.
    pub(super) fn hang_motion(
        &mut self,
        terrain: &impl Terrain,
        direction: i32,
        cursor: u16,
    ) -> i32 {
        let (x, feet) = self.position();
        let feet = (feet + 4) & !7;
        self.y = feet * UNIT;
        self.velocity_y = 0;
        let row = feet - HANG_HEAD + HANG_WALL_DROP;
        let clear = |px: i32| terrain.material_at(px, row).for_player() == Material::EMPTY;
        let ahead = x + direction * HANG_WALL_AHEAD;
        self.hang_stop = direction == 0 || !clear(ahead);
        if self.hang_stop || cursor & 0xff != 0 {
            return 0;
        }
        // $20268..$20284: probe every 8 px beyond; a wall ends the step there.
        let mut step = hang_steps()[usize::from(cursor >> 8) & 7];
        let mut travelled = 0;
        while travelled < step {
            travelled += 8;
            if !clear(ahead + direction * travelled) {
                step = travelled;
            }
        }
        direction * step * UNIT
    }
}
