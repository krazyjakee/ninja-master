//! ROM tables and readers shared by every enemy type.
use super::Ctx;
use crate::{
    combat::{Blast, Effect, EffectKind, Enemy, Launch, Motion},
    rom::{Result, Sine, byte, long, pointer, sequence, signed, signed_byte, word, word_i, words},
    sound_events::Cues,
    sound_events::Table,
};

/// A `Func_015054` list at `p`: (metatile, dx, dy) words up to a
/// negative metatile word.
pub(crate) fn metatile_list(rom: &[u8], mut p: usize) -> Result<Vec<(u8, i32, i32)>> {
    let mut list = Vec::new();
    while word(rom, p)? & 0x8000 == 0 {
        list.push((
            word(rom, p)? as u8,
            signed(rom, p + 2)?,
            signed(rom, p + 4)?,
        ));
        p += 6;
    }
    Ok(list)
}

/// `Object_HandlerTable` and `Object_StatsTable` entries of one type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TypeStats {
    pub(crate) handler: usize,
    pub(crate) score_bcd: u16,
    pub(crate) contact_damage: u8,
    pub(crate) health_counter: u8,
    pub(crate) placeholder_mapping: u16,
}

/// ROM tables shared by every type: the effect list (`$16F0`, dispatch
/// `$365A2`), the explosion and debris bursts, the shared death launch and
/// the stats/handler tables.
#[derive(Clone, Debug)]
pub(crate) struct Shared {
    pub(crate) sine: Sine,
    /// `Data_00A148`: the tangent table searched by `Func_0094D2`.
    tangents: [u16; 64],
    /// `$15CB2`: the type-$24 head (`$16032`): mapping, dx, dy, vy, gravity, bounces.
    pub(crate) head: [i32; 6],
    pub(crate) head_flips: [u16; 4],
    pub(crate) debris_angles: [u16; 4],
    pub(crate) debris_seq: Vec<u16>,
    pub(crate) debris_gravity: i32,
    pub(crate) sparkle_angles: [u16; 3],
    pub(crate) sparkle_seq: Vec<u16>,
    pub(crate) sparkle_gravity: i32,
    pub(crate) debris_sound: u8,
    /// `$12E14`: shared hit/death arc (type $44 and ROM-style types).
    pub(crate) fall: Launch,
    pub(crate) blasts: [Blast; 2],
    /// `$36D9A`: the wider explosion (d7 = 3) of the `$0C` riders.
    pub(crate) wide_blast: Blast,
    /// `Data_0303DE`: muzzle-flash frame list (last counter value, mapping).
    pub(crate) muzzle: Vec<(u8, u16)>,
    /// `$30500`: last counter, mapping and tile-base attribute.
    pub(crate) mech_smoke: Vec<(u16, u16, u16)>,
    /// `Data_03051A`: `Func_031A7A`'s (last counter, mapping) animation.
    pub(crate) charge_sparkle: Vec<(u8, u16)>,
    /// `Data_031AD6`: (random-angle mask, addition), indexed by D1 / 4.
    pub(crate) charge_sparkle_angles: [(u16, u16); 5],
    /// `Data_031AEA`'s VDP base for the list-1 `$1B` sparkle.
    pub(crate) charge_sparkle_base: u16,
    /// List-1 shell casings: type 2 (`$3699C`) and type `$1C` (`$36D04`).
    pub(crate) casings: [Casing; 2],
    pub(crate) puffs: u16,
    pub(crate) puff_seq: Vec<u16>,
    pub(crate) puff_gravity: i32,
    pub(crate) puff_angle: u16,
    pub(crate) puff_spread: u16,
    /// `Data_036878`: list-9 type-5 dust spawned by `Player_SpawnDust`.
    pub(crate) dust_seq: Vec<u16>,
    pub(crate) hit_seq: Vec<u16>,
    pub(crate) kill_seq: Vec<u16>,
    /// `Data_036834`: the deflect spark (list-1 type 1, `$36948`).
    pub(crate) deflect_seq: Vec<u16>,
    pub(crate) leaf_mapping: u16,
    pub(crate) leaf_base: u16,
    pub(crate) leaf_wiggle: [i32; 4],
    pub(crate) leaf_gravity: i32,
    pub(crate) leaf_sway: u16,
    pub(crate) poof_seq: Vec<u16>,
    pub(crate) poof_angles: [u16; 8],
    pub(crate) deflect_sound: u8,
    /// `$1BF26`: the hit-flash color written over palette line 1 colors 0..7.
    pub(crate) flash_color: u16,
    /// `Data_01B9E0`: palette-line-1 colors 0..7 selected by the samurai's
    /// `obj_health` counter (`Func_01B9F8`). Native health is that counter + 1.
    pub(crate) samurai_health_palettes: [[u16; 8]; 6],
    /// `Func_01BEF6`: the `Data_043012` words a starting `$B4` copies over
    /// palette-line-1 colors 1..8. The health rows then replace 0..7,
    /// leaving color 8 behind.
    pub(crate) samurai_entry: [u16; 8],
    /// `Func_01BEDA`: the `Data_042FF2` words a dying samurai copies over
    /// colors 1..8 once its white flash ends (`$1BFB4`).
    pub(crate) samurai_death: [u16; 8],
    /// `Func_036FF2`: the small explosion of [`Ctx::small_blast`].
    pub(crate) small_blast: SmallBlast,
    /// Sound commands of the events this module requests.
    pub(crate) cues: Cues,
    /// `$5B56 + id * 16`: command byte of each effect ID 0..64.
    pub(crate) effects: crate::sound_events::Effects,
    /// `$12E9A`/`$68A8` entries of the 64 type offsets.
    pub(crate) stats: Vec<TypeStats>,
    /// The ambient particles and the water splash (`crate::combat::ambient`).
    pub(crate) ambient: crate::combat::ambient::Rules,
    /// The 1-1 leader's rock wall and gate (`super::forest::props`).
    pub(crate) leader_props: super::forest::props::Props,
    pub(crate) karura: super::round6::KaruraEffects,
}
impl Shared {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        let blast = |d4: usize, d7: usize, d6: usize| -> Result<Blast> {
            Ok(Blast {
                y_shift: u32::from(byte(rom, d4)?),
                x_shift: u32::from(byte(rom, d7)?),
                sound: Table::Effect.command_at(rom, d6)?,
            })
        };
        let wiggle = |i: usize| signed_byte(rom, 0x151e2 + i);
        let effects = crate::sound_events::Effects::load(rom)?;
        let mut stats = Vec::with_capacity(64);
        for index in 0..64usize {
            let kind = index * 4;
            let handler = pointer(rom, 0x12e9a + kind)?;
            let damage_health = word(rom, 0x68aa + kind)?;
            stats.push(TypeStats {
                handler,
                score_bcd: word(rom, 0x68a8 + kind)?,
                contact_damage: (damage_health >> 8) as u8,
                health_counter: damage_health as u8,
                placeholder_mapping: crate::combat::placeholder_mapping(rom, handler),
            });
        }
        let mut samurai_health_palettes = [[0; 8]; 6];
        for (health, palette) in samurai_health_palettes.iter_mut().enumerate() {
            let source = pointer(rom, 0x1b9e0 + health * 4)?;
            *palette = words(rom, source)?;
        }
        Ok(Self {
            sine: Sine::load(rom)?,
            tangents: words(rom, 0xa148)?,
            head: [
                i32::from(word(rom, 0x15cb2)?),
                i32::from(word(rom, 0x15cb4)? as i16),
                i32::from(word(rom, 0x15cb6)? as i16),
                i32::from(word(rom, 0x15cb8)? as i16) << 16,
                i32::from(word(rom, 0x15cba)?),
                i32::from(word(rom, 0x15cbc)? as i16),
            ],
            head_flips: words(rom, 0x15cbe)?,
            debris_angles: words(rom, 0x36f3e)?,
            debris_seq: sequence(rom, 0x36f46)?,
            debris_gravity: i32::from(word(rom, 0x36f78)?),
            sparkle_angles: words(rom, 0x36a16)?,
            sparkle_seq: sequence(rom, 0x36a1e)?,
            sparkle_gravity: i32::from(word(rom, 0x36a54)?),
            debris_sound: Table::Effect.command_at(rom, 0x36f37)?,
            fall: Launch::read(rom, 0x12e1a, 0x12e22, 0x12e28)?,
            // $36DAE (ordinary) and $36D90 (the $B8 finale): moveq d4/d7/d6.
            blasts: [
                blast(0x36daf, 0x36db1, 0x36db5)?,
                blast(0x36d91, 0x36d93, 0x36d97)?,
            ],
            wide_blast: blast(0x36d9b, 0x36d9d, 0x36da1)?,
            casings: [
                Casing::read(rom, 0x369c8, 0x369e0, 0x369ec, 0x3699a, 0x36a00)?,
                Casing::read(rom, 0x36d2e, 0x36d46, 0x36d52, 0x36d02, 0x36d66)?,
            ],
            muzzle: {
                let mut frames = Vec::new();
                for i in 0..8 {
                    let last = word(rom, 0x303de + i * 4)?;
                    if last & 0x8000 != 0 {
                        break;
                    }
                    frames.push((last as u8, word(rom, 0x303e0 + i * 4)?));
                }
                frames
            },
            mech_smoke: (0..4)
                .map(|i| {
                    Ok((
                        word(rom, 0x30500 + i * 6)?,
                        word(rom, 0x30502 + i * 6)?,
                        word(rom, 0x30504 + i * 6)?,
                    ))
                })
                .collect::<Result<_>>()?,
            charge_sparkle: {
                let mut frames = Vec::new();
                for i in 0..8 {
                    let last = word(rom, 0x3051a + i * 4)?;
                    if last & 0x8000 != 0 {
                        break;
                    }
                    frames.push((last as u8, word(rom, 0x3051c + i * 4)?));
                }
                frames
            },
            charge_sparkle_angles: [
                (word(rom, 0x31ad6)?, word(rom, 0x31ad8)?),
                (word(rom, 0x31ada)?, word(rom, 0x31adc)?),
                (word(rom, 0x31ade)?, word(rom, 0x31ae0)?),
                (word(rom, 0x31ae2)?, word(rom, 0x31ae4)?),
                (word(rom, 0x31ae6)?, word(rom, 0x31ae8)?),
            ],
            charge_sparkle_base: word(rom, 0x31aee)?,
            puffs: word(rom, 0x36e9a)?,
            puff_seq: sequence(rom, 0x36d7c)?,
            puff_gravity: i32::from(word(rom, 0x36e78)?),
            puff_angle: word(rom, 0x36e58)?,
            puff_spread: word(rom, 0x36e46)?,
            dust_seq: sequence(rom, 0x36878)?,
            hit_seq: sequence(rom, 0x36856)?,
            kill_seq: sequence(rom, 0x368ac)?,
            deflect_seq: sequence(rom, 0x36834)?,
            leaf_mapping: word(rom, 0x1519e)?,
            leaf_base: word(rom, 0x151a4)?,
            leaf_wiggle: [wiggle(0)?, wiggle(1)?, wiggle(2)?, wiggle(3)?],
            leaf_gravity: i32::from(word(rom, 0x1522e)?),
            leaf_sway: word(rom, 0x15234)?,
            poof_seq: sequence(rom, 0x36784)?,
            poof_angles: words(rom, 0x36774)?,
            deflect_sound: Table::Effect.command_at(rom, 0x368ed)?,
            flash_color: word(rom, 0x1bf26)?,
            samurai_health_palettes,
            samurai_entry: words(rom, 0x43012)?,
            samurai_death: words(rom, 0x42ff2)?,
            small_blast: SmallBlast::load(rom)?,
            cues: Cues::load(rom)?,
            effects,
            stats,
            ambient: crate::combat::ambient::Rules::load(rom)?,
            leader_props: super::forest::props::Props::load(rom)?,
            karura: super::round6::KaruraEffects::load(rom)?,
        })
    }
    /// Every mapping the shared effects can select.
    /// `Func_0094D2`: the 256-step angle from `from` to `to` (0 = right, `$40` =
    /// down), by binary search of the `Data_00A148` tangent table.
    pub(crate) fn angle_to(&self, from: (i32, i32), to: (i32, i32)) -> u8 {
        let dx = to.0 - from.0;
        let dy = to.1 - from.1;
        let (neg_x, neg_y) = (dx < 0, dy < 0);
        let dx = dx.abs();
        let mut angle: u8 = 0;
        if dy != 0 {
            angle = 0x40;
            if dx != 0 {
                let dy = dy.abs();
                let quotient = (dy << 8) / dx;
                // divu overflow leaves the dividend (dy << 8) in the register.
                let ratio = if quotient > 0xffff {
                    ((dy << 8) & 0xffff) as u16
                } else {
                    quotient as u16
                };
                let (mut lo, mut hi) = (0usize, 63usize);
                loop {
                    let mid = (lo + hi) >> 1;
                    if ratio >= self.tangents[mid] {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                    if hi - lo == 1 {
                        break;
                    }
                }
                angle = lo as u8;
            }
            if neg_x != neg_y {
                angle = angle.wrapping_neg();
            }
        }
        if neg_x {
            angle = angle.wrapping_add(0x80);
        }
        angle
    }
}

/// A list-1 shell casing: ejected at 0.5 px/tick opposite to the facing,
/// -2 px/tick up, falling for `life` ticks (`$2A` countdown, `$36A08`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Casing {
    pub(crate) vx: i32,
    pub(crate) vy: i32,
    pub(crate) gravity: i32,
    pub(crate) mapping: u16,
    pub(crate) life: i32,
}
impl Casing {
    /// Operands of `move.w #vx,d0` / `move.w #vy,d0` (both `<< 4`),
    /// the gravity and `$2A` immediates and the mapping word.
    fn read(
        rom: &[u8],
        vx: usize,
        vy: usize,
        gravity: usize,
        mapping: usize,
        life: usize,
    ) -> Result<Self> {
        Ok(Self {
            vx: i32::from(word(rom, vx)? as i16) << 4,
            vy: i32::from(word(rom, vy)? as i16) << 4,
            gravity: i32::from(word(rom, gravity)? as i16),
            mapping: word(rom, mapping)?,
            life: i32::from(word(rom, life)?),
        })
    }
}

/// `Func_01D480` offset table: (x, y) byte pairs for the unflipped and the
/// flipped parent (`obj_tile` $800), then the extra Y for parent
/// `obj_flags` bit 1 clear/set (a crouching posture).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Offsets {
    pub(crate) pairs: [(i32, i32); 2],
    pub(crate) extra: [i32; 2],
}
impl Offsets {
    pub(crate) fn read(rom: &[u8], p: usize) -> Result<Self> {
        let b = |i: usize| signed_byte(rom, p + i);
        Ok(Self {
            pairs: [(b(0)?, b(1)?), (b(2)?, b(3)?)],
            extra: [b(4)?, b(5)?],
        })
    }
    /// Position relative to a parent at (x, y).
    pub(crate) fn place(&self, x: i32, y: i32, flipped: bool, posture: bool) -> (i32, i32) {
        let (dx, dy) = self.pairs[usize::from(flipped)];
        (x + dx, y + dy + self.extra[usize::from(posture)])
    }
}

/// `move.w #n,$18(a0)`-style writes: replace the integer word of a 16.16
/// value and keep its fraction.
pub(crate) fn set_whole(value: i32, whole: i32) -> i32 {
    (whole << 16) | (value & 0xffff)
}
/// `move.w d0,$1A(a0)`-style writes: replace the fraction word.
pub(crate) fn set_fraction(value: i32, fraction: u16) -> i32 {
    (value & !0xffff) | i32::from(fraction)
}

/// `$1BA50`-style frame lists: (last frame-counter value, mapping) pairs read
/// as (byte, byte offset into the mapping word table `table`), ending at a
/// negative byte. Used by `$B4`/`$B8` (`$1BA1E`) and `$B0` (`$192BE`).
pub(crate) fn frame_list(rom: &[u8], p: usize, table: usize) -> Result<Vec<(u8, u16)>> {
    let mut out = Vec::new();
    for i in 0..16 {
        let last = byte(rom, p + i * 2)?;
        if last & 0x80 != 0 {
            return Ok(out);
        }
        let offset = usize::from(byte(rom, p + i * 2 + 1)?);
        out.push((last, word(rom, table + offset)?));
    }
    Err("unterminated frame list".into())
}
/// `$1BA58`: the first entry whose last value is not below the counter; the
/// second value reports the exact boundary (`+$29` bit 7).
pub(crate) fn frame_at(frames: &[(u8, u16)], counter: u16) -> Option<(usize, u16, bool)> {
    frames
        .iter()
        .enumerate()
        .find(|(_, (last, _))| u16::from(*last) >= counter)
        .map(|(i, (last, mapping))| (i, *mapping, u16::from(*last) == counter))
}

/// `Func_036FF2`: visible sound `sound` and a chain of `count` puffs (`$30`
/// = 2, 1, 0), each in one of 16 random directions (`angle & mask`) at
/// twice the unit speed. Each puff allocates the next on its first update
/// (`$37052` -> `Func_037006`); list 1 is walked through the live `obj_next`
/// links and allocation appends at the tail, so the whole chain starts in
/// the same pass. The ROM steps `Data_036FE6` one mapping per tick; the
/// shared `Debris` sequence holds the same mappings longer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SmallBlast {
    pub(crate) first: u16,
    pub(crate) sound: u8,
    pub(crate) count: u16,
    pub(crate) mask: u16,
}
impl SmallBlast {
    fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            first: word(rom, 0x36fe6)?,
            sound: byte(rom, 0x36ff3)?,
            count: u16::from(byte(rom, 0x37003)?) + 1,
            mask: word(rom, 0x37066)?,
        })
    }
}

/// List-1 type `$18` wreckage (`Func_01625E`, list `$2A` / 4 of `$16256`;
/// pieces `$162C2`): (mapping, dx, dy, 16.16 horizontal speed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Wreckage {
    pub(crate) pieces: Vec<(u16, i32, i32, i32)>,
    pub(crate) vy: i32,
    pub(crate) gravity: i32,
    pub(crate) bounces: u8,
}
impl Wreckage {
    pub(crate) fn load(rom: &[u8], list: usize) -> Result<Self> {
        let speeds = |param: i32| -> Result<i32> {
            Ok(i32::from(word(rom, 0x162b2 + param as usize * 2)?) << 4)
        };
        let mut pieces = Vec::new();
        let list = pointer(rom, 0x16256 + list * 4)?;
        for i in 0..8 {
            let p = list + i * 8;
            let mapping = word(rom, p)?;
            if mapping & 0x8000 != 0 {
                break;
            }
            pieces.push((
                mapping,
                signed(rom, p + 2)?,
                signed(rom, p + 4)?,
                speeds(word_i(rom, p + 6)?)?,
            ));
        }
        Ok(Self {
            pieces,
            vy: long(rom, 0x162d4)?,
            gravity: word_i(rom, 0x162ce)?,
            bounces: byte(rom, 0x162f7)?,
        })
    }
    /// Thrown up at -4.5 px/tick against the object's motion, bouncing
    /// twice at half speed, in the art of type `kind`.
    pub(crate) fn throw(&self, cx: &mut Ctx, e: &Enemy, kind: u16) {
        let base = cx.base(kind);
        let backward = e.motion.vx < 0;
        for &(mapping, dx, dy, speed) in &self.pieces {
            let mut piece = Effect::new(EffectKind::Pieces, e.x + dx, e.y + dy, mapping);
            piece.base = base;
            piece.flip = e.facing_left;
            piece.motion = Motion {
                vx: if backward { -speed } else { speed },
                vy: self.vy,
                gravity: self.gravity,
                ..Motion::default()
            };
            piece.phase = self.bounces + 1;
            piece.counter = 600;
            cx.effect(piece);
        }
    }
}

/// `$1FBF6 + ID`: contact damage of list-4 projectile type `id` (`$1CD46`).
pub(crate) fn shot_damage(rom: &[u8], id: usize) -> Result<u8> {
    byte(rom, 0x1fbf6 + id)
}
/// `Func_01DD9C`: obj_timer and the motion cleared.
pub(crate) fn stop(e: &mut Enemy) {
    e.timer = 0;
    e.motion.stop();
}
