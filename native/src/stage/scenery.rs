//! `$39BE4` scenery sprite lists: sprites completing background art.
use crate::rom::{Result, SpritePiece, pointer, signed, sprite_mapping, word};

/// How a scenery record is positioned each frame (`$39BB4` dispatch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneryMotion {
    /// `$39D50`: fixed VDP position per part from `$39D40`, with half of
    /// the `$FF16C4` bob offset (`asr.w #1`) added to Y.
    Fixed([(i32, i32); 4]),
    /// `$39DA0`: X = plane B HScroll of line 0 (`$FFF082`) + record X, in
    /// nine bits. X 0 is hidden (a sprite at X 0 would mask later sprites).
    PlaneB,
}

/// A `$39BE4` scenery record: sprites that complete background art the two
/// planes cannot hold. `$39A02` appends them after every object sprite, so
/// they draw behind all other sprites.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenery {
    /// Record word 1: offset into the `$39BD4` update branches.
    pub kind: u16,
    pub motion: SceneryMotion,
    /// Record `$10`: X added to the plane B scroll.
    pub x: i32,
    /// VDP Y of plane-B-following records.
    pub y: i32,
    pub mapping: u16,
    pub pieces: Vec<SpritePiece>,
}
impl Scenery {
    /// VDP sprite origin (128 = display left/top) for a plane B HScroll
    /// value (the negated sample offset) of line 0; `None` while hidden.
    /// `vscroll_offset` is `Stage_VScrollOffset`.
    pub fn position(
        &self,
        plane_b_hscroll: i32,
        part: usize,
        vscroll_offset: i32,
    ) -> Option<(i32, i32)> {
        match self.motion {
            SceneryMotion::Fixed(table) => {
                let (x, y) = table[part.min(3)];
                Some((x, y + (vscroll_offset >> 1)))
            }
            SceneryMotion::PlaneB => {
                let x = (plane_b_hscroll + self.x) & 0x1ff;
                (x != 0).then_some((x, self.y))
            }
        }
    }
}

/// `MOVE.W #imm,d(A1)` (`$337C`) operand at `at`.
fn move_to_a1(rom: &[u8], at: usize) -> Result<u16> {
    match word(rom, at)? {
        0x337c => word(rom, at + 2),
        op => Err(format!(
            "expected MOVE.W #imm,d(A1) at ${at:06x}, found ${op:04x}"
        )),
    }
}

/// `$39BE4`: an art selector pair (always the resident `$80000` patterns),
/// then up to sixteen (update, X, unused, mapping) records ending with a
/// negative word. A negative first word means no scenery.
pub fn scenery_list(rom: &[u8], list: usize) -> Result<Vec<Scenery>> {
    let mut out = Vec::new();
    if word(rom, list)? & 0x8000 != 0 {
        return Ok(out);
    }
    for record in (list + 4..).step_by(8).take(16) {
        let kind = word(rom, record)?;
        if kind & 0x8000 != 0 {
            break;
        }
        let x = signed(rom, record + 2)?;
        let (motion, y, mapping) = match kind {
            0 => {
                if word(rom, 0x39d56)? != 0x45f9 {
                    return Err("expected LEA of the $39D40 table".into());
                }
                let table = pointer(rom, 0x39d58)?;
                let mut positions = [(0, 0); 4];
                for (i, p) in positions.iter_mut().enumerate() {
                    *p = (signed(rom, table + i * 4)?, signed(rom, table + i * 4 + 2)?);
                }
                (
                    SceneryMotion::Fixed(positions),
                    0,
                    move_to_a1(rom, 0x39d50)?,
                )
            }
            4 => (
                SceneryMotion::PlaneB,
                i32::from(move_to_a1(rom, 0x39d9a)?),
                move_to_a1(rom, 0x39d94)?,
            ),
            0xc => (
                SceneryMotion::PlaneB,
                i32::from(move_to_a1(rom, 0x39dd6)?),
                word(rom, record + 6)?,
            ),
            _ => {
                return Err(format!(
                    "scenery update ${kind:02x} at ${record:06x} is not interpreted"
                ));
            }
        };
        out.push(Scenery {
            kind,
            motion,
            x,
            y,
            mapping,
            pieces: sprite_mapping(rom, mapping)?.1,
        });
    }
    Ok(out)
}
