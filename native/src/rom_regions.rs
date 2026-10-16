//! A native address view of regional cartridges. The Japanese image keeps its
//! original streams intact in the upper bank; only table/code addresses use the
//! port's reference layout. Nothing is decoded from another region's image.
use super::regions_data::{JP_RANGES, JP_WORD_ADDRESSES};
use crate::assets::crc32;
const SIZE: usize = 0x100000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Usa,
    Europe,
    Japan,
}
impl Region {
    pub fn name(self) -> &'static str {
        match self {
            Self::Usa => "USA (1993 retail)",
            Self::Europe => "Europe (1993 retail)",
            Self::Japan => "Japan / Korea (The Super Shinobi II, 1993 retail)",
        }
    }
}
pub fn checksum(data: &[u8]) -> u32 {
    crc32(if is_japanese_view(data) {
        &data[SIZE..]
    } else {
        data
    })
}

pub fn identify(data: &[u8]) -> Result<Region, String> {
    let raw = if is_japanese_view(data) {
        &data[SIZE..]
    } else {
        data
    };
    if raw.len() != SIZE || raw.get(0x100..0x104) != Some(b"SEGA") {
        return Err("Choose a plain 1 MiB Shinobi III / The Super Shinobi II retail cartridge image (.bin).".into());
    }
    match crc32(raw) {
        0x5381506f => Ok(Region::Usa),
        0x0b6d3eb5 => Ok(Region::Europe),
        0x5b412816 => Ok(Region::Japan),
        crc => Err(format!(
            "Unsupported ROM (CRC32 {crc:08x}). Supported retail dumps: US 5381506f, EU 0b6d3eb5, JP 5b412816. Use a plain binary dump without a copier header or interleaving."
        )),
    }
}
pub fn prepare(data: Vec<u8>) -> Result<Vec<u8>, String> {
    if identify(&data)? != Region::Japan || is_japanese_view(&data) {
        return Ok(data);
    }
    let mut view = data.clone();
    for &(start, end, source) in JP_RANGES {
        view[start..end].copy_from_slice(&data[source..source + end - start]);
    }
    for &at in JP_WORD_ADDRESSES.iter().chain([0x1d5a, 0xa5d8].iter()) {
        let value = usize::from(u16::from_be_bytes([view[at], view[at + 1]]));
        let value = canonical(value) as u16;
        view[at..at + 2].copy_from_slice(&value.to_be_bytes());
    }
    // Retain regional identity and preserve every variable-length data stream.
    view[0x100..0x200].copy_from_slice(&data[0x100..0x200]);
    view.extend(data);
    Ok(view)
}
pub(super) fn is_japanese_view(rom: &[u8]) -> bool {
    rom.len() == SIZE * 2 && rom.get(0x1f0) == Some(&b'J')
}
pub(super) fn canonical(address: usize) -> usize {
    match address {
        0x1cd8 => return 0x1e18,
        0xa20c => return 0xa62e,
        _ => {}
    }
    JP_RANGES
        .iter()
        .find_map(|&(start, end, source)| {
            (source <= address && address < source + end - start).then(|| start + address - source)
        })
        .unwrap_or(address)
}
/// Original stream/table address, used by decoders with fixed entry points.
pub fn source(rom: &[u8], address: usize) -> usize {
    if !is_japanese_view(rom) || address >= SIZE {
        return address;
    }
    SIZE + JP_RANGES
        .iter()
        .find_map(|&(start, end, source)| {
            (start <= address && address < end).then(|| source + address - start)
        })
        .unwrap_or(address)
}
pub(super) fn pointer(rom: &[u8], value: usize) -> usize {
    if !is_japanese_view(rom) || value == 0 || value >= SIZE {
        return value;
    }
    if value < 0x39de2 {
        canonical(value)
    } else {
        SIZE + value
    }
}
