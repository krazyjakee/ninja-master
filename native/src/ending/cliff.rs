//! Ending_S00..S14: original cliff planes, falling wing and flash.
use super::{MASK_SPRITES, sprite_masks};
use crate::{
    front_vdp::{FrontVdp, art, palette_set},
    render::fade_step,
    rom::{Result, SpritePiece, sprite_mapping},
    vdp::Vdp,
};

pub(super) const WING_START: u32 = 334;
const FLASH_START: u32 = WING_START + 448 + 121;
const RESTORE_START: u32 = FLASH_START + 28 + 121;
pub(super) const TICKS: u32 = RESTORE_START + 56;

pub(super) struct Cliff {
    pub vdp: Vdp,
    palette: [u16; 64],
    wing: Vec<SpritePiece>,
    sparks: Vec<Vec<SpritePiece>>,
    masks: [[u16; 4]; MASK_SPRITES],
}
impl Cliff {
    pub fn load(rom: &[u8]) -> Result<Self> {
        let mut front = FrontVdp::new(0);
        front.lz_tiles(rom, art(rom, 0x28, 0)?)?;
        front.lz_byte_map(rom, art(rom, 0x28, 8)?)?;
        front.lz_byte_map(rom, art(rom, 0x28, 12)?)?;
        let palette = palette_set(rom, 0x43)?;
        front.vdp.cram = palette;
        let mut cliff = Self {
            vdp: front.vdp,
            palette,
            wing: sprite_mapping(rom, 0x3a8)?.1,
            sparks: vec![sprite_mapping(rom, 0x392)?.1, sprite_mapping(rom, 0x3a9)?.1],
            masks: sprite_masks(rom)?,
        };
        cliff.tick(0);
        Ok(cliff)
    }
    pub fn tick(&mut self, age: u32) {
        let pan = if age < 78 {
            -80
        } else {
            (-80 + ((age - 78) * 5 / 16) as i32).min(0)
        };
        for row in &mut self.vdp.hscroll {
            row[0] = pan as i16;
        }
        if age.is_multiple_of(4) {
            self.palette[40..45].rotate_left(1); // $10A60
        }
        let target = if (FLASH_START..RESTORE_START).contains(&age) {
            std::array::from_fn(|i| if i < 32 { 0 } else { 0xeee })
        } else {
            self.palette
        };
        let fading = (33..47).contains(&age)
            || (FLASH_START..FLASH_START + 28).contains(&age)
            || (RESTORE_START..TICKS).contains(&age);
        let period = if age >= RESTORE_START {
            8
        } else if age >= FLASH_START {
            4
        } else {
            2
        };
        if fading && age.is_multiple_of(period) {
            for (color, target) in self.vdp.cram.iter_mut().zip(target) {
                *color = fade_step(*color, target);
            }
        } else if age < 33 {
            self.vdp.cram.fill(0);
        } else if !fading {
            self.vdp.cram = target;
        }
        self.vdp.sprites.fill([0; 4]);
        self.vdp.sprites[..MASK_SPRITES].copy_from_slice(&self.masks);
        if age >= RESTORE_START {
            // $10882 / $10A4E: sparks keep rising from the ground during
            // relighting and the epilogue, after the wing record is freed.
            let mut slot = MASK_SPRITES;
            for back in (0..60u32).step_by(2) {
                if back > age - RESTORE_START {
                    continue;
                }
                let t = age - RESTORE_START - back;
                let i = ((back >> 2) & 7) as usize;
                let attr = [
                    0x40fd, 0x40ff, 0x40fe, 0x40ff, 0x610a, 0x690a, 0x60cb, 0x60cf,
                ][i];
                let map = usize::from(i >= 6);
                put(
                    &mut self.vdp,
                    &mut slot,
                    &self.sparks[map],
                    208 + (t * 13 % 31) as i32 - (back / 8) as i32,
                    144 + (t * 7 % 15) as i32 - (back * back / 128) as i32,
                    attr,
                );
            }
            if slot > MASK_SPRITES {
                self.vdp.sprites[slot - 1][1] &= !0x7f;
            }
            return;
        }
        if age < WING_START {
            return;
        }
        let flight = age - WING_START;
        let x = 144 + (flight / 8) as i32;
        let y = (flight / 4) as i32;
        let mut slot = MASK_SPRITES;
        put(&mut self.vdp, &mut slot, &self.wing, x, y, 0x6000);
        // $108CA: sixty-frame sparks trail the wing; $10A4E adds ground
        // sparks during the final relighting. Deterministic positions keep
        // the native scene reproducible without consuming gameplay RNG.
        for back in (0..60u32).step_by(2) {
            if back > flight {
                continue;
            }
            let t = flight - back;
            let frame = ((back >> 2) & 3) as usize;
            let attr = [0x40fd, 0x40ff, 0x40fe, 0x40ff][frame];
            put(
                &mut self.vdp,
                &mut slot,
                &self.sparks[0],
                144 + (t / 8) as i32 + ((t * 13 % 31) as i32 - 15) - (back / 4) as i32,
                (t / 4) as i32 - (back / 2) as i32,
                attr,
            );
        }
        if slot > MASK_SPRITES {
            self.vdp.sprites[slot - 1][1] &= !0x7f;
        }
    }
}
fn put(vdp: &mut Vdp, slot: &mut usize, pieces: &[SpritePiece], x: i32, y: i32, base: u16) {
    for p in pieces {
        if *slot >= vdp.sprites.len() {
            return;
        }
        vdp.sprites[*slot] = [
            ((y + p.y + 128) & 511) as u16,
            (((p.width - 1) << 10) | ((p.height - 1) << 8) | (*slot + 1)) as u16,
            p.attr.wrapping_add(base),
            ((x + p.x + 128) & 511) as u16,
        ];
        *slot += 1;
    }
}
