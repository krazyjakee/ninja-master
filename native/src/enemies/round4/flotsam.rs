//! Type `$3C`, `Enemy3C` (`$168DA`): the floating debris spawner.
use super::{Mines, settle, water_speed};
use crate::{
    combat::{Enemy, Projectile, Spawn},
    enemies::{Behavior, Ctx, HitResponse},
    game::Terrain,
    rom::{Result, pointer, signed, word},
};

const FLOTSAM_INDEX: usize = 0;

/// `$3C`: walks `Data_01B966`, camera X thresholds ending at a negative
/// word; each threshold the camera passes allocates list-4 `$2C` debris
/// (mapping `$1DA`) at camera + ($1C0, $C0), which floats left with the
/// water. Freed at the end of the table.
pub(crate) struct Flotsam {
    thresholds: Vec<i32>,
    offset: (i32, i32),
    mapping: u16,
    /// `$16950`: the debris's whole `obj_tile` (high priority, base 0).
    tile: u16,
    mines: Mines,
}
impl Flotsam {
    pub(super) fn load(rom: &[u8], mines: Mines) -> Result<Self> {
        // $168E2: lea Data_01B966,a1.
        let table = pointer(rom, 0x168e4)?;
        let mut thresholds = Vec::new();
        for i in 0..64 {
            let value = signed(rom, table + i * 2)?;
            if value < 0 {
                break;
            }
            thresholds.push(value);
        }
        Ok(Self {
            thresholds,
            offset: (signed(rom, 0x1692a)?, signed(rom, 0x16936)?),
            mapping: word(rom, 0x1694c)?,
            tile: word(rom, 0x16952)?,
            mines,
        })
    }
    /// `Func_016914`, followed by the debris's first `Func_01776E` call.
    fn launch(&self, cx: &mut Ctx) {
        let (x, y) = (cx.camera.0 + self.offset.0, cx.camera.1 + self.offset.1);
        let mut debris = Projectile::new(x, y, 0, true, self.mapping, self.mines.damage);
        debris.passes_terrain = true;
        // Func_01776E: freed left of the camera, not by the ±256 px test
        // (it starts 448 px right of the camera).
        debris.left_edge_only = true;
        debris.contact_blast = Some(cx.shared.wide_blast);
        debris.owner = Some(cx.current);
        let landed = y >= self.mines.line;
        if landed {
            // Func_0177B4: mapping + 1, water flow and a wake splash.
            debris.mapping += 1;
            debris.velocity_x = -water_speed();
        } else {
            debris.motion.vx = i32::from(cx.random() & 0x7fff);
        }
        // $16950 `move.w #$8000,8(a6)`: on the surfer's priority layer.
        if cx.shoot_tiled(debris, self.tile) && landed {
            self.mines.splash(cx, (x, y));
        }
    }
}
impl Behavior for Flotsam {
    fn label(&self, _kind: u16) -> String {
        "Enemy3C $168DA".into()
    }
    fn hit(&self, _enemy: &Enemy, _kind: u16) -> HitResponse {
        HitResponse::Pass
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        settle(cx, &self.mines);
        if e.routine == 0 {
            // $168E2
            e.vars[FLOTSAM_INDEX] = 0;
            e.routine = 4;
        }
        // $168F0: one threshold per tick.
        let Some(&threshold) = self.thresholds.get(e.vars[FLOTSAM_INDEX] as usize) else {
            Ctx::free(e);
            return;
        };
        if threshold >= cx.camera.0 {
            return;
        }
        self.launch(cx);
        e.vars[FLOTSAM_INDEX] += 1;
    }
}
