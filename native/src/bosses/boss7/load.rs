//! `Boss7::load`: the handler's operands, scripts and tables.
use super::*;

impl Boss7 {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        let w = |p: usize| word_i(rom, p);
        let b = |p: usize| signed_byte(rom, p);
        let base = word(rom, 0x34746)?;
        let mut scripts = Vec::with_capacity(24);
        for i in 0..24 {
            let mut p = crate::rom::pointer(rom, 0x3474e + i * 4)?;
            let mut script = Vec::new();
            for _ in 0..64 {
                let offset = byte(rom, p)?;
                if offset == 0xff {
                    break;
                }
                let ticks = u16::from(byte(rom, p + 1)?).max(1);
                script.push((base + u16::from(offset), ticks));
                p += 2;
            }
            scripts.push(script);
        }
        let voices = (0..32)
            .map(|id| Table::Voice.command(rom, id))
            .collect::<Result<_>>()?;
        let mut rain = Vec::new();
        for i in 0..128 {
            let index = byte(rom, 0x339f2 + i)?;
            if index & 0x80 != 0 {
                break;
            }
            rain.push(usize::from(index));
        }
        let mut stars = [((0, 0), (0, 0)); 3];
        for (i, star) in stars.iter_mut().enumerate() {
            *star = (
                (b(0x1e9f0 + i * 2)?, b(0x1e9f1 + i * 2)?),
                (
                    signed(rom, 0x1e9f6 + i * 4)? << 4,
                    signed(rom, 0x1e9f8 + i * 4)? << 4,
                ),
            );
        }
        let mut fan = [((0, 0), (0, 0)); 8];
        for (i, shot) in fan.iter_mut().enumerate() {
            *shot = (
                (b(0x1ea6e + i * 2)?, b(0x1ea6f + i * 2)?),
                (
                    byte(rom, 0x1ea7e + i * 2)?,
                    u32::from(byte(rom, 0x1ea7f + i * 2)?),
                ),
            );
        }
        let mut pieces = Vec::with_capacity(16);
        for i in 0..16 {
            pieces.push((b(0x1edf8 + i * 2)?, b(0x1edf9 + i * 2)?));
        }
        let mut energy = [(0, 0); 9];
        for (i, shot) in energy.iter_mut().enumerate() {
            *shot = (
                byte(rom, 0x1f194 + i)?,
                base + u16::from(byte(rom, 0x1f19d + i)?),
            );
        }
        let mut aura_offsets = [(0, 0); 4];
        for (i, offset) in aura_offsets.iter_mut().enumerate() {
            *offset = (b(0x1ef4e + i * 2)?, b(0x1ef4f + i * 2)?);
        }
        let damage = |p: usize| -> Result<u8> { shot_damage(rom, usize::from(byte(rom, p)?)) };
        Ok(Self {
            base,
            player_classes: std::array::from_fn(|i| byte(rom, 0x344ec + i).unwrap_or(255)),
            first_palette: crate::rom::words(rom, 0x36582)?,
            second_palette: crate::rom::words(rom, 0x36562)?,
            scripts,
            half: half_turn(rom)?,
            voices,
            first: word(rom, 0x331a2)?,
            arena: w(0x33182)?,
            span: w(0x346a2)?,
            fall_gravity: w(0x33190)?,
            intro_script: w(0x3319c)?,
            health: b(0x331a9)?,
            intro_sound: byte(rom, 0x331fb)?,
            backdrop: byte(rom, 0x34e3)?,
            idle: w(0x33234)?,
            near: w(0x33288)?,
            slash_script: w(0x33294)?,
            throw_script: w(0x332b4)?,
            spin_script: w(0x332d6)?,
            wave_script: w(0x332ec)?,
            run_speed: long(rom, 0x34484)?,
            run_min: w(0x344a2)?,
            run_frame: word(rom, 0x344ac)?,
            after_slash: w(0x333a8)?,
            blade_frame: word(rom, 0x333b6)?,
            step_frame: word(rom, 0x333d4)?,
            step_start: w(0x333dc)?,
            step_vx: long(rom, 0x333e4)?,
            step_stop: w(0x333fc)?,
            throw_frame: word(rom, 0x3340e)?,
            throw_sound: byte(rom, 0x3341d)?,
            throw_count: b(0x33425)?,
            dash_after_throw: w(0x3346c)?,
            jump_script: w(0x3347c)?,
            windup_frame: word(rom, 0x33498)?,
            windup_voice: byte(rom, 0x334ab)?,
            dash_script: w(0x334c4)?,
            dash_vx: long(rom, 0x334ca)?,
            afterimage: word(rom, 0x334e4)?,
            brake_range: w(0x334fe)?,
            brake: w(0x33506)?,
            dash_end: word(rom, 0x33534)?,
            landing_script: w(0x335be)?,
            jump_frame: word(rom, 0x3360e)?,
            jump_vx: long(rom, 0x33628)?,
            margin: w(0x33636)?,
            room: w(0x3363a)?,
            far_side: w(0x33646)?,
            jump_vy: long(rom, 0x33658)?,
            gravity: w(0x33660)?,
            jump_sound: word(rom, 0x33666)? as u8,
            jump2_frame: word(rom, 0x33674)?,
            jump2_vx: long(rom, 0x33684)?,
            jump2_vy: long(rom, 0x33696)?,
            fan_cursor: w(0x336a2)?,
            fan_sound: byte(rom, 0x336b1)?,
            fan_count: b(0x336b9)?,
            uppercut_frame: word(rom, 0x3376c)?,
            uppercut_vy: long(rom, 0x33780)?,
            uppercut_gravity: w(0x33788)?,
            uppercut_range: w(0x34438)?,
            uppercut_script: w(0x34468)?,
            uppercut_vx: long(rom, 0x3446e)?,
            hold_frame: word(rom, 0x337ae)?,
            hold: w(0x337fe)?,
            wave_sound: word(rom, 0x3383e)? as u8,
            wave_end_script: w(0x33864)?,
            wave_hit_sound: word(rom, 0x3386a)? as u8,
            hover_script: w(0x3436a)?,
            leap_script: w(0x3437a)?,
            hover: w(0x338e0)?,
            hover_sound: byte(rom, 0x338ed)?,
            hover_vy: long(rom, 0x3390a)?,
            death_step: w(0x3393c)?,
            death_voice: word(rom, 0x33956)? as u8,
            rain_interval: w(0x33972)?,
            rain_rise: long(rom, 0x33978)?,
            rain_sound: byte(rom, 0x339e9)?,
            rain,
            // (from distance, countdown, vx, vy) of $33AC4..$33B08.
            leap: [
                (
                    w(0x33ac6)?,
                    w(0x33acc)?,
                    long(rom, 0x33ad2)?,
                    long(rom, 0x33ad8)?,
                ),
                (
                    w(0x33ae2)?,
                    w(0x33ae8)?,
                    long(rom, 0x33aee)?,
                    long(rom, 0x33af4)?,
                ),
                (0, w(0x33afe)?, 0, long(rom, 0x33b06)?),
            ],
            dive_script: w(0x33b42)?,
            dive_vx: long(rom, 0x33b5c)?,
            dive_vy: long(rom, 0x33b70)?,
            bounce_script: w(0x33ba6)?,
            bounce_vx: long(rom, 0x33bac)?,
            bounce_vy: long(rom, 0x33bc0)?,
            retreat: w(0x33d08)?,
            mijin_frame: word(rom, 0x33d46)?,
            mijin_step: w(0x33d64)?,
            mijin_sound: word(rom, 0x33d82)? as u8,
            mijin_after: word(rom, 0x33e8e)?,
            aura_sound: word(rom, 0x33eac)? as u8,
            hurt_frame: word(rom, 0x33fd2)?,
            second_health: b(0x34017)?,
            knock_wait: w(0x345f6)?,
            knock_vx: long(rom, 0x345d0)?,
            knock_vy: long(rom, 0x345e4)?,
            death_script: w(0x34044)?,
            fade_step: w(0x34092)?,
            end_sound: byte(rom, 0x340ff)?,
            boom_sound: byte(rom, 0x34127)?,
            combo_window: b(0x3459f)?,
            combo_max: b(0x345b9)?,
            deflect_cooldown: b(0x34389)?,
            deflect_sound: byte(rom, 0x3439f)?,
            broken_script: w(0x342b6)?,
            broken_voice: byte(rom, 0x342bb)?,
            high_script: w(0x342c4)?,
            low_range: w(0x342d2)?,
            low_script: w(0x342d8)?,
            flash: (word(rom, 0x34626)?, word(rom, 0x34632)?),
            // $342F8/$34300: frames that deflect weapons in an attack.
            guard: [word(rom, 0x342fa)?, word(rom, 0x34302)?],
            star_damage: damage(0x33427)?,
            star_frames: std::array::from_fn(|i| {
                std::array::from_fn(|side| base + u16::from(rom[0x1ea02 + i * 2 + side]))
            }),
            fan_frames: std::array::from_fn(|i| {
                std::array::from_fn(|side| base + u16::from(rom[0x1ea8e + i * 2 + side]))
            }),
            stars,
            fan,
            // (x offset, rise, blade script, burst script, damage, sound).
            blade: (
                w(0x1e49e)?,
                w(0x1e4b0)?,
                w(0x1e4bc)?,
                w(0x1e57c)?,
                damage(0x333c5)?,
                word(rom, 0x1e58c)? as u8,
            ),
            pieces,
            piece_damage: damage(0x1e5a9)?,
            energy,
            energy_damage: damage(0x339c5)?,
            energy_rise: w(0x1f1ec)?,
            spinner_script: w(0x1efd0)?,
            spinner_shot: word(rom, 0x1f042)?,
            spinner_sound: word(rom, 0x1f076)? as u8,
            // (rise, script, shift, acceleration, damage) of $1F298.
            wave: (
                w(0x1f2d8)?,
                w(0x1f2e4)?,
                w(0x1f342)?,
                w(0x1f346)?,
                damage(0x3382f)?,
            ),
            wave_frames: [word(rom, 0x1f414)?, word(rom, 0x1f424)?],
            wave_drop: w(0x1f366)?,
            wave_hold: word(rom, 0x338a2)?,
            run_voice: byte(rom, 0x3329d)?,
            spin_voice: byte(rom, 0x337d3)?,
            broken_frame: word(rom, 0x33c7c)?,
            broken_cry: byte(rom, 0x33c8b)?,
            // Variant 1 (bit 0 set / clear), variant 2 (set / clear), variant 3.
            aura_frames: [
                word(rom, 0x1f088)?,
                word(rom, 0x1f098)?,
                word(rom, 0x1f124)?,
                word(rom, 0x1f134)?,
                word(rom, 0x1f18e)?,
            ],
            aura_offsets,
            aura_damage: damage(0x33797)?,
            aura_voices: [word(rom, 0x1f0be)? as u8, word(rom, 0x1f108)? as u8],
            aura_ticks: w(0x1ef02)?,
            aura_frame: word(rom, 0x1ef3a)?,
            pillar_frame: word(rom, 0x37d72)?,
            pillar_spacing: w(0x37d6a)?,
            pillar_velocity: long(rom, 0x37d8e)?,
            dive_sparks: (0..8)
                .map(|i| {
                    let at = 0x36b26 + i * 8;
                    Ok((
                        w(at)?,
                        w(at + 2)?,
                        word(rom, at + 4)?,
                        u32::from(word(rom, at + 6)?),
                    ))
                })
                .collect::<Result<_>>()?,
            dive_spark_script: (0..16)
                .map(|i| word(rom, 0x36b66 + i * 2))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .take_while(|&frame| frame != 0xffff)
                .map(|frame| (frame, 1))
                .collect(),
            boom_script: w(0x38ed0)?,
        })
    }
}
