//! The EXPANSIONS menu: installed expansion packs, started as new runs.
use super::{Expansion, KIND, Library, library_directory};
use crate::{
    front_end::{Front, Title},
    input::{Button, Input},
    menu::{Menu, MenuEvent},
    modes::{Audio, Mode, ModeChange},
    render::Frame,
    text::Style,
};
use std::sync::Arc;

const WHITE: [u8; 3] = [220, 224, 232];
const GOLD: [u8; 3] = [255, 210, 100];
const GREY: [u8; 3] = [128, 140, 160];
const HELP_CHARS: usize = 34;

pub struct ExpansionScreen {
    front: Front,
    library: Library,
    menu: Menu,
    back: bool,
    activate: bool,
    message: String,
    repeat_direction: i32,
    repeat_ticks: u16,
    audio: Audio,
}

impl ExpansionScreen {
    /// The installed packs, with `selected` (an expansion id) chosen.
    pub fn new(front: Front, selected: Option<Arc<str>>, message: String) -> Self {
        let library = Library::new(library_directory(), &KIND);
        Self::with_library(front, library, selected, message)
    }
    pub(crate) fn with_library(
        mut front: Front,
        library: Library,
        selected: Option<Arc<str>>,
        message: String,
    ) -> Self {
        // The title music carries on, as in the practice menus.
        let mut audio = Audio::default();
        let music = crate::sound_events::Table::Music
            .command(&front.rom, 0x0d)
            .ok();
        audio.request(music);
        front.music = music;
        let mut menu = Menu::new(56, 7, 18);
        if let Some(id) = selected {
            menu.selected = library
                .packs
                .iter()
                .position(|pack| {
                    Expansion::open(&pack.path).is_ok_and(|expansion| expansion.id == id)
                })
                .unwrap_or(0);
        }
        menu.reveal(library.packs.len());
        Self {
            front,
            library,
            menu,
            back: false,
            activate: false,
            message,
            repeat_direction: 0,
            repeat_ticks: 0,
            audio,
        }
    }
    fn count(&self) -> usize {
        self.library.packs.len()
    }
    fn activate(&mut self) -> Option<ModeChange> {
        let pack = self.library.packs.get(self.menu.selected)?;
        let expansion = Expansion::open(&pack.path).map(Arc::new);
        match expansion.and_then(|e| e.check(&self.front.rom).map(|()| e)) {
            Ok(expansion) => Some(ModeChange::StartExpansion(self.front.clone(), expansion)),
            Err(error) => {
                self.message = error.to_ascii_uppercase();
                None
            }
        }
    }
    fn text(&self, frame: &mut Frame, x: i32, y: i32, text: &str, color: [u8; 3]) {
        // The original font map has no ASCII hyphen glyph.
        for (i, ch) in text.chars().enumerate() {
            let x = x + i as i32 * 8;
            if ch == '-' {
                frame.rect(x + 2, y + 4, 4, 1, color);
            } else {
                self.front
                    .font
                    .draw_text(frame, x, y, &ch.to_string(), Style::plain(color, 1));
            }
        }
    }
}

impl Mode for ExpansionScreen {
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        if std::mem::take(&mut self.back)
            || input.pressed(Button::Mode)
            || input.pressed(Button::Ninjutsu)
        {
            self.activate = false;
            return Some(ModeChange::Replace(Box::new(Title::menu(
                self.front.clone(),
            ))));
        }
        let direction = i32::from(input.held(Button::Down)) - i32::from(input.held(Button::Up));
        let pressed = i32::from(input.pressed(Button::Down)) - i32::from(input.pressed(Button::Up));
        if direction != self.repeat_direction {
            self.repeat_ticks = 0;
        }
        self.repeat_direction = direction;
        let repeat =
            direction != 0 && self.repeat_ticks >= 24 && (self.repeat_ticks - 24).is_multiple_of(5);
        self.repeat_ticks = self.repeat_ticks.saturating_add(1);
        if pressed != 0 || repeat {
            self.menu
                .step(if pressed != 0 { pressed } else { direction }, self.count());
            self.message.clear();
        }
        if std::mem::take(&mut self.activate)
            || [Button::Start, Button::Jump, Button::Attack]
                .iter()
                .any(|&b| input.pressed(b))
        {
            return self.activate();
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        frame.clear([8, 12, 24]);
        self.text(frame, 24, 16, "EXPANSIONS", GOLD);
        self.text(frame, 184, 16, "A/ESC/VIEW BACK", WHITE);
        self.text(frame, 24, 36, "NEW STAGES BEYOND THE ORIGINAL", WHITE);
        for (row, pack) in self
            .library
            .packs
            .iter()
            .enumerate()
            .skip(self.menu.first)
            .take(self.menu.visible)
        {
            let y = self.menu.top + (row - self.menu.first) as i32 * self.menu.row_height;
            if row == self.menu.selected {
                frame.rect(22, y, 276, 18, [48, 56, 80]);
            }
            let name: String = pack.name.to_ascii_uppercase().chars().take(32).collect();
            self.text(frame, 28, y + 5, &name, WHITE);
        }
        if self.count() == 0 {
            self.text(
                frame,
                28,
                self.menu.top + 5,
                "NO EXPANSIONS INSTALLED",
                GREY,
            );
        }
        if self.menu.first > 0 {
            self.text(frame, 280, 46, "^", WHITE);
        }
        if self.menu.first + self.menu.visible < self.count() {
            self.text(frame, 280, 184, "V", WHITE);
        }
        let help = if !self.message.is_empty() {
            self.message.clone()
        } else if self.count() == 0 {
            "ADD PACKS TO THE EXPANSIONS FOLDER".into()
        } else if !self.library.errors.is_empty() {
            format!("{} PACKS COULD NOT BE READ", self.library.errors.len())
        } else {
            "PROGRESS HERE IS NOT RECORDED".into()
        };
        self.text(
            frame,
            24,
            196,
            &help.chars().take(HELP_CHARS).collect::<String>(),
            GOLD,
        );
        self.text(frame, 24, 212, "UP/DOWN SELECT  ENTER/START PLAY", WHITE);
    }
    fn menu_event(&mut self, event: MenuEvent) {
        match event {
            MenuEvent::Back => {
                self.back = true;
                self.activate = false;
            }
            MenuEvent::Scroll(delta) => self.menu.scroll(delta, self.count()),
            MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) => {
                if matches!(event, MenuEvent::Click(..))
                    && (184..304).contains(&x)
                    && (8..32).contains(&y)
                {
                    self.back = true;
                } else if let Some(row) = self.menu.hit(x, y, self.count()) {
                    self.menu.selected = row;
                    self.activate |= matches!(event, MenuEvent::Click(..));
                }
            }
        }
    }
    fn name(&self) -> &'static str {
        "Expansions"
    }
    fn take_audio(&mut self) -> Audio {
        std::mem::take(&mut self.audio)
    }
}
