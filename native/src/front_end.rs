//! The front end (#44): `Mode_SegaLogo`, `Mode_Intro`, `Mode_Story` and
//! `Mode_Title`, and the session state they hand between them.
//!
//! Power-on runs the attract cycle `SegaLogo -> Intro -> Story -> Intro ->
//! Title -> DemoStart -> SegaLogo`; Start skips to the title
//! options in force, OPTIONS opens `Mode_Options` (`crate::options`), the
//! native QUIT ends the program, and its idle timeout starts the attract
//! demo (`crate::attract`). The pause window's QUIT TO MENU comes back to
//! the title menu (`Title::menu`). Game over
//! without a continue (`crate::game_over`) and the ending (`crate::ending`)
//! come back to `Mode_SegaLogo`.
//!
//! Every screen is the cartridge's picture, repeated as VDP writes into a
//! `crate::vdp::Vdp`: the SEGA logo (`crate::sega_logo`), `Mode_Intro`'s
//! SHINOBI logo and Musashi reveal (`crate::intro`), the story over the
//! moon (`crate::story`) and the title (`crate::title_art`), on which

use std::rc::Rc;

use crate::{
    attract::Demo,
    game::Game,
    input::{Button, Input},
    menu::{Menu, MenuEvent},
    modes::{Audio, Mode, ModeChange},
    options::{GameOptions, OptionsScreen},
    render::{FADE_STEPS, Fade, Frame},
    sega_logo::{self, Exit, Logo, Variant},
    sound_events::Table,
    story::Story,
    text::Font,
    title_art::TitleArt,
};

pub use crate::intro::{INTRO_TICKS, Intro, REVEAL_TICKS, TITLE_FADE_MASK};

/// Native stand-in for the gold of the title font (`$02AE`), also on the
/// round screens and game over.
pub const GOLD: u16 = 0x02ae;
/// CRAM white of the menu and caption text.
pub const WHITE: u16 = 0x0eee;
/// Native title/menu soundtrack: the intro's SHINOBI sound-test record.
const TITLE_MUSIC: u8 = 0x0d;
/// `HighScore_Init` (`$115B4`): 10000 at power-on.
pub const HIGH_SCORE_INIT: u32 = 10_000;
/// `Title_FadeIn` then 12 seconds of `Title_PressStart`. Native menus stay
/// open until dismissed; the attract demo still runs from the idle title.
pub const TITLE_TICKS: u32 = 12 * 60;
pub const MENU_TICKS: u32 = 8 * 60;
/// `Title_Init`'s call runs through 29 frames after the reveal (3132..3160
/// in the attract capture, the interrupt of the 28th lost); nothing
/// changes on screen until `Title_FadeIn`. (After Start it took 28 to 30.)
pub const TITLE_INIT_FRAMES: u32 = 29;
const TITLE_INIT_LOST: u32 = 28;
/// SEGA logo (the zoom, as at power-on): frames 18..233 of the attract
/// capture, the last call installing `Mode_Intro`. Start skips from every
/// phase in the native port.
pub const LOGO_TICKS: u32 = sega_logo::ZOOM_TICKS;
pub const LOGO_START_FROM: u32 = 0;
/// `SegaLogo_S00`: voice `$0D` ("SEGA").
const SEGA_VOICE: u8 = 0x0d;
/// What survives between the front-end modes: in the cartridge the
/// `$FF0000..$FF001F` block (options, secrets, `High_Score`,
/// `Demo_Counter`), plus the native effect switches.
#[derive(Clone)]
pub struct Front {
    pub rom: Rc<[u8]>,
    pub font: Font,
    pub options: GameOptions,
    pub title_selection: usize,
    pub title_first: usize,
    pub return_to_menu: bool,
    pub high_score: u32,
    pub demo_counter: u16,
    pub effects: [bool; 4],
    /// `$FF16E8`: the frame mask of palette fades. `Intro_S10` leaves 3
    /// (a fade step every fourth frame) for the title; `Title_FadeIn`
    /// clears it.
    pub fade_mask: u16,
    /// `Frame_Counter` (`$FF0022`): the front-end screens time some steps
    /// by its low bits.
    pub frame_counter: u16,
    /// `$FF001C`: the random word. The cartridge rotates it in the idle
    /// loop, so its value depends on CPU timing; natively it is stepped
    /// with `Func_0096EE` once per SEGA logo.
    pub random: u16,
    /// `$FFF000`: the CRAM copy as the last screen left it. `Title_FadeIn`
    /// fades from it: white after the reveal, black after a fade out.
    pub palette: [u16; 64],
    /// Music carried between front-end screens; cleared when sound stops.
    pub(crate) music: Option<u8>,
}
impl Front {
    /// Power-on state.
    pub fn load(rom: &[u8]) -> Result<Self, String> {
        Ok(Self {
            rom: Rc::from(rom),
            font: Font::load(rom)?,
            options: GameOptions::default(),
            title_selection: 0,
            title_first: 0,
            return_to_menu: false,
            high_score: HIGH_SCORE_INIT,
            demo_counter: 0,
            effects: [true; 4],
            fade_mask: 0,
            frame_counter: 0,
            random: 0,
            palette: [0; 64],
            music: None,
        })
    }
    /// The state a finished game leaves behind.
    pub fn from_game(game: &Game) -> Self {
        Self {
            rom: Rc::from(game.rom()),
            font: game.font().clone(),
            options: game.options,
            title_selection: 0,
            title_first: 0,
            return_to_menu: true,
            high_score: game.high_score.max(game.score()),
            demo_counter: game.demo_counter,
            effects: game.effects(),
            fade_mask: 0,
            frame_counter: 0,
            random: 0,
            palette: [0; 64],
            music: None,
        }
    }
    /// Carry the session into a freshly loaded game.
    pub fn prepare(&self, game: &mut Game) {
        game.high_score = self.high_score;
        game.demo_counter = self.demo_counter;
        game.set_effects(self.effects);
    }
    /// `Title_ModeGame`: round 1-1 with the options applied.
    pub fn new_game(&self) -> Result<Game, String> {
        let mut game = Game::load(&self.rom)?;
        self.prepare(&mut game);
        game.start_session(self.options);
        Ok(game)
    }
}

/// The `Main_Handler` writes out of the front-end screens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    SegaLogo,
    /// `Mode_Intro`; `true` for the pass after the story.
    Intro(bool),
    Story,
    Title,
    Options,
    /// `Title_ModeGame`: `Mode_Game`, which starts with the round card.
    Game,
    Continue,
    /// `Title_ModeDemo`: `Mode_DemoStart` counts the demo and runs it.
    Demo,
    /// Native: the title menu's QUIT.
    Quit,
}

/// Build the mode a front-end screen leaves to.
pub fn go(front: &Front, next: Next) -> ModeChange {
    let front = front.clone();
    let mode: Box<dyn Mode> = match next {
        Next::SegaLogo => Box::new(SegaLogo::new(front)),
        Next::Intro(second) => Box::new(Intro::new(front, second)),
        Next::Story => Box::new(Story::new(front)),
        Next::Title => Box::new(Title::new(front)),
        Next::Options => Box::new(OptionsScreen::new(front)),
        Next::Game => return ModeChange::NewGame(front),
        Next::Continue => return ModeChange::Continue(front),
        Next::Demo => {
            let mut front = front;
            front.demo_counter = front.demo_counter.wrapping_add(1);
            match Demo::new(front.clone()) {
                Ok(demo) => Box::new(demo),
                Err(_) => Box::new(SegaLogo::new(front)),
            }
        }
        Next::Quit => return ModeChange::Quit,
    };
    ModeChange::Replace(mode)
}

/// What most front-end states share: a fade in on entry (`$5EA`), a fade
/// out before the next handler is installed (`$67A`), a frame count and
/// the sound requests.
pub struct Screen {
    pub front: Front,
    pub fade: Fade,
    pub ticks: u32,
    pub audio: Audio,
    leaving: Option<Next>,
}
impl Screen {
    pub fn new(front: Front) -> Self {
        Self {
            front,
            fade: Fade::In(0),
            ticks: 0,
            audio: Audio::default(),
            leaving: None,
        }
    }
    /// A screen whose palette is set at once rather than faded in.
    pub fn shown(front: Front) -> Self {
        Self {
            fade: Fade::In(FADE_STEPS),
            ..Self::new(front)
        }
    }
    /// Count the frame and advance the fade; the next mode once a fade out
    /// has finished.
    pub fn step(&mut self) -> Option<ModeChange> {
        self.ticks = self.ticks.wrapping_add(1);
        self.front.frame_counter = self.front.frame_counter.wrapping_add(1);
        self.fade = self.fade.advance();
        match self.leaving {
            Some(next) if self.fade.finished() => Some(go(&self.front, next)),
            _ => None,
        }
    }
    pub fn faded_in(&self) -> bool {
        self.fade == Fade::In(FADE_STEPS)
    }
    pub fn leaving(&self) -> bool {
        self.leaving.is_some()
    }
    /// Fade out from wherever the fade is, then install `next`.
    pub fn leave(&mut self, next: Next) {
        if self.leaving.is_none() {
            self.leaving = Some(next);
            self.fade = match self.fade {
                Fade::In(step) => Fade::Out(FADE_STEPS.saturating_sub(step)),
                out => out,
            };
        }
    }
}

/// `Mode_SegaLogo` (`$69A8`): `crate::sega_logo` on the VDP model.
pub struct SegaLogo {
    front: Front,
    logo: Option<Logo>,
    audio: Audio,
}
impl SegaLogo {
    /// `SegaLogo_S00` picks the animation from bit 2 of `$FF001C`. The
    /// cartridge rotates that word while it waits for each frame, so the
    /// choice depends on CPU timing (power-on gives the zoom); natively
    /// the word is stepped with `Func_0096EE` once per logo instead.
    pub fn new(mut front: Front) -> Self {
        let variant = Variant::from_random(front.random);
        crate::enemies::next_random(&mut front.random);
        Self::with_variant(front, variant)
    }
    /// The logo with a given animation.
    pub fn with_variant(mut front: Front, variant: Variant) -> Self {
        let logo = Logo::new(&front.rom, variant, front.frame_counter).ok();
        let mut audio = Audio::default();
        // SegaLogo_S00: Func_004AB4 stops the sound, then the SEGA voice.
        audio.push(0xff);
        front.music = None;
        audio.request(Table::Voice.command(&front.rom, SEGA_VOICE).ok());
        Self { front, logo, audio }
    }
    pub fn variant(&self) -> Option<Variant> {
        self.logo.as_ref().map(|logo| logo.variant)
    }
}
impl Mode for SegaLogo {
    fn options(&self) -> Option<GameOptions> {
        Some(self.front.options)
    }
    fn set_options(&mut self, options: GameOptions) {
        self.front.options = options;
    }
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        let exit = match self.logo.as_mut() {
            Some(logo) => logo.tick(input.pressed(Button::Start))?,
            None => Exit::Intro,
        };
        if exit == Exit::Title {
            self.audio.push(0xff);
            self.front.music = None;
        }
        if let Some(logo) = &self.logo {
            self.front.frame_counter = logo.v.frame_counter;
            self.front.fade_mask = logo.v.fade_mask;
        }
        Some(go(
            &self.front,
            match exit {
                Exit::Intro => Next::Intro(false),
                Exit::Title => Next::Title,
            },
        ))
    }
    fn draw(&self, frame: &mut Frame) {
        match &self.logo {
            Some(logo) => logo.v.vdp.render(frame, Fade::In(FADE_STEPS)),
            None => frame.clear([0; 3]),
        }
    }
    fn name(&self) -> &'static str {
        "SegaLogo"
    }
    fn take_audio(&mut self) -> Audio {
        std::mem::take(&mut self.audio)
    }
}

/// `Mode_Title` (`$774E`) over the title picture (`crate::title_art`).
pub struct Title {
    screen: Screen,
    /// The selected visible row: New Game, optional Continue, Options,
    /// Quit; `None` while PRESS START BUTTON is up.
    pub menu: Option<usize>,
    viewport: Menu,
    confirmation: Option<Menu>,
    resume_available: bool,
    message: String,
    pointer_input: Input,
    /// Frames left before the demo (`$FF00F6` one-second ticks).
    timer: u32,
    /// The picture, sprites and CRAM as `Title_Init` built them.
    art: Option<TitleArt>,
    /// `Title_FadeIn`: the `$FF16E8` frame mask of its CRAM updates while
    /// the fade runs; `None` once `Title_PressStart` has taken over.
    fading: Option<u32>,
    /// The CRAM, sprite table and scroll on screen: the palette, sprite
    /// and scroll changes of a frame (the fade, `Func_007B74`, the menu
    /// cursor) reach the VDP in the next frame's interrupt; plane writes
    /// show at once.
    shown: Shown,
}
/// What the interrupt last copied to the VDP.
struct Shown {
    cram: [u16; 64],
    sprites: [[u16; 4]; crate::vdp::SPRITES],
    hscroll: Vec<[i16; 2]>,
}
impl Shown {
    fn of(vdp: &crate::vdp::Vdp) -> Self {
        Self {
            cram: vdp.cram,
            sprites: vdp.sprites,
            hscroll: vdp.hscroll.clone(),
        }
    }
}
impl Title {
    pub fn new(mut front: Front) -> Self {
        let mut art = TitleArt::title(&front.rom).ok();
        // Title_Init leaves the CRAM as it was; Title_FadeIn fades from it.
        if let Some(art) = art.as_mut() {
            art.vdp.cram = front.palette;
        }
        let shown = match &art {
            Some(art) => Shown::of(&art.vdp),
            None => Shown::of(&crate::vdp::Vdp::default()),
        };
        front.palette = [0; 64];
        let fading = Some(u32::from(front.fade_mask));
        let mut screen = Screen::shown(front);
        // Carry intro audio into Title_Init. Entry after the SEGA skip,
        // Options or QUIT TO MENU needs to start the soundtrack.
        let music = Table::Music.command(&screen.front.rom, TITLE_MUSIC).ok();
        if screen.front.music != music {
            screen.audio.request(music);
            screen.front.music = music;
        }
        let return_to_menu = screen.front.return_to_menu;
        let mut title = Self {
            viewport: Menu::new(152, 3, 16),
            confirmation: None,
            resume_available: false,
            message: String::new(),
            pointer_input: Input::default(),
            // The title fades its own CRAM in; the screen fade only runs
            // the fade out.
            screen,
            menu: None,
            timer: TITLE_TICKS,
            art,
            fading,
            shown,
        };
        if return_to_menu {
            title.open_menu();
        }
        title
    }
    /// Native: the title with its menu already open, where the pause
    /// window's QUIT TO MENU leads.
    pub fn menu(front: Front) -> Self {
        let mut title = Self::new(front);
        title.open_menu();
        title
    }
    /// Open the menu, restoring the previous selection and scroll position.
    fn open_menu(&mut self) {
        self.viewport.selected = self.screen.front.title_selection;
        self.viewport.first = self.screen.front.title_first;
        self.viewport.reveal(4);
        self.menu = Some(self.viewport.selected);
        self.timer = MENU_TICKS;
        if let Some(art) = self.art.as_mut() {
            let _ = art.text(&self.screen.front.rom, crate::title_art::BLANK);
            // Preserve the cartridge menu's sprite-chain head. The native
            // panel covers the original cursor while drawing its own focus.
            art.title_cursor(0);
        }
    }
    fn activate(&mut self) {
        if let Some(confirm) = &self.confirmation {
            if confirm.selected == 1 {
                self.screen.leave(Next::Game);
            }
            self.confirmation = None;
            return;
        }
        match self.viewport.selected {
            0 if self.resume_available => self.confirmation = Some(Menu::new(168, 2, 16)),
            0 => self.screen.leave(Next::Game),
            1 if self.resume_available => self.screen.leave(Next::Continue),
            item if item == 1 + usize::from(self.resume_available) => {
                self.screen.front.title_selection = self.viewport.selected;
                self.screen.front.title_first = self.viewport.first;
                self.screen.front.return_to_menu = true;
                self.screen.leave(Next::Options);
            }
            _ => self.screen.leave(Next::Quit),
        }
    }
    /// `Title_FadeIn`: one CRAM update on the frames `$FF16E8` allows;
    /// true while it still runs.
    fn fade_in(&mut self) -> bool {
        let Some(mask) = self.fading else {
            return false;
        };
        if u32::from(self.screen.front.frame_counter) & mask != 0 {
            return true;
        }
        if self.art.as_mut().is_some_and(TitleArt::fade_in) {
            return true;
        }
        // Done: $FF16E8 = 0 and Title_PressStart runs in the same frame.
        self.fading = None;
        self.screen.front.fade_mask = 0;
        false
    }
}
impl Mode for Title {
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        if let Some(change) = self.screen.step() {
            return Some(change);
        }
        if let Some(art) = &self.art {
            self.shown = Shown::of(&art.vdp);
        }
        let ticks = self.screen.ticks;
        if ticks <= TITLE_INIT_FRAMES {
            if ticks == TITLE_INIT_LOST {
                let counter = &mut self.screen.front.frame_counter;
                *counter = counter.wrapping_sub(1);
            }
            return None;
        }
        if self.screen.leaving() || self.fade_in() {
            return None;
        }
        if let Some(art) = self.art.as_mut() {
            art.animate(u32::from(self.screen.front.frame_counter));
        }
        let mut input = input;
        input.pressed |= std::mem::take(&mut self.pointer_input).pressed;
        match self.menu {
            None if input.pressed != 0 => self.open_menu(),
            None => {}
            Some(_) => {
                let step = i32::from(input.pressed(Button::Down) || input.pressed(Button::Right))
                    - i32::from(input.pressed(Button::Up) || input.pressed(Button::Left));
                if let Some(menu) = &mut self.confirmation {
                    menu.step(step, 2);
                } else {
                    self.viewport
                        .step(step, 3 + usize::from(self.resume_available));
                    self.menu = Some(self.viewport.selected);
                }
                if input.pressed(Button::Start)
                    || input.pressed(Button::Attack)
                    || input.pressed(Button::Jump)
                {
                    self.activate();
                }
                // Open menus remain available until the player leaves them.
                self.timer = MENU_TICKS;
            }
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.screen.leave(Next::Demo); // Title_Timeout
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        match &self.art {
            Some(art) => {
                let mut vdp = art.vdp.clone();
                vdp.cram = self.shown.cram;
                vdp.sprites = self.shown.sprites;
                vdp.hscroll.clone_from(&self.shown.hscroll);
                vdp.render(frame, self.screen.fade);
            }
            None => frame.clear([0; 3]),
        }
        if self.menu.is_some() {
            let font = &self.screen.front.font;
            if let Some(menu) = &self.confirmation {
                frame.rect(16, 128, 288, 24, [8, 12, 24]);
                font.draw_text(
                    frame,
                    24,
                    136,
                    "REPLACE SAVED RUN?",
                    crate::text::Style::plain([255, 210, 100], 1),
                );
                menu.draw(frame, font, &["CANCEL".into(), "START NEW GAME".into()]);
            } else {
                let mut rows = vec!["NEW GAME".into()];
                if self.resume_available {
                    rows.push("CONTINUE".into());
                }
                rows.extend(["OPTIONS".into(), "QUIT".into()]);
                self.viewport.draw(frame, font, &rows);
                if !self.message.is_empty() {
                    let text: String = self.message.chars().take(36).collect();
                    font.draw_text(
                        frame,
                        16,
                        216,
                        &text,
                        crate::text::Style::plain([255, 210, 100], 1),
                    );
                }
            }
        }
    }
    fn menu_context(&mut self, available: bool, status: &str) {
        self.resume_available = available;
        self.viewport.reveal(3 + usize::from(available));
        if self.menu.is_some() {
            self.menu = Some(self.viewport.selected);
        }
        if !status.is_empty() {
            self.message = status.to_ascii_uppercase();
        }
    }
    fn options(&self) -> Option<GameOptions> {
        Some(self.screen.front.options)
    }
    fn set_options(&mut self, options: GameOptions) {
        self.screen.front.options = options;
    }
    fn menu_event(&mut self, event: MenuEvent) {
        if self.screen.leaving() || self.fading.is_some() || self.screen.ticks <= TITLE_INIT_FRAMES
        {
            return;
        }
        if self.menu.is_none() {
            if matches!(event, MenuEvent::Click(..)) {
                self.open_menu();
            }
            return;
        }
        if matches!(event, MenuEvent::Back) {
            self.pointer_input = Input::default();
            if self.confirmation.take().is_none() {
                self.menu = None;
            }
            return;
        }
        let count = if self.confirmation.is_some() {
            2
        } else {
            3 + usize::from(self.resume_available)
        };
        let menu = self.confirmation.as_mut().unwrap_or(&mut self.viewport);
        match event {
            MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) => {
                if let Some(row) = menu.hit(x, y, count) {
                    menu.selected = row;
                    if matches!(event, MenuEvent::Click(..)) {
                        self.pointer_input.pressed |= Button::Start.mask();
                    }
                }
            }
            MenuEvent::Scroll(delta) => menu.scroll(delta, count),
            MenuEvent::Back => {}
        }
        self.menu = Some(self.viewport.selected);
    }
    fn name(&self) -> &'static str {
        "Title"
    }
    fn take_audio(&mut self) -> Audio {
        std::mem::take(&mut self.screen.audio)
    }
}
