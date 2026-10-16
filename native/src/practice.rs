//! Stage practice menus and persisted visual ghosts. Simulation uses only game ticks.
use crate::{
    front_end::{Front, Title},
    input::{Button, Input},
    menu::{Menu, MenuEvent},
    modes::{Audio, Mode, ModeChange},
    profile::Data,
    render::Frame,
    stage::StageId,
    text::Style,
};
use serde::{Deserialize, Serialize};
use std::rc::Rc;

pub(crate) const MAX_GHOST_FRAMES: usize = 60 * 60 * 60 + 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Explore,
    TimeTrial,
}
impl Kind {
    fn title(self) -> &'static str {
        match self {
            Self::Explore => "EXPLORE",
            Self::TimeTrial => "TIME TRIAL",
        }
    }
}

/// Part, world position, animation frame, facing, and optional horse frame.
/// Arrays keep a recording compact on disk; no ROM art is stored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GhostFrame(pub u8, pub i32, pub i32, pub u16, pub bool, pub Option<u16>);
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub ticks: u64,
    pub frames: Vec<GhostFrame>,
}
impl Record {
    pub(crate) fn valid(&self) -> bool {
        self.ticks > 0
            && self.frames.len() <= MAX_GHOST_FRAMES
            && self.ticks == self.frames.len().saturating_sub(1) as u64
            && self.frames.iter().all(|f| {
                f.0 < 32
                    && (-65536..=65536).contains(&f.1)
                    && (-65536..=65536).contains(&f.2)
                    && f.3 < 256
                    && f.5.is_none_or(|n| n < 256)
            })
    }
}
pub struct Attempt {
    pub completed: bool,
    pub valid: bool,
    pub record: Record,
}
#[derive(Clone)]
pub(crate) struct Session {
    pub front: Front,
    pub kind: Kind,
    pub stage: StageId,
    pub ticks: u64,
    pub valid: bool,
    pub frames: Vec<GhostFrame>,
    pub ghost: Option<Rc<Record>>,
}

pub fn stage_key(stage: StageId) -> String {
    format!("{}-{}", stage.round, stage.substage)
}
pub fn unlocked(data: &Data, stage: StageId) -> bool {
    data.completed_stages.contains(&stage_key(stage))
        // Existing profiles can prove whole-round clears via their achievements.
        || data.unlocked.contains_key(&stage.round_index())
}
pub fn time(ticks: u64) -> String {
    format!(
        "{:02}:{:02}.{:02}",
        ticks / 3600,
        ticks / 60 % 60,
        ticks % 60 * 100 / 60
    )
}

pub struct PracticeScreen {
    front: Front,
    data: Data,
    kind: Option<Kind>,
    stages: Vec<StageId>,
    menu: Menu,
    back: bool,
    activate: bool,
    message: String,
    repeat_direction: i32,
    repeat_ticks: u16,
    audio: Audio,
}
impl PracticeScreen {
    pub fn new(
        mut front: Front,
        data: Data,
        kind: Option<Kind>,
        selected: Option<StageId>,
        message: String,
    ) -> Self {
        let mut audio = Audio::default();
        let music = crate::sound_events::Table::Music
            .command(&front.rom, 0x0d)
            .ok();
        audio.request(music);
        front.music = music;
        let stages = crate::stage::all_substages(&front.rom).unwrap_or_default();
        let mut menu = Menu::new(56, 7, 18);
        if let Some(stage) = selected {
            menu.selected = stages
                .iter()
                .position(|&id| id == stage.with_part(0))
                .unwrap_or(0);
        }
        menu.reveal(if kind.is_some() { stages.len() } else { 2 });
        Self {
            front,
            data,
            kind,
            stages,
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
        if self.kind.is_some() {
            self.stages.len()
        } else {
            2
        }
    }
    fn activate(&mut self) -> Option<ModeChange> {
        if let Some(kind) = self.kind {
            let stage = *self.stages.get(self.menu.selected)?;
            if !unlocked(&self.data, stage) {
                self.message = "COMPLETE THIS STAGE IN NORMAL GAME".into();
                return None;
            }
            Some(ModeChange::StartPractice(self.front.clone(), kind, stage))
        } else {
            self.kind = Some(if self.menu.selected == 0 {
                Kind::Explore
            } else {
                Kind::TimeTrial
            });
            self.menu.selected = 0;
            self.menu.first = 0;
            self.menu.reveal(self.stages.len());
            self.message.clear();
            None
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
impl Mode for PracticeScreen {
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        if std::mem::take(&mut self.back)
            || input.pressed(Button::Mode)
            || input.pressed(Button::Ninjutsu)
        {
            // Back wins over confirmation from any device in this frame.
            self.activate = false;
            if let Some(kind) = self.kind.take() {
                self.menu = Menu::new(56, 7, 18);
                self.menu.selected = usize::from(kind == Kind::TimeTrial);
                self.message.clear();
                self.repeat_direction = 0;
                self.repeat_ticks = 0;
                return None;
            }
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
        let white = [220, 224, 232];
        let gold = [255, 210, 100];
        self.text(
            frame,
            24,
            16,
            self.kind.map_or("PRACTICE", Kind::title),
            gold,
        );
        self.text(frame, 184, 16, "A/ESC/VIEW BACK", white);
        self.text(
            frame,
            24,
            36,
            match self.kind {
                Some(Kind::TimeTrial) => "STAGE              BEST TIME",
                Some(Kind::Explore) => "SELECT A STAGE TO PRACTICE",
                None => "SELECT A PRACTICE MODE",
            },
            white,
        );
        if self.kind.is_none() {
            for (row, label) in ["EXPLORE", "TIME TRIAL"].iter().enumerate() {
                let y = self.menu.top + row as i32 * self.menu.row_height;
                if row == self.menu.selected {
                    frame.rect(22, y, 276, 18, [48, 56, 80]);
                }
                self.text(frame, 28, y + 5, label, white);
            }
        } else {
            for (row, &stage) in self
                .stages
                .iter()
                .enumerate()
                .skip(self.menu.first)
                .take(self.menu.visible)
            {
                let y = self.menu.top + (row - self.menu.first) as i32 * self.menu.row_height;
                if row == self.menu.selected {
                    frame.rect(22, y, 276, 18, [48, 56, 80]);
                }
                let open = unlocked(&self.data, stage);
                let color = if open { white } else { [128, 140, 160] };
                self.text(
                    frame,
                    28,
                    y + 5,
                    &format!("STAGE {}-{}", stage.round, stage.substage),
                    color,
                );
                let value = if !open {
                    "LOCKED".into()
                } else if self.kind == Some(Kind::Explore) {
                    "EXPLORE".into()
                } else {
                    self.data
                        .time_trials
                        .get(&stage_key(stage))
                        .filter(|r| r.valid())
                        .map_or("--:--.--".into(), |r| time(r.ticks))
                };
                self.text(frame, 288 - value.len() as i32 * 8, y + 5, &value, color);
            }
            if self.menu.first > 0 {
                self.text(frame, 280, 46, "^", white);
            }
            if self.menu.first + self.menu.visible < self.count() {
                self.text(frame, 280, 184, "V", white);
            }
        }
        let help = if !self.message.is_empty() {
            &self.message
        } else if self.kind == Some(Kind::TimeTrial) {
            "RACE YOUR BEST GHOST. START: RACE"
        } else if self.kind == Some(Kind::Explore) {
            "NO ENEMIES. START: EXPLORE"
        } else if self.menu.selected == 0 {
            "EXPLORE STAGES WITHOUT ENEMIES"
        } else {
            "RACE THE GHOST OF YOUR BEST TIME"
        };
        self.text(
            frame,
            24,
            196,
            &help.chars().take(34).collect::<String>(),
            gold,
        );
        self.text(frame, 24, 212, "UP/DOWN SELECT  ENTER/START CHOOSE", white);
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
    fn options(&self) -> Option<crate::options::GameOptions> {
        Some(self.front.options)
    }
    fn set_options(&mut self, options: crate::options::GameOptions) {
        self.front.options = options;
    }
    fn name(&self) -> &'static str {
        "Practice"
    }
    fn take_audio(&mut self) -> Audio {
        std::mem::take(&mut self.audio)
    }
}

#[cfg(test)]
mod tests;
