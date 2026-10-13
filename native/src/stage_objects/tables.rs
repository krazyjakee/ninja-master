//! Handler constants and tables read from the cartridge.
use crate::{
    combat::Hitbox,
    rom::{Result, Sine, byte, long, sequence, signed, signed_byte, word, words},
    sound_events::{SoundEvent, Table},
};

/// A `move.l #imm,+$10(a0)` hitbox: x offset, y offset, half width, half height.
fn hitbox_at(rom: &[u8], at: usize) -> Result<Hitbox> {
    Ok(Hitbox {
        x: signed_byte(rom, at)?,
        y: signed_byte(rom, at + 1)?,
        half_width: signed_byte(rom, at + 2)?,
        half_height: signed_byte(rom, at + 3)?,
    })
}

/// Handler constants and tables, each read from the instruction or table
/// cited (the tests check the opcodes).
#[derive(Clone, Debug)]
pub(super) struct Tables {
    /// `$9748`: the 1024-step sine table of `$94A0`.
    pub(super) sine: Sine,
    /// `$5B56 + id * 16`: effect commands requested through `$4A4E`.
    effects: crate::sound_events::Effects,
    /// `$29708` passes 1 and `$2710E` adds `$EF`: mapping `$F0`.
    pub(super) box_mapping: u16,
    /// `$29706`: `+$28` = 7, so the probe reads material 7 (a one-way floor).
    pub(super) box_material: u8,
    /// `$29796`: the standing slab, (0,-21) half 12x3.
    pub(super) box_stand: Hitbox,
    /// `$29788`: the box body that attacks hit, (0,-12) half 12x12.
    pub(super) box_attack: Hitbox,
    /// `$29758`, `$2975E`, `$2976C`, `$29776`: the step-up rule.
    pub(super) snap_above: i32,
    pub(super) snap_below: i32,
    pub(super) snap_state: usize,
    pub(super) snap_height: i32,
    /// `$297B6`: voice 14 when a sword breaks a box.
    pub(super) box_voice: u8,
    /// `$35D6C`, `$35D7C`, `$35D8C`: mapping, score and effect id by contents.
    pub(super) pickup_mappings: [u16; 8],
    pub(super) pickup_scores: [u16; 8],
    pub(super) pickup_sounds: [u8; 8],
    /// `$35EA2`: ticks a pickup waits before blinking.
    pub(super) pickup_wait: i32,
    /// `$35EC6`, `$36146`: ticks per blink step; `$36154`: steps.
    pub(super) blink_period: i32,
    pub(super) blink_steps: i32,
    /// `$35F02`, `$35F08`: the collected item rises 16 + 1 ticks at -1.5 px.
    pub(super) rise_ticks: i32,
    pub(super) rise_speed: i32,
    /// `$35D26`: the sparkle after collection (0 = blank).
    pub(super) sparkle: Vec<u16>,
    /// `$12570`: the 1UP effect.
    pub(super) one_up: u8,
    /// `$35F72`, `$35F80`: shurikens per item; `$35FA4`: the limit (799).
    pub(super) ammo: [u16; 2],
    pub(super) ammo_limit: u16,
    /// `$35FC2`: health per item.
    pub(super) health: u8,
    /// `$3609E`: the ninjutsu stock limit (99).
    pub(super) ninjutsu_limit: u8,
    /// `$36036`: the bonus streak limit (4).
    pub(super) bonus_limit: u8,
    /// List-1 type 22 (`$36C6E`): mapping and angle by index, the hold,
    /// gravity and the sparkle it ends with.
    pub(super) debris_mappings: [u16; 4],
    pub(super) debris_angles: [u16; 4],
    pub(super) debris_hold: i32,
    pub(super) debris_gravity: i32,
    pub(super) debris_sequence: Vec<u16>,
    /// `$36878`: list-1 type-5 smoke; `$299EC`: six list-4 type-$2F
    /// bursts (X offset, X/Y velocity, repeats, period).
    pub(super) barrel_smoke: Vec<u16>,
    pub(super) barrel_bursts: [[i32; 5]; 6],
    /// List-4 type 30 (`$1D52E`): first mapping, lead-in, digit period,
    /// digit mappings, the period once touched, the fuse and its mapping.
    pub(super) bomb_first: u16,
    pub(super) bomb_lead: i32,
    pub(super) bomb_period: i32,
    pub(super) bomb_digits: [u16; 6],
    pub(super) bomb_touched_period: i32,
    pub(super) bomb_fuse: i32,
    pub(super) bomb_blink: u16,
    pub(super) bomb_sound: u8,
    /// List-4 type 3 (`$1D0EA`, `$1D144`): the three flame angles, their
    /// gravity, mapping sequence and damage (`$1FBF6 + 3`).
    pub(super) flame_angles: [u16; 3],
    pub(super) flame_gravity: i32,
    pub(super) flame_sequence: Vec<u16>,
    pub(super) flame_damage: u8,
    pub(super) flame_drop: i32,
    /// The 5-2 floor fire: the bomb tests 8 px below itself (`$1D65A`
    /// `addq.w #8,d3`), fires wait 5 (`$1D6C2`) or 4 (`$375BC`) ticks,
    /// step 16 px (`$1D680`, `$1D68C`, `-16` at `$1D6A0`) and burn through
    /// `Data_037516`.
    pub(super) fire_drop: i32,
    pub(super) fire_delay: i32,
    pub(super) fire_next_delay: i32,
    pub(super) fire_step: i32,
    pub(super) fire_sequence: Vec<u16>,
    /// Type-3 signed duration pairs selected by its parameter (`$291FC`).
    pub(super) route_paths: [Vec<(i8, i8)>; 4],
}
impl Tables {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let effects = crate::sound_events::Effects::load(rom)?;
        let mut barrel_bursts = [[0; 5]; 6];
        for (index, burst) in barrel_bursts.iter_mut().enumerate() {
            *burst = words::<5>(rom, 0x299ec + index * 10)?.map(|v| i32::from(v as i16));
        }
        let mut pickup_sounds = [0; 8];
        for (i, sound) in pickup_sounds.iter_mut().enumerate() {
            *sound = byte(rom, 0x35d8c + i)?;
        }
        let mut route_paths: [Vec<(i8, i8)>; 4] = std::array::from_fn(|_| Vec::new());
        for (route, path) in route_paths.iter_mut().enumerate() {
            let offset = usize::from(word(rom, 0x291fc + route * 2)?);
            let mut at = 0x291fc + offset;
            for _ in 0..64 {
                let pair = (byte(rom, at)? as i8, byte(rom, at + 1)? as i8);
                path.push(pair);
                at += 2;
                if pair == (0, 0) {
                    break;
                }
            }
        }
        Ok(Self {
            box_mapping: (word(rom, 0x2970a)? + word(rom, 0x27110)?),
            box_material: byte(rom, 0x29707)?,
            box_stand: hitbox_at(rom, 0x29798)?,
            box_attack: hitbox_at(rom, 0x2978a)?,
            snap_above: signed(rom, 0x2975a)?,
            snap_below: signed(rom, 0x29760)?,
            snap_state: usize::from(word(rom, 0x2976e)?),
            snap_height: signed(rom, 0x29778)?,
            box_voice: SoundEvent::ItemBoxHit.command(rom)?,
            pickup_mappings: words(rom, 0x35d6c)?,
            pickup_scores: words(rom, 0x35d7c)?,
            pickup_sounds,
            pickup_wait: signed(rom, 0x35ea4)?,
            blink_period: signed(rom, 0x35ec8)?,
            blink_steps: signed(rom, 0x36156)?,
            rise_ticks: signed(rom, 0x35f04)?,
            rise_speed: long(rom, 0x35f0a)?,
            sparkle: sequence(rom, 0x35d26)?,
            one_up: Table::Effect.command_at(rom, 0x12571)?,
            ammo: [
                u16::from(byte(rom, 0x35f73)?),
                u16::from(byte(rom, 0x35f81)?),
            ],
            ammo_limit: word(rom, 0x35fac)?,
            health: byte(rom, 0x35fc3)?,
            ninjutsu_limit: byte(rom, 0x360a7)?,
            bonus_limit: byte(rom, 0x3603f)?,
            debris_mappings: words(rom, 0x36c1c)?,
            debris_angles: words(rom, 0x36c66)?,
            debris_hold: signed(rom, 0x36cc2)?,
            debris_gravity: signed(rom, 0x36cbc)?,
            debris_sequence: sequence(rom, 0x36c24)?,
            barrel_smoke: sequence(rom, 0x36878)?,
            barrel_bursts,
            bomb_first: word(rom, 0x1d582)?,
            bomb_lead: signed(rom, 0x1d588)?,
            bomb_period: signed(rom, 0x1d598)?,
            bomb_digits: words(rom, 0x1d4c6)?,
            bomb_touched_period: signed(rom, 0x1d5c8)?,
            bomb_fuse: signed(rom, 0x1d610)?,
            bomb_blink: word(rom, 0x1d61e)?,
            bomb_sound: SoundEvent::BombExplosion.command(rom)?,
            flame_angles: words(rom, 0x1d0e2)?,
            flame_gravity: signed(rom, 0x1d170)?,
            flame_sequence: sequence(rom, 0x1d0d4)?,
            flame_damage: byte(rom, 0x1fbf6 + 3)?,
            flame_drop: signed(rom, 0x1d10e)?,
            fire_drop: match (word(rom, 0x1d65a)? >> 9) & 7 {
                0 => 8,
                quick => i32::from(quick),
            },
            fire_delay: signed(rom, 0x1d6c4)?,
            fire_next_delay: signed(rom, 0x375be)?,
            fire_step: signed(rom, 0x1d682)?,
            fire_sequence: sequence(rom, 0x37516)?,
            route_paths,
            sine: Sine::load(rom)?,
            effects,
        })
    }
    pub(super) fn effect(&self, id: u8) -> Option<u8> {
        // A zero id is a handler's "no sound".
        (id != 0).then(|| self.effects.command(id))?
    }
}
