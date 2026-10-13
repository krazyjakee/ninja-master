//! Player animation sequences decoded from the user's cartridge at runtime.
use crate::rom::{Result, SPRITE_MAPPINGS, byte, word, words};
use std::ops::Range;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerAnimation {
    pub state: usize,
    pub source: Range<usize>,
    /// Increment of the original 8.8 animation cursor on each tick.
    pub speed: u8,
    /// Indices into PlayerAssets::frames; $ff restarts the sequence.
    pub frames: Arc<[usize]>,
}
impl PlayerAnimation {
    /// Frame after `tick` uninterrupted `$214CA` steps from cursor zero. This is
    /// only the free-running loop: gameplay handlers hold, rewrite or end the
    /// cursor, so play mode uses `movement::PlayerAnimator` instead.
    pub fn frame_at(&self, tick: u64) -> usize {
        if self.speed == 0 {
            return self.frames[0];
        }
        let speed = u64::from(self.speed);
        // $214F0 clears both bytes of the cursor when it encounters $ff.
        let period = (self.frames.len() as u64 * 256).div_ceil(speed);
        self.frames[((tick % period) * speed / 256) as usize]
    }
}

struct PlayerTables {
    animations: [PlayerAnimation; 76],
    hang_steps: [i32; 8],
    hang_phase_next: [u8; 8],
    body_tops: [i32; 202],
}
static PLAYER_TABLES: OnceLock<Result<PlayerTables>> = OnceLock::new();

fn decode_tables(rom: &[u8]) -> Result<PlayerTables> {
    let animations = decode(rom)?;
    let hang_steps = words::<8>(rom, 0x2028e)?.map(|v| i32::from(v as i16));
    let hang_phase_next = rom
        .get(0x202b2..0x202ba)
        .ok_or("truncated ceiling movement table")?
        .try_into()
        .map_err(|_| "invalid ceiling movement table")?;
    let mut body_tops = [0; 202];
    for (frame, top) in body_tops.iter_mut().enumerate() {
        let id = usize::from(word(rom, 0x23ade + frame * 4)?);
        let mapping = SPRITE_MAPPINGS + usize::from(word(rom, SPRITE_MAPPINGS + id * 2)?);
        *top = i32::from(byte(rom, mapping + 1)? as i8) - i32::from(byte(rom, mapping + 3)?);
    }
    Ok(PlayerTables {
        animations,
        hang_steps,
        hang_phase_next,
        body_tops,
    })
}

fn tables() -> &'static PlayerTables {
    PLAYER_TABLES
        .get()
        .expect("load PlayerAssets before player tables")
        .as_ref()
        .expect("validated player tables")
}

fn decode(rom: &[u8]) -> Result<[PlayerAnimation; 76]> {
    let mut shared = std::collections::BTreeMap::<usize, Arc<[usize]>>::new();
    let mut sequences = Vec::with_capacity(76);
    for state in 0..76 {
        let start = 0x235f6 + usize::from(word(rom, 0x235f6 + state * 2)?);
        if !(0x2368e..0x23ade).contains(&start) {
            return Err("player animation pointer outside sequence area".into());
        }
        let mut end = start + 1;
        while end < 0x23ade && byte(rom, end)? != 0xff {
            if byte(rom, end)? >= 202 {
                return Err("invalid player animation frame".into());
            }
            end += 1;
        }
        if end == start + 1 || end == 0x23ade {
            return Err("empty or unterminated player animation".into());
        }
        let frames = shared.entry(start).or_insert_with(|| {
            rom[start + 1..end]
                .iter()
                .map(|&b| usize::from(b))
                .collect()
        });
        sequences.push(PlayerAnimation {
            state,
            source: start..end + 1,
            speed: byte(rom, start)?,
            frames: Arc::clone(frames),
        });
    }
    sequences
        .try_into()
        .map_err(|_| "invalid animation count".into())
}

pub(super) fn load_sequences(rom: &[u8]) -> Result<&'static [PlayerAnimation; 76]> {
    PLAYER_TABLES
        .get_or_init(|| decode_tables(rom))
        .as_ref()
        .map(|tables| &tables.animations)
        .map_err(Clone::clone)
}

/// Available after `PlayerAssets::load` has validated and decoded the cartridge.
pub fn sequences() -> &'static [PlayerAnimation; 76] {
    &tables().animations
}

pub fn hang_steps() -> &'static [i32; 8] {
    &tables().hang_steps
}
pub fn hang_phase_next() -> &'static [u8; 8] {
    &tables().hang_phase_next
}
pub fn player_body_top(frame: usize) -> i32 {
    tables().body_tops.get(frame).copied().unwrap_or(-54)
}

/// Animation for a player state, following cartridge initialization.
pub fn sequence(id: usize) -> &'static PlayerAnimation {
    &sequences()[id]
}

/// One call of the original stepper `$214CA..$21516`. The frame is read at
/// the cursor's high byte BEFORE the speed is added; `$FF` there restarts the
/// sequence with both cursor bytes cleared (`$214F0`). The returned flag is
/// +$2A bit 0 (`$21510`): the advanced cursor now indexes the terminator.
pub fn step_cursor(speed: u8, frames: &[usize], cursor: &mut u16) -> (usize, bool) {
    let mut index = usize::from(*cursor >> 8);
    if index >= frames.len() {
        *cursor = 0;
        index = 0;
    }
    let frame = frames[index];
    *cursor = cursor.wrapping_add(u16::from(speed));
    (frame, usize::from(*cursor >> 8) >= frames.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let mut rom = vec![0; 0x23ade];
        for state in 0..76 {
            let entry = 0x235f6 + state * 2;
            rom[entry..entry + 2].copy_from_slice(&0x98_u16.to_be_bytes());
        }
        rom[0x2368e..0x23693].copy_from_slice(&[23, 17, 4, 9, 0xff]);
        rom
    }

    #[test]
    fn runtime_animation_decoder_preserves_aliases_and_cursor_order() {
        let sequences = decode(&fixture()).unwrap();
        assert_eq!(sequences[0].source, 0x2368e..0x23693);
        assert_eq!(sequences[0].speed, 23);
        assert_eq!(sequences[0].frames.as_ref(), [17, 4, 9]);
        assert!(Arc::ptr_eq(&sequences[0].frames, &sequences[75].frames));
        let mut cursor = 3 * 256;
        assert_eq!(
            step_cursor(23, &sequences[0].frames, &mut cursor),
            (17, false)
        );
        assert_eq!(cursor, 23);
    }

    #[test]
    fn runtime_animation_decoder_rejects_invalid_records() {
        assert!(decode(&[]).is_err());
        let mut rom = fixture();
        rom[0x235f6..0x235f8].fill(0);
        assert!(decode(&rom).is_err());
        let mut rom = fixture();
        rom[0x2368f] = 202;
        assert!(decode(&rom).is_err());
        let mut rom = fixture();
        rom[0x2368f] = 0xff;
        assert!(decode(&rom).is_err());
        let mut rom = fixture();
        rom[0x23692] = 0;
        assert!(decode(&rom).is_err());
    }

    #[test]
    fn player_movement_tables_and_hitboxes_are_read_from_rom() {
        let mut rom = fixture();
        rom.resize(0x4322a, 0);
        rom[0x2028e..0x20290].copy_from_slice(&(-13_i16).to_be_bytes());
        rom[0x202b2] = 5;
        rom[SPRITE_MAPPINGS..SPRITE_MAPPINGS + 2].copy_from_slice(&2_u16.to_be_bytes());
        rom[SPRITE_MAPPINGS + 3] = (-4_i8) as u8;
        rom[SPRITE_MAPPINGS + 5] = 7;
        let tables = decode_tables(&rom).unwrap();
        assert_eq!(tables.hang_steps[0], -13);
        assert_eq!(tables.hang_phase_next[0], 5);
        assert!(tables.body_tops.iter().all(|&top| top == -11));
        assert!(decode_tables(&rom[..SPRITE_MAPPINGS + 4]).is_err());
    }
}
