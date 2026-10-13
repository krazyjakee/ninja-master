//! Stage setup decoded from cartridge tables and instruction operands.
use super::{id::StageId, planes::MapData};
use crate::rom::{Result, pointer, word};

pub(super) struct Overlay {
    pub source: usize,
    pub placements: Vec<u32>,
    pub base: u16,
    pub width: usize,
    pub height: usize,
}
pub(super) struct StagePlan {
    pub pattern: Option<(usize, usize)>,
    pub plane_b: Vec<(usize, MapData)>,
    pub note: Option<&'static str>,
    pub overlays: Vec<Overlay>,
}

fn lea(rom: &[u8], at: usize) -> Result<usize> {
    if !matches!(word(rom, at)?, 0x41f9 | 0x43f9 | 0x45f9 | 0x4df9) {
        return Err(format!("unexpected setup instruction at ${at:06x}"));
    }
    pointer(rom, at + 2)
}

fn immediate(rom: &[u8], at: usize) -> Result<u16> {
    if word(rom, at)? & 0xf1ff != 0x303c {
        return Err(format!("unexpected setup instruction at ${at:06x}"));
    }
    word(rom, at + 2)
}

fn overlay(
    rom: &[u8],
    stream: usize,
    commands: &[usize],
    base: usize,
    width: usize,
    height: usize,
) -> Result<Overlay> {
    let placements = commands
        .iter()
        .map(|&at| {
            if word(rom, at)? != 0x203c {
                return Err(format!("unexpected overlay instruction at ${at:06x}"));
            }
            let command = pointer(rom, at + 2)? as u32;
            Ok(((command >> 16) & 0x3fff) | ((command & 3) << 14))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Overlay {
        source: lea(rom, stream)?,
        placements,
        base: immediate(rom, base)?,
        width: usize::from(immediate(rom, width)?) + 1,
        height: usize::from(immediate(rom, height)?) + 1,
    })
}

pub(super) fn stage_plan(rom: &[u8], id: StageId, mode: u16) -> Result<StagePlan> {
    let (r, s, p) = (id.round_index(), id.substage_index(), id.part_index());
    let routine = word(rom, 0x2fc8 + r * 8 + s * 2)?;
    let mut plane_b = Vec::new();
    let mut note = None;
    let fixed: &[(usize, MapData)] = match routine {
        0x30f4 => &[(0x30f4, MapData::Compressed), (0x3100, MapData::Compressed)],
        0x312c => &[(0x312c, MapData::Raw), (0x3140, MapData::Compressed)],
        0x3272 => &[(0x3272, MapData::Compressed), (0x327e, MapData::Raw)],
        0x3292 => &[(0x3292, MapData::Raw), (0x32a6, MapData::Compressed)],
        0x32e6 => &[(0x32e6, MapData::Compressed)],
        0x303c => {
            plane_b.push((
                pointer(rom, lea(rom, 0x3000)? + r * 16 + s * 4)?,
                MapData::Compressed,
            ));
            &[]
        }
        0x310c => {
            plane_b.push((
                lea(rom, if p == 0 { 0x3114 } else { 0x3120 })?,
                MapData::Compressed,
            ));
            &[]
        }
        0x32b2 => {
            plane_b.push((pointer(rom, lea(rom, 0x32b2)? + p * 4)?, MapData::Indirect));
            &[]
        }
        0x1ab2 => {
            note = Some("boss code draws plane B");
            &[]
        }
        0x326c => {
            note = Some("plane B cleared");
            &[]
        }
        0x319a => {
            note = Some("procedural boss backdrop");
            &[]
        }
        _ => return Err(format!("unknown plane B routine ${routine:04x}")),
    };
    for &(at, kind) in fixed {
        plane_b.push((lea(rom, at)?, kind));
    }
    let pattern = match (r, s) {
        (1, 0) => Some((lea(rom, 0x2d3e)?, 16)),
        (3, 0) => Some((lea(rom, 0x2d48)?, 16)),
        (3, 2) => Some((lea(rom, 0x2d7a)?, 32)),
        _ => None,
    };
    let overlays = if mode & 0x10 != 0 && r == 1 {
        vec![
            overlay(rom, 0x2ea0, &[0x2eb2], 0x2eb8, 0x2ebc, 0x2ec0)?,
            overlay(
                rom,
                0x2ed0,
                &[0x2ee2, 0x2eec, 0x2ef6, 0x2f00],
                0x2f06,
                0x2f0a,
                0x2f0e,
            )?,
        ]
    } else if mode & 0x10 != 0 && r == 3 && p != 2 {
        vec![overlay(
            rom,
            0x2f30,
            &[0x2f42, 0x2f4c],
            0x2f52,
            0x2f56,
            0x2f5a,
        )?]
    } else {
        Vec::new()
    };
    Ok(StagePlan {
        pattern,
        plane_b,
        note,
        overlays,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_setup_reads_operands_and_rejects_unknown_routines() {
        let mut rom = vec![0; 0x4000];
        rom[0x2fc8..0x2fca].copy_from_slice(&0x30f4_u16.to_be_bytes());
        for (at, source) in [(0x30f4, 0x77777_u32), (0x3100, 0x88888)] {
            rom[at..at + 2].copy_from_slice(&0x41f9_u16.to_be_bytes());
            rom[at + 2..at + 6].copy_from_slice(&source.to_be_bytes());
        }
        let plan = stage_plan(&rom, StageId::FOREST, 0).unwrap();
        assert_eq!(
            plan.plane_b,
            [
                (0x77777, MapData::Compressed),
                (0x88888, MapData::Compressed)
            ]
        );
        assert!(plan.overlays.is_empty());
        assert!(plan.pattern.is_none());
        rom[0x30f4..0x30f6].fill(0);
        assert!(stage_plan(&rom, StageId::FOREST, 0).is_err());
        rom[0x2fc8..0x2fca].fill(0);
        assert!(stage_plan(&rom, StageId::FOREST, 0).is_err());
        assert!(stage_plan(&[], StageId::FOREST, 0).is_err());
    }
}
