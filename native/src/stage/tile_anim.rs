//! The `$39602` tile animation routines as data.
use super::{LIMIT, data::TILE_SLOTS};
use crate::assets::musashi;
use crate::render::Tile;
use crate::rom::{Result, pointer, word};

/// `Vegetation_Buffer`: the tile animation routines decode into `$FF5F10`.
const ANIMATION_BUFFER: usize = 0xff5f10;

/// When a tile animation channel transfers, by routine tick (1 = the first
/// `$390E2` call after `$390AE` installed the routine).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    /// From tick `first`: frame `(start + (tick - base) / period) % frames`.
    Ticks {
        first: u64,
        base: u64,
        period: u64,
        start: usize,
    },
    /// `$395AC`: frame `$FF5F06 & 7`, the negated quarter background
    /// position `$205A` stores (`floor(-X/4)`).
    QuarterScroll { first: u64 },
}

/// One table of six-byte records (RAM source long, destination tile word);
/// each transfer copies `patterns` tiles.
#[derive(Clone, Debug)]
pub struct TileChannel {
    pub table: usize,
    pub frames: Vec<(usize, Vec<Tile>)>,
    pub clock: Clock,
}
impl TileChannel {
    fn load(
        rom: &[u8],
        buffer: &[u8],
        table: usize,
        count: usize,
        patterns: usize,
        clock: Clock,
    ) -> Result<Self> {
        let mut frames = Vec::new();
        for i in 0..count {
            let record = table + i * 6;
            let offset = pointer(rom, record)?
                .checked_sub(ANIMATION_BUFFER)
                .ok_or("animation source outside buffer")?;
            let dest = usize::from(word(rom, record + 4)?);
            if dest + patterns > TILE_SLOTS {
                return Err("invalid animation destination".into());
            }
            let bytes = buffer
                .get(offset..offset + patterns * 32)
                .ok_or("truncated animation patch")?;
            frames.push((
                dest,
                bytes.as_chunks::<32>().0.iter().map(|&b| Tile(b)).collect(),
            ));
        }
        Ok(Self {
            table,
            frames,
            clock,
        })
    }
    pub fn frame(&self, tick: u64, background_x: i32) -> Option<usize> {
        let index = match self.clock {
            Clock::Ticks {
                first,
                base,
                period,
                start,
            } => {
                if tick < first {
                    return None;
                }
                start + (tick.saturating_sub(base) / period) as usize
            }
            Clock::QuarterScroll { first } => {
                if tick < first {
                    return None;
                }
                (-background_x).div_euclid(4).rem_euclid(8) as usize
            }
        };
        Some(index % self.frames.len())
    }
    fn transfer(&self, index: usize, tiles: &mut [Tile]) {
        let (dest, patch) = &self.frames[index];
        if let Some(target) = tiles.get_mut(*dest..*dest + patch.len()) {
            target.clone_from_slice(patch);
        }
    }
}

/// A `$39602` tile animation routine as data: Musashi streams decoded into
/// the `$FF5F10` buffer and the record tables its transfer code walks.
#[derive(Clone, Debug)]
pub struct TileAnimation {
    pub routine: usize,
    pub channels: Vec<TileChannel>,
    /// `$391BC`: every frame is transferred once, descending, at this tick.
    pub prime: Option<u64>,
    /// `$391DA` leaves its state when it sees the `$FF0199` event flag;
    /// `$3922C` decodes other sources and `$3926E` runs these channels,
    /// clocked from that call (tick 0).
    pub event_channels: Vec<TileChannel>,
    tick: u64,
    event_tick: Option<u64>,
}
impl TileAnimation {
    /// `None` for `$390E0` (`rts`).
    pub fn load(rom: &[u8], routine: usize, part: usize) -> Result<Option<Self>> {
        let mut buffer = Vec::new();
        let mut event_buffer = Vec::new();
        let decode = |buffer: &mut Vec<u8>, root: usize, entry: usize, offset: usize| {
            let source = pointer(rom, pointer(rom, root)? + entry)?;
            let (bytes, _) = musashi(rom, source, LIMIT)?;
            if buffer.len() < offset + bytes.len() {
                buffer.resize(offset + bytes.len(), 0);
            }
            buffer[offset..offset + bytes.len()].copy_from_slice(&bytes);
            Result::Ok(())
        };
        let ticks = |first: u64, base: u64, period: u64, start: usize| Clock::Ticks {
            first,
            base,
            period,
            start,
        };
        let mut channels = Vec::new();
        let mut event_channels = Vec::new();
        let mut prime = None;
        match routine {
            0x390e0 => return Ok(None),
            // 1-1: $39112 decodes, $391BC primes all twenty records on the
            // second call, then $391DA advances one record every fourth call.
            0x390f4 => {
                decode(&mut buffer, 0x80010, 0x0c, 0)?;
                prime = Some(2);
                channels.push(TileChannel::load(
                    rom,
                    &buffer,
                    0x39144,
                    20,
                    4,
                    ticks(6, 2, 4, 0),
                )?);
                decode(&mut event_buffer, 0x80010, 0x74, 0)?;
                event_channels.push(TileChannel::load(
                    rom,
                    &event_buffer,
                    0x39256,
                    4,
                    4,
                    ticks(2, 1, 8, 0),
                )?);
            }
            // 4-2: $392B0 decodes; $3935A transfers five tables every
            // sixteenth call from the second, frame `$FF5F03 & 3`.
            0x3929e => {
                decode(&mut buffer, 0x80010, 0x3c, 0)?;
                for (table, patterns) in [
                    (0x392e2, 4),
                    (0x392fa, 4),
                    (0x39312, 4),
                    (0x3932a, 2),
                    (0x39342, 2),
                ] {
                    channels.push(TileChannel::load(
                        rom,
                        &buffer,
                        table,
                        4,
                        patterns,
                        ticks(2, 2, 16, 1),
                    )?);
                }
            }
            // 3-1 (the 2-3 slot is a boss arena): four decodes, then $39524.
            0x393ca => {
                for (root, entry, offset) in [
                    (0x80010, 0x5c, 0),
                    (0x80010, 0x6c, 0x400),
                    (0x80014, 0x2c, 0xb00),
                    (0x80010, 0x1c, 0xd00),
                ] {
                    decode(&mut buffer, root, entry, offset)?;
                }
                channels.push(if part == 1 {
                    // $395D4: 24 patterns alternate every call.
                    TileChannel::load(rom, &buffer, 0x39518, 2, 24, ticks(5, 4, 1, 0))?
                } else {
                    TileChannel::load(rom, &buffer, 0x39494, 8, 8, ticks(5, 4, 1, 0))?
                });
                // $39566: every ninth call, six frames.
                channels.push(TileChannel::load(
                    rom,
                    &buffer,
                    0x394c4,
                    6,
                    8,
                    ticks(5, 5, 9, 1),
                )?);
                channels.push(TileChannel::load(
                    rom,
                    &buffer,
                    0x394e8,
                    8,
                    2,
                    Clock::QuarterScroll { first: 5 },
                )?);
            }
            _ => {
                return Err(format!(
                    "tile animation routine ${routine:06x} is not interpreted"
                ));
            }
        }
        Ok(Some(Self {
            routine,
            channels,
            prime,
            event_channels,
            tick: 0,
            event_tick: None,
        }))
    }
    /// The `$391BC` priming: every frame once, descending, which leaves the
    /// first frame of records sharing a destination (1-1's even frames).
    pub fn prime(&self, tiles: &mut [Tile]) {
        for channel in &self.channels {
            for index in (0..channel.frames.len()).rev() {
                channel.transfer(index, tiles);
            }
        }
    }
    /// 4-2's `$FF5F03 & 3`: the step `$3935A` last transferred (0 before
    /// the first), which the idle 4-2 bouncers also read (`$29EC4`).
    pub fn step(&self) -> usize {
        self.channels
            .first()
            .and_then(|channel| channel.frame(self.tick, 0))
            .unwrap_or(0)
    }
    /// One `$390E2` call. `event_flag` is `$FF0199` bit 0; `background_x`
    /// approximates `$FFE010`.
    pub fn tick(&mut self, event_flag: bool, background_x: i32, tiles: &mut [Tile]) {
        self.tick += 1;
        if let Some(event) = &mut self.event_tick {
            *event += 1;
        } else if event_flag && !self.event_channels.is_empty() {
            self.event_tick = Some(0);
        }
        if let Some(event) = self.event_tick {
            for channel in &self.event_channels {
                if let Some(index) = channel.frame(event, background_x) {
                    channel.transfer(index, tiles);
                }
            }
            return;
        }
        if self.prime == Some(self.tick) {
            self.prime(tiles);
        }
        for channel in &self.channels {
            if let Some(index) = channel.frame(self.tick, background_x) {
                channel.transfer(index, tiles);
            }
        }
    }
}
