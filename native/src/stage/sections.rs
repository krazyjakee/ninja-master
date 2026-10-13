//! `$116DC` part records: starts, exits and handoff offsets of every part.
use crate::rom::{Result, pointer, word};

#[derive(Clone, Debug)]
pub struct Section {
    pub record: usize,
    pub layout: usize,
    pub object_selection: usize,
    pub camera_start: (i32, i32),
    pub player_start: (i32, i32),
    pub exit: (i32, i32),
    pub flags: u16,
    pub mode_flags: u16,
    /// $11736 duplicates this pair for both camera and player handoff.
    pub transition_offset: (i32, i32),
    /// $1177E -> $FF0172, consumed by the final camera-lock path $ACAC/$AD34.
    pub exit_camera_y: i32,
    pub raw: [u16; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitKind {
    StreamPart,
    CameraLock,
}

impl Section {
    /// $ABC0 proximity portion only; caller retains global inhibition/state
    /// checks at $AB94 ($FF39F7 bit0 / $FF0028 bit2).
    pub fn exit_reached(&self, player: (i32, i32), runtime_mode_flags: u8) -> bool {
        let dx = (i64::from(player.0) - i64::from(self.exit.0)).abs();
        let dy = (i64::from(player.1) - i64::from(self.exit.1)).abs();
        if runtime_mode_flags & 0x40 != 0 {
            dx < 48 && dy < 48
        } else if self.flags & 1 != 0 {
            dx < 256 && dy < 24
        } else {
            dx < 24
        }
    }
    pub fn exit_kind(&self) -> ExitKind {
        if self.flags & 0x40 != 0 {
            ExitKind::CameraLock
        } else {
            ExitKind::StreamPart
        }
    }
    /// Apply the incoming section's offsets, never its initial spawn position.
    pub fn handoff_player_fixed8(&self, player: (i32, i32)) -> (i32, i32) {
        (
            handoff_fixed8(player.0, self.transition_offset.0 as i16),
            handoff_fixed8(player.1, self.transition_offset.1 as i16),
        )
    }
    pub fn handoff_camera_pixels(&self, camera: (i32, i32)) -> (i32, i32) {
        (
            handoff_word(camera.0, self.transition_offset.0 as i16),
            handoff_word(camera.1, self.transition_offset.1 as i16),
        )
    }
}

fn handoff_word(coordinate: i32, offset: i16) -> i32 {
    // $11CDE..$11D0A mask only integer words; ADD.W wraps at sixteen bits.
    i32::from(((coordinate as u16) & 0x1ff).wrapping_add(offset as u16) as i16)
}
fn handoff_fixed8(coordinate: i32, offset: i16) -> i32 {
    handoff_word(coordinate.div_euclid(256), offset) * 256 + coordinate.rem_euclid(256)
}

/// Part records of any round/substage (0-based indices, as in `$FF37A6/$FF37A8`).
/// `$116DC` selects count-prefixed round/substage tables and 32-byte part
/// records. Layouts: `$4AB36 + round*4 -> + substage*32 + part*4` (`$2C48`);
/// objects: `$41EFA -> round -> substage -> part` (`$129F8`).
pub fn sections(rom: &[u8], round: usize, substage: usize) -> Result<Vec<Section>> {
    let rounds = usize::from(word(rom, 0x688a)?);
    if round >= rounds || rounds > 8 {
        return Err(format!("round index {round} outside {rounds} rounds"));
    }
    let stage = pointer(rom, 0x688c + round * 4)?;
    let substages = usize::from(word(rom, stage)?);
    if substage >= substages || substages > 4 {
        return Err(format!(
            "substage index {substage} outside {substages} substages"
        ));
    }
    let parts = pointer(rom, stage + 2 + substage * 4)?;
    let count = usize::from(word(rom, parts)?);
    if count == 0 || count > 8 {
        return Err("invalid section count".into());
    }
    let layouts = pointer(rom, 0x4ab36 + round * 4)? + substage * 32;
    let objects = pointer(rom, pointer(rom, 0x41efa + round * 4)? + substage * 4)?;
    (0..count)
        .map(|part| {
            let record = parts + 2 + part * 32;
            let mut raw = [0; 16];
            for (i, w) in raw.iter_mut().enumerate() {
                *w = word(rom, record + i * 2)?;
            }
            Ok(Section {
                record,
                layout: pointer(rom, layouts + part * 4)?,
                object_selection: pointer(rom, objects + part * 4)?,
                camera_start: (i32::from(raw[0] as i16), i32::from(raw[1] as i16)),
                player_start: (i32::from(raw[2] as i16), i32::from(raw[3] as i16)),
                exit: (i32::from(raw[5] as i16), i32::from(raw[6] as i16)),
                flags: raw[4],
                mode_flags: raw[7],
                transition_offset: (i32::from(raw[9] as i16), i32::from(raw[10] as i16)),
                exit_camera_y: i32::from(raw[12] as i16),
                raw,
            })
        })
        .collect()
}
