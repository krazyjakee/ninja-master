//! Original player animation, sprite mappings, DMA art and sound catalog.
use super::{PlayerAnimation, musashi, verified};
use crate::combat::object_visible;
use crate::render::{Flip, Frame, HwSprite, Lines, SatWriter, SpriteCell, Tile};
use crate::rom::{Result, SPRITE_MAPPING_COUNT, SPRITE_MAPPINGS, pointer, sprite_mapping, word};
use std::ops::Range;

const FRAME_TABLE: usize = 0x23ade;
const FRAME_COUNT: usize = (0x23e06 - FRAME_TABLE) / 4;

#[derive(Clone)]
struct PlayerTile {
    tile: Tile,
    x: i32,
    y: i32,
    palette: usize,
    flip: Flip,
}

#[derive(Clone)]
pub struct PlayerFrame {
    art_id: Option<usize>,
    pub mapping_id: usize,
    pub mapping_range: Range<usize>,
    pub transfer_range: Range<usize>,
    pub art_ranges: Vec<Range<usize>>,
    /// A frame whose DMA state must be retained before this frame's transfers.
    pub predecessor: Option<usize>,
    tiles: Vec<PlayerTile>,
    /// The mapping's pieces (hardware sprites) in SAT order; each holds a
    /// run of `tiles`.
    pieces: Vec<PlayerPiece>,
}

/// One mapping piece of a player frame: a hardware sprite.
#[derive(Clone)]
struct PlayerPiece {
    x: i32,
    y: i32,
    width: u8,
    height: u8,
    tiles: Range<usize>,
}
impl PlayerFrame {
    pub fn art_bounds(&self) -> (i32, i32, usize, usize) {
        let left = self.tiles.iter().map(|t| t.x).min().unwrap_or(0);
        let top = self.tiles.iter().map(|t| t.y).min().unwrap_or(0);
        let right = self.tiles.iter().map(|t| t.x + 8).max().unwrap_or(8);
        let bottom = self.tiles.iter().map(|t| t.y + 8).max().unwrap_or(8);
        (left, top, (right - left) as usize, (bottom - top) as usize)
    }
    /// Cells in an assembled template, retaining the mapping's own flips.
    pub(crate) fn art_cells(&self) -> impl Iterator<Item = (&Tile, i32, i32, usize, Flip)> {
        self.tiles
            .iter()
            .map(|t| (&t.tile, t.x, t.y, t.palette, t.flip))
    }
    /// Original player origin: horizontal center, feet. Palette bank zero is Joe.
    pub fn draw(&self, frame: &mut Frame, palettes: &Lines<'_>, x: i32, y: i32, flip_x: bool) {
        self.draw_on_line(frame, palettes, (x, y), flip_x, 0);
    }
    /// `draw` for a record whose tile word selects palette `line`, which
    /// `Object_DrawSprite` XORs into each piece (the 2-1 horse's `$2000`).
    pub fn draw_on_line(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        (x, y): (i32, i32),
        flip_x: bool,
        line: usize,
    ) {
        // `Object_DrawSprite` writes the player record like any object.
        if !object_visible((x, y)) {
            return;
        }
        let art = self.art_id.and_then(|id| {
            frame.art_pack().and_then(|pack| {
                pack.player_cells(id, self.tiles.iter().map(|t| (t.x, t.y, t.flip)))
            })
        });
        // The first SAT sprite wins overlaps: paint the pieces in reverse.
        for piece in self.pieces.iter().rev() {
            let width = i32::from(piece.width) * 8;
            let sprite = HwSprite {
                x: x + if flip_x { -piece.x - width } else { piece.x },
                y: y + piece.y,
                width: piece.width,
                height: piece.height,
                writer: SatWriter::Object,
            };
            let tiles = &self.tiles[piece.tiles.clone()];
            let Some(first) = tiles.first() else {
                continue;
            };
            let cells = tiles.iter().enumerate().map(|(offset, tile)| {
                (
                    SpriteCell {
                        tile: &tile.tile,
                        x: x + if flip_x { -tile.x - 8 } else { tile.x },
                        y: y + tile.y,
                        flip: Flip {
                            x: tile.flip.x ^ flip_x,
                            y: tile.flip.y,
                        },
                    },
                    art.as_ref()
                        .map(|art| art[piece.tiles.start + offset].clone()),
                )
            });
            frame.sprite_art(sprite, palettes[first.palette ^ line], cells);
        }
    }
}

#[derive(Clone)]
pub struct PlayerAssets {
    pub animations: &'static [PlayerAnimation; 76],
    pub frames: Vec<PlayerFrame>,
    /// [`OVERLAYS`], built at load.
    overlays: Vec<(u16, PlayerFrame)>,
}
impl PlayerAssets {
    pub const FRAME_COUNT: usize = FRAME_COUNT;
    pub fn load(rom: &[u8]) -> Result<Self> {
        verified(rom)?;
        // Always-resident player patterns ($228D6), followed by per-frame DMA.
        let resident = resident_vram(rom, 0)?;
        let frames = Self::frames(rom, &resident)?;
        let overlays = Self::overlays(rom, &resident)?;
        Ok(Self {
            animations: super::player_animation::load_sequences(rom)?,
            frames,
            overlays,
        })
    }
    /// Every frame descriptor's DMA over the resident patterns `base`.
    fn frames(rom: &[u8], base: &[Option<u8>]) -> Result<Vec<PlayerFrame>> {
        let mut frames = Vec::new();
        let mut vram = base.to_vec();
        // DMA writes since `vram` last matched `base`.
        let mut written: Vec<Range<usize>> = Vec::new();
        for id in 0..FRAME_COUNT {
            let entry = FRAME_TABLE + id * 4;
            let mapping_id = word(rom, entry)?;
            if mapping_id >= SPRITE_MAPPING_COUNT {
                return Err("invalid player mapping index".into());
            }
            let mapping = SPRITE_MAPPINGS
                + usize::from(word(rom, SPRITE_MAPPINGS + usize::from(mapping_id) * 2)?);
            // Sequences 18 and 37 enter frame 64 through 63; its transfer
            // intentionally retains tile $2c8 from that preceding frame.
            if id != 64 {
                for range in written.drain(..) {
                    vram[range.clone()].copy_from_slice(&base[range]);
                }
            }
            let predecessor = predecessor(id);
            if let Some(before) = predecessor.filter(|&before| before > id) {
                let (_, transfers) = transfers(rom, before)?;
                dma(rom, &transfers, &mut vram, &mut written)?;
            }
            let (transfer_range, transfers) = transfers(rom, id)?;
            dma(rom, &transfers, &mut vram, &mut written)?;
            let (tiles, pieces, mapping_end) = mapping_tiles(rom, id, mapping_id, &vram)?;
            frames.push(PlayerFrame {
                art_id: Some(id),
                mapping_id: usize::from(mapping_id),
                mapping_range: mapping..mapping_end,
                transfer_range,
                art_ranges: transfers.iter().map(|t| t.source.clone()).collect(),
                predecessor,
                tiles,
                pieces,
            });
        }
        Ok(frames)
    }
    /// Object mappings drawn from the player's DMA'd patterns ([`OVERLAYS`]):
    /// `mapping` as it shows while the player's VRAM holds frame `after`'s
    /// transfers over the ordinary resident patterns.
    pub fn overlay(&self, mapping: u16) -> Option<&PlayerFrame> {
        self.overlays
            .iter()
            .find(|(id, _)| *id == mapping)
            .map(|(_, frame)| frame)
    }
    fn overlays(rom: &[u8], base: &[Option<u8>]) -> Result<Vec<(u16, PlayerFrame)>> {
        let mut out = Vec::new();
        for &(mapping_id, after) in OVERLAYS {
            let mut vram = base.to_vec();
            let mut written = Vec::new();
            if let Some(before) = predecessor(after) {
                dma(rom, &transfers(rom, before)?.1, &mut vram, &mut written)?;
            }
            let (transfer_range, transfers) = transfers(rom, after)?;
            dma(rom, &transfers, &mut vram, &mut written)?;
            let mapping = SPRITE_MAPPINGS
                + usize::from(word(rom, SPRITE_MAPPINGS + usize::from(mapping_id) * 2)?);
            let (tiles, pieces, mapping_end) = mapping_tiles(rom, after, mapping_id, &vram)?;
            out.push((
                mapping_id,
                PlayerFrame {
                    art_id: None,
                    mapping_id: usize::from(mapping_id),
                    mapping_range: mapping..mapping_end,
                    transfer_range,
                    art_ranges: transfers.iter().map(|t| t.source.clone()).collect(),
                    predecessor: Some(after),
                    tiles,
                    pieces,
                },
            ));
        }
        Ok(out)
    }
    pub fn report(&self) -> String {
        let mut report = format!(
            "Player: {} animation states, {} frame descriptors\n",
            self.animations.len(),
            self.frames.len()
        );
        for (id, a) in self.animations.iter().enumerate() {
            report += &format!(
                "animation {id}: ${:06x}..${:06x}, speed {}/256, frames {:?}\n",
                a.source.start, a.source.end, a.speed, a.frames
            );
        }
        for (id, f) in self.frames.iter().enumerate() {
            report += &format!(
                "frame {id}: mapping ${:06x}..${:06x}, DMA ${:06x}..${:06x}",
                f.mapping_range.start,
                f.mapping_range.end,
                f.transfer_range.start,
                f.transfer_range.end
            );
            for range in &f.art_ranges {
                report += &format!(", art ${:06x}..${:06x}", range.start, range.end);
            }
            if let Some(id) = f.predecessor {
                report += &format!(", retains DMA state from frame {id}");
            }
            report.push('\n');
        }
        report
    }
}

/// Player VRAM holding only resident pattern set `entry` of root `$80000`
/// (0 ordinary, 1 powered), as `Player_PowerDown`/`Player_PowerUp` load it
/// through `Func_0046DA`; the per-frame DMA goes over it.
fn resident_vram(rom: &[u8], entry: usize) -> Result<Vec<Option<u8>>> {
    let source = pointer(rom, pointer(rom, 0x80000)? + entry * 4)?;
    let first = usize::from(word(rom, source)?);
    let count = usize::from(word(rom, source + 2)?) + 1;
    let bytes = musashi(rom, source + 4, 65536)?.0;
    let mut base = vec![None; 65536];
    if bytes.len() < count * 32 || (first + count) * 32 > base.len() {
        return Err("invalid resident player tile transfer".into());
    }
    for i in 0..count * 32 {
        base[first * 32 + i] = Some(bytes[i]);
    }
    Ok(base)
}

/// A frame whose DMA state a frame keeps under its own transfers. Sequences
/// 18 and 37 enter frame 64 through 63, which leaves tile `$2C8`. Frame 182
/// (`$B6`, the surf death's floating board, mapping `$451C0`) transfers
/// only to `$5600`, for the thrown rider ([`OVERLAYS`]); its own pieces at
/// tiles `$300..$318` are frame 191's (`$BF`) second transfer to `$6000`,
/// which slot 0 of the surf death leaves (frame 132, slot 1, writes only
/// `$5600`; Genesis Plus GX VRAM at 4-1 frame 625). State 30 (the surf
/// Mijin death) enters frame 182 from the technique's frames; native
/// assumes the same patterns there.
fn predecessor(id: usize) -> Option<usize> {
    match id {
        64 => Some(63),
        182 => Some(191),
        _ => None,
    }
}

/// Object mappings that draw on the player's DMA'd patterns with tile word
/// 0, and the player frame whose transfers they show: the rider thrown off
/// in a surf death (`Func_026CCE`, `special::thrown`) is the art that the
/// death's slot 1 (frame 132) and later slots (frame 182) load at `$5600`,
/// while the player's own frame 182 shows the board left behind.
pub const OVERLAYS: &[(u16, usize)] = &[(0x1d, 132), (0x1f, 182)];

/// One player DMA record: `count` bytes from ROM `source` to VRAM `destination`.
struct Transfer {
    source: Range<usize>,
    destination: usize,
}

/// Frame `id`'s DMA list: its table range and transfers.
fn transfers(rom: &[u8], id: usize) -> Result<(Range<usize>, Vec<Transfer>)> {
    let transfer = FRAME_TABLE + usize::from(word(rom, FRAME_TABLE + id * 4 + 2)?);
    let mut cursor = transfer;
    let mut out = Vec::new();
    loop {
        if !(0x23e06..0x24932).contains(&cursor) || cursor + 10 > 0x24932 {
            return Err("player DMA record outside transfer table".into());
        }
        // DMA count is little endian; source is a 23-bit word address.
        let count = usize::from(u16::from_le_bytes([rom[cursor], rom[cursor + 1]])) * 2;
        if rom[cursor + 2] != 0x97 {
            return Err("invalid player DMA register".into());
        }
        let source =
            ((usize::from(rom[cursor + 3]) << 16) | usize::from(word(rom, cursor + 4)?)) * 2;
        let destination = usize::from(word(rom, cursor + 6)?);
        // Lists can split inside a tile at source bank boundaries.
        // For example frame 30 writes 152 bytes then 1032 bytes at
        // $5600 and $5698; together these form 37 complete patterns.
        if count == 0 || destination % 2 != 0 || destination + count > 65536 {
            return Err("odd destination or oversized player DMA transfer".into());
        }
        if rom.get(source..source + count).is_none() {
            return Err("player DMA outside ROM".into());
        }
        out.push(Transfer {
            source: source..source + count,
            destination,
        });
        let last = word(rom, cursor + 8)? != 0;
        cursor += 10;
        if last {
            return Ok((transfer..cursor, out));
        }
    }
}

/// Apply `transfers` to `vram`, noting the written ranges.
fn dma(
    rom: &[u8],
    transfers: &[Transfer],
    vram: &mut [Option<u8>],
    written: &mut Vec<Range<usize>>,
) -> Result<()> {
    for t in transfers {
        let data = rom.get(t.source.clone()).ok_or("player DMA outside ROM")?;
        for (i, byte) in data.iter().enumerate() {
            vram[t.destination + i] = Some(*byte);
        }
        written.push(t.destination..t.destination + data.len());
    }
    Ok(())
}

/// The pieces of `mapping_id` over `vram`, and the end of its mapping
/// record (hitbox bytes, then six bytes per piece). `id` names the frame
/// in errors.
fn mapping_tiles(
    rom: &[u8],
    id: usize,
    mapping_id: u16,
    vram: &[Option<u8>],
) -> Result<(Vec<PlayerTile>, Vec<PlayerPiece>, usize)> {
    let mapping =
        SPRITE_MAPPINGS + usize::from(word(rom, SPRITE_MAPPINGS + usize::from(mapping_id) * 2)?);
    let (_, pieces) = sprite_mapping(rom, mapping_id)?;
    let (mut tiles, mut out) = (Vec::new(), Vec::new());
    for piece in &pieces {
        let (width, height) = (piece.width, piece.height);
        let first = tiles.len();
        let flip = Flip {
            x: piece.attr & 0x800 != 0,
            y: piece.attr & 0x1000 != 0,
        };
        for col in 0..width {
            for row in 0..height {
                let index = usize::from(piece.attr & 0x7ff) + col * height + row;
                // Frame zero is the intentionally blank mapping on tile zero.
                let tile = if index == 0 {
                    Tile([0; 32])
                } else {
                    let mut bytes = [0; 32];
                    for (i, byte) in bytes.iter_mut().enumerate() {
                        *byte = vram.get(index * 32 + i).and_then(|b| *b).ok_or_else(|| {
                            format!("player frame {id} missing tile ${index:x} byte {i}")
                        })?;
                    }
                    Tile(bytes)
                };
                tiles.push(PlayerTile {
                    tile,
                    palette: usize::from((piece.attr >> 13) & 3),
                    flip,
                    x: piece.x + (if flip.x { width - col - 1 } else { col }) as i32 * 8,
                    y: piece.y + (if flip.y { height - row - 1 } else { row }) as i32 * 8,
                });
            }
        }
        out.push(PlayerPiece {
            x: piece.x,
            y: piece.y,
            width: width as u8,
            height: height as u8,
            tiles: first..tiles.len(),
        });
    }
    Ok((tiles, out, mapping + 4 + pieces.len() * 6))
}

/// Sound headers and channel entry points; extraction does not synthesize audio.
#[derive(Debug)]
pub struct SoundCue {
    pub command: u8,
    pub header: Range<usize>,
    pub instruments: usize,
    pub channels: Vec<usize>,
}
pub struct SoundAssets {
    pub music: Vec<SoundCue>,
    pub effects: Vec<SoundCue>,
    pub z80_image: Range<usize>,
}
impl SoundAssets {
    pub fn load(rom: &[u8]) -> Result<Self> {
        verified(rom)?;
        let mut music = Vec::new();
        let mut effects = Vec::new();
        for (table, count, first, is_music) in
            [(0x6ce3c, 28, 0x81, true), (0x6cf2c, 60, 0x9d, false)]
        {
            for i in 0..count {
                let start = pointer(rom, table + i * 4)?;
                let mut cursor = start + if is_music { 6 } else { 4 };
                let instruments = start + usize::from(word(rom, start)?);
                let mut channels = Vec::new();
                if is_music {
                    for _ in 0..*rom.get(start + 2).ok_or("short music header")? {
                        channels.push(start + usize::from(word(rom, cursor)?));
                        cursor += 4;
                    }
                    for _ in 0..*rom.get(start + 3).ok_or("short music header")? {
                        channels.push(start + usize::from(word(rom, cursor)?));
                        cursor += 6;
                    }
                } else {
                    for _ in 0..*rom.get(start + 3).ok_or("short SFX header")? {
                        channels.push(start + usize::from(word(rom, cursor + 2)?));
                        cursor += 6;
                    }
                }
                if cursor > rom.len()
                    || instruments >= rom.len()
                    || channels.iter().any(|p| *p >= rom.len())
                {
                    return Err("sound header references outside ROM".into());
                }
                let cue = SoundCue {
                    command: first + i as u8,
                    header: start..cursor,
                    instruments,
                    channels,
                };
                if is_music {
                    music.push(cue);
                } else {
                    effects.push(cue);
                }
            }
        }
        Ok(Self {
            music,
            effects,
            z80_image: 0x6ad3e..0x6cc27,
        })
    }
    pub fn report(&self) -> String {
        let mut out = format!(
            "Sound Z80 image: ${:06x}..${:06x}\n",
            self.z80_image.start, self.z80_image.end
        );
        for (name, cues) in [("music", &self.music), ("SFX", &self.effects)] {
            for cue in cues {
                out += &format!(
                    "{name} ${:02x}: header ${:06x}..${:06x}, instruments ${:06x}, channels",
                    cue.command, cue.header.start, cue.header.end, cue.instruments
                );
                for channel in &cue.channels {
                    out += &format!(" ${channel:06x}");
                }
                out.push('\n');
            }
        }
        out
    }
}
