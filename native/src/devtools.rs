//! Native developer tools behind the desktop F1 panel: memory and disk save
//! states, frame control, stage warps and cheats. None of this is in the
//! cartridge; it only drives `Modes` and `Game` through their own state, so
//! it stays independent of SDL and the panel that shows it.

use crate::{game::Game, modes::Modes, stage::StageId};
use std::path::{Path, PathBuf};
pub(crate) mod replay;
use replay::{Event, History, Reader};
pub const THUMB_WIDTH: usize = 160;
pub const THUMB_HEIGHT: usize = 112;

/// Save-state slots.
pub const SLOTS: usize = 8;
/// Simulation speeds offered by the panel, as ticks per 60 Hz tick.
pub const SPEEDS: [f32; 6] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0];
const NORMAL_SPEED: usize = 2;
/// Lives kept by the infinite-lives cheat (the HUD shows one digit).
const LOCKED_LIVES: u8 = 9;

/// A copy of the gameplay taken by `DevTools::save`.
pub struct SaveState {
    game: Option<Box<Game>>,
    stage: StageId,
    history: Option<History>,
    checksum: u32,
    pub name: String,
    pub thumbnail: Vec<u8>,
    /// Desktop ticks run when it was taken.
    pub tick: u64,
}
impl SaveState {
    pub fn stage(&self) -> StageId {
        self.stage
    }
    pub fn frame_checksum(&self) -> u32 {
        self.checksum
    }
}

/// A substage the ROM defines, and how many parts it streams.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Substage {
    pub id: StageId,
    pub parts: u8,
}

pub struct DevTools {
    rom: Vec<u8>,
    pub stages: Vec<Substage>,
    pub slots: [Option<SaveState>; SLOTS],
    pub slot: usize,
    /// Freeze the simulation; `step` still advances it.
    pub paused: bool,
    steps: usize,
    /// Index into `SPEEDS`.
    pub speed: usize,
    carry: f32,
    /// Keep the spare lives at `LOCKED_LIVES`.
    pub infinite_lives: bool,
    /// Keep health at its maximum (falls and crushes still kill).
    pub lock_health: bool,
    /// The outcome of the last action, for the panel.
    pub status: String,
    save_dir: Option<PathBuf>,
}

impl DevTools {
    pub fn new(rom: &[u8]) -> Self {
        let stages = (1..=7)
            .flat_map(|round| (1..=3).map(move |substage| StageId::new(round, substage, 0)))
            .filter_map(|id| {
                let sections =
                    crate::stage::sections(rom, id.round_index(), id.substage_index()).ok()?;
                Some(Substage {
                    id,
                    parts: u8::try_from(sections.len()).ok()?,
                })
            })
            .collect();
        Self {
            rom: rom.to_vec(),
            stages,
            slots: Default::default(),
            slot: 0,
            paused: false,
            steps: 0,
            speed: NORMAL_SPEED,
            carry: 0.0,
            infinite_lives: false,
            lock_health: false,
            status: String::new(),
            save_dir: None,
        }
    }

    /// How many simulation ticks to run for `real` elapsed 60 Hz ticks,
    /// after the pause, single steps and the speed.
    pub fn scale_ticks(&mut self, real: usize) -> usize {
        if self.paused {
            self.carry = 0.0;
            return std::mem::take(&mut self.steps);
        }
        self.steps = 0;
        self.carry += real as f32 * SPEEDS[self.speed];
        let ticks = self.carry.floor();
        self.carry -= ticks;
        ticks as usize
    }
    pub fn step(&mut self) {
        self.paused = true;
        self.steps += 1;
    }

    pub fn before_tick(&self, modes: &mut Modes) {
        if self.speed != NORMAL_SPEED || self.paused || self.infinite_lives || self.lock_health {
            modes.mark_practice();
        }
    }

    /// Cheats that hold a value, applied after every simulation tick.
    pub fn after_tick(&self, modes: &mut Modes) {
        if self.speed != NORMAL_SPEED || self.paused || self.infinite_lives || self.lock_health {
            modes.mark_practice();
        }
        if self.infinite_lives || self.lock_health {
            modes.record(Event::Locks(self.infinite_lives, self.lock_health));
        }
        let Some(game) = modes.game_mut() else {
            return;
        };
        let hud = &mut game.combat.hud;
        if self.infinite_lives {
            hud.lives = hud.lives.max(LOCKED_LIVES);
        }
        if self.lock_health && hud.health > 0 {
            hud.health = hud.max_health;
        }
    }

    pub fn save(&mut self, modes: &Modes, tick: u64) {
        if modes.game().is_some_and(|g| g.practice.is_some()) {
            self.status = "save: unavailable during practice".into();
            return;
        }
        let name = self.slots[self.slot].as_ref().map_or_else(
            || format!("Slot {}", self.slot + 1),
            |state| state.name.clone(),
        );
        let Some(state) = SaveState::capture(modes, tick, name) else {
            self.status = "save: no game running".into();
            return;
        };
        self.status = format!("saved slot {} ({})", self.slot + 1, state.stage);
        self.slots[self.slot] = Some(state);
        self.persist();
    }

    /// Install the slot's gameplay as the running mode, dropping any pause
    /// or transition screen in front of it.
    pub fn load(&mut self, modes: &mut Modes) {
        let Some(state) = &self.slots[self.slot] else {
            self.status = format!("load: slot {} is empty", self.slot + 1);
            return;
        };
        let restored = match (&state.game, &state.history) {
            (Some(game), _) => Ok((game.clone(), state.history.clone())),
            (None, Some(history)) => history
                .restore_checked(&self.rom, state.stage, state.checksum)
                .map(|(game, history)| (game, Some(history))),
            _ => Err("save has no gameplay history".into()),
        };
        let (mut game, history) = match restored {
            Ok(game) => game,
            Err(error) => {
                self.status = format!("load: {error}");
                return;
            }
        };
        modes.history = history;
        game.restart_music();
        self.status = format!("loaded slot {} ({})", self.slot + 1, game.stage_id);
        modes.replace(game);
        modes.loaded_profile_run();
        modes.record(Event::Resume);
    }

    pub fn clear(&mut self) {
        if let Some(dir) = &self.save_dir {
            let path = dir.join(format!("slot-{}.nms", self.slot + 1));
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                self.status = format!("clear: {error}");
                return;
            }
        }
        self.slots[self.slot] = None;
        self.status = format!("cleared slot {}", self.slot + 1);
    }

    /// Raise the next-substage request, as a finished boss or the final
    /// stream exit does: ROUND CLEAR and the round card follow.
    pub fn skip_substage(&mut self, modes: &mut Modes) {
        modes.mark_practice();
        modes.record(Event::Skip);
        match modes.game_mut() {
            Some(game) => {
                game.request_next_substage();
                self.status = format!("skipping {}", game.stage_id);
            }
            None => self.status = "skip: no game running".into(),
        }
    }

    /// Start `id` directly. A running game keeps its score, lives, weapons
    /// and options; from the front end a new NORMAL game starts.
    pub fn warp(&mut self, modes: &mut Modes, id: StageId) {
        modes.mark_practice();
        let loaded = match modes.game() {
            Some(game) => game.load_next(id).map(|mut next| {
                next.restart_music();
                next
            }),
            None => Game::load_stage(&self.rom, id).map(|mut game| {
                game.start_session(crate::options::GameOptions::default());
                game
            }),
        };
        match loaded {
            Ok(game) => {
                modes.record(Event::Warp(id));
                self.status = format!("warped to {id}");
                modes.replace(Box::new(game));
                modes.loaded_profile_run();
            }
            Err(error) => self.status = format!("warp {id}: {error}"),
        }
    }

    /// The running game's POW power-up (`plr_status` bit 7, #70).
    pub fn powered(&self, modes: &Modes) -> Option<bool> {
        modes.game().map(|game| game.combat.weapons.powered)
    }
    /// Set or clear the power-up as the POW item or a hit would.
    pub fn set_powered(&mut self, modes: &mut Modes, powered: bool) {
        modes.mark_practice();
        modes.record(Event::Powered(powered));
        let Some(game) = modes.game_mut() else {
            return;
        };
        game.combat.weapons.powered = powered;
        self.status = if powered {
            "powered up"
        } else {
            "powered down"
        }
        .into();
    }

    pub fn refill(&mut self, modes: &mut Modes) {
        modes.mark_practice();
        modes.record(Event::Refill);
        let Some(game) = modes.game_mut() else {
            return;
        };
        let hud = &mut game.combat.hud;
        hud.health = hud.max_health;
        game.combat.weapons.refill_ninjutsu();
        self.status = "health and ninjutsu refilled".into();
    }

    pub fn set_invincible(&self, modes: &mut Modes, value: bool) {
        modes.mark_practice();
        modes.record(Event::Invincible(value));
        if let Some(game) = modes.game_mut() {
            game.options.invincible = value;
        }
    }
    pub fn set_infinite(&self, modes: &mut Modes, value: bool) {
        modes.mark_practice();
        modes.record(Event::Infinite(value));
        if let Some(game) = modes.game_mut() {
            game.options.infinite = value;
        }
    }

    /// Begin a desktop recording and read disk-slot metadata. Games are replayed
    /// only when Load is explicitly clicked; opening the panel never replays.
    pub fn enable_disk_saves(
        &mut self,
        modes: &mut Modes,
        effects: [bool; 4],
        dir: Option<PathBuf>,
    ) {
        modes.history = Some(History {
            boot: modes.game().is_none(),
            stage: modes.game().map_or(StageId::FOREST, |g| g.stage_id),
            effects,
            events: Vec::new(),
        });
        if let Some(game) = modes.game() {
            modes.record(Event::Options(game.options));
        }
        self.save_dir = dir;
        if let Some(dir) = &self.save_dir {
            let mut errors = Vec::new();
            for slot in 0..SLOTS {
                let path = dir.join(format!("slot-{}.nms", slot + 1));
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        match SaveState::decode(&bytes, crate::rom::regions::checksum(&self.rom)) {
                            Ok(state) => self.slots[slot] = Some(state),
                            Err(error) => errors.push(format!("slot {}: {error}", slot + 1)),
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => errors.push(format!("slot {}: {error}", slot + 1)),
                }
            }
            self.status = errors.join("; ");
        }
    }

    pub fn disk_saves_enabled(&self) -> bool {
        self.save_dir.is_some()
    }

    pub fn rename(&mut self, name: &str) {
        if let Some(state) = &mut self.slots[self.slot] {
            state.name = name.chars().filter(|c| !c.is_control()).take(64).collect();
            self.persist();
        }
    }

    fn persist(&mut self) {
        let (Some(dir), Some(state)) = (&self.save_dir, &self.slots[self.slot]) else {
            return;
        };
        let result = state.write(dir, self.slot, crate::rom::regions::checksum(&self.rom));
        if let Err(error) = result {
            self.status = format!("saved in memory; disk write failed: {error}");
        }
    }
}

impl SaveState {
    pub(crate) fn capture(modes: &Modes, tick: u64, name: String) -> Option<Self> {
        let game = modes.game()?;
        if game.practice.is_some() {
            return None;
        }
        let mut frame = crate::render::Frame::default();
        game.draw(&mut frame);
        let pixels = frame.rgb();
        let thumbnail = (0..THUMB_HEIGHT)
            .flat_map(|y| {
                (0..THUMB_WIDTH).flat_map(move |x| {
                    let at = ((y * 2) * crate::WIDTH + x * 2) * 3;
                    pixels[at..at + 3].iter().copied()
                })
            })
            .collect();
        Some(Self {
            game: Some(Box::new(game.clone())),
            stage: game.stage_id,
            history: modes.history.clone(),
            checksum: crate::assets::crc32(pixels),
            name,
            thumbnail,
            tick,
        })
    }
    pub(crate) fn restore(&self, rom: &[u8]) -> Result<(Box<Game>, History), String> {
        let history = self.history.as_ref().ok_or("no replay history")?;
        if let Some(game) = &self.game {
            return Ok((game.clone(), history.clone()));
        }
        history.restore_checked(rom, self.stage, self.checksum)
    }
    fn write(&self, dir: &Path, slot: usize, rom_crc: u32) -> Result<(), String> {
        self.write_path(&dir.join(format!("slot-{}.nms", slot + 1)), rom_crc)
    }
    pub(crate) fn write_path(&self, path: &Path, rom_crc: u32) -> Result<(), String> {
        let history = self.history.as_ref().ok_or("no replay history")?;
        let mut out = b"NMSAVE02".to_vec();
        out.extend(rom_crc.to_le_bytes());
        out.extend(self.tick.to_le_bytes());
        out.extend(self.checksum.to_le_bytes());
        out.extend([self.stage.round, self.stage.substage, self.stage.part]);
        out.extend((self.name.len() as u16).to_le_bytes());
        out.extend(self.name.as_bytes());
        out.extend(&self.thumbnail);
        history.encode(&mut out);
        let crc = crate::assets::crc32(&out);
        out.extend(crc.to_le_bytes());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, out).map_err(|e| e.to_string())?;
        std::fs::rename(temporary, path).map_err(|e| e.to_string())
    }
    pub(crate) fn decode(bytes: &[u8], rom_crc: u32) -> Result<Self, String> {
        if bytes.len() < 12 || bytes.len() > 16_000_000 {
            return Err("invalid save size".into());
        }
        let (payload, crc) = bytes.split_at(bytes.len() - 4);
        if crate::assets::crc32(payload) != u32::from_le_bytes(crc.try_into().unwrap()) {
            return Err("save checksum failed".into());
        }
        let mut reader = Reader { bytes: payload };
        let version = reader.take(8)?;
        if !matches!(version, b"NMSAVE01" | b"NMSAVE02") || reader.u32()? != rom_crc {
            return Err("save version or ROM mismatch".into());
        }
        let tick = u64::from_le_bytes(reader.take(8)?.try_into().unwrap());
        let checksum = reader.u32()?;
        let stage = reader.stage()?;
        let length = reader.u16()? as usize;
        if length > 256 {
            return Err("save name is too long".into());
        }
        let name = String::from_utf8(reader.take(length)?.to_vec()).map_err(|e| e.to_string())?;
        let thumbnail = reader.take(THUMB_WIDTH * THUMB_HEIGHT * 3)?.to_vec();
        let history = History::decode(&mut reader)?;
        if !reader.bytes.is_empty() {
            return Err("unexpected saved data".into());
        }
        Ok(Self {
            game: None,
            stage,
            history: Some(history),
            checksum,
            name,
            thumbnail,
            tick,
        })
    }
}
