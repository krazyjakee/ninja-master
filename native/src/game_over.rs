//! `Mode_GameOver` (`$F6C0`, #44): GAME OVER, CONTINUE n / NEW GAME.
//!
//! `Mode_Game` installs it when `Lives` goes negative (`$B1BC`); the
//! finished game waits inside it. With continues left, `GameOver_FadeIn`
//! draws GAME OVER, the scores and the continue box and plays music `$0E`;
//! `GameOver_ContinueMenu` toggles CONTINUE / NEW GAME with Up/Down, and
//! after 600 idle frames starts the TIME: countdown (9 to 0, any input goes
//! back to the menu). Start on CONTINUE spends a continue and, after the
//! fade (`GameOver_Leave`), `Mode_Game` state `$28` restarts the substage
//! with a new set of lives (`Game::continue_game`) behind the round card.
//! Start on NEW GAME fades straight to `Mode_SegaLogo`. With no continues,
//! or when the countdown runs out, the plain GAME OVER screen (music `$11`)
//! waits five seconds or a button, then `Mode_SegaLogo` follows.

use crate::{
    WIDTH,
    front_end::{Front, GOLD, Next, WHITE, go},
    game::Game,
    input::{Button, Input},
    modes::{Audio, Mode, ModeChange, any_button, held_game, held_game_mut, take_held_audio},
    render::{Fade, Frame, md_color},
    round_screens::StageClear,
    sound_events::Table,
    text::{PLANE_A, Style, TextBlock, blank_zeros},
};

/// Text blocks: GAME OVER (`Func_004188`, title font), HIGH SCORE / SCORE
/// and the continue box (`Func_00A4F2`, frame bytes from 'b'), TIME:.
const GAME_OVER_TEXT: usize = 0xfa80;
const SCORES_TEXT: usize = 0xfa94;
const CONTINUE_TEXT: usize = 0xfae0;
const TIME_TEXT: usize = 0xfb46;
/// `Data_00FAD4` / `Data_00FADA`: the eight score digits start one column
/// before the placeholder "1234567" (rows 0 and 2 of the scores block).
const SCORE_DIGITS_AT: usize = 10;
/// `Data_00FB30`: the continues digit, box row 1 column 12;
/// `GameOver_Cursor`: '>' at column 1 of box rows 1 and 3.
const CONTINUES_AT: (usize, usize) = (12, 1);
const CURSOR_COLUMN: usize = 1;
/// The countdown digit at VRAM `$CCC6` (row 25, column 35).
const DIGIT_CELL: (i32, i32) = (35, 25);
/// `GameOver_ShowGameOver` scrolls plane A down 64 pixels (`$FFC0`).
const PLAIN_SCROLL: i32 = 64;
/// `GameOver_FadeIn` music `$0E` (GETUFU); `GameOver_Music` `$11` (GAME OVER).
const CONTINUE_MUSIC: u8 = 0x0e;
const GAME_OVER_MUSIC: u8 = 0x11;
/// Effect `$0B` as TIME: appears and after each digit.
const DIGIT_EFFECT: u8 = 0x0b;
/// `$FF00FA` = `$258`: idle frames before TIME:.
pub const MENU_TICKS: u16 = 0x258;
/// `GameOver_CountdownWait` holds a digit `$3C` frames, `GameOver_Countdown`
/// scrolls in the next over 8; nine digits after 9, then one more wait.
pub const DIGIT_TICKS: u16 = 0x3c + 8;
pub const COUNTDOWN_TICKS: u16 = 0x3c + 9 * DIGIT_TICKS + 1;
/// `GameOver_Wait`: five one-second ticks.
pub const PLAIN_TICKS: u16 = 5 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leave {
    /// Start on CONTINUE (`Game_Requests` bit 4).
    Continue,
    /// Start on NEW GAME.
    NewGame,
    /// The countdown ran out: the plain GAME OVER follows.
    TimeUp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `GameOver_FadeIn`.
    FadeIn,
    /// `GameOver_ContinueMenu`: idle frames left before TIME:.
    Menu(u16),
    /// `GameOver_CountdownWait` / `GameOver_Countdown`: frames since TIME:.
    Countdown(u16),
    /// `GameOver_Leave`.
    Leave(Leave),
    /// `GameOver_ShowGameOver` and the fade in of `GameOver_Music`.
    PlainIn,
    /// `GameOver_Wait`: frames left.
    Plain(u16),
    /// `GameOver_FadeOut`.
    PlainOut,
}

/// The screen's ROM text blocks.
struct Texts {
    title: TextBlock,
    scores: TextBlock,
    /// The continue box, frame bytes already in the window drawer's order.
    record: TextBlock,
    time: TextBlock,
}
impl Texts {
    fn read(rom: &[u8]) -> Result<Self, String> {
        let mut record = TextBlock::read(rom, CONTINUE_TEXT)?;
        // Func_00A4F2 maps bytes from 'a' through Data_00A498 from
        // index 0; the window drawer counts from 'a' = 1.
        for byte in &mut record.text {
            if *byte >= b'b' {
                *byte -= 1;
            }
        }
        Ok(Self {
            title: TextBlock::read(rom, GAME_OVER_TEXT)?,
            scores: TextBlock::read(rom, SCORES_TEXT)?,
            record,
            time: TextBlock::read(rom, TIME_TEXT)?,
        })
    }
}

pub struct GameOver {
    /// The finished game (`Mode_Game`, state `$28` next).
    saved: Option<Box<dyn Mode>>,
    pub step: Step,
    fade: Fade,
    /// `$FF00F6`: 0 CONTINUE, 1 NEW GAME.
    pub cursor: u8,
    audio: Audio,
    texts: Option<Texts>,
    pointer_input: Input,
}
impl GameOver {
    /// `ModeChange::Suspend` constructor for `$B1BC`.
    pub fn suspend(mode: Box<dyn Mode>) -> Box<dyn Mode> {
        Box::new(Self::new(mode))
    }
    pub fn new(mode: Box<dyn Mode>) -> Self {
        let continues = mode.game().map_or(0, |game| game.continues);
        let texts = mode.game().and_then(|game| Texts::read(game.rom()).ok());
        Self {
            saved: Some(mode),
            // GameOver_Init: with no continues, straight to state $10.
            step: if continues == 0 {
                Step::PlainIn
            } else {
                Step::FadeIn
            },
            fade: Fade::In(0),
            cursor: 0,
            audio: Audio::default(),
            texts,
            pointer_input: Input::default(),
        }
    }
    fn music(&mut self, index: u8) {
        self.audio.request(
            held_game(&self.saved).and_then(|g| Table::Music.command(g.rom(), index).ok()),
        );
    }
    fn effect(&mut self, index: u8) {
        self.audio.request(
            held_game(&self.saved).and_then(|g| Table::Effect.command(g.rom(), index).ok()),
        );
    }
    /// `GameOver_MoveCursor`: Up or Down toggles the two items.
    fn move_cursor(&mut self, input: Input) {
        if input.pressed(Button::Up) || input.pressed(Button::Down) {
            self.cursor ^= 1;
        }
    }
    fn leave(&mut self, leave: Leave) {
        self.step = Step::Leave(leave);
        self.fade = Fade::Out(0);
    }
    fn to_sega_logo(&self) -> Option<ModeChange> {
        let front = Front::from_game(held_game(&self.saved)?);
        Some(go(&front, Next::SegaLogo))
    }
    /// `GameOver_ModeGame` with `Game_Requests` bit 4: state `$28` goes
    /// back to state 4 and the round card.
    fn continue_game(&self) -> Option<ModeChange> {
        let game = held_game(&self.saved)?;
        match game.continue_game() {
            Ok(game) => Some(ModeChange::Replace(StageClear::round_card(Box::new(game)))),
            Err(_) => self.to_sega_logo(),
        }
    }
    /// The countdown digit shown `ticks` after TIME: appeared.
    pub fn digit(ticks: u16) -> u16 {
        9 - (ticks / DIGIT_TICKS).min(9)
    }
    fn draw_menu(&self, frame: &mut Frame, game: &Game, texts: &Texts) {
        let font = game.font();
        let white = Style::plain(md_color(WHITE), 1);
        let mut scores = texts.scores.clone();
        let width = scores.width;
        for (row, value) in [(0, game.high_score.max(game.score())), (2, game.score())] {
            let at = row * width + SCORE_DIGITS_AT;
            if let Some(slot) = scores.text.get_mut(at..at + 8) {
                slot.copy_from_slice(blank_zeros(value, 8).as_bytes());
            }
        }
        let shadow = Style::plain([0; 3], 1);
        let (score_col, score_row) = scores.cell(PLANE_A);
        font.draw_cells(
            frame,
            score_col * 8 - 4,
            score_row * 8 + 4,
            scores.width,
            &scores.text,
            shadow,
        );
        font.draw_block(frame, &scores, PLANE_A, white);
        let mut record = texts.record.text.clone();
        let width = texts.record.width;
        record[CONTINUES_AT.1 * width + CONTINUES_AT.0] = b'0' + game.continues.min(9);
        for row in [1, 3] {
            record[row * width + CURSOR_COLUMN] = b' ';
        }
        let cursor_row = 1 + usize::from(self.cursor) * 2;
        record[cursor_row * width + CURSOR_COLUMN] = b'>';
        let (col, row) = texts.record.cell(PLANE_A);
        font.draw_window(frame, col * 8 - 4, row * 8 + 4, width, &record, shadow);
        font.draw_window(frame, col * 8, row * 8, width, &record, white);
        if let Step::Countdown(ticks) = self.step {
            font.draw_block(frame, &texts.time, PLANE_A, white);
            let digit = char::from(b'0' + Self::digit(ticks) as u8).to_string();
            let phase = ticks % DIGIT_TICKS;
            let y = DIGIT_CELL.1 * 8;
            let x = DIGIT_CELL.0 * 8;
            let mut cell = Frame::default();
            cell.clear(md_color(0x222));
            let shift = if phase >= 60 {
                i32::from(phase - 59)
            } else {
                0
            };
            font.draw_text(&mut cell, x, y - shift, &digit, white);
            if shift > 0 {
                let next =
                    char::from(b'0' + Self::digit(ticks).saturating_sub(1) as u8).to_string();
                font.draw_text(&mut cell, x, y + 8 - shift, &next, white);
            }
            let rgb = cell.rgb();
            for row in y..y + 8 {
                for col in x..x + 8 {
                    let at = (row as usize * WIDTH + col as usize) * 3;
                    frame.pixel(col, row, [rgb[at], rgb[at + 1], rgb[at + 2]]);
                }
            }
        }
    }
}
impl Mode for GameOver {
    fn tick(&mut self, mut input: Input) -> Option<ModeChange> {
        input.pressed |= std::mem::take(&mut self.pointer_input).pressed;
        self.fade = self.fade.advance();
        let any = input.held != 0 || input.pressed != 0;
        match self.step {
            Step::FadeIn => {
                if self.fade.finished() {
                    self.music(CONTINUE_MUSIC);
                    self.step = Step::Menu(MENU_TICKS);
                }
            }
            Step::Menu(ticks) => {
                self.move_cursor(input);
                if input.pressed(Button::Start) {
                    if self.cursor == 0 {
                        if let Some(game) = held_game_mut(&mut self.saved) {
                            game.continues = game.continues.saturating_sub(1);
                        }
                        self.leave(Leave::Continue);
                    } else {
                        self.leave(Leave::NewGame);
                    }
                } else if any {
                    self.step = Step::Menu(MENU_TICKS);
                } else if ticks <= 1 {
                    self.effect(DIGIT_EFFECT);
                    self.step = Step::Countdown(0);
                } else {
                    self.step = Step::Menu(ticks - 1);
                }
            }
            Step::Countdown(ticks) => {
                if any {
                    // $F9D4: back to the menu, TIME: blanked.
                    self.move_cursor(input);
                    self.step = Step::Menu(MENU_TICKS);
                } else if ticks + 1 >= COUNTDOWN_TICKS {
                    self.leave(Leave::TimeUp);
                } else {
                    let ticks = ticks + 1;
                    if ticks.is_multiple_of(DIGIT_TICKS) && ticks <= 9 * DIGIT_TICKS {
                        self.effect(DIGIT_EFFECT);
                    }
                    self.step = Step::Countdown(ticks);
                }
            }
            Step::Leave(leave) => {
                if self.fade.finished() {
                    match leave {
                        Leave::Continue => return self.continue_game(),
                        Leave::NewGame => return self.to_sega_logo(),
                        Leave::TimeUp => {
                            self.step = Step::PlainIn;
                            self.fade = Fade::In(0);
                        }
                    }
                }
            }
            Step::PlainIn => {
                if self.fade.finished() {
                    self.music(GAME_OVER_MUSIC);
                    self.step = Step::Plain(PLAIN_TICKS);
                }
            }
            Step::Plain(ticks) => {
                if any_button(input) || ticks <= 1 {
                    self.step = Step::PlainOut;
                    self.fade = Fade::Out(0);
                } else {
                    self.step = Step::Plain(ticks - 1);
                }
            }
            Step::PlainOut => {
                if self.fade.finished() {
                    return self.to_sega_logo();
                }
            }
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        frame.clear(md_color(0x222));
        let (Some(game), Some(texts)) = (held_game(&self.saved), &self.texts) else {
            return;
        };
        let plain = matches!(self.step, Step::PlainIn | Step::Plain(_) | Step::PlainOut);
        let title = &texts.title;
        let (_, row) = title.cell(PLANE_A);
        let x = (WIDTH as i32 - title.width as i32 * 16) / 2;
        let y = row * 8 + if plain { PLAIN_SCROLL } else { 0 };
        let gold = Style::plain(md_color(GOLD), 2);
        if !plain {
            game.font().draw_cells(
                frame,
                x - 4,
                y + 4,
                title.width,
                &title.text,
                Style::plain([0; 3], 2),
            );
        }
        game.font()
            .draw_cells(frame, x, y, title.width, &title.text, gold);
        if !plain {
            self.draw_menu(frame, game, texts);
        }
        frame.fade(self.fade);
    }
    fn menu_event(&mut self, event: crate::menu::MenuEvent) {
        use crate::menu::MenuEvent;
        if !matches!(self.step, Step::Menu(_) | Step::Countdown(_)) {
            return;
        }
        if matches!(event, MenuEvent::Back) {
            self.cursor = 1;
            self.pointer_input.pressed |= Button::Start.mask();
            return;
        }
        if let MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) = event
            && let Some(texts) = &self.texts
        {
            let (col, row) = texts.record.cell(PLANE_A);
            if (col * 8..(col + texts.record.width as i32) * 8).contains(&x) {
                for item in 0..2 {
                    let top = (row + 1 + item * 2) * 8;
                    if (top..top + 8).contains(&y) {
                        self.cursor = item as u8;
                        if matches!(event, MenuEvent::Click(..)) {
                            self.pointer_input.pressed |= Button::Start.mask();
                        }
                    }
                }
            }
        }
    }
    fn name(&self) -> &'static str {
        "GameOver"
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
