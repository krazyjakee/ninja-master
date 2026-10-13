//! Type `$C8`, `EnemyC8` (`$19B0A`), with its dart (list-4 `$2D`, `$19B80`).
use crate::{
    combat::{Enemy, Projectile, Spawn},
    enemies::{Behavior, Ctx, HitResponse},
    game::Terrain,
    rom::{Result, byte, word},
};

/// `$2C` countdown to the next dart, `$2E` the dart's wait.
const NEXT: usize = 0;
const WAIT: usize = 1;

/// `$C8`: an invisible launcher. Every 4 * N ticks (N = spawn flags low
/// byte) it places a dart (`$378`) that waits 2 * N ticks and then flies at
/// 8 px/tick, rightward for flag bit 8 (`$2A` byte bit 0) and leftward
/// otherwise (effect `$0A`); damage 1.
pub(crate) struct DartTrap {
    mapping: u16,
    damage: u8,
    speed: i32,
    sound: u8,
}
impl DartTrap {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            mapping: word(rom, 0x19ba4)?,
            // $19BA8: move.w #$101,6(a0) (damage 1, health 1).
            damage: byte(rom, 0x19baa)?,
            // $19BF8: asl.w #3 of +-1 (count in bits 9-11).
            speed: 1 << ((word(rom, 0x19bf8)? >> 9) & 7),
            sound: byte(rom, 0x19bff)?,
        })
    }
    /// `$19B3E`: the next dart, waiting at the launcher without damage.
    fn place(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        let n = i32::from(spawn.flags & 0xff);
        e.vars[WAIT] = n * 2;
        e.vars[NEXT] = n * 4;
        // `$19B80` sets the frame after its first `Object_LoadHitbox`.
        let mut dart = Projectile::new(e.x, e.y, 0, true, self.mapping, 0)
            // $19BAE tests the high byte ($2A), independently of the
            // delay byte ($2B), and sets the arrow's flip while waiting.
            .with_art(0, spawn.flags & 0x0100 != 0)
            .unset_frame();
        dart.owner = Some(cx.current);
        dart.dart = true;
        dart.no_collision = true;
        dart.passes_terrain = true;
        cx.shoot(dart);
    }
    /// `$19BDE`: darts whose wait is over fly.
    fn release(&self, cx: &mut Ctx, e: &Enemy, spawn: &Spawn) {
        let me = cx.current;
        let wait = e.vars[WAIT].max(0) as u16;
        let right = spawn.flags & 0x0100 != 0;
        let mut launched = false;
        for dart in cx.projectiles.iter_mut() {
            if dart.owner == Some(me)
                && dart.mapping == self.mapping
                && dart.velocity_x == 0
                && dart.damage == 0
                && dart.age >= wait
            {
                dart.velocity_x = if right { self.speed } else { -self.speed };
                // $378 faces left; preserve the waiting arrow's flip as
                // its direction becomes available to the sprite renderer.
                dart.faces_left = true;
                dart.damage = self.damage;
                dart.no_collision = false;
                dart.passes_terrain = false;
                launched = true;
            }
        }
        if launched {
            cx.visible_sound(e.x, e.y, self.sound);
        }
    }
}
impl Behavior for DartTrap {
    fn label(&self, _kind: u16) -> String {
        "EnemyC8 $19B0A (+ dart $19B80)".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        self.release(cx, e, spawn);
        match e.routine {
            0 => {
                // $19B22
                e.motion.stop();
                e.mapping = 0;
                e.routine = 1;
                self.place(cx, e, spawn);
                e.vars[NEXT] -= 1;
            }
            _ => {
                // $19B78: routine 1 (dispatched * 4); a dart every 4N - 1.
                e.vars[NEXT] -= 1;
                if e.vars[NEXT] == 0 {
                    self.place(cx, e, spawn);
                    e.vars[NEXT] -= 1;
                }
            }
        }
    }
    fn hit(&self, _enemy: &Enemy, _kind: u16) -> HitResponse {
        HitResponse::Pass
    }
}
