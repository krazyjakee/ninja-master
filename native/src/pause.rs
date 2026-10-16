//! `Mode_Pause` (`$B4D2`, #50): the pause window is also the ninjutsu menu.
//!
//! `Game_CheckPause` (`$B7B2`) ends every play frame: Start plays effect
//! `$0E` (PAUSE), saves `Mode_Game` in `Main_HandlerSaved` and installs
//! `Mode_Pause`. `Pause_S00` builds the window map from `Data_00B290`
//! (`Func_00B9F2`) with the high score and the selected technique's line,
//! `Pause_S04` reveals it one row per frame, `Pause_S08` waits eight frames
//! and `Pause_S0C` is the menu: with a use left (`Ninjutsu_Stock`), a
//! direction press steps `Ninjutsu_Select` (`$FF37E7`, wrapping 0..=3) and
//! plays effect `$0B`; Start closes the window row by row (`Pause_S10`),
//! plays effect `$40` (MUSIC RESTART) and restores `Mode_Game` (`$B748`).
//! The world does not advance while paused, but `Mode_Pause_Refresh`
//! (`$B4B2`) keeps the stage's palette routine and tile animation running
//! (`Game::refresh_paused`).
//!
//! Native additions: B opens a QUIT TO MENU prompt and C confirms it,
//! fading out to the title menu (`Title::menu`, whose QUIT ends the
//! program), and the music plays quieter while the window is open
//! (`PAUSE_MUSIC_GAIN`). The cartridge shows the selected technique's
//! icon in its HUD (tiles `$72..$77`); `Game::draw` does the same.

use crate::{
    front_end::{Front, Title},
    game::Game,
    input::{Button, Input},
    modes::{Audio, Mode, ModeChange, held_game, held_game_mut, take_held_audio},
    render::{FADE_STEPS, Fade, Frame},
    sound_events::{SoundEvent, Table},
    text::{TextBlock, WINDOW, blank_zeros},
};

/// `Data_00B286`: the window header and cells; `Func_00B9F2` reads nine
/// 40-byte rows.
const WINDOW_BLOCK: usize = 0xb286;
pub const WINDOW_ROWS: usize = 9;
/// `Func_00B87C` writes the high score's eight digits at `$FF7826`: window
/// row 3, column 19.
const HIGH_SCORE_CELL: (usize, usize) = (19, 3);
/// `Data_00B414`: the "JITSU OF ..." lines, `$26` bytes per technique,
/// copied to `$FF7942` (row 7, column 1) by `Func_00BA3A`.
const NAMES: usize = 0xb414;
const NAME_LENGTH: usize = 0x26;
const NAME_CELL: (usize, usize) = (1, 7);
/// `Pause_S08`: frames between the full window and the menu (`$FF00F6` = 8).
pub const SETTLE_TICKS: u8 = 8;
/// Effects `$0E` (PAUSE, `$B7E0`) and `$40` (MUSIC RESTART, `$B736`).
const PAUSE_EFFECT: u8 = 0x0e;
/// Native: the music level while paused (about -10 dB).
pub const PAUSE_MUSIC_GAIN: f64 = 0.3;
const RESUME_EFFECT: u8 = 0x40;
/// Driver command `$FF`: stop all sound (`Func_004AB4`), as the front end
/// does on entry.
const STOP_SOUND: u8 = 0xff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// `Pause_S04`: window rows shown so far.
    Opening(u8),
    /// `Pause_S08`: frames left.
    Settle(u8),
    /// `Pause_S0C`.
    Menu,
    /// Native QUIT TO MENU prompt.
    ConfirmQuit,
    /// Native: fade steps taken on the way to the title menu.
    Leaving(u8),
    /// `Pause_S10`: rows still shown; restores `Mode_Game` after zero.
    Closing(u8),
}

pub struct Pause {
    /// `Main_HandlerSaved`.
    saved: Option<Box<dyn Mode>>,
    pub stage: Stage,
    /// `Data_00B290` with the high score patched in (`Pause_S00`).
    window: Vec<u8>,
    width: usize,
    /// Screen row of the window's first row: the header's `$B180` is
    /// window-plane row 3, below the three HUD rows.
    top: i32,
    /// `Data_00B414` rows.
    names: Vec<Vec<u8>>,
    audio: Audio,
    pointer_input: Input,
}
impl Pause {
    /// `ModeChange::Suspend` constructor for `Game_CheckPause`.
    pub fn suspend(mode: Box<dyn Mode>) -> Box<dyn Mode> {
        Box::new(Self::new(mode))
    }
    pub fn new(mode: Box<dyn Mode>) -> Self {
        let mut pause = Self {
            saved: None,
            stage: Stage::Opening(0),
            window: Vec::new(),
            width: 0,
            top: 3,
            names: Vec::new(),
            audio: Audio::default(),
            pointer_input: Input::default(),
        };
        if let Some(game) = mode.game() {
            let rom = game.rom();
            if let Ok(block) = TextBlock::read(rom, WINDOW_BLOCK) {
                pause.width = block.width;
                pause.top = block.cell(WINDOW).1;
                pause.window = block.text[..block.width * WINDOW_ROWS].to_vec();
                // Pause_S00 copies Score to the display, then writes High_Score.
                let digits = blank_zeros(game.high_score, 8);
                let at = HIGH_SCORE_CELL.1 * block.width + HIGH_SCORE_CELL.0;
                pause.window[at..at + 8].copy_from_slice(digits.as_bytes());
            }
            pause.names = (0..4)
                .filter_map(|i| rom.get(NAMES + i * NAME_LENGTH..NAMES + (i + 1) * NAME_LENGTH))
                .map(<[u8]>::to_vec)
                .collect();
            pause
                .audio
                .request(Table::Effect.command(rom, PAUSE_EFFECT).ok());
        }
        if pause.window.is_empty() {
            pause.stage = Stage::Closing(0);
        }
        pause.saved = Some(mode);
        pause
    }
    /// The window rows for the current selection.
    fn cells(&self, selected: usize) -> Vec<u8> {
        let mut cells = self.window.clone();
        if let Some(name) = self.names.get(selected) {
            let at = NAME_CELL.1 * self.width + NAME_CELL.0;
            if let Some(slot) = cells.get_mut(at..at + name.len()) {
                slot.copy_from_slice(name);
            }
        }
        cells
    }
    /// Window rows visible in this stage.
    fn rows(&self) -> usize {
        match self.stage {
            Stage::Opening(rows) | Stage::Closing(rows) => usize::from(rows),
            _ => WINDOW_ROWS,
        }
    }
    /// `Pause_S0C`: `inp_new_y`, or `inp_new_x` when that is zero, steps the
    /// selection independently of the casting stock.
    fn select(&mut self, input: Input) {
        let vertical =
            i32::from(input.pressed(Button::Down)) - i32::from(input.pressed(Button::Up));
        let horizontal =
            i32::from(input.pressed(Button::Right)) - i32::from(input.pressed(Button::Left));
        let step = if vertical != 0 { vertical } else { horizontal };
        let Some(game) = held_game_mut(&mut self.saved) else {
            return;
        };
        let (selected, _) = game.ninjutsu();
        if vertical == 0 && horizontal == 0 {
            return;
        }
        game.select_ninjutsu((selected as i32 + step).rem_euclid(4) as usize);
        if let Ok(command) = SoundEvent::NinjutsuSelect.command(game.rom()) {
            self.audio.push(command);
        }
    }
}
impl Mode for Pause {
    fn tick(&mut self, mut input: Input) -> Option<ModeChange> {
        input.pressed |= std::mem::take(&mut self.pointer_input).pressed;
        // Mode_Pause_Refresh ahead of every state (#94).
        if let Some(game) = held_game_mut(&mut self.saved) {
            game.refresh_paused();
        }
        match self.stage {
            Stage::Opening(rows) => {
                let rows = rows + 1;
                self.stage = if usize::from(rows) >= WINDOW_ROWS {
                    Stage::Settle(SETTLE_TICKS)
                } else {
                    Stage::Opening(rows)
                };
            }
            Stage::Settle(ticks) => {
                self.stage = if ticks <= 1 {
                    Stage::Menu
                } else {
                    Stage::Settle(ticks - 1)
                };
            }
            Stage::Menu => {
                if input.pressed(Button::Start) {
                    self.stage = Stage::Closing(WINDOW_ROWS as u8 - 1);
                } else if input.pressed(Button::Attack) {
                    self.stage = Stage::ConfirmQuit;
                } else {
                    self.select(input);
                }
            }
            Stage::ConfirmQuit => {
                if input.pressed(Button::Jump) {
                    self.stage = Stage::Leaving(0);
                    return None;
                }
                if input.pressed(Button::Attack) || input.pressed(Button::Start) {
                    self.stage = Stage::Menu;
                }
            }
            Stage::Closing(0) => {
                self.audio.request(
                    held_game(&self.saved)
                        .and_then(|game| Table::Effect.command(game.rom(), RESUME_EFFECT).ok()),
                );
                return self.saved.take().map(ModeChange::Replace);
            }
            Stage::Closing(rows) => self.stage = Stage::Closing(rows - 1),
            Stage::Leaving(step) if step < FADE_STEPS => self.stage = Stage::Leaving(step + 1),
            Stage::Leaving(_) => {
                let game = held_game(&self.saved)?;
                let title = Title::menu(Front::from_game(game));
                self.audio.push(STOP_SOUND);
                return Some(ModeChange::Replace(Box::new(title)));
            }
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        let Some(game) = held_game(&self.saved) else {
            return;
        };
        game.draw(frame);
        let font = game.font();
        let style = font.window_style();
        let rows = self.rows().min(WINDOW_ROWS);
        let cells = self.cells(game.ninjutsu().0);
        // Pause_S04/S10 draw the last `rows` rows of the map from the top of
        // the window area (VDP register $12 grows from 3 to 12).
        let top = self.top;
        let hud_extra = (crate::hud::PIXEL_HEIGHT - crate::hud::HEIGHT * 8) as i32;
        let first = WINDOW_ROWS - rows;
        font.draw_window(
            frame,
            0,
            top * 8 + hud_extra,
            self.width,
            &cells[first * self.width..],
            style,
        );
        let prompt = match self.stage {
            Stage::Menu => "B: QUIT TO MENU",
            Stage::ConfirmQuit => "QUIT TO MENU? C: YES  B: NO",
            Stage::Leaving(step) => {
                frame.fade(Fade::Out(step));
                return;
            }
            _ => return,
        };
        let x = (crate::WIDTH as i32 - prompt.len() as i32 * 8) / 2;
        let y = (top + WINDOW_ROWS as i32 + 1) * 8 + hud_extra;
        font.draw_text(frame, x, y, prompt, style);
    }
    fn checkpoint_requested(&self) -> bool {
        self.stage == Stage::Leaving(0)
    }
    fn menu_event(&mut self, event: crate::menu::MenuEvent) {
        use crate::menu::MenuEvent;
        let button = match (self.stage, event) {
            (Stage::Menu, MenuEvent::Back) => Some(Button::Start),
            (Stage::ConfirmQuit, MenuEvent::Back) => Some(Button::Attack),
            (Stage::ConfirmQuit, MenuEvent::Click(x, _, _)) if (16..304).contains(&x) => {
                Some(if x < 180 {
                    Button::Jump
                } else {
                    Button::Attack
                })
            }
            (Stage::Menu, MenuEvent::Click(x, y, alternate)) if (16..304).contains(&x) => {
                let hud_extra = (crate::hud::PIXEL_HEIGHT - crate::hud::HEIGHT * 8) as i32;
                let jitsu_y = (self.top + NAME_CELL.1 as i32) * 8 + hud_extra;
                let quit_y = (self.top + WINDOW_ROWS as i32 + 1) * 8 + hud_extra;
                if (jitsu_y..jitsu_y + 8).contains(&y) {
                    Some(if x < 160 || alternate {
                        Button::Left
                    } else {
                        Button::Right
                    })
                } else if (quit_y..quit_y + 8).contains(&y) {
                    Some(Button::Attack)
                } else if (self.top * 8..jitsu_y).contains(&y) {
                    Some(Button::Start)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(button) = button {
            self.pointer_input.pressed |= button.mask();
        }
    }
    fn name(&self) -> &'static str {
        "Pause"
    }
    /// Native: the music plays on quieter while the window is open. The
    /// cartridge keeps it at full level (Genesis Plus GX: the same level from
    /// the pause at frame 30 to MUSIC RESTART at 343).
    fn music_gain(&self) -> f64 {
        PAUSE_MUSIC_GAIN
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
