use super::{
    CATALOG, Data, Stats, duration,
    presentation::{compact_count, compact_duration, status_copy, wrap},
};
use crate::{
    front_end::{Front, Title},
    input::{Button, Input},
    menu::{Menu, MenuEvent},
    modes::{Mode, ModeChange},
    render::{Color, Frame},
    text::Style,
};

const WHITE: Color = [220, 224, 232];
const MUTED: Color = [164, 176, 196];
const GOLD: Color = [255, 210, 100];
const GREEN: Color = [144, 224, 168];
const WARNING: Color = [255, 176, 128];
const FOCUS: Color = [48, 56, 80];
const TABS: [(i32, i32); 3] = [(16, 80), (104, 80), (192, 112)];
const ROW_HEIGHT: i32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScreenKind {
    Achievements,
    Statistics,
}

struct Metric {
    name: &'static str,
    value: fn(&Stats) -> u64,
    time: bool,
}
const METRICS: [Metric; 10] = [
    Metric {
        name: "PLAY TIME",
        value: |s| s.ticks,
        time: true,
    },
    Metric {
        name: "GAMES CLEARED",
        value: |s| s.completions,
        time: false,
    },
    Metric {
        name: "ROUNDS CLEARED",
        value: |s| s.rounds,
        time: false,
    },
    Metric {
        name: "DEATHS",
        value: |s| s.deaths,
        time: false,
    },
    Metric {
        name: "CONTINUES",
        value: |s| s.continues,
        time: false,
    },
    Metric {
        name: "ENEMY KILLS",
        value: |s| s.enemies,
        time: false,
    },
    Metric {
        name: "STARS THROWN",
        value: |s| s.shurikens,
        time: false,
    },
    Metric {
        name: "MELEE HITS",
        value: |s| s.melee_hits,
        time: false,
    },
    Metric {
        name: "MAGIC USED",
        value: |s| s.ninjutsu,
        time: false,
    },
    Metric {
        name: "ITEMS COLLECTED",
        value: |s| s.pickups,
        time: false,
    },
];

pub struct ProfileScreen {
    front: Front,
    kind: ScreenKind,
    data: Data,
    menus: [Menu; 3],
    tab: usize,
    back: bool,
    status: String,
    show_status: bool,
    pointer: Option<(i32, i32)>,
    repeat_direction: i32,
    repeat_ticks: u16,
}
impl ProfileScreen {
    pub fn new(front: Front, kind: ScreenKind, data: Data, status: String) -> Self {
        let menu = match kind {
            ScreenKind::Achievements => Menu::new(56, 7, ROW_HEIGHT),
            ScreenKind::Statistics => Menu::new(68, 6, ROW_HEIGHT),
        };
        // Compare the latest run with totals from its own category initially.
        let tab = usize::from(
            kind == ScreenKind::Statistics && data.last_run.as_ref().is_some_and(|r| r.practice),
        );
        Self {
            front,
            kind,
            data,
            menus: [menu; 3],
            tab,
            back: false,
            show_status: !status.is_empty(),
            status,
            pointer: None,
            repeat_direction: 0,
            repeat_ticks: 0,
        }
    }
    fn achievements(&self) -> Vec<usize> {
        (0..CATALOG.len())
            .filter(|i| match self.tab {
                1 => !self.data.unlocked.contains_key(i),
                2 => self.data.unlocked.contains_key(i),
                _ => true,
            })
            .collect()
    }
    fn count(&self) -> usize {
        match self.kind {
            ScreenKind::Achievements => self.achievements().len(),
            ScreenKind::Statistics if self.tab == 2 => self.data.best_rounds.len(),
            ScreenKind::Statistics => METRICS.len(),
        }
    }
    fn change_tab(&mut self, tab: usize) {
        if self.kind == ScreenKind::Statistics && self.tab < 2 && tab < 2 {
            // Keep the same metric visible when comparing the two totals.
            self.menus[tab] = self.menus[self.tab];
        }
        self.tab = tab;
        let count = self.count();
        self.menus[tab].reveal(count);
        self.repeat_direction = 0;
        self.repeat_ticks = 0;
        self.show_status = false;
    }
    fn text(&self, frame: &mut Frame, x: i32, y: i32, text: &str, color: Color) {
        let font = &self.front.font;
        let style = Style::plain(color, 1);
        for (i, byte) in text.bytes().enumerate() {
            let x = x + i as i32 * 8;
            // The cartridge map omits commas/apostrophes and maps the ASCII
            // hyphen to a different mark. Keep names and numeric ranges readable.
            match byte {
                b'-' => frame.rect(x + 2, y + 4, 4, 1, color),
                b'\'' => font.draw_tile(frame, font.tile(b'.') | 0x1000, x, y, style),
                b',' => {
                    font.draw_tile(frame, font.tile(b'.'), x, y, style);
                    frame.pixel(x + 4, y + 7, color);
                }
                _ => font.draw_tile(frame, font.tile(byte), x, y, style),
            }
        }
    }
    fn right(&self, frame: &mut Frame, x: i32, y: i32, text: &str, color: Color) {
        self.text(frame, x - text.len() as i32 * 8, y, text, color);
    }
    fn hovered(&self, left: i32, top: i32, width: i32, height: i32) -> bool {
        self.pointer.is_some_and(|(x, y)| {
            (left..left + width).contains(&x) && (top..top + height).contains(&y)
        })
    }
    fn draw_tabs(&self, frame: &mut Frame) {
        let labels = match self.kind {
            ScreenKind::Achievements => ["ALL", "LOCKED", "UNLOCKED"],
            ScreenKind::Statistics => ["STANDARD", "PRACTICE", "BEST TIMES"],
        };
        for (i, (&label, &(x, width))) in labels.iter().zip(TABS.iter()).enumerate() {
            let active = i == self.tab;
            let hovered = self.hovered(x, 36, width, 16);
            frame.rect(
                x,
                36,
                width,
                16,
                if active || hovered {
                    FOCUS
                } else {
                    [18, 24, 40]
                },
            );
            if active {
                frame.rect(x, 51, width, 1, GOLD);
            }
            self.text(
                frame,
                x + (width - label.len() as i32 * 8) / 2,
                40,
                label,
                if active { GOLD } else { WHITE },
            );
        }
    }
    fn totals(&self) -> &Stats {
        if self.tab == 1 {
            &self.data.practice
        } else {
            &self.data.lifetime
        }
    }
    fn value(&self, metric: &Metric, stats: Option<&Stats>, compact: bool) -> String {
        let Some(stats) = stats else {
            return "--".into();
        };
        let value = (metric.value)(stats);
        match (metric.time, compact) {
            (true, true) => compact_duration(value),
            (true, false) => duration(value),
            (false, true) => compact_count(value),
            (false, false) => value.to_string(),
        }
    }
    fn draw_rows(&self, frame: &mut Frame) {
        let menu = self.menus[self.tab];
        let height = menu.visible as i32 * ROW_HEIGHT;
        frame.rect(16, menu.top - 2, 296, height + 4, [12, 18, 32]);
        let indices = self.achievements();
        let count = self.count();
        if self.kind == ScreenKind::Statistics {
            self.text(
                frame,
                24,
                56,
                if self.tab == 2 { "ROUND" } else { "STATISTIC" },
                MUTED,
            );
            if self.tab != 2 {
                self.right(frame, 216, 56, "RUN", MUTED);
            }
            self.right(
                frame,
                288,
                56,
                if self.tab == 2 { "BEST TIME" } else { "TOTAL" },
                MUTED,
            );
        }
        for index in (menu.first..count).take(menu.visible) {
            let y = menu.top + (index - menu.first) as i32 * ROW_HEIGHT;
            if index == menu.selected {
                frame.rect(20, y, 276, ROW_HEIGHT, FOCUS);
            }
            if self.kind == ScreenKind::Statistics && self.tab != 2 {
                for x in [148, 220] {
                    frame.rect(x, y, 1, ROW_HEIGHT, [36, 44, 64]);
                }
            }
            let y = y + 2;
            match self.kind {
                ScreenKind::Achievements => {
                    let id = indices[index];
                    let unlocked = self.data.unlocked.contains_key(&id);
                    if unlocked {
                        for (dx, dy) in [(0, 3), (1, 4), (2, 5), (3, 4), (4, 3), (5, 2), (6, 1)] {
                            frame.pixel(25 + dx, y + dy, GREEN);
                        }
                    } else {
                        frame.rect(25, y, 7, 1, MUTED);
                        frame.rect(25, y + 6, 7, 1, MUTED);
                        frame.rect(25, y, 1, 7, MUTED);
                        frame.rect(31, y, 1, 7, MUTED);
                    }
                    self.text(
                        frame,
                        40,
                        y,
                        &CATALOG[id].name.to_ascii_uppercase(),
                        if unlocked || index == menu.selected {
                            WHITE
                        } else {
                            MUTED
                        },
                    );
                    self.right(frame, 288, y, &CATALOG[id].points.to_string(), MUTED);
                }
                ScreenKind::Statistics if self.tab == 2 => {
                    self.text(frame, 24, y, &format!("ROUND {}", index + 1), WHITE);
                    let time =
                        self.data.best_rounds[index].map_or_else(|| "--".into(), compact_duration);
                    self.right(frame, 288, y, &time, WHITE);
                }
                ScreenKind::Statistics => {
                    let metric = &METRICS[index];
                    self.text(frame, 24, y, metric.name, WHITE);
                    self.right(
                        frame,
                        216,
                        y,
                        &self.value(metric, self.data.last_run.as_ref().map(|r| &r.stats), true),
                        WHITE,
                    );
                    self.right(
                        frame,
                        288,
                        y,
                        &self.value(metric, Some(self.totals()), true),
                        WHITE,
                    );
                }
            }
        }
        if count == 0 {
            self.text(
                frame,
                32,
                80,
                if self.tab == 1 {
                    "EVERY CHALLENGE IS COMPLETE"
                } else {
                    "NO ACHIEVEMENTS UNLOCKED YET"
                },
                GOLD,
            );
            self.text(
                frame,
                32,
                100,
                if self.tab == 1 {
                    "ALL 49 ACHIEVEMENTS EARNED"
                } else {
                    "START NEW GAME TO EARN THEM"
                },
                WHITE,
            );
        }
        let range = if count == 0 {
            "0 OF 0".into()
        } else {
            format!(
                "{}-{} OF {count}",
                menu.first + 1,
                (menu.first + menu.visible).min(count)
            )
        };
        self.text(frame, 16, 144, &range, MUTED);
        if self.kind == ScreenKind::Achievements {
            let points: u16 = self
                .data
                .unlocked
                .keys()
                .filter_map(|i| CATALOG.get(*i))
                .map(|a| a.points)
                .sum();
            let total: u16 = CATALOG.iter().map(|a| a.points).sum();
            self.right(frame, 304, 144, &format!("{points}/{total} PTS"), MUTED);
        } else {
            self.right(
                frame,
                304,
                144,
                if self.tab == 2 {
                    "STANDARD RUNS ONLY"
                } else {
                    "EXACT VALUES BELOW"
                },
                MUTED,
            );
        }
        if count > menu.visible {
            let track = height - 24;
            let thumb = (track * menu.visible as i32 / count as i32).max(8);
            let position = (track - thumb) * menu.first as i32 / (count - menu.visible) as i32;
            frame.rect(301, menu.top + 12, 2, track, FOCUS);
            frame.rect(300, menu.top + 12 + position, 4, thumb, MUTED);
            for (down, enabled) in [
                (false, menu.first > 0),
                (true, menu.first + menu.visible < count),
            ] {
                let top = if down {
                    menu.top + height - 7
                } else {
                    menu.top + 3
                };
                for row in 0..3 {
                    let half = if down { 2 - row } else { row };
                    frame.rect(
                        302 - half,
                        top + row,
                        half * 2 + 1,
                        1,
                        if enabled { WHITE } else { FOCUS },
                    );
                }
            }
        }
    }
    fn details(&self, frame: &mut Frame) {
        frame.rect(16, 155, 288, 1, FOCUS);
        if self.show_status {
            self.text(frame, 16, 160, "PROFILE NOT SAVING", WARNING);
            for (row, line) in wrap(status_copy(&self.status), 36)
                .iter()
                .take(4)
                .enumerate()
            {
                self.text(frame, 16, 172 + row as i32 * 10, line, WHITE);
            }
            return;
        }
        let selected = self.menus[self.tab].selected;
        match self.kind {
            ScreenKind::Achievements => {
                let Some(&id) = self.achievements().get(selected) else {
                    self.text(frame, 16, 160, "TRY ANOTHER FILTER", GOLD);
                    self.text(frame, 16, 176, "LEFT/RIGHT OR CLICK THE TABS", WHITE);
                    return;
                };
                let item = &CATALOG[id];
                let unlocked = self.data.unlocked.contains_key(&id);
                self.text(
                    frame,
                    16,
                    160,
                    &format!(
                        "{} - {} POINTS",
                        if unlocked { "UNLOCKED" } else { "LOCKED" },
                        item.points
                    ),
                    if unlocked { GREEN } else { GOLD },
                );
                for (row, line) in wrap(item.description, 36).iter().enumerate() {
                    self.text(frame, 16, 176 + row as i32 * 10, line, WHITE);
                }
            }
            ScreenKind::Statistics if self.tab == 2 => {
                self.text(
                    frame,
                    16,
                    160,
                    &format!("ROUND {} - BEST CLEAR TIME", selected + 1),
                    GOLD,
                );
                if let Some(time) = self.data.best_rounds[selected] {
                    self.text(frame, 16, 176, &duration(time), WHITE);
                    self.text(frame, 16, 192, "ACTIVE PLAY TIME - STANDARD RUNS", MUTED);
                } else {
                    self.text(frame, 16, 176, "NO STANDARD CLEAR RECORDED", WHITE);
                    self.text(frame, 16, 192, "CLEAR THIS ROUND TO SET A TIME", MUTED);
                }
            }
            ScreenKind::Statistics => {
                let metric = &METRICS[selected];
                self.text(frame, 16, 160, metric.name, GOLD);
                self.text(
                    frame,
                    16,
                    174,
                    &format!(
                        "RUN   {}",
                        self.value(metric, self.data.last_run.as_ref().map(|r| &r.stats), false)
                    ),
                    WHITE,
                );
                self.text(
                    frame,
                    16,
                    186,
                    &format!("TOTAL {}", self.value(metric, Some(self.totals()), false)),
                    WHITE,
                );
                let hint = if self.data.last_run.is_none() {
                    "START NEW GAME TO RECORD A RUN"
                } else if self.data.last_run.as_ref().is_some_and(|r| r.practice) {
                    "PRACTICE RUN - ACHIEVEMENTS OFF"
                } else if metric.time {
                    "TIME COUNTS ACTIVE PLAY ONLY"
                } else {
                    "TOTALS KEEP YOUR LIFETIME PROGRESS"
                };
                self.text(frame, 16, 200, hint, MUTED);
            }
        }
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
        if input.pressed != 0 {
            self.pointer = None;
        }
        if !self.status.is_empty() && (input.pressed(Button::Attack) || input.pressed(Button::Jump))
        {
            self.show_status = !self.show_status;
        }
        let tab_step = i32::from(
            input.pressed(Button::Right)
                || (self.status.is_empty()
                    && (input.pressed(Button::Attack) || input.pressed(Button::Jump))),
        ) - i32::from(input.pressed(Button::Left));
        if tab_step != 0 {
            self.change_tab((self.tab as i32 + tab_step).rem_euclid(3) as usize);
        }
        let held = i32::from(input.held(Button::Down)) - i32::from(input.held(Button::Up));
        if held == 0 || held != self.repeat_direction {
            self.repeat_ticks = 0;
        } else {
            self.repeat_ticks = self.repeat_ticks.saturating_add(1);
        }
        self.repeat_direction = held;
        let pressed = i32::from(input.pressed(Button::Down)) - i32::from(input.pressed(Button::Up));
        let step = if pressed != 0 {
            pressed
        } else if self.repeat_ticks >= 20 && (self.repeat_ticks - 20).is_multiple_of(4) {
            held
        } else {
            0
        };
        if step != 0 {
            let count = self.count();
            self.menus[self.tab].step(step, count);
            self.show_status = false;
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        frame.center_content();
        frame.clear([8, 12, 24]);
        let unlocked = (0..CATALOG.len())
            .filter(|i| self.data.unlocked.contains_key(i))
            .count();
        let title = match self.kind {
            ScreenKind::Achievements => format!("ACHIEVEMENTS {unlocked}/{}", CATALOG.len()),
            ScreenKind::Statistics => "STATISTICS".into(),
        };
        self.text(frame, 16, 8, &title, GOLD);
        if self.hovered(184, 4, 120, 16) {
            frame.rect(184, 4, 120, 16, FOCUS);
        }
        self.text(frame, 192, 8, "START/ESC BACK", WHITE);
        let summary = if !self.status.is_empty() {
            "PROFILE NOT SAVING - B FOR DETAILS"
        } else {
            match self.kind {
                ScreenKind::Achievements
                    if self.data.last_run.as_ref().is_some_and(|r| r.practice) =>
                {
                    "PRACTICE RUN - NO NEW UNLOCKS"
                }
                ScreenKind::Achievements => "LOCAL ACHIEVEMENTS",
                ScreenKind::Statistics if self.tab == 2 => "BEST COMPLETED STANDARD RUNS",
                ScreenKind::Statistics => match &self.data.last_run {
                    None => "NO RUN RECORDED YET",
                    Some(r) if r.practice => "LATEST RUN: PRACTICE",
                    Some(_) => "LATEST RUN: STANDARD",
                },
            }
        };
        self.text(
            frame,
            16,
            24,
            summary,
            if self.status.is_empty() {
                MUTED
            } else {
                WARNING
            },
        );
        self.draw_tabs(frame);
        self.draw_rows(frame);
        self.details(frame);
        self.text(
            frame,
            16,
            214,
            if self.status.is_empty() {
                "UP/DOWN SCROLL  LEFT/RIGHT TABS"
            } else {
                "UP/DOWN SCROLL  B ERROR DETAILS"
            },
            MUTED,
        );
    }
    fn name(&self) -> &'static str {
        "Profile"
    }
    fn menu_context(&mut self, _: bool, status: &str) {
        if self.status != status {
            self.status = status.to_owned();
            self.show_status = !status.is_empty();
        }
    }
    fn menu_event(&mut self, event: MenuEvent) {
        let count = self.count();
        match event {
            MenuEvent::Back => self.back = true,
            MenuEvent::Scroll(n) => {
                self.menus[self.tab].scroll(n, count);
                self.show_status = false;
            }
            MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) => {
                self.pointer = Some((x, y));
                let click = matches!(event, MenuEvent::Click(..));
                if click && (184..304).contains(&x) && (4..24).contains(&y) {
                    self.back = true;
                    return;
                }
                if click
                    && !self.status.is_empty()
                    && (16..304).contains(&x)
                    && (22..34).contains(&y)
                {
                    self.show_status = !self.show_status;
                    return;
                }
                if click && (36..52).contains(&y) {
                    if let Some(tab) = TABS
                        .iter()
                        .position(|&(left, width)| (left..left + width).contains(&x))
                    {
                        self.change_tab(tab);
                    }
                    return;
                }
                let menu = self.menus[self.tab];
                let height = menu.visible as i32 * ROW_HEIGHT;
                if click
                    && count > menu.visible
                    && (296..312).contains(&x)
                    && (menu.top..menu.top + height).contains(&y)
                {
                    let track = height - 24;
                    let thumb = (track * menu.visible as i32 / count as i32).max(8);
                    let thumb_top = menu.top
                        + 12
                        + (track - thumb) * menu.first as i32 / (count - menu.visible) as i32;
                    let delta = if y < menu.top + 12 {
                        -1
                    } else if y >= menu.top + height - 12 {
                        1
                    } else if y < thumb_top {
                        -(menu.visible as i32)
                    } else if y >= thumb_top + thumb {
                        menu.visible as i32
                    } else {
                        0
                    };
                    self.menus[self.tab].scroll(delta, count);
                    self.show_status = false;
                } else if let Some(row) = menu.hit(x, y, count) {
                    self.menus[self.tab].selected = row;
                    self.show_status = false;
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

#[cfg(test)]
#[path = "screen_tests.rs"]
mod tests;
