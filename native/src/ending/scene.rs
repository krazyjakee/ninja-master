//! Staff-roll planes, rider and raster motion from `$1061C..$10E70`.
use super::{MASK_SPRITES, sprite_masks};
use crate::{
    front_vdp::{FONT_ART, FrontVdp, art, palette_set, text_cells},
    render::fade_step,
    rom::{Result, Sine, SpritePiece, sprite_mapping, words},
    vdp::Vdp,
};

pub(super) struct Scene {
    pub vdp: Vdp,
    lines: Vec<Vec<u16>>,
    written: usize,
    rider: Vec<Vec<SpritePiece>>,
    sine: Sine,
    age: u32,
    wind: i32,
    sunset: [u16; 16],
    masks: [[u16; 4]; MASK_SPRITES],
}

impl Scene {
    pub fn load(rom: &[u8], roll: &[String]) -> Result<Self> {
        let mut front = FrontVdp::new(0);
        front.lz_tiles(rom, art(rom, 0x28, 4)?)?;
        front.one_bpp_tiles(rom, FONT_ART, 0, 1)?;
        front.lz_byte_map(rom, art(rom, 0x28, 0x14)?)?;
        let mut vdp = front.vdp;
        vdp.cram = palette_set(rom, 0x47)?;
        for col in 0..64 {
            let tile = u16::from(rom[0x10dd0 + ((63 - col) & 7)]);
            vdp.write_word(0xea00 + col * 2, 0x2140 + tile);
            vdp.write_word(0xea80 + col * 2, 0x21f2);
            vdp.write_word(0xeb00 + col * 2, 0x21f3);
        }
        for (line, scroll) in vdp.hscroll.iter_mut().enumerate() {
            scroll[0] = if (33..190).contains(&line) { 0 } else { 256 };
        }
        Ok(Self {
            vdp,
            lines: roll
                .iter()
                .map(|s| text_cells(rom, s.as_bytes()))
                .collect::<Result<_>>()?,
            written: 0,
            rider: (0x3aa..=0x3ad)
                .map(|id| Ok(sprite_mapping(rom, id)?.1))
                .collect::<Result<_>>()?,
            sine: Sine::load(rom)?,
            age: 0,
            wind: 0,
            sunset: words(rom, 0x42b42)?,
            masks: sprite_masks(rom)?,
        })
    }

    pub fn tick(&mut self, shown: usize, scrolled: u32, sunset: bool, rider_x: i32) {
        self.age = self.age.wrapping_add(1);
        for line in self.written..shown {
            let address = 0xc014 + ((line * 256) & 0x1fff);
            // The ROM clears three following rows so old text cannot wrap back in.
            for row in 0..4 {
                for col in 0..19 {
                    let tile = if row == 0 { self.lines[line][col] } else { 0 };
                    self.vdp.write_word(
                        0xc000 + ((address - 0xc000 + row * 128 + col * 2) & 0x1fff),
                        0x8000 | tile,
                    );
                }
            }
        }
        self.written = shown;
        self.vdp.vscroll[0] = (304 + scrolled) as i16;
        for line in 31..159 {
            self.vdp.hscroll[line][1] = self.vdp.hscroll[line + 1][1];
        }
        self.wind = self
            .wind
            .wrapping_add(self.sine.polar((self.age as u16).wrapping_mul(0x240)).0);
        self.vdp.hscroll[159][1] = ((self.wind >> 16) - 16) as i16;
        for line in 160..169 {
            self.vdp.hscroll[line][1] =
                ((-(self.age as i32) * if line < 165 { 3 } else { 5 }) >> 1) as i16;
        }
        if sunset && self.age.is_multiple_of(4) {
            for (color, target) in self.vdp.cram[16..32].iter_mut().zip(self.sunset) {
                *color = fade_step(*color, target);
            }
        }
        self.vdp.sprites.fill([0; 4]);
        self.vdp.sprites[..MASK_SPRITES].copy_from_slice(&self.masks);
        let pieces = &self.rider[((self.age >> 3) & 3) as usize];
        for (i, p) in pieces.iter().enumerate() {
            self.vdp.sprites[MASK_SPRITES + i] = [
                ((0xa2 + p.y + 128) & 511) as u16,
                (((p.width - 1) << 10)
                    | ((p.height - 1) << 8)
                    | if i + 1 < pieces.len() {
                        MASK_SPRITES + i + 1
                    } else {
                        0
                    }) as u16,
                p.attr.wrapping_add(0x2000),
                ((rider_x + p.x + 128) & 511) as u16,
            ];
        }
    }
}
