//! VDP name tables and the `$3020` plane map placements that fill them.
use super::COLUMNS;
use super::LIMIT;
use crate::assets::musashi;
use crate::combat::{Plane, PlaneOp};
use crate::raster::SineWave;
use crate::rom::{Result, Sine, byte, long, pointer, word, words};
use crate::vdp::command_address;

/// Plane A/B name tables are 64x64 cells (VDP register $10 = $11).
const PLANE_CELLS: usize = 64;
pub const PLANE_A_VRAM: u16 = 0xc000;
pub const PLANE_B_VRAM: u16 = 0xe000;

/// A 64x64 VDP name table. `None` cells were not written by the modeled loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameTable {
    pub vram_base: u16,
    pub cells: Vec<Option<u16>>,
}
impl NameTable {
    pub fn new(vram_base: u16) -> Self {
        Self {
            vram_base,
            cells: vec![None; PLANE_CELLS * PLANE_CELLS],
        }
    }
    pub fn write(&mut self, address: u32, value: u16) -> Result<()> {
        let offset = address
            .checked_sub(u32::from(self.vram_base))
            .map(|o| o as usize / 2)
            .filter(|o| *o < self.cells.len() && address.is_multiple_of(2))
            .ok_or_else(|| format!("name-table write outside plane: ${address:04x}"))?;
        self.cells[offset] = Some(value);
        Ok(())
    }
    /// The attribute at a cell, wrapping; unwritten cells are 0.
    pub fn get(&self, col: usize, row: usize) -> u16 {
        self.cells[(row % PLANE_CELLS) * PLANE_CELLS + col % PLANE_CELLS].unwrap_or(0)
    }

    // Written cells in VRAM address order, big-endian words.
}

/// Name tables written by boss setup rather than the generic stage plane
/// routines. These belong to the stage presentation so the asset viewer and
/// native game consume the same recovered data without depending on SDL or a
/// boss behavior implementation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BossPlanes {
    /// 3-3: `Func_00319A`'s corridor on plane A and the Hydra head block on B.
    Hydra {
        plane_a: NameTable,
        plane_b: NameTable,
        /// The head's initial 16.16 position, converted to pixels.
        head: (i32, i32),
    },
    /// 5-3: the giant mech's 21x18 block on plane B at its initial position.
    Mech {
        plane_b: NameTable,
        body: (i32, i32),
        body_palette: [u16; 16],
        arena_palette: [u16; 16],
    },
    /// 4-3: Boss4 (`loc_02DBB4`) clears plane A (`Func_00459C`), which
    /// removes the surf's wave overlay, and draws the gunboat hull there.
    /// Its `Func_02EE38` then positions the hull with the plane A scroll.
    Hull { plane_a: NameTable },
}

impl BossPlanes {
    pub fn load(rom: &[u8], round: u8, substage: u8) -> Result<Option<Self>> {
        match (round, substage) {
            (3, 3) => Ok(Some(Self::load_hydra(rom)?)),
            (4, 3) => Ok(Some(Self::load_hull(rom)?)),
            (5, 3) => Ok(Some(Self::load_mech(rom)?)),
            _ => Ok(None),
        }
    }

    /// `$319A`: build the 3-3 corridor exactly as the cartridge does, then
    /// install `Data_02D2EE`, the Hydra head's initial plane-B word map.
    fn load_hydra(rom: &[u8]) -> Result<Self> {
        let mut plane_a = NameTable::new(PLANE_A_VRAM);
        let map_command = pointer(rom, 0x319c)? as u32;
        let map_base = word(rom, 0x31a2)?;
        let width = usize::from(word(rom, 0x31a6)?) + 1;
        let height = usize::from(word(rom, 0x31aa)?) + 1;
        let (map, _) = musashi(rom, pointer(rom, 0x31ae)?, LIMIT)?;
        if map.len() < width * height {
            return Err("short 3-3 corridor map".into());
        }
        write_byte_map(&mut plane_a, map_command, map_base, width, height, &map)?;

        // `$314C`: mirror source rows 0..7 into rows 62..55 with V-flip.
        let mirror_base = word(rom, 0x3172)?;
        let mirror_width = usize::from(word(rom, 0x316c)?) + 1;
        let commands = pointer(rom, 0x31c0)?;
        for row in 0..8 {
            write_byte_map(
                &mut plane_a,
                pointer(rom, commands + row * 4)? as u32,
                mirror_base,
                mirror_width,
                1,
                &map[row * mirror_width..],
            )?;
        }

        // `$31D2`: two generated floor rows, eight repeats of four bytes.
        let repeats = usize::from(byte(rom, 0x31d3)?) + 1;
        let step_a = pointer(rom, 0x31d6)? as u32;
        let step_b = pointer(rom, 0x31dc)? as u32;
        let first = pointer(rom, 0x31e2)? as u32;
        let mut rows = [Vec::new(), Vec::new()];
        for _ in 0..repeats {
            let mut value = first;
            rows[0].extend(value.to_be_bytes());
            value = value.wrapping_add(step_a);
            rows[1].extend(value.to_be_bytes());
            value = value.wrapping_add(step_b);
            rows[0].extend(value.to_be_bytes());
            value = value.wrapping_add(step_a);
            rows[1].extend(value.to_be_bytes());
        }
        let floor = rows.concat();
        let floor_width = usize::from(word(rom, 0x3204)?) + 1;
        let floor_height = usize::from(word(rom, 0x3208)?) + 1;
        if floor.len() < floor_width * floor_height {
            return Err("short generated 3-3 floor".into());
        }
        write_byte_map(
            &mut plane_a,
            pointer(rom, 0x31fa)? as u32,
            word(rom, 0x3200)?,
            floor_width,
            floor_height,
            &floor,
        )?;

        let mut plane_b = NameTable::new(PLANE_B_VRAM);
        write_word_map(&mut plane_b, rom, 0x2d2ee)?;
        Ok(Self::Hydra {
            plane_a,
            plane_b,
            head: (long(rom, 0x2c1fa)? >> 16, long(rom, 0x2c302)? >> 16),
        })
    }

    /// `loc_02DBB4`: the cleared plane A with the gunboat hull.
    fn load_hull(rom: &[u8]) -> Result<Self> {
        let mut plane_a = NameTable::new(PLANE_A_VRAM);
        apply_lz_word_map(&mut plane_a, rom, GUNBOAT_MAP)?;
        Ok(Self::Hull { plane_a })
    }

    /// `$30634`/`$4358`: the 5-3 boss rewrites this block every frame. The
    /// preview uses the initial `$40/$44 = $C8/$D8` position.
    fn load_mech(rom: &[u8]) -> Result<Self> {
        let mut plane_b = NameTable::new(PLANE_B_VRAM);
        write_word_map(&mut plane_b, rom, 0x35a28)?;
        Ok(Self::Mech {
            plane_b,
            body: (0xc8, 0xd8),
            body_palette: words(rom, 0x427c2)?,
            arena_palette: words(rom, 0x428c2)?,
        })
    }
}

/// Boss4's `Func_00429A` calls: the gunboat (`loc_02DBB4`) and the flying
/// machine (`Func_02E1EA`). Each is followed by the hull's `+$40`/`+$42`
/// scroll offsets (`move.l #imm,$40(a0)`).
pub(crate) const GUNBOAT_MAP: u32 = 0x2dc18;
pub(crate) const FLYER_MAP: u32 = 0x2e20c;

/// One `Func_00429A` call site: LZ source, VDP command, base attribute and
/// the width and height in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LzWordMapCall {
    pub source: usize,
    pub command: u32,
    pub base: u16,
    pub width: usize,
    pub height: usize,
}

/// Decode `lea src,a1` / `move.l #cmd,d0` / `move.w #base,d3` /
/// `moveq #w-1,d1` / `moveq #h-1,d2` / `jsr Func_00429A` at `at`.
pub fn lz_word_map_call(rom: &[u8], at: u32) -> Result<LzWordMapCall> {
    let at = at as usize;
    let opcodes = [(0, 0x43f9), (6, 0x203c), (12, 0x363c), (20, 0x4eb9)];
    for (offset, opcode) in opcodes {
        if word(rom, at + offset)? != opcode {
            return Err(format!("no Func_00429A call at ${at:06x}"));
        }
    }
    if byte(rom, at + 16)? != 0x72
        || byte(rom, at + 18)? != 0x74
        || pointer(rom, at + 22)? != 0x429a
    {
        return Err(format!("no Func_00429A call at ${at:06x}"));
    }
    Ok(LzWordMapCall {
        source: pointer(rom, at + 2)?,
        command: pointer(rom, at + 8)? as u32,
        base: word(rom, at + 14)?,
        width: usize::from(byte(rom, at + 17)?) + 1,
        height: usize::from(byte(rom, at + 19)?) + 1,
    })
}

fn apply_lz_word_map(table: &mut NameTable, rom: &[u8], at: u32) -> Result<()> {
    let call = lz_word_map_call(rom, at)?;
    let (data, _) = musashi(rom, call.source, LIMIT)?;
    if data.len() < call.width * call.height * 2 {
        return Err(format!("short LZ word map at ${:06x}", call.source));
    }
    let address = command_address(call.command) as u32;
    for row in 0..call.height {
        for col in 0..call.width {
            let i = (row * call.width + col) * 2;
            let cell = u16::from_be_bytes([data[i], data[i + 1]]);
            table.write(
                address + (row * 128 + col * 2) as u32,
                cell.wrapping_add(call.base),
            )?;
        }
    }
    Ok(())
}

/// Display lines of the VDP scroll tables (H40, 224 lines).
pub const RASTER_LINES: usize = 224;

/// Boss2's plane A wave (`Func_02B832`, 2-3): with VDP register `$0B` = 3
/// each of lines 12..211 takes line 0's H-scroll plus
/// `sine[phase * 2 + i * $20] * amplitude >> 16 >> 6` (`muls.w`, `swap`,
/// `asr.w #6`), up to ±127 px at the `$7F00` cap. The low word of the
/// product lands in plane B, which 2-3 leaves blank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineWave {
    /// `$2E(a0)`.
    pub amplitude: u16,
    /// `$30(a0)`.
    pub phase: u16,
}

impl LineWave {
    /// The first waved line (`lea $30(a1),a1`: 12 longs) and the line count
    /// (`move.w #$C7,d3`).
    pub const FIRST: usize = 12;
    pub const LINES: usize = 200;
    /// H-scroll added to display line `line` (0 outside the waved lines).
    pub(crate) fn offset(self, line: usize, sine: &Sine) -> i32 {
        let Some(i) = line.checked_sub(Self::FIRST).filter(|&i| i < Self::LINES) else {
            return 0;
        };
        SineWave::scaled(self.phase.wrapping_mul(2), 0x20, self.amplitude).offset(i, sine)
    }
}

/// One boss-drawn plane as the boss leaves it each frame: its name table
/// patched through [`PlaneOp`]s, the table the stage loaded (which
/// [`PlaneOp::Restore`] reads) and the scroll the boss writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BossPlane {
    pub table: NameTable,
    loaded: NameTable,
    /// World position of cell (0, 0) while the boss has not written the
    /// plane's scroll: the plane then follows the camera.
    origin: (i32, i32),
    /// H-scroll the boss adds to that placement on each line (VDP value;
    /// the Hydra's laser band, `loc_02D1C2`).
    pub hscroll: [i16; RASTER_LINES],
    /// H-scroll of each 8-line band and V-scroll of each 2-cell column,
    /// as VDP values, once the boss writes them (`Func_02D130`); they
    /// replace the camera placement.
    pub written: Option<([i16; 28], [i16; COLUMNS])>,
}

impl BossPlane {
    fn new(table: &NameTable, origin: (i32, i32)) -> Self {
        Self {
            table: table.clone(),
            loaded: table.clone(),
            origin,
            hscroll: [0; RASTER_LINES],
            written: None,
        }
    }
    /// Plane row shown at line 0 of each two-cell screen column.
    pub fn vscroll(&self, camera_y: i32) -> [i32; COLUMNS] {
        match &self.written {
            Some((_, columns)) => columns.map(i32::from),
            None => [camera_y - self.origin.1; COLUMNS],
        }
    }
    /// Sample offset (negated H-scroll) of display line `line`.
    pub fn shift(&self, line: usize, camera_x: i32) -> i32 {
        match &self.written {
            Some((bands, _)) => -i32::from(bands[(line / 8).min(bands.len() - 1)]),
            None => {
                let hscroll = self.hscroll.get(line).copied().unwrap_or(0);
                camera_x - self.origin.0 - i32::from(hscroll)
            }
        }
    }
}

/// The planes a boss draws itself (#96), from [`BossPlanes`]: 3-3's
/// corridor (A) and Hydra (B), 5-3's mech body (B). `Game` applies the
/// boss's [`PlaneOp`]s here and the viewer draws these planes in place of
/// the stage's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BossRaster {
    pub plane_a: Option<BossPlane>,
    pub plane_b: Option<BossPlane>,
    /// CRAM lines from 0 the boss rewrites, reloaded with the planes by
    /// `Mode_StageLoad` after a lost life (the Hydra's 0..2).
    pub cram_lines: usize,
}

impl BossRaster {
    pub fn for_stage(data: &super::StageData) -> Option<Self> {
        if let Some(planes) = &data.boss_planes {
            return Some(Self::new(planes));
        }
        if (data.id.round, data.id.substage) != (6, 3) {
            return None;
        }
        let mut table = NameTable::new(PLANE_A_VRAM);
        for row in 0..64 {
            for col in 0..64 {
                let cell = data
                    .metatile_at_pixel(col * 8, row * 8)
                    .and_then(|id| data.metatiles.get(usize::from(id)))
                    .map_or(0, |tiles| tiles[((row & 1) * 2 + (col & 1)) as usize]);
                table
                    .write(u32::from(PLANE_A_VRAM) + (row * 128 + col * 2) as u32, cell)
                    .ok()?;
            }
        }
        Some(Self {
            plane_a: Some(BossPlane::new(&table, (0, 0))),
            plane_b: None,
            cram_lines: 4,
        })
    }
    pub fn new(planes: &BossPlanes) -> Self {
        match planes {
            BossPlanes::Hydra {
                plane_a,
                plane_b,
                head,
            } => Self {
                plane_a: Some(BossPlane::new(plane_a, (0, 0))),
                // The head block rests at its initial position.
                plane_b: Some(BossPlane::new(plane_b, (head.0, head.1 + 0x110))),
                cram_lines: 3,
            },
            // Initial placement until Boss5 writes its first live position.
            BossPlanes::Mech { plane_b, body, .. } => Self {
                plane_a: None,
                plane_b: Some(BossPlane::new(plane_b, (body.0 - 0x54, body.1 + 0x130))),
                cram_lines: 4,
            },
            // Before `Func_02EE38` writes the scroll, the `$23C0` VSRAM
            // word $100 (camera Y) leaves the hull's rows off-screen.
            BossPlanes::Hull { plane_a } => Self {
                plane_a: Some(BossPlane::new(plane_a, (0, 0))),
                plane_b: None,
                cram_lines: 0,
            },
        }
    }

    fn plane(&mut self, plane: Plane) -> Option<&mut BossPlane> {
        match plane {
            Plane::A => self.plane_a.as_mut(),
            Plane::B => self.plane_b.as_mut(),
        }
    }
    /// The plane holding VRAM address `vram`.
    fn table(&mut self, vram: u16) -> Result<&mut BossPlane> {
        let plane = if vram >= PLANE_B_VRAM {
            Plane::B
        } else {
            Plane::A
        };
        self.plane(plane)
            .ok_or_else(|| format!("no boss-drawn plane at ${vram:04x}"))
    }

    /// Apply one boss plane operation.
    pub fn apply(&mut self, rom: &[u8], op: PlaneOp) -> Result<()> {
        match op {
            PlaneOp::KabukiPanel { door, frame } => {
                let vram = [0xc086, 0xc09e, 0xc0b6, 0xc606, 0xc61e, 0xc636]
                    .get(usize::from(door))
                    .copied()
                    .ok_or("invalid kabuki door")?;
                let start = pointer(rom, 0x329c0 + usize::from(frame % 6) * 4)?;
                let (start, height) = if door < 3 {
                    (start + 20, 10)
                } else {
                    (start, 11)
                };
                let table = &mut self.table(vram)?.table;
                for row in 0..height {
                    for col in 0..10 {
                        let cell = word(rom, start + (row * 10 + col) * 2)?.wrapping_add(0x6080);
                        table.write(u32::from(vram) + (row * 128 + col * 2) as u32, cell)?;
                    }
                }
            }
            PlaneOp::MechBody { x, y, flip } => {
                let plane = self.table(PLANE_B_VRAM)?;
                plane.origin = (x - 0x54, y + 0x130);
                plane.written = None;
                for row in 45..63 {
                    for col in 0..21 {
                        let cell = plane.loaded.get(if flip { 20 - col } else { col }, row)
                            ^ if flip { 0x800 } else { 0 };
                        plane
                            .table
                            .write(u32::from(PLANE_B_VRAM) + (row * 128 + col * 2) as u32, cell)?;
                    }
                }
            }
            PlaneOp::WordMap(header) => {
                let command = pointer(rom, header as usize)? as u32;
                let plane = self.table(command_address(command) as u16)?;
                write_word_map(&mut plane.table, rom, header as usize)?;
            }
            PlaneOp::ByteMap(header) => {
                let command = pointer(rom, header as usize)? as u32;
                let plane = self.table(command_address(command) as u16)?;
                // Plane_ReadMapHeader: command, base, width-1, height-1.
                let at = header as usize;
                let width = usize::from(word(rom, at + 6)?) + 1;
                let height = usize::from(word(rom, at + 8)?) + 1;
                let data = rom
                    .get(at + 10..at + 10 + width * height)
                    .ok_or("truncated byte map")?;
                let base = word(rom, at + 4)?;
                write_byte_map(&mut plane.table, command, base, width, height, data)?;
            }
            PlaneOp::LzWordMap(at) => {
                let command = lz_word_map_call(rom, at)?.command;
                let plane = self.table(command_address(command) as u16)?;
                apply_lz_word_map(&mut plane.table, rom, at)?;
            }
            PlaneOp::Fill {
                vram,
                width,
                height,
                cell,
            } => {
                let table = &mut self.table(vram)?.table;
                for row in 0..u32::from(height) {
                    for col in 0..u32::from(width) {
                        table.write(u32::from(vram) + row * 128 + col * 2, cell)?;
                    }
                }
            }
            PlaneOp::Restore {
                vram,
                width,
                height,
            } => {
                let plane = self.table(vram)?;
                for row in 0..u32::from(height) {
                    for col in 0..u32::from(width) {
                        let address = u32::from(vram) + row * 128 + col * 2;
                        let offset = (address - u32::from(plane.loaded.vram_base)) as usize / 2;
                        let cell = plane.loaded.get(offset % 64, offset / 64);
                        plane.table.write(address, cell)?;
                    }
                }
            }
            PlaneOp::Scroll {
                plane,
                bands,
                columns,
            } => {
                if let Some(plane) = self.plane(plane) {
                    plane.written = Some((bands, columns));
                }
            }
            PlaneOp::HScroll {
                plane,
                first,
                count,
                value,
            } => {
                if let Some(plane) = self.plane(plane) {
                    let first = usize::from(first);
                    let end = (first + usize::from(count)).min(RASTER_LINES);
                    plane.hscroll[first.min(end)..end].fill(value);
                }
            }
            // The stage's own plane A (`Game` keeps the wave in the viewer).
            PlaneOp::Wave { .. }
            | PlaneOp::FullScroll
            | PlaneOp::BossArt(_)
            | PlaneOp::FinalBossScroll { .. } => {}
        }
        Ok(())
    }
}

fn write_byte_map(
    table: &mut NameTable,
    command: u32,
    base: u16,
    width: usize,
    height: usize,
    data: &[u8],
) -> Result<()> {
    let address = command_address(command) as u32;
    for row in 0..height {
        for col in 0..width {
            table.write(
                address + (row * 128 + col * 2) as u32,
                u16::from(data[row * width + col]).wrapping_add(base),
            )?;
        }
    }
    Ok(())
}

/// A `Plane_DrawWordMapHeader` block: command, base, width-1, height-1,
/// followed by row-major words.
fn write_word_map(table: &mut NameTable, rom: &[u8], header: usize) -> Result<()> {
    let address = command_address(pointer(rom, header)? as u32) as u32;
    let base = word(rom, header + 4)?;
    let width = usize::from(word(rom, header + 6)?) + 1;
    let height = usize::from(word(rom, header + 8)?) + 1;
    for row in 0..height {
        for col in 0..width {
            let cell = word(rom, header + 10 + (row * width + col) * 2)?;
            table.write(
                address + (row * 128 + col * 2) as u32,
                cell.wrapping_add(base),
            )?;
        }
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapData {
    /// Musashi stream immediately after the placement records (`$3042`).
    Compressed,
    /// Uncompressed cells after the placement records (`$305A` with A5 = A0).
    Raw,
    /// A longword after the placements points to the Musashi stream (`$32B2`).
    Indirect,
}
/// `$3020` header: base attribute, width-1, height-1, flags (bit 0 byte map),
/// placement count-1, then six-byte (VDP command, flip flags) placements.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaneMap {
    pub descriptor: usize,
    pub data: MapData,
    pub stream: usize,
    pub end: usize,
    pub base: u16,
    pub width: usize,
    pub height: usize,
    pub placements: Vec<(u32, u16)>,
    pub cells: Vec<u16>,
}
impl PlaneMap {
    pub fn load(rom: &[u8], descriptor: usize, data: MapData) -> Result<Self> {
        let base = word(rom, descriptor)?;
        let width = usize::from(byte(rom, descriptor + 2)?) + 1;
        let height = usize::from(byte(rom, descriptor + 3)?) + 1;
        let byte_map = byte(rom, descriptor + 4)? & 1 != 0;
        let count = usize::from(byte(rom, descriptor + 5)?) + 1;
        let mut placements = Vec::new();
        for i in 0..count {
            let p = descriptor + 6 + i * 6;
            placements.push((pointer(rom, p)? as u32, word(rom, p + 4)?));
        }
        let after = descriptor + 6 + count * 6;
        let size = width * height * if byte_map { 1 } else { 2 };
        let (bytes, stream, end) = match data {
            MapData::Raw => (
                rom.get(after..after + size)
                    .ok_or("truncated raw plane map")?
                    .to_vec(),
                after,
                after + size,
            ),
            MapData::Compressed => {
                let (bytes, end) = musashi(rom, after, LIMIT)?;
                (bytes, after, end)
            }
            MapData::Indirect => {
                let stream = pointer(rom, after)?;
                let (bytes, end) = musashi(rom, stream, LIMIT)?;
                (bytes, stream, end)
            }
        };
        if bytes.len() < size {
            return Err(format!("short plane map at ${descriptor:06x}"));
        }
        let cells = if byte_map {
            bytes[..size].iter().map(|v| u16::from(*v)).collect()
        } else {
            bytes[..size]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&b| u16::from_be_bytes(b))
                .collect()
        };
        Ok(Self {
            descriptor,
            data,
            stream,
            end,
            base,
            width,
            height,
            placements,
            cells,
        })
    }
    /// `$306A..$30F2`: plain placements add the base; flip placements OR their
    /// flags into it. Bit 12 reads the whole map backwards (`$43A0/$43EC`);
    /// bit 11 reverses each row (`$4314/$4358`).
    pub fn apply(&self, table: &mut NameTable) -> Result<()> {
        let n = self.width * self.height;
        for &(command, flags) in &self.placements {
            let address = command_address(command) as u32;
            let base = if flags & 0x1800 != 0 {
                self.base | flags
            } else {
                self.base
            };
            for row in 0..self.height {
                for col in 0..self.width {
                    let index = if flags & 0x1000 != 0 {
                        n - 1 - (row * self.width + col)
                    } else if flags & 0x800 != 0 {
                        row * self.width + self.width - 1 - col
                    } else {
                        row * self.width + col
                    };
                    table.write(
                        address + (row * 128 + col * 2) as u32,
                        self.cells[index].wrapping_add(base),
                    )?;
                }
            }
        }
        Ok(())
    }
}

/// Plane B setup routine from `$2FC8` (`$2FA6`, round*8 + substage*2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaneB {
    pub routine: u16,
    pub maps: Vec<PlaneMap>,
    /// Set when the routine is not a data-driven map placement.
    pub unmodeled: Option<&'static str>,
    pub table: NameTable,
}
