//! ROM decoding for the hydra encounter.
use super::*;

impl Hydra {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut scripts: [Vec<u8>; 4] = Default::default();
        for (i, script) in scripts.iter_mut().enumerate() {
            let mut p = pointer(rom, 0x2c17a + i * 4)?;
            loop {
                let b = byte(rom, p)?;
                script.push(b);
                p += 1;
                if b & 0x80 != 0 || script.len() > 32 {
                    break;
                }
            }
        }
        let words4 = |p: usize| -> Result<[i32; 4]> { Ok(words::<4>(rom, p)?.map(i32::from)) };
        let mut jitter = [(0, 0); 8];
        for (i, j) in jitter.iter_mut().enumerate() {
            *j = (signed(rom, 0x2c43c + i * 2)?, signed(rom, 0x2c44c + i * 2)?);
        }
        let mut spits = [Spit {
            vx: 0,
            vy: 0,
            ax: 0,
            ay: 0,
            dx: 0,
            dy: 0,
        }; 3];
        for (i, s) in spits.iter_mut().enumerate() {
            let p = 0x2cafe + i * 16;
            *s = Spit {
                vx: long(rom, p)?,
                vy: long(rom, p + 4)?,
                ax: signed(rom, p + 8)?,
                ay: signed(rom, p + 10)?,
                dx: signed(rom, p + 12)?,
                dy: signed(rom, p + 14)?,
            };
        }
        let weak = crate::combat::Mapping::load(rom, word(rom, 0x2c13e)?)?.bounds;
        let patch_list = |mut p: usize| -> Result<Vec<u32>> {
            let mut list = Vec::new();
            loop {
                let header = long(rom, p)? as u32;
                if header == 0 {
                    break;
                }
                list.push(header);
                p += 4;
                if list.len() > 16 {
                    return Err("unterminated Hydra patch list".into());
                }
            }
            Ok(list)
        };
        let mut laser_widths = [0; 32];
        for (i, width) in laser_widths.iter_mut().enumerate() {
            *width = byte(rom, 0x2d006 + i)?;
        }
        let flesh = HealthPalette::load(
            rom,
            1,
            (0..16).map(|health| Ok(0x42062 + usize::from(word(rom, 0x2bfca + health * 2)?))),
        )?;
        Ok(Self {
            body: Record::load(rom, 0x2c138)?,
            face: Record::load(rom, 0x2c522)?,
            neck: Record::load(rom, 0x2c530)?,
            chin: Record::load(rom, 0x2c53e)?,
            jaw: Record::load(rom, 0x2c878)?,
            tongue: Record::load(rom, 0x2c63c)?,
            spit_record: Record::load(rom, 0x2ca50)?,
            laser: Record::load(rom, 0x2cf4e)?,
            splash: Record::load(rom, 0x2cb64)?,
            scripts,
            first_script: word(rom, 0x2c150)?,
            start: (long(rom, 0x2c156)?, long(rom, 0x2c15e)?),
            rest_x: long(rom, 0x2c1fa)?,
            dive_dx: signed(rom, 0x2c248)?,
            dive_accel: long(rom, 0x2c276)?,
            floor_y: long(rom, 0x2c27e)?,
            dive_end_x: word_i(rom, 0x2c298)?,
            rise_offset: word_i(rom, 0x2c2b6)?,
            rise_min_x: i32::from(byte(rom, 0x2c2bb)?),
            top_y: long(rom, 0x2c302)?,
            entry_sound: byte(rom, 0x2c251)?,
            hurt_ticks: word_i(rom, 0x2c33e)?,
            damage_voice: Table::Voice.command_at(rom, 0x2c355)?,
            death_drool: word_i(rom, 0x2c396)?,
            death_voice: Table::Voice.command_at(rom, 0x2c3e9)?,
            jitter,
            waits: words4(0x2c480)?,
            inhales: words4(0x2c8e0)?,
            drools: words4(0x2cc94)?,
            spit_sound: byte(rom, 0x2ca5f)?,
            spit_y: word_i(rom, 0x2cac6)?,
            spits,
            hit_step: word(rom, 0x2cddc)?,
            rest_step: word(rom, 0x2d106)?,
            inhale_sound: byte(rom, 0x2c937)?,
            inhale_sound_at: word_i(rom, 0x2c930)?,
            drop_max: word(rom, 0x2c8d0)? as u8,
            recoil: word(rom, 0x2c9ac)? as u8,
            laser_sound: byte(rom, 0x2cf5d)?,
            laser_ticks: word(rom, 0x2cf9e)? as u8,
            laser_start: word(rom, 0x2cfae)? as u8,
            tongue_x: word_i(rom, 0x2c708)?,
            tongue_rise: long(rom, 0x2c854)?,
            tongue_sweep: word(rom, 0x2c6d0)? as u8,
            tongue_step: word_i(rom, 0x2c7a2)?,
            tongue_drool_dx: long(rom, 0x2c678)? >> 16,
            tongue_frame: word(rom, 0x2c71a)?,
            splash_scale: [4, 4, 4, 6, 6, 5, 5, 8],
            splash_small: word(rom, 0x2cd0e)?,
            weak,
            flesh,
            eyes: [
                patch_list(pointer(rom, 0x2ce7e)?)?,
                patch_list(pointer(rom, 0x2ce92)?)?,
            ],
            vents: [
                pointer(rom, 0x2d066)? as u32,
                pointer(rom, 0x2d06a)? as u32,
                pointer(rom, 0x2d06e)? as u32,
            ],
            laser_widths,
            laser_white: words(rom, pointer(rom, 0x2d27a)?)?,
            corridor: words(rom, pointer(rom, 0x2d224)?)?,
        })
    }
}
