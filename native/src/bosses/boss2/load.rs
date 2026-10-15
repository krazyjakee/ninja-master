//! ROM decoding for the flying machine.
use super::*;

impl Boss2 {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        let table = 0x2be92;
        let mut animations = Vec::with_capacity(13);
        for routine in 0..13 {
            let at = table + usize::from(word(rom, table + routine * 2)?);
            let mut frames = Vec::new();
            for i in 1..16 {
                let frame = byte(rom, at + i)?;
                if frame == 0xff {
                    break;
                }
                frames.push(frame);
            }
            animations.push(Animation {
                speed: i32::from(byte(rom, at)?),
                frames,
            });
        }
        let base = word(rom, 0x2be7e)?;
        let mut flicker = [0; 4];
        for (i, frame) in flicker.iter_mut().enumerate() {
            *frame = base + word(rom, 0x2be8a + i * 2)?;
        }
        let mut orb_vx = [0; 4];
        for (i, vx) in orb_vx.iter_mut().enumerate() {
            *vx = signed(rom, 0x25f20 + i * 2)? << 16;
        }
        let mut burst = Vec::with_capacity(8);
        for i in 0..8 {
            let b = |p: usize| -> Result<i32> { Ok((signed_byte(rom, p)? << 16) >> 2) };
            burst.push((b(0x26068 + i * 2)?, b(0x26069 + i * 2)?));
        }
        let shot_damage = byte(rom, 0x25e62)?;
        // List-4 `$33` (`$25E06`) loads its box before a routine sets the
        // frame, and no spawner sets it: each first box is a point.
        let mut beam = Projectile::new(0, 0, 0, true, word(rom, 0x26114)?, byte(rom, 0x2611a)?)
            .with_motion(still())
            .unset_frame();
        // `move.w #4,$28(a0)` countdown after the set-up frame.
        beam.life = 5;
        beam.passes_terrain = true;
        let mut shot =
            Projectile::new(0, 0, 0, true, word(rom, 0x25e56)?, shot_damage).unset_frame();
        shot.passes_terrain = true;
        // $25E6E: the floor or the player (bit 14) ends it in dust.
        shot.contact_dust = true;
        let mut orb = Projectile::new(0, 0, 0, true, word(rom, 0x25f2a)?, byte(rom, 0x25f36)?)
            .with_motion(Motion {
                vy: signed(rom, 0x25f48)? << 16,
                gravity: word_i(rom, 0x25f5c)?,
                ..Motion::default()
            })
            .unset_frame();
        orb.passes_terrain = true;
        orb.life = u16::MAX;
        orb.stays = true;
        // $2603A: a weapon leaves the list-1 `$2B` puff and effect $1B.
        orb.shot_down = Some(ShotDown {
            blast: None,
            first: word(rom, 0x2623c)?,
            frames: (word(rom, 0x262a8)? - word(rom, 0x2623c)?) as u8,
            period: word(rom, 0x2629e)? + 1,
            sound: Some(Table::Effect.command_at(rom, 0x26057)?),
            smoke: None,
        });
        let effect = |id_at: usize| Table::Effect.command_at(rom, id_at);
        let mut flare_motions = [Motion::default(); 8];
        for (i, motion) in flare_motions.iter_mut().enumerate() {
            let at = 0x263c6 + i * 8;
            *motion = Motion {
                vx: signed(rom, at)? << 16,
                vy: signed(rom, at + 2)? << 16,
                ax: signed(rom, at + 4)?,
                gravity: signed(rom, at + 6)?,
                ..Motion::default()
            };
        }
        let flare_first = word(rom, 0x2635a)?;
        let mut rows = [[0; 7]; 5];
        for (i, row) in rows.iter_mut().enumerate() {
            *row = crate::rom::words(rom, 0x2bdaa + i * 14)?;
        }
        let mut order = [0; 16];
        for (i, offset) in order.iter_mut().enumerate() {
            *offset = byte(rom, 0x2bdf0 + i)? / 14;
        }
        let mut pairs = [[0; 2]; 8];
        for (i, pair) in pairs.iter_mut().enumerate() {
            *pair = crate::rom::words(rom, 0x2be00 + i * 4)?;
        }
        Ok(Self {
            animations,
            flicker,
            base,
            health: word_i(rom, 0x2b2ec)?,
            health_palette: HealthPalette::table(rom, 1, 0x2afa4, 11)?,
            center: (word_i(rom, 0x2b2ce)?, word_i(rom, 0x2b2da)?),
            radius: word_i(rom, 0x2b2e2)?,
            spiral_end: word_i(rom, 0x2b50c)?,
            hover_ticks: word_i(rom, 0x2b64a)?,
            hover_step: word_i(rom, 0x2b652)?,
            hover_speed: word_i(rom, 0x2b5fa)?,
            hover_bounds: (word_i(rom, 0x2b5f4)?, word_i(rom, 0x2b600)?),
            hover_band: (word_i(rom, 0x2b610)?, word_i(rom, 0x2b61a)?),
            bob_amplitude: word_i(rom, 0x2b658)?,
            field: (
                i32::from(byte(rom, 0x2b128)?),
                i32::from(byte(rom, 0x2b129)?),
            ),
            hurt_immunity: word(rom, 0x2b37a)?,
            hit_immunity: word(rom, 0x2b212)?,
            charge_y: word_i(rom, 0x2b6ac)?,
            beam_timing: word_i(rom, 0x2b696)?,
            beam_shots: word_i(rom, 0x2b69c)?,
            beam_drift: word_i(rom, 0x2b75c)?,
            beam,
            beam_y: word_i(rom, 0x26120)?,
            sweep_offset: word_i(rom, 0x2b818)?,
            sweep_height: word_i(rom, 0x2b8dc)?,
            shot,
            shot_speed: (
                i32::from(byte(rom, 0x2b927)?) << 16,
                word_i(rom, 0x2b920)? << 16,
            ),
            shot_offset: (i32::from(byte(rom, 0x2b92b)?), word_i(rom, 0x2b918)?),
            shot_floor: word_i(rom, 0x25e7c)?,
            burst_y: word_i(rom, 0x2b9ba)?,
            burst_ticks: word_i(rom, 0x2b9d6)?,
            rise_y: word_i(rom, 0x2ba3c)?,
            flash_ticks: word_i(rom, 0x2ba74)?,
            orb,
            orb_vx,
            orb_rest: word_i(rom, 0x25fb2)?,
            orb_bob_step: word(rom, 0x25fe6)?,
            orb_spin: (word(rom, 0x25f7c)?, 0x100 / word(rom, 0x25f90)?),
            burst,
            warp_y: word_i(rom, 0x2bbf0)?,
            warp_side: word_i(rom, 0x2bc16)?,
            warp_center: word_i(rom, 0x2bc32)?,
            attacks: [
                (CHARGE, word_i(rom, 0x2bc92)?, word_i(rom, 0x2bc98)?),
                (FLASH_IN, word_i(rom, 0x2bcb8)?, word_i(rom, 0x2bcbe)?),
                (BURST, word_i(rom, 0x2bcce)?, word_i(rom, 0x2bcd4)?),
            ],
            death_fall: (word_i(rom, 0x2b40c)?, signed(rom, 0x2b412)?),
            arena: (
                word_i(rom, 0x2b41c)?,
                word_i(rom, 0x2b424)?,
                word_i(rom, 0x2b438)?,
            ),
            // $2B1CA / $2B1C2: move.b #1 or #2,obj_health(a3).
            contact: [byte(rom, 0x2b1cd)?, byte(rom, 0x2b1c5)?],
            // $2B29A: move.b #$30,Object_ListParams+2 (a dbf count).
            list4: usize::from(byte(rom, 0x2b29d)?) + 1,
            sounds: Sounds {
                entry: effect(0x2b2f5)?,
                hurt: [effect(0x2b33d)?, effect(0x2b341)?],
                // $2B230 moveq #6 / Func_004A36: music 6, FADE OUT.
                death_music: Table::Music.command_at(rom, 0x2b231)?,
                fall: effect(0x2b3eb)?,
                boom: effect(0x2b493)?,
                beam: effect(0x2b74d)?,
                laser: effect(0x26137)?,
                flash: effect(0x2b7c1)?,
                gun: effect(0x2b8fd)?,
                drop: effect(0x2b9df)?,
                charge: effect(0x2bca1)?,
                burst: effect(0x26057)?,
            },
            pieces: Pieces {
                // $2BEE2 moveq #5,d7 / $2BF0E moveq #7,d7: dbf counts.
                debris: (
                    usize::from(byte(rom, 0x2bee3)?) + 1,
                    word(rom, 0x26418)?,
                    word_i(rom, 0x2643a)?,
                    word_i(rom, 0x26440)?,
                ),
                flares: (
                    usize::from(byte(rom, 0x2bf0f)?) + 1,
                    flare_first,
                    (word(rom, 0x263b8)? - flare_first) as u8,
                    flare_motions,
                ),
                delay_mask: word(rom, 0x2bf32)?,
                flare_period: word(rom, 0x2637e)?,
                streak: (
                    word(rom, 0x2bcec)?,
                    word_i(rom, 0x2bcfe)?,
                    word_i(rom, 0x2bbd6)?,
                    word_i(rom, 0x2bc42)?,
                ),
                trail_tile: word(rom, 0x2bfb2)?,
                hover_trail: (word(rom, 0x2b58c)?, word_i(rom, 0x2b592)?),
                burst_trail: (word(rom, 0x2bfac)?, word_i(rom, 0x2bfc4)?),
                orbit: (
                    word(rom, 0x2b6dc)?,
                    word_i(rom, 0x2b6e4)?,
                    word_i(rom, 0x262c8)?,
                    word(rom, 0x262ce)?,
                ),
                casing: (
                    word(rom, 0x26510)?,
                    word_i(rom, 0x26516)?,
                    i32::from(byte(rom, 0x2651f)?),
                    signed(rom, 0x26536)?,
                    word_i(rom, 0x2653e)?,
                ),
                casing_tile: word(rom, 0x2651c)?,
            },
            wave: Wave {
                phase_step: word_i(rom, 0x2b876)?,
                growth: word_i(rom, 0x2b880)?,
                cap: word_i(rom, 0x2b884)?,
            },
            cycle: Cycle { rows, order, pairs },
            white: word(rom, 0x2b77e)?,
            fade_end: word_i(rom, 0x2b7d6)?,
        })
    }
}
