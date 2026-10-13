//! `StageData`: everything one stage part loads from the ROM.
use super::{
    LIMIT,
    content::{StagePlan, stage_plan},
    flags::{AutoScroll, ModeFlags, RuntimeFlags, SpecialMode},
    id::{StageId, StageRoutines, stage_routines},
    planes::{BossPlanes, MapData, NameTable, PLANE_A_VRAM, PLANE_B_VRAM, PlaneB, PlaneMap},
    scroll::{CameraBounds, ScrollModel},
    sections::{Section, sections},
};
use crate::{
    assets::{kosinski, musashi},
    render::Tile,
    rom::{Result, byte, pointer, word, words},
};
use std::ops::Range;

/// `$AB36` stops writing at `$FF377C`: the layout buffer holds 8192 bytes.
pub const LAYOUT_BYTES: usize = 8192;
pub const TILE_SLOTS: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Column-major, 32 metatiles per column (512 pixels high).
    Horizontal,
    /// Row-major, 32 metatiles per row (512 pixels wide).
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutSource {
    /// `$AB36` stream; `truncated` means the stream continued past 8192 bytes.
    Rle {
        start: usize,
        end: usize,
        truncated: bool,
    },
    /// `$2D3E/$2D48/$2D7A`: a fixed pattern copied into every column.
    Pattern { start: usize, rows: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadKind {
    Background,
    Foreground,
    Object,
    Boss,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileUpload {
    pub kind: UploadKind,
    pub name: String,
    pub source: usize,
    pub end: usize,
    pub first_tile: usize,
    pub tile_count: usize,
    pub remapped: bool,
    /// Bytes the transfer needs beyond the decompressed stream. `$460E` DMAs
    /// whatever stale RAM follows (3-1, 4-1 and 6-1 part 1 objects, the 6-1
    /// background's last six tiles); the
    /// native loader zero-fills them and the emulator checks exclude them.
    pub missing_bytes: usize,
}

/// Complete data for one stage part, suitable for a generic runtime.
#[derive(Clone)]
pub struct StageData {
    pub id: StageId,
    pub part_count: usize,
    pub section: Section,
    pub mode: ModeFlags,
    pub flags: RuntimeFlags,
    pub special: SpecialMode,
    pub autoscroll: AutoScroll,
    pub camera: CameraBounds,
    pub orientation: Orientation,
    pub layout: Vec<u8>,
    /// Layout bytes changed at runtime (`crate::stage::TerrainEdit`) and
    /// the values the part loaded with.
    pub edits: Vec<(usize, u8)>,
    pub layout_source: LayoutSource,
    pub metatiles: Vec<[u16; 4]>,
    pub metatile_range: Range<usize>,
    /// 256 bytes indexed by metatile: bits 7..4 exclude quadrants, low nibble material.
    pub collision: Vec<u8>,
    pub collision_range: Range<usize>,
    pub tiles: Vec<Tile>,
    pub loaded: Vec<bool>,
    pub uploads: Vec<TileUpload>,
    /// Address of the four palette IDs (`$43042` or boss `$43202`).
    pub palette_source: usize,
    pub palette_ids: [u8; 4],
    pub palette_words: [[u16; 16]; 4],
    /// Object selection header: up to four (remap, palette, flags, group) records.
    pub object_records: Vec<[u16; 4]>,
    /// Spawn records following the selection header (`$12A40`, at most 32).
    pub spawns: Vec<[u16; 4]>,
    pub plane_a_overlay: Option<NameTable>,
    pub plane_b: PlaneB,
    /// Name tables drawn by boss setup rather than the generic plane routines.
    pub boss_planes: Option<BossPlanes>,
    pub routines: StageRoutines,
    pub scroll: ScrollModel,
}

/// Exact `$12B22` nibble remap: 0/1 unchanged, others +7. The 68000 ORs an
/// overflowing sum into the following pixel before it is processed.
fn remap_object_pixels(bytes: &mut [u8]) {
    for chunk in bytes.as_chunks_mut::<4>().0 {
        let mut d0 = u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        for _ in 0..8 {
            let n = d0 & 15;
            d0 &= !15;
            d0 |= if n >= 2 { n + 7 } else { n };
            d0 = d0.rotate_right(4);
        }
        chunk.copy_from_slice(&d0.to_be_bytes());
    }
}

/// `$AB36`, stopping (like the original) once the 8192-byte buffer is full.
fn layout_rle(rom: &[u8], start: usize) -> Result<(Vec<u8>, usize, bool)> {
    let mut p = start;
    let mut out = Vec::with_capacity(LAYOUT_BYTES);
    loop {
        let count = byte(rom, p)?;
        p += 1;
        if count == 255 {
            let count = byte(rom, p)?;
            p += 1;
            if count == 255 {
                return Ok((out, p, false));
            }
            for _ in 0..if count == 0 { 256 } else { usize::from(count) } {
                if out.len() == LAYOUT_BYTES {
                    return Ok((out, p, true));
                }
                out.push(byte(rom, p)?);
                p += 1;
            }
        } else {
            let value = byte(rom, p)?;
            p += 1;
            for _ in 0..=count {
                if out.len() == LAYOUT_BYTES {
                    return Ok((out, p, true));
                }
                out.push(value);
            }
        }
    }
}

impl StageData {
    pub fn load(rom: &[u8], id: StageId) -> Result<Self> {
        let (r, s, p) = (id.round_index(), id.substage_index(), id.part_index());
        let all = sections(rom, r, s)?;
        let section = all
            .get(p)
            .cloned()
            .ok_or_else(|| format!("{id}: substage has {} parts", all.len()))?;
        let plan = stage_plan(rom, id, section.raw[7])?;
        let raw = section.raw;
        let mode = ModeFlags(raw[7]);
        let flags = RuntimeFlags::from_record(raw[4], raw[7]);
        let special = if mode.boss_arena() {
            SpecialMode::BossArena
        } else if mode.mounted() && mode.player_high_priority() {
            SpecialMode::Surf
        } else if mode.mounted() {
            SpecialMode::Horse
        } else {
            SpecialMode::Normal
        };
        let camera = CameraBounds::for_section(rom, &section)?;
        if camera.stride != 32 {
            return Err(format!("{id}: unsupported layout stride {}", camera.stride));
        }
        let orientation = if flags.vertical() {
            Orientation::Vertical
        } else {
            Orientation::Horizontal
        };
        // $2C02: fixed patterns for 2-1, 4-1 and 4-3; otherwise $4AB36 streams.
        let (layout, layout_source) = if let Some((start, rows)) = plan.pattern {
            let bytes = rom.get(start..start + rows).ok_or("short layout pattern")?;
            let mut layout = vec![0; LAYOUT_BYTES];
            for column in layout.as_chunks_mut::<32>().0 {
                column[..rows].copy_from_slice(bytes);
            }
            (layout, LayoutSource::Pattern { start, rows })
        } else {
            let (layout, end, truncated) = layout_rle(rom, section.layout)?;
            (
                layout,
                LayoutSource::Rle {
                    start: section.layout,
                    end,
                    truncated,
                },
            )
        };
        if !layout.len().is_multiple_of(32) {
            return Err(format!("{id}: incomplete layout line"));
        }
        let pair = pointer(rom, 0xff23e + r * 16 + s * 4)?;
        let (start, collision_start) = (pointer(rom, pair)?, pointer(rom, pair + 4)?);
        if collision_start < start || collision_start - start > 256 * 8 {
            return Err(format!("{id}: invalid metatile table"));
        }
        let metatiles = (start..collision_start)
            .step_by(8)
            .map(|q| words(rom, q))
            .collect::<Result<Vec<_>>>()?;
        if layout.iter().any(|m| usize::from(*m) >= metatiles.len()) {
            return Err(format!("{id}: layout references unknown metatile"));
        }
        let collision = rom
            .get(collision_start..collision_start + 256)
            .ok_or("short collision table")?
            .to_vec();
        let routines = stage_routines(rom, r, s)?;
        let mut data = Self {
            id,
            part_count: all.len(),
            section,
            mode,
            flags,
            special,
            autoscroll: AutoScroll(raw[13]),
            camera,
            orientation,
            layout,
            edits: Vec::new(),
            layout_source,
            metatiles,
            metatile_range: start..collision_start,
            collision,
            collision_range: collision_start..collision_start + 256,
            tiles: vec![Tile([0; 32]); TILE_SLOTS],
            loaded: vec![false; TILE_SLOTS],
            uploads: Vec::new(),
            palette_source: 0,
            palette_ids: [0; 4],
            palette_words: [[0; 16]; 4],
            object_records: Vec::new(),
            spawns: Vec::new(),
            plane_a_overlay: None,
            plane_b: PlaneB {
                routine: routines.plane_b,
                maps: Vec::new(),
                unmodeled: plan.note,
                table: NameTable::new(PLANE_B_VRAM),
            },
            boss_planes: BossPlanes::load(rom, id.round, id.substage)?,
            routines,
            scroll: ScrollModel::for_routine(routines.scroll, p)?,
        };
        data.load_art(rom)?;
        data.load_palettes(rom)?;
        for &(descriptor, kind) in &plan.plane_b {
            let map = PlaneMap::load(rom, descriptor, kind)?;
            map.apply(&mut data.plane_b.table)?;
            data.plane_b.maps.push(map);
        }
        data.load_plane_a_overlay(rom, &plan)?;
        Ok(data)
    }

    fn install(&mut self, mut upload: TileUpload, bytes: &[u8]) -> Result<()> {
        let size = upload.tile_count * 32;
        if upload.first_tile + upload.tile_count > TILE_SLOTS {
            return Err(format!(
                "{}: invalid tile transfer {}",
                self.id, upload.name
            ));
        }
        let mut padded = bytes[..size.min(bytes.len())].to_vec();
        upload.missing_bytes = size - padded.len();
        padded.resize(size, 0);
        for (i, tile) in padded.as_chunks::<32>().0.iter().enumerate() {
            self.tiles[upload.first_tile + i] = Tile(*tile);
            self.loaded[upload.first_tile + i] = true;
        }
        self.uploads.push(upload);
        Ok(())
    }

    /// Upload order of `$115C0`: background art (`$80014`, step 2), foreground
    /// art (`$80010`, step 4), object groups (`$12A56`) and boss art (`$2A41E`).
    fn load_art(&mut self, rom: &[u8]) -> Result<()> {
        let (r, s) = (self.id.round_index(), self.id.substage_index());
        for (root, kind, name) in [
            (0x80014, UploadKind::Background, "background"),
            (0x80010, UploadKind::Foreground, "foreground"),
        ] {
            // $11634: table + round*16 + substage*4 -> $46FA header + Kosinski.
            let source = pointer(rom, pointer(rom, root)? + r * 16 + s * 4)?;
            let first_tile = usize::from(word(rom, source)?);
            let tile_count = usize::from(word(rom, source + 2)?) + 1;
            let (bytes, end) = kosinski(rom, source + 4, LIMIT)?;
            self.install(
                TileUpload {
                    kind,
                    name: name.into(),
                    source,
                    end,
                    first_tile,
                    tile_count,
                    remapped: false,
                    missing_bytes: 0,
                },
                &bytes,
            )?;
        }
        // $129F8: $41EFA -> round -> substage -> part selection.
        let selection = self.section.object_selection;
        for i in 0..4 {
            let record = words(rom, selection + i * 8)?;
            if record[0] & 0x8000 != 0 {
                break;
            }
            self.object_records.push(record);
        }
        for i in 0..32 {
            let spawn = words(rom, selection + 32 + i * 8)?;
            if spawn[0] & 0x8000 != 0 {
                break;
            }
            self.spawns.push(spawn);
        }
        let object_root = pointer(rom, 0x80008)?;
        let indices = pointer(rom, object_root)?;
        let groups = pointer(rom, object_root + 4)?;
        let descriptors = pointer(rom, 0x80004)?;
        let mut base = 0usize;
        for record in self.object_records.clone() {
            let remap = record[0] != 0;
            let mut group = pointer(
                rom,
                groups + pointer(rom, indices + usize::from(record[3]))?,
            )?;
            loop {
                let dest = word(rom, group)?;
                let id = usize::from(word(rom, group + 2)?);
                group += 4;
                if dest & 0x8000 != 0 {
                    base += id;
                    break;
                }
                let descriptor = descriptors + id * 6;
                let tile_count = usize::from(word(rom, descriptor)?) + 1;
                let source = pointer(rom, descriptor + 2)?;
                let (mut bytes, end) = musashi(rom, source, LIMIT)?;
                if remap {
                    let size = (tile_count * 32).min(bytes.len());
                    remap_object_pixels(&mut bytes[..size]);
                }
                self.install(
                    TileUpload {
                        kind: UploadKind::Object,
                        name: format!("object ${id:02x}"),
                        source,
                        end,
                        first_tile: (base + usize::from(dest)) & 0xffff,
                        tile_count,
                        remapped: remap,
                        missing_bytes: 0,
                    },
                    &bytes,
                )?;
            }
        }
        if self.mode.boss_arena() {
            self.load_boss_art(rom, r)?;
        }
        Ok(())
    }

    /// `Boss_LoadArt` ($2A41E), also called with round + 1/+2 by Boss7.
    pub(crate) fn load_boss_art(&mut self, rom: &[u8], round_index: usize) -> Result<()> {
        // $2A41E: $8000C -> round -> (descriptor base, dest/id/unused list).
        let table = pointer(rom, pointer(rom, 0x8000c)? + round_index * 4)?;
        let descriptors = pointer(rom, table)?;
        let mut entry = pointer(rom, table + 4)?;
        while word(rom, entry)? & 0x8000 == 0 {
            let first_tile = usize::from(word(rom, entry)?);
            let id = usize::from(word(rom, entry + 2)?);
            entry += 6;
            let tile_count = usize::from(word(rom, descriptors + id * 6)?) + 1;
            let source = pointer(rom, descriptors + id * 6 + 2)?;
            let (bytes, end) = musashi(rom, source, LIMIT)?;
            self.install(
                TileUpload {
                    kind: UploadKind::Boss,
                    name: format!("boss ${id:02x}"),
                    source,
                    end,
                    first_tile,
                    tile_count,
                    remapped: false,
                    missing_bytes: 0,
                },
                &bytes,
            )?;
        }
        Ok(())
    }

    /// `Func_00346A`: replace the arena planes and line-2 fade target.
    pub(crate) fn final_boss_arena(&mut self, rom: &[u8]) -> Result<()> {
        let map = PlaneMap::load(rom, 0x406fa, MapData::Compressed)?;
        let mut foreground = NameTable::new(PLANE_A_VRAM);
        map.apply(&mut foreground)?;
        // $3496: the same 20x14 byte map upwards from VRAM $C680.
        for row in 0..14 {
            for col in 0..20 {
                foreground.write(
                    0xc680 - row as u32 * 128 + col as u32 * 2,
                    map.cells[row * 20 + col].wrapping_add(0x5200),
                )?;
            }
        }
        self.plane_a_overlay = Some(foreground);
        self.plane_b.table = NameTable::new(PLANE_B_VRAM);
        PlaneMap::load(rom, 0x40780, MapData::Compressed)?.apply(&mut self.plane_b.table)?;
        self.palette_words[2] = words(rom, 0x34f6)?;
        Ok(())
    }

    /// `$858`: ordinary IDs at `$43042 + (round*16 + (substage&1)*8 + part)*4`;
    /// boss arenas use `$43202 + round*4` (`$8BC`). `$1300` then patches bank
    /// one from the object records unless the boss flag is set.
    fn load_palettes(&mut self, rom: &[u8]) -> Result<()> {
        let (r, s, p) = (
            self.id.round_index(),
            self.id.substage_index(),
            self.id.part_index(),
        );
        self.palette_source = if self.mode.boss_arena() {
            0x43202 + r * 4
        } else {
            0x43042 + ((r * 16 + (s & 1) * 8 + p) & 0xff) * 4
        };
        for bank in 0..4 {
            let id = byte(rom, self.palette_source + bank)?;
            self.palette_ids[bank] = id;
            self.palette_words[bank] = words(rom, 0x42062 + usize::from(id) * 32)?;
        }
        if self.mode.boss_arena() {
            return Ok(());
        }
        for record in self.object_records.clone() {
            let special = match record[3] {
                0xb4 | 0xb8 => Some(0x42fd2),
                0xf8 => Some(0x43012),
                _ => None,
            };
            if let Some(source) = special {
                self.palette_words[1] = words(rom, source)?;
                return Ok(()); // $1364/$136C return after the full-bank copy.
            }
            let remap = usize::from(record[0]);
            if remap > 1 {
                return Err(format!("{}: unsupported palette remap {remap}", self.id));
            }
            for i in 0..8 - remap {
                self.palette_words[1][1 + remap * 8 + i] = word(
                    rom,
                    0x42da2 + usize::from(record[1]) * 16 + remap * 2 + i * 2,
                )?;
            }
        }
        Ok(())
    }

    /// `$2DF4`, runtime flags bit 5: horse (`$2E9A`) and surf (`$2F1E`, not
    /// part 2) draw byte maps into plane A instead of the metatile layout.
    fn load_plane_a_overlay(&mut self, rom: &[u8], plan: &StagePlan) -> Result<()> {
        if self.flags.0 & 0x20 == 0 || plan.overlays.is_empty() {
            return Ok(());
        }
        let mut table = NameTable::new(PLANE_A_VRAM);
        for overlay in &plan.overlays {
            let (bytes, _) = musashi(rom, overlay.source, LIMIT)?;
            let (width, height) = (overlay.width, overlay.height);
            if bytes.len() < width * height {
                return Err("short plane A overlay".into());
            }
            for &address in &overlay.placements {
                for row in 0..height {
                    for col in 0..width {
                        table.write(
                            address + (row * 128 + col * 2) as u32,
                            u16::from(bytes[row * width + col]).wrapping_add(overlay.base),
                        )?;
                    }
                }
            }
        }
        self.plane_a_overlay = Some(table);
        Ok(())
    }

    /// Layout size in pixels.
    pub fn pixel_size(&self) -> (i32, i32) {
        let lines = (self.layout.len() / 32 * 16) as i32;
        match self.orientation {
            Orientation::Horizontal => (lines, 512),
            Orientation::Vertical => (512, lines),
        }
    }
    pub fn metatile_at(&self, col: i32, row: i32) -> Option<u8> {
        self.layout_index(col, row).map(|index| self.layout[index])
    }
    pub fn upload_range(&self, kind: UploadKind) -> Range<usize> {
        let mut uploads = self.uploads.iter().filter(|u| u.kind == kind);
        let Some(first) = uploads.next() else {
            return 0..0;
        };
        uploads.fold(
            first.first_tile..first.first_tile + first.tile_count,
            |r, u| r.start.min(u.first_tile)..r.end.max(u.first_tile + u.tile_count),
        )
    }

    pub fn report(&self) -> String {
        let s = &self.section;
        let mut out = format!(
            "US ROM CRC32 5381506f - round {}\nPart record ${:06x}: {:04x?}\n",
            self.id, s.record, s.raw
        );
        out += &format!(
            "Parts: {}; special mode {:?}; mode flags ${:04x}; runtime flags ${:04x} ({:?}, VDP $0B={:02x}); autoscroll ${:04x} velocity {:?}\n",
            self.part_count,
            self.special,
            self.mode.0,
            self.flags.0,
            self.orientation,
            self.flags.vdp_mode_3(),
            self.autoscroll.0,
            self.autoscroll.velocity()
        );
        out += &format!(
            "Camera start {:?}, player start {:?}, exit {:?}, bounds {:?}\n",
            s.camera_start, s.player_start, s.exit, self.camera
        );
        let (w, h) = self.pixel_size();
        out += &format!(
            "Layout {:?}: {w}x{h} pixels; metatiles ${:06x}..${:06x} ({}); collision ${:06x}..${:06x}\n",
            self.layout_source,
            self.metatile_range.start,
            self.metatile_range.end,
            self.metatiles.len(),
            self.collision_range.start,
            self.collision_range.end
        );
        out += &format!(
            "Routines: scroll ${:04x}, plane B ${:04x}, palette ${:04x}, animation ${:06x}, scenery ${:06x}\n",
            self.routines.scroll,
            self.routines.plane_b,
            self.routines.palette,
            self.routines.animation,
            self.routines.scenery
        );
        out += &format!(
            "Palettes ${:06x}: IDs {:02x?}\n",
            self.palette_source, self.palette_ids
        );
        for u in &self.uploads {
            out += &format!(
                "{:?} {}: ROM ${:06x}..${:06x}, tiles ${:03x}..${:03x} ({}), remap={}, missing bytes={}\n",
                u.kind,
                u.name,
                u.source,
                u.end,
                u.first_tile,
                u.first_tile + u.tile_count,
                u.tile_count,
                u.remapped,
                u.missing_bytes
            );
        }
        for m in &self.plane_b.maps {
            out += &format!(
                "Plane B map ${:06x} {:?}: {}x{} stream ${:06x}..${:06x}, {} placements\n",
                m.descriptor,
                m.data,
                m.width,
                m.height,
                m.stream,
                m.end,
                m.placements.len()
            );
        }
        if let Some(note) = self.plane_b.unmodeled {
            out += &format!("Plane B: {note}\n");
        }
        match &self.boss_planes {
            Some(BossPlanes::Hydra {
                plane_a, plane_b, ..
            }) => {
                out += &format!(
                    "Boss planes: 3-3 corridor {} cells, Hydra head {} cells\n",
                    plane_a.cells.iter().flatten().count(),
                    plane_b.cells.iter().flatten().count()
                );
            }
            Some(BossPlanes::Mech { plane_b, .. }) => {
                out += &format!(
                    "Boss planes: 5-3 mech body {} cells, post-fade palettes\n",
                    plane_b.cells.iter().flatten().count()
                );
            }
            Some(BossPlanes::Hull { plane_a }) => {
                out += &format!(
                    "Boss planes: 4-3 gunboat hull {} cells on a cleared plane A\n",
                    plane_a.cells.iter().flatten().count()
                );
            }
            None => {}
        }
        out += &format!(
            "Object records {:04x?}; {} spawn records\n",
            self.object_records,
            self.spawns.len()
        );
        out
    }
}
