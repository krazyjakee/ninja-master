//! Karura's list-1 death sparks, shed feathers and falling-body trail.
use crate::{
    combat::{Effect, EffectKind},
    enemies::ctx::next_random,
    game::Terrain,
    rom::{Result, byte, word, words},
};

#[derive(Clone, Debug)]
pub(crate) struct Rules {
    pub(crate) spark_angles: [u8; 7],
    pub(crate) feather_angles: [u8; 16],
    spark_frames: [u8; 64],
    feather_frames: [u8; 32],
    feather_tiles: [u8; 4],
    headings: [u8; 64],
    trail_frames: [u16; 24],
    pub(crate) spark_gravity: i32,
    pub(crate) feather_gravity: i32,
}

impl Rules {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        fn bytes<const N: usize>(rom: &[u8], address: usize) -> Result<[u8; N]> {
            let mut out = [0; N];
            for (i, value) in out.iter_mut().enumerate() {
                *value = byte(rom, address + i)?;
            }
            Ok(out)
        }
        Ok(Self {
            spark_angles: bytes(rom, 0x379f4)?,
            feather_angles: bytes(rom, 0x3881c)?,
            spark_frames: bytes(rom, 0x379fc)?,
            feather_frames: bytes(rom, 0x3882c)?,
            feather_tiles: bytes(rom, 0x3884c)?,
            headings: bytes(rom, 0x38ba0)?,
            trail_frames: words(rom, 0x37e18)?,
            spark_gravity: i32::from(word(rom, 0x37972)?),
            feather_gravity: i32::from(word(rom, 0x384ac)?),
        })
    }

    /// `$38AFC`: direction of the long velocity in 128 angle units.
    fn heading(&self, effect: &Effect) -> usize {
        let vx = effect.motion.vx >> 8;
        let vy = effect.motion.vy >> 8;
        let (x, y) = (vx.abs(), vy.abs());
        let angle = if x == 0 {
            32
        } else if y == 0 {
            0
        } else {
            let ratio = y * 16 / x;
            if ratio < 64 {
                i32::from(self.headings[ratio as usize])
            } else {
                27 + [81, 108, 163, 256]
                    .into_iter()
                    .filter(|&limit| ratio >= limit)
                    .count() as i32
            }
        };
        let angle = match (vx < 0, vy < 0) {
            (false, false) => 128 - angle,
            (true, false) => 64 + angle,
            (false, true) => angle,
            (true, true) => 64 - angle,
        };
        angle.rem_euclid(128) as usize
    }

    pub(crate) fn update(
        &self,
        effect: &mut Effect,
        terrain: &dyn Terrain,
        random: &mut u16,
    ) -> Option<u16> {
        match effect.kind {
            EffectKind::KaruraSpark => {
                let delta = *self.spark_frames.get(usize::from(effect.age - 1))?;
                if delta == 0 {
                    effect.motion.stop();
                    Some(0)
                } else {
                    Some(0x348 + u16::from(delta))
                }
            }
            EffectKind::KaruraTrail => {
                // Both variants free on their twelfth update, before drawing.
                let tick = usize::from(effect.age - 1);
                (tick < 11).then(|| self.trail_frames[tick + usize::from(effect.phase) * 12])
            }
            EffectKind::KaruraFeather => {
                if effect.phase == 2 {
                    effect.counter -= 1;
                    return (effect.counter > 0).then_some(if effect.counter & 1 == 0 {
                        0x28a
                    } else {
                        0
                    });
                }
                if effect.phase == 0 && effect.motion.vy >= 4 << 16 {
                    // $38512: fall at 2 px/tick and reverse the horizontal
                    // acceleration, producing the feather's flutter.
                    effect.motion.vy = (2 << 16) | (effect.motion.vy & 0xffff);
                    effect.motion.gravity = 0;
                    effect.motion.ax = if effect.motion.vx < 0 {
                        0x4000
                    } else {
                        -0x4000
                    };
                    effect.counter = 4 + i32::from(next_random(random) % 3);
                    effect.phase = 1;
                }
                if effect.phase == 1
                    && effect.motion.vx.abs() >= effect.counter << 16
                    && effect.motion.vx.signum() == effect.motion.ax.signum()
                {
                    effect.motion.ax = -effect.motion.ax;
                    effect.motion.vy = -1 << 16;
                } else if effect.phase == 1 && effect.motion.vy < 2 << 16 {
                    effect.motion.gravity = 0x1000;
                }
                let x = effect.x + ((effect.motion.vx + effect.motion.ax) >> 16);
                let y = effect.y + ((effect.motion.vy + effect.motion.gravity) >> 16);
                if effect.motion.vy >= 0 && terrain.object_solid_at(x, y, false) {
                    effect.phase = 2;
                    effect.counter = 50;
                    effect.motion.stop();
                }
                let angle = self.heading(effect);
                let frame = 0x277 + u16::from(self.feather_frames[angle / 4]);
                effect.base =
                    [0x2000, 0x3000, 0x2800, 0x3800][usize::from(self.feather_tiles[angle / 32])];
                Some(frame)
            }
            _ => unreachable!("Karura particle update received another kind"),
        }
    }
}
