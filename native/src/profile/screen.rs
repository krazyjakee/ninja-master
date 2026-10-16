use super::{CATALOG, Data, Stats, duration};
use crate::{
    front_end::{Front, Title},
    input::{Button, Input},
    menu::{Menu, MenuEvent},
    modes::{Mode, ModeChange},
    render::Frame,
    text::Style,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKind {
    Achievements,
    Statistics,
}
pub struct ProfileScreen {
    front: Front,
    kind: ScreenKind,
    data: Data,
    menu: Menu,
    practice: bool,
    back: bool,
    status: String,
}
impl ProfileScreen {
    pub fn new(front: Front, kind: ScreenKind, data: Data, status: String) -> Self {
        Self {
            front,
            kind,
            data,
            menu: Menu::new(48, 6, 16),
            practice: false,
            back: false,
            status,
        }
    }
    fn rows(&self) -> Vec<String> {
        match self.kind {
            ScreenKind::Achievements => CATALOG
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    format!(
                        "{} {}",
                        if self.data.unlocked.contains_key(&i) {
                            "U"
                        } else {
                            "L"
                        },
                        a.name.to_ascii_uppercase()
                    )
                })
                .collect(),
            ScreenKind::Statistics => {
                let empty = Stats::default();
                let run = self.data.last_run.as_ref().map_or(&empty, |r| &r.stats);
                let lifetime = if self.practice {
                    &self.data.practice
                } else {
                    &self.data.lifetime
                };
                let mut rows = vec![format!(
                    "PLAY TIME {} / {}",
                    duration(run.ticks),
                    duration(lifetime.ticks)
                )];
                for (label, a, b) in [
                    ("GAMES CLEARED", run.completions, lifetime.completions),
                    ("ROUNDS CLEARED", run.rounds, lifetime.rounds),
                    ("DEATHS", run.deaths, lifetime.deaths),
                    ("CONTINUES", run.continues, lifetime.continues),
                    ("ENEMY KILLS", run.enemies, lifetime.enemies),
                    ("STARS THROWN", run.shurikens, lifetime.shurikens),
                    ("MELEE HITS", run.melee_hits, lifetime.melee_hits),
                    ("MAGIC USED", run.ninjutsu, lifetime.ninjutsu),
                    ("ITEMS COLLECTED", run.pickups, lifetime.pickups),
                ] {
                    rows.push(format!("{label} {} / {}", count(a), count(b)));
                }
                for (i, time) in self.data.best_rounds.iter().enumerate() {
                    rows.push(format!(
                        "BEST ROUND {}: {}",
                        i + 1,
                        time.map_or_else(|| "--".into(), duration)
                    ));
                }
                rows
            }
        }
    }
    fn text(&self, frame: &mut Frame, y: i32, text: &str, color: [u8; 3]) {
        self.front
            .font
            .draw_text(frame, 16, y, text, Style::plain(color, 1));
    }
}
impl Mode for ProfileScreen {
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        if self.back
            || input.pressed(Button::Start)
            || input.pressed(Button::Mode)
            || input.pressed(Button::Ninjutsu)
        {
            return Some(ModeChange::Replace(Box::new(Title::menu(
                self.front.clone(),
            ))));
        }
        let step = i32::from(input.pressed(Button::Down)) - i32::from(input.pressed(Button::Up));
        self.menu.step(step, self.rows().len());
        if self.kind == ScreenKind::Statistics
            && (input.pressed(Button::Left)
                || input.pressed(Button::Right)
                || input.pressed(Button::Attack)
                || input.pressed(Button::Jump))
        {
            self.practice = !self.practice;
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        frame.center_content();
        frame.clear([8, 12, 24]);
        let white = [220, 224, 232];
        let gold = [255, 210, 100];
        let title = match self.kind {
            ScreenKind::Achievements => format!("ACHIEVEMENTS {}/49", self.data.unlocked.len()),
            ScreenKind::Statistics => "STATISTICS".into(),
        };
        self.text(frame, 8, &title, gold);
        if self.kind == ScreenKind::Achievements {
            let points: u16 = self
                .data
                .unlocked
                .keys()
                .filter_map(|i| CATALOG.get(*i))
                .map(|a| a.points)
                .sum();
            self.text(
                frame,
                24,
                &format!("LOCAL ONLY - {points}/590 POINTS"),
                white,
            );
        } else {
            self.text(
                frame,
                24,
                if self.practice {
                    "LATEST RUN / PRACTICE TOTALS"
                } else {
                    "LATEST RUN / STANDARD TOTALS"
                },
                white,
            );
        }
        let rows: Vec<_> = self
            .rows()
            .into_iter()
            .map(|row| row.chars().take(34).collect())
            .collect();
        self.menu.draw(frame, &self.front.font, &rows);
        if self.menu.first > 0 {
            self.front
                .font
                .draw_text(frame, 276, 38, "UP", Style::plain(white, 1));
        }
        if self.kind == ScreenKind::Achievements {
            let item = &CATALOG[self.menu.selected];
            let mut line = String::new();
            let mut y = 160;
            for word in item.description.split_whitespace() {
                if line.len() + word.len() + 1 > 35 {
                    self.text(frame, y, &line.to_ascii_uppercase(), white);
                    line.clear();
                    y += 10;
                }
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str(word);
            }
            self.text(frame, y, &line.to_ascii_uppercase(), white);
            let state = if self.data.unlocked.contains_key(&self.menu.selected) {
                "UNLOCKED"
            } else {
                "LOCKED"
            };
            self.text(
                frame,
                198,
                &format!("{state} - {} POINTS", item.points),
                gold,
            );
        } else {
            let label = match &self.data.last_run {
                None => "NO RUN YET",
                Some(r) if r.practice => "LATEST RUN: PRACTICE",
                Some(_) => "LATEST RUN: STANDARD",
            };
            self.text(frame, 160, label, gold);
            self.text(frame, 176, "LEFT/RIGHT: TOTALS TYPE", white);
            self.text(frame, 190, "BEST TIMES: STANDARD RUNS", white);
        }
        if self.status.is_empty() {
            self.text(frame, 214, "START / ESC: BACK", white);
        } else {
            self.text(
                frame,
                214,
                &self.status.chars().take(36).collect::<String>(),
                gold,
            );
        }
        // An explicit mouse target, outside the scroll viewport.
        self.front
            .font
            .draw_text(frame, 264, 8, "BACK", Style::plain(white, 1));
    }
    fn name(&self) -> &'static str {
        "Profile"
    }
    fn menu_event(&mut self, event: MenuEvent) {
        let count = self.rows().len();
        match event {
            MenuEvent::Back => self.back = true,
            MenuEvent::Scroll(n) => self.menu.scroll(n, count),
            MenuEvent::Click(x, y, _) if (256..320).contains(&x) && (0..24).contains(&y) => {
                self.back = true
            }
            MenuEvent::Click(_, y, _)
                if self.kind == ScreenKind::Statistics && (168..200).contains(&y) =>
            {
                self.practice = !self.practice
            }
            MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) => {
                if let Some(row) = self.menu.hit(x, y, count) {
                    self.menu.selected = row;
                }
            }
        }
    }
    fn set_options(&mut self, options: crate::options::GameOptions) {
        self.front.options = options;
    }
    fn options(&self) -> Option<crate::options::GameOptions> {
        Some(self.front.options)
    }
}

fn count(value: u64) -> String {
    for (unit, divisor) in [
        ("T", 1_000_000_000_000u64),
        ("B", 1_000_000_000),
        ("M", 1_000_000),
    ] {
        if value >= divisor {
            return format!(
                "{}.{:01}{unit}",
                value / divisor,
                (value % divisor) / (divisor / 10)
            );
        }
    }
    value.to_string()
}
