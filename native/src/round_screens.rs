//! Substage and round transitions (#49).
//!
//! `Mode_Game` state `$20` consumes the `$FF0029` bit 1 request, fades the
//! finished substage out (`$67A`) and then, as `$B010..$B1A0` decide:
//! - another substage of the round: load it (`Mode_StageLoad`) and fade in;
//! - a new round: `Mode_RoundClear` (`$F032`) tallies the bonuses into the
//!   score, `Mode_RoundCard` (`$BA9A`) names the round, then load and fade in;
//! - past round 7: `Mode_Ending` (`$B1A0`).
//!
//! `StageClear` runs that sequence as a temporary mode holding the finished
//! `Game` (its `Main_HandlerSaved`), and hands the next `Game` back through
//! `ModeChange::Replace`. `StageClear::round_card` is the card alone, as
//! `Mode_Game` state `$C` shows it when a game starts or continues (#44).
//! Round clear shows the defeated boss's ROM portrait before the tally.
//! The round card loads the ROM map and progress markers; its titles still
//! use the native font at double size.

use crate::{
    ending::Ending,
    front_end::GOLD,
    front_vdp::{FrontVdp, art, map_header, palette_set},
    game::Game,
    input::Input,
    modes::{Audio, Mode, ModeChange, any_button, held_game, held_game_mut, take_held_audio},
    render::{Fade, Frame, md_color},
    rom::{byte, pointer, sprite_mapping, word, words},
    sound_events::{SoundEvent, Table},
    stage::StageId,
    text::{PLANE_A, Style, TextBlock, blank_zeros},
    vdp::Vdp,
};

/// `$FF37EC` word bits the bonus rules read. They collect over a whole
/// round and are cleared after its tally (`$F276`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoundFlags {
    /// Bit 0: a shuriken was thrown (`$231CA`, `$2320E`, `$2324E`, `$232D8`).
    pub thrown: bool,
    /// Bit 1: a ninjutsu started (`$22EC8`).
    pub ninjutsu: bool,
    /// Bit 2: the player took damage (`$20400`, `$21E92`).
    pub damaged: bool,
}

/// `Func_00F5E2` (`$F5E2..$F6BA`), run as the round clear draws its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bonuses {
    /// Points still to add in tally order, as the round clear lists them:
    /// SHINOBI, NO NINJITSU, TECHNICAL, SPECIAL, PERFECT (`$FF37EE..$FF37F6`).
    pub values: [u32; 5],
    /// `$FF37ED` bit 3: the PERFECT row is shown (`$F2B2`, `$F33C`).
    pub perfect: bool,
}
impl Bonuses {
    /// - SHINOBI 30000 without a thrown shuriken;
    /// - NO NINJITSU 5000 per remaining use (`$FF37E6`) without a ninjutsu;
    /// - TECHNICAL 500 per point of health (`$FF37E9`);
    /// - SPECIAL 10000 at full health (`$FF37E9` = `$FF37E8`);
    /// - PERFECT 50000 without damage when the three conditional bonuses
    ///   above were all earned. (`$FF37EC` bit 8 would also refuse it, but
    ///   nothing in the ROM sets that bit.)
    pub fn new(flags: RoundFlags, uses: u8, health: u8, max_health: u8) -> Self {
        let mut missing = 3;
        let mut earned = |condition: bool, points: u32| {
            if condition {
                missing -= 1;
                points
            } else {
                0
            }
        };
        let shinobi = earned(!flags.thrown, 30_000);
        let no_ninjutsu = earned(!flags.ninjutsu, 5_000 * u32::from(uses));
        let special = earned(health == max_health, 10_000);
        let technical = 500 * u32::from(health);
        let perfect = !flags.damaged && missing == 0;
        Self {
            values: [
                shinobi,
                no_ninjutsu,
                technical,
                special,
                if perfect { 50_000 } else { 0 },
            ],
            perfect,
        }
    }
    pub fn total(&self) -> u32 {
        self.values.iter().sum()
    }
    /// `Func_00F488`: one tally frame moves up to 1000 points of the first
    /// bonus that is left into the score. Returns the points moved.
    pub fn tally_step(&mut self) -> u32 {
        let Some(value) = self.values.iter_mut().find(|value| **value > 0) else {
            return 0;
        };
        let step = (*value).min(TALLY_STEP);
        *value -= step;
        step
    }
}

/// `$F4A6`: `$10` BCD hundreds (1000 points) per tally frame.
pub const TALLY_STEP: u32 = 1000;
/// Waits measured in the 1-3 capture (`$FF0029` poked to 2), each ended
/// early by A/B/C/Start: `RoundClear_S0C` (46 -> 186), `S1C` (247 -> 366)
/// and `S24` (469 -> 606).
pub const TITLE_TICKS: u16 = 140;
pub const SCORES_TICKS: u16 = 119;
pub const RESULT_TICKS: u16 = 137;
/// `Mode_RoundCard` states `$0C..$20` ran 625 -> 1266 before the fade-out.
pub const CARD_TICKS: u16 = 641;
/// The round card fades with `$FF16E8` = 3: one step every fourth frame.
pub const CARD_FADE_INTERVAL: u32 = 4;
/// `Func_004A36` indices: ROUND CLEAR (`RoundClear_S04`) and STORM WIND
/// (`RoundCard_Init`).
const ROUND_CLEAR_MUSIC: u8 = 0x0a;
const ROUND_CARD_MUSIC: u8 = 0x09;
/// Text blocks: ROUND CLEAR (`$ECB6`, title font), the score table
/// (`$ECCC`, drawn by `Func_00F298`), "ROUND n" (`RoundCard_Titles`
/// `$BE70`) and the round names (`RoundCard_TitleTexts` `$EC84`).
const ROUND_CLEAR_TITLE: usize = 0xecb6;
const SCORE_TABLE: usize = 0xeccc;
const ROUND_NUMBERS: usize = 0xbe70;
const ROUND_NAMES: usize = 0xec84;
/// `RoundCard_S04/S08`: map art and two plane maps.
const CARD_ART: [usize; 4] = [0x3a1d4, 0xbe8c, 0xc1fc, 0xd372];
const CARD_MAP: usize = 0xe90e;
const CARD_LZ_MAP: usize = 0xe950;
const CARD_PALETTE_SET: usize = 9;
/// `RoundCard_S10`: three palette IDs per round, terminated by `$FF`.
const CARD_PALETTE_IDS: usize = 0xba54;
/// `RoundCard_S18`: the current round's highlighted marker palette.
const CARD_MARKER_PALETTE_IDS: usize = 0xba70;
const PALETTE_WORDS: usize = 0x42062;
const CARD_MARKERS: usize = 0xeb34;
const CARD_CAMERA: usize = 0xba7e;
const CARD_MARKER_MAPPING: u16 = 0x370;
/// `$F2D4`..`$F34C`: offsets of the numbers in the score table text.
const HIGH_SCORE_AT: usize = 16;
const SCORE_AT: usize = 64;
const BONUS_AT: [usize; 5] = [138, 186, 234, 282, 330];
/// Rows `$F2BC` drops when PERFECT is not shown.
const PERFECT_ROWS: usize = 2;

/// `Func_00F440`: six Kosinski portraits in the `$80000 + $2C` art table.
/// The map at `$F40E` places tiles `$200..$2FF` on palette line 2 in a
/// 16 x 16 square at (96, 56). `Func_00F418` selects palette set `$46`.
fn boss_portrait(rom: &[u8], round: usize) -> crate::rom::Result<Vdp> {
    if round >= 6 {
        return Err("round clear has portraits only for rounds 1..6".into());
    }
    let mut v = FrontVdp::new(0);
    v.kosinski_tiles(rom, art(rom, 0x2c, round * 4)?)?;
    let (address, base, width, height) = map_header(rom, 0xf40e)?;
    let cells: Vec<u16> = (0..width * height).map(|i| i as u16).collect();
    v.vdp.write_map(address, width, base, &cells);
    v.vdp.cram = palette_set(rom, 0x46)?;
    Ok(v.vdp)
}

/// `RoundCard_Init/S04/S08`: the map spans 64 cells across plane B, while
/// the small plane-A map and the marker sprites show progress through it.
fn round_card_map(rom: &[u8], round: usize) -> crate::rom::Result<Vdp> {
    if round >= 7 {
        return Err("round card has seven marker records".into());
    }
    let mut front = FrontVdp::new(0);
    for at in CARD_ART {
        front.lz_tiles(rom, at)?;
    }
    front.byte_map(rom, CARD_MAP)?;
    front.lz_byte_map(rom, CARD_LZ_MAP)?;
    let mut vdp = front.vdp;
    vdp.cram = palette_set(rom, CARD_PALETTE_SET)?;
    for line in 1..=3 {
        let id = usize::from(byte(rom, CARD_PALETTE_IDS + round * 4 + line - 1)?);
        let colors: [u16; 16] = words(rom, PALETTE_WORDS + id * 32)?;
        vdp.cram[line * 16..line * 16 + 16].copy_from_slice(&colors);
    }
    let marker_palette = usize::from(byte(rom, CARD_MARKER_PALETTE_IDS + round * 2)?);
    let colors: [u16; 16] = words(rom, PALETTE_WORDS + marker_palette * 32)?;
    vdp.cram[48..64].copy_from_slice(&colors);

    // Data_00BA7E: the destination camera for this round. The ROM pans to
    // it during the card; show that destination so the current marker stays
    // on screen even for rounds 5-7.
    let camera = i32::from(word(rom, CARD_CAMERA + round * 4)?);
    for line in &mut vdp.hscroll {
        line[1] = -camera as i16;
    }
    let pieces = sprite_mapping(rom, CARD_MARKER_MAPPING)?.1;
    let mut slot = 0;
    for marker in 0..=round {
        let at = CARD_MARKERS + marker * 8;
        let x = i32::from(word(rom, at)?) - camera;
        let y = i32::from(word(rom, at + 2)?);
        let base = word(rom, at + 4)?.wrapping_add(if marker == round { 0x2000 } else { 0 });
        for piece in &pieces {
            vdp.sprites[slot] = [
                ((y + piece.y + 128) & 511) as u16,
                (((piece.width - 1) << 10) | ((piece.height - 1) << 8) | (slot + 1)) as u16,
                base.wrapping_add(piece.attr),
                ((x + piece.x + 128) & 511) as u16,
            ];
            slot += 1;
        }
    }
    vdp.sprites[slot - 1][1] &= !0x7f;
    Ok(vdp)
}

/// $BCA8 pans one pixel per update; $BD98 redraws the map's four-frame
/// marker every eight updates. Sprite coordinates follow the same camera.
fn animate_card(vdp: &mut Vdp, rom: &[u8], round: usize, age: u32) {
    let destination = i32::from(word(rom, CARD_CAMERA + round * 4).unwrap_or(0));
    let camera = (age.saturating_sub(120) as i32).min(destination);
    for row in &mut vdp.hscroll {
        row[1] = -camera as i16;
    }
    let pieces = sprite_mapping(rom, CARD_MARKER_MAPPING)
        .map(|(_, p)| p.len())
        .unwrap_or(4);
    for (slot, sprite) in vdp
        .sprites
        .iter_mut()
        .take((round + 1) * pieces)
        .enumerate()
    {
        let marker = slot / pieces;
        let piece = slot % pieces;
        if let Ok((_, mappings)) = sprite_mapping(rom, CARD_MARKER_MAPPING) {
            let x = i32::from(word(rom, CARD_MARKERS + marker * 8).unwrap_or(0)) - camera;
            sprite[3] = ((x + mappings[piece].x + 128) & 511) as u16;
        }
    }
    if age.is_multiple_of(8) {
        let at = pointer(rom, 0xeb24 + (((age / 8) & 3) as usize * 4)).unwrap_or(0);
        if let Ok((address, base, width, height)) = map_header(rom, at) {
            for row in 0..height {
                for col in 0..width {
                    if let Ok(tile) = byte(rom, at + 10 + row * width + col) {
                        vdp.write_word(
                            address + row * 128 + col * 2,
                            base.wrapping_add(u16::from(tile)),
                        );
                    }
                }
            }
        }
    }
}

/// One step of the sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `Mode_Game` state `$24`: the finished substage fades out.
    FadeOut,
    /// `RoundClear_S00..S0C`: ROUND CLEAR over the defeated boss portrait.
    ClearTitle(u16),
    /// `RoundClear_S14..S1C`: the score table.
    ClearScores(u16),
    /// `RoundClear_S20`.
    ClearTally,
    /// `RoundClear_S24`.
    ClearResult(u16),
    /// `RoundClear_S28`.
    ClearFadeOut,
    /// `RoundCard_S0C..S20`, then `S24` fading out.
    Card(u16),
    CardFadeOut,
}

pub struct StageClear {
    /// `Main_HandlerSaved`: the finished gameplay.
    saved: Option<Box<dyn Mode>>,
    pub step: Step,
    fade: Fade,
    /// The substage `$20` moves to; `None` past the last round.
    next: Option<StageId>,
    bonuses: Bonuses,
    /// The ROUND CLEAR title and the score table.
    clear: [Option<TextBlock>; 2],
    /// Loaded only for round clear; the score table replaces the portrait.
    portrait: Option<Vdp>,
    /// "ROUND n" and the round name of `next`, read as the card begins.
    card: [Option<TextBlock>; 2],
    /// ROM planes, art, palette and markers for the progress map.
    card_map: Option<Vdp>,
    ticks: u32,
    audio: Audio,
    /// Only the round card: hand the held game back afterwards instead of
    /// loading `next`.
    resume: bool,
}
impl StageClear {
    /// `ModeChange::Suspend` constructor for `Mode_Game` state `$20`.
    pub fn suspend(mode: Box<dyn Mode>) -> Box<dyn Mode> {
        Box::new(Self::new(mode))
    }
    pub fn new(mode: Box<dyn Mode>) -> Self {
        let game = mode.game();
        let clear = [ROUND_CLEAR_TITLE, SCORE_TABLE]
            .map(|address| TextBlock::read(game?.rom(), address).ok());
        Self {
            next: game.and_then(Game::next_stage),
            saved: Some(mode),
            step: Step::FadeOut,
            fade: Fade::Out(0),
            bonuses: Bonuses::default(),
            clear,
            portrait: None,
            card: [None, None],
            card_map: None,
            ticks: 0,
            audio: Audio::default(),
            resume: false,
        }
    }
    /// `Mode_Game` state `$C` for a new game or a continue: `RoundCard_Init`
    /// runs because `Game_InitLives` cleared `Game_Requests` bit 5. The held
    /// game, already loaded, then fades in with its stage music.
    pub fn round_card(mode: Box<dyn Mode>) -> Box<dyn Mode> {
        let mut card = Self::new(mode);
        card.next = held_game(&card.saved).map(|game| game.stage_id);
        card.resume = true;
        card.begin(Step::Card(CARD_TICKS));
        Box::new(card)
    }
    pub fn bonuses(&self) -> Bonuses {
        self.bonuses
    }
    fn music(&mut self, index: u8) {
        self.audio.request(
            held_game(&self.saved).and_then(|g| Table::Music.command(g.rom(), index).ok()),
        );
    }
    fn begin(&mut self, step: Step) {
        self.step = step;
        // The title stays up while the score table appears.
        self.fade = match step {
            Step::FadeOut | Step::ClearFadeOut | Step::CardFadeOut => Fade::Out(0),
            Step::ClearTitle(_) | Step::Card(_) => Fade::In(0),
            _ => self.fade,
        };
        match step {
            Step::ClearTitle(_) => {
                self.portrait = held_game(&self.saved)
                    .and_then(|g| boss_portrait(g.rom(), g.stage_id.round_index()).ok());
                self.music(ROUND_CLEAR_MUSIC);
            }
            // Func_00F298 computes the bonuses as it draws the table.
            Step::ClearScores(_) => {
                if let Some(bonuses) = held_game(&self.saved).map(Game::bonuses) {
                    self.bonuses = bonuses;
                }
            }
            Step::Card(_) => {
                self.music(ROUND_CARD_MUSIC);
                // RoundCard_Titles and RoundCard_TitleTexts: pointers per round.
                let rom = held_game(&self.saved).map(Game::rom);
                let round = self.next.map(StageId::round_index);
                self.card = [ROUND_NUMBERS, ROUND_NAMES].map(|table| {
                    let address = pointer(rom?, table + round? * 4).ok()?;
                    TextBlock::read(rom?, address).ok()
                });
                self.card_map = rom.zip(round).and_then(|(rom, round)| {
                    let mut map = round_card_map(rom, round).ok()?;
                    animate_card(&mut map, rom, round, 0);
                    Some(map)
                });
            }
            _ => {}
        }
    }
    /// After the fade-out: the next substage, the round clear or the ending.
    fn after_fade_out(&mut self) -> Option<ModeChange> {
        let game = held_game(&self.saved)?;
        match self.next {
            None => Some(ModeChange::Replace(Box::new(Ending::new(game)))),
            Some(next) if next.round != game.stage_id.round => {
                self.begin(Step::ClearTitle(TITLE_TICKS));
                None
            }
            Some(next) => Some(self.load(next)),
        }
    }
    /// `Mode_StageLoad` and state `$14`: the next substage fades in while
    /// it plays. A load failure ends the session in the ending.
    fn load(&self, next: StageId) -> ModeChange {
        let game = held_game(&self.saved).expect("StageClear holds the finished game");
        match game.load_next(next) {
            Ok(mut next) => {
                next.fade_in();
                ModeChange::Replace(Box::new(next))
            }
            Err(_) => ModeChange::Replace(Box::new(Ending::new(game))),
        }
    }
    fn draw_round_clear(&self, frame: &mut Frame, game: &Game) {
        if matches!(self.step, Step::ClearTitle(_))
            && let Some(portrait) = &self.portrait
        {
            portrait.render(frame, Fade::Out(0));
        }
        let font = game.font();
        let [title, table] = &self.clear;
        if let Some(title) = title {
            // Func_0041AC's 2x2-cell characters start at the header's cell.
            let (col, row) = title.cell(PLANE_A);
            let gold = Style::plain(md_color(GOLD), 2);
            font.draw_cells(frame, col * 8, row * 8, title.width, &title.text, gold);
        }
        if matches!(self.step, Step::ClearTitle(_)) {
            return;
        }
        let Some(mut table) = table.clone() else {
            return;
        };
        let mut put = |at: usize, text: &str| {
            if let Some(slot) = table.text.get_mut(at..at + text.len()) {
                slot.copy_from_slice(text.as_bytes());
            }
        };
        // Func_00F298 / Func_00F35A: eight-digit scores; bonuses in four BCD
        // digits of hundreds before the table's own "00".
        put(
            HIGH_SCORE_AT,
            &blank_zeros(game.high_score.max(game.score()), 8),
        );
        put(SCORE_AT, &blank_zeros(game.score(), 8));
        for (at, value) in BONUS_AT.iter().zip(self.bonuses.values) {
            put(*at, &blank_zeros(value / 100, 4));
        }
        if !self.bonuses.perfect {
            table.height -= PERFECT_ROWS;
            table.text.truncate(table.width * table.height);
        }
        font.draw_block(frame, &table, PLANE_A, Style::plain(font.foreground, 1));
    }
    fn draw_round_card(&self, frame: &mut Frame, game: &Game) {
        let font = game.font();
        let [number, name] = &self.card;
        if let Some(number) = number {
            let (col, row) = number.cell(PLANE_A);
            let gold = Style::plain(md_color(GOLD), 2);
            font.draw_cells(frame, col * 8, row * 8, number.width, &number.text, gold);
        }
        if let Some(name) = name {
            font.draw_block(frame, name, PLANE_A, Style::plain(font.foreground, 1));
        }
    }
}
impl Mode for StageClear {
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        self.ticks = self.ticks.wrapping_add(1);
        let skip = any_button(input);
        // Fades advance every frame, the round card's every fourth.
        let card = matches!(self.step, Step::Card(_) | Step::CardFadeOut);
        if !card || self.ticks.is_multiple_of(CARD_FADE_INTERVAL) {
            self.fade = self.fade.advance();
        }
        match self.step {
            Step::FadeOut => {
                if self.fade.finished() {
                    return self.after_fade_out();
                }
            }
            Step::ClearTitle(ticks) => {
                self.step = if ticks <= 1 || skip {
                    Step::ClearScores(SCORES_TICKS)
                } else {
                    Step::ClearTitle(ticks - 1)
                };
                if let Step::ClearScores(_) = self.step {
                    self.begin(self.step);
                }
            }
            Step::ClearScores(ticks) => {
                self.step = if ticks <= 1 || skip {
                    Step::ClearTally
                } else {
                    Step::ClearScores(ticks - 1)
                };
            }
            Step::ClearTally => {
                // Func_00F488: effect $0B on every eighth frame counter value.
                if self.ticks.is_multiple_of(8)
                    && let Some(game) = held_game(&self.saved)
                {
                    self.audio
                        .request(SoundEvent::NinjutsuSelect.command(game.rom()).ok());
                }
                let points = self.bonuses.tally_step();
                if let Some(game) = held_game_mut(&mut self.saved) {
                    game.add_score(points);
                }
                if self.bonuses.total() == 0 {
                    self.step = Step::ClearResult(RESULT_TICKS);
                }
            }
            Step::ClearResult(ticks) => {
                if ticks <= 1 || skip {
                    self.begin(Step::ClearFadeOut);
                } else {
                    self.step = Step::ClearResult(ticks - 1);
                }
            }
            Step::ClearFadeOut => {
                if self.fade.finished() {
                    // $F276: the round's bonus flags start again.
                    if let Some(game) = held_game_mut(&mut self.saved) {
                        game.round_flags = RoundFlags::default();
                    }
                    self.begin(Step::Card(CARD_TICKS));
                }
            }
            Step::Card(ticks) => {
                if let (Some(map), Some(game), Some(next)) =
                    (&mut self.card_map, held_game(&self.saved), self.next)
                {
                    animate_card(
                        map,
                        game.rom(),
                        next.round_index(),
                        u32::from(CARD_TICKS - ticks),
                    );
                }
                // Func_00BDD6: a button jumps to the fade-out (state $24).
                if ticks <= 1 || skip {
                    self.begin(Step::CardFadeOut);
                } else {
                    self.step = Step::Card(ticks - 1);
                }
            }
            Step::CardFadeOut => {
                if self.fade.finished() {
                    if self.resume {
                        let mut mode = self.saved.take()?;
                        if let Some(game) = mode.game_mut() {
                            game.fade_in();
                            game.restart_music();
                        }
                        return Some(ModeChange::Replace(mode));
                    }
                    let next = self.next?;
                    return Some(self.load(next));
                }
            }
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        let Some(game) = held_game(&self.saved) else {
            frame.clear([0; 3]);
            return;
        };
        match self.step {
            Step::FadeOut => game.draw(frame),
            Step::Card(_) | Step::CardFadeOut => {
                if let Some(map) = &self.card_map {
                    map.render(frame, Fade::Out(0));
                } else {
                    frame.clear([0; 3]);
                }
                self.draw_round_card(frame, game);
            }
            _ => {
                frame.clear([0; 3]);
                self.draw_round_clear(frame, game);
            }
        }
        frame.fade(self.fade);
    }
    fn name(&self) -> &'static str {
        match self.step {
            Step::FadeOut => "Game",
            Step::Card(_) | Step::CardFadeOut => "RoundCard",
            _ => "RoundClear",
        }
    }
    fn take_audio(&mut self) -> Audio {
        take_held_audio(&mut self.saved, &mut self.audio)
    }
    fn game(&self) -> Option<&Game> {
        held_game(&self.saved)
    }
    fn game_mut(&mut self) -> Option<&mut Game> {
        held_game_mut(&mut self.saved)
    }
}
