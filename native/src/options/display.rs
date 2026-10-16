use super::*;

const TOPS: [i32; 11] = [44, 56, 68, 96, 108, 136, 148, 160, 172, 184, 184];
const BACK: usize = 9;
const RESET: usize = 10;
const LABELS: [&str; 9] = [
    "FULLSCREEN",
    "WINDOW",
    "VIEW",
    "ASPECT",
    "PIXEL SCALE",
    "WAVES",
    "CRT FILTER",
    "CURVATURE",
    "SCANLINES",
];

impl OptionsScreen {
    pub(super) fn change_page(&mut self, display: bool) {
        if self.display_page != display {
            self.display_page = display;
            self.repeat = 0;
            self.secret = 0;
            self.feedback.clear();
            self.screen.audio.push(STOP_ALL);
        }
    }

    fn display_enabled(&self, row: usize) -> bool {
        self.display_available
            && match row {
                1 => !self.display.fullscreen,
                3 => !self.display.widescreen,
                6 => self.crt_error.is_none(),
                7 => self.display.crt && self.crt_error.is_none(),
                _ => true,
            }
    }

    fn display_step(&mut self, delta: i32) {
        if delta == 0 || !self.display_enabled(self.display_cursor) {
            return;
        }
        let old = self.display;
        let display = &mut self.display;
        match self.display_cursor {
            0 => display.fullscreen = !display.fullscreen,
            1 => {
                display.scale = (i64::from(display.scale) + i64::from(delta)).clamp(
                    i64::from(crate::settings::MIN_SCALE),
                    i64::from(crate::settings::MAX_SCALE),
                ) as u32
            }
            2 => display.widescreen = !display.widescreen,
            3 => display.aspect = (display.aspect as i32 + delta).rem_euclid(3) as usize,
            4 => display.integer_scale = !display.integer_scale,
            5 => display.smooth_wave_effects = !display.smooth_wave_effects,
            6 => display.crt = !display.crt,
            7 => {
                display.crt_curvature =
                    (i32::from(display.crt_curvature) + delta).clamp(0, 100) as u8
            }
            8 => display.scanlines = (i32::from(display.scanlines) + delta).clamp(0, 100) as u8,
            _ => {}
        }
        self.display_changed |= self.display != old;
    }

    pub(super) fn display_menu(&mut self, input: Input) {
        let vertical =
            i32::from(input.pressed(Button::Down)) - i32::from(input.pressed(Button::Up));
        if vertical != 0 {
            let next = self.display_cursor as i32 + vertical;
            if !(0..TOPS.len() as i32).contains(&next) {
                self.tabs_focused = true;
            } else {
                self.display_cursor = next as usize;
            }
            return;
        }
        let horizontal =
            i32::from(input.pressed(Button::Right)) - i32::from(input.pressed(Button::Left));
        let activate = [Button::Attack, Button::Jump, Button::Ninjutsu]
            .iter()
            .any(|&button| input.pressed(button));
        if self.display_cursor >= BACK {
            if horizontal != 0 {
                self.display_cursor = if horizontal < 0 { BACK } else { RESET };
            } else if activate {
                if self.display_cursor == BACK {
                    self.exit();
                } else if self.display_available {
                    let defaults = DisplaySettings::default();
                    self.display_changed |= self.display != defaults;
                    self.display = defaults;
                }
            }
        } else {
            self.display_step(if horizontal != 0 {
                horizontal
            } else if activate {
                1
            } else {
                0
            });
        }
    }

    pub(super) fn display_event(&mut self, event: MenuEvent) {
        match event {
            MenuEvent::Scroll(delta) => {
                self.tabs_focused = false;
                self.repeat = 0;
                self.display_cursor =
                    (self.display_cursor as i64 + i64::from(delta)).clamp(0, RESET as i64) as usize;
            }
            MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) if (24..304).contains(&x) => {
                let row = (0..BACK)
                    .find(|&row| (TOPS[row]..TOPS[row] + 12).contains(&y))
                    .or_else(|| {
                        if (TOPS[BACK]..TOPS[BACK] + 12).contains(&y) {
                            if x < 112 {
                                Some(BACK)
                            } else if x >= 120 {
                                Some(RESET)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    });
                if let Some(row) = row {
                    self.tabs_focused = false;
                    self.repeat = 0;
                    self.display_cursor = row;
                    if let MenuEvent::Click(_, _, alternate) = event {
                        if row >= BACK {
                            self.display_menu(Input {
                                held: 0,
                                pressed: Button::Attack.mask(),
                            });
                        } else if matches!(row, 7 | 8)
                            && (216..280).contains(&x)
                            && !alternate
                            && self.display_enabled(row)
                        {
                            let value = ((x - 216) * 100 / 63) as u8;
                            let target = if row == 7 {
                                &mut self.display.crt_curvature
                            } else {
                                &mut self.display.scanlines
                            };
                            self.display_changed |= *target != value;
                            *target = value;
                        } else {
                            self.display_step(if x < VALUE_MIDDLE || alternate { -1 } else { 1 });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn draw_tabs(&self, frame: &mut Frame) {
        for (display, x, width, label) in [(false, 112, 64, "GAME"), (true, 184, 112, "DISPLAY")] {
            let active = self.display_page == display;
            frame.rect(x, 12, width, 16, if active { FOCUS } else { PANEL });
            if active {
                frame.rect(x, 27, width, 1, GOLD);
                if self.tabs_focused {
                    frame.rect(x, 12, width, 1, GOLD);
                }
            }
            self.draw_text(
                frame,
                x + (width - label.len() as i32 * 8) / 2,
                16,
                label,
                if active { GOLD } else { MUTED },
            );
        }
    }

    fn display_help(&self) -> &'static str {
        if !self.display_available {
            return "DISPLAY SETTINGS REQUIRE DESKTOP";
        }
        match self.display_cursor {
            0 => "DESKTOP FULLSCREEN / F11 TO TOGGLE",
            1 if self.display.fullscreen => "LEAVE FULLSCREEN TO RESIZE WINDOW",
            1 => "WINDOW SIZE IN WHOLE PIXEL STEPS",
            2 => "WIDESCREEN FITS WINDOW OR MONITOR",
            3 if self.display.widescreen => "WIDESCREEN CHOOSES ASPECT RATIO",
            3 => "NATIVE PIXELS / 4:3 / FILL WINDOW",
            4 => "INTEGER: SHARP / FIT: LARGER VIEW",
            5 => "SMOOTH OR ORIGINAL SCENERY WAVES",
            6 | 7 if self.crt_error.is_some() => "CRT UNAVAILABLE / FLAT OUTPUT",
            6 => "PHOSPHOR BEAMS / GRILLE / GLOW",
            7 if !self.display.crt => "ENABLE CRT TO ADJUST THE CURVATURE",
            7 => "0: FLAT GLASS / 25: SUBTLE CURVE",
            8 => "0: OFF / 100: DARKEST SCANLINES",
            RESET => "B OR CLICK: RESET DISPLAY ONLY",
            _ => "DISPLAY CHANGES APPLY IMMEDIATELY",
        }
    }

    pub(super) fn draw_display(&self, frame: &mut Frame) {
        for (top, label, end) in [
            (32, "WINDOW", 88),
            (84, "SCALING", 96),
            (124, "EFFECTS", 96),
        ] {
            self.draw_text(frame, 32, top, label, GOLD);
            frame.rect(end, top + 4, 296 - end, 1, FOCUS);
        }
        let display = self.display;
        let values = [
            if display.fullscreen { "ON" } else { "OFF" }.into(),
            format!("{}X", display.scale),
            if display.widescreen {
                "WIDESCREEN"
            } else {
                "ORIGINAL"
            }
            .into(),
            if display.widescreen {
                "AUTO"
            } else {
                ["10:7 PIXELS", "4:3 DISPLAY", "FILL WINDOW"][display.aspect.min(2)]
            }
            .into(),
            if display.integer_scale {
                "INTEGER"
            } else {
                "FIT"
            }
            .into(),
            if display.smooth_wave_effects {
                "SMOOTH"
            } else {
                "STEPPED"
            }
            .into(),
            if self.crt_error.is_some() {
                "UNAVAILABLE"
            } else if display.crt {
                "ON"
            } else {
                "OFF"
            }
            .into(),
            format!("{}%", display.crt_curvature),
            format!("{}%", display.scanlines),
        ];
        for row in 0..=RESET {
            let (left, width) = match row {
                BACK => (24, 88),
                RESET => (120, 184),
                _ => (24, 280),
            };
            let selected = self.display_cursor == row && !self.tabs_focused;
            if selected {
                frame.rect(left, TOPS[row], width, 12, FOCUS);
                frame.rect(left, TOPS[row] + 2, 2, 8, GOLD);
            }
            let enabled = row == BACK || self.display_enabled(row);
            let color = if !enabled {
                MUTED
            } else if selected {
                GOLD
            } else {
                WHITE
            };
            let label = match row {
                BACK => "BACK",
                RESET => "RESET DISPLAY",
                _ => LABELS[row],
            };
            self.draw_text(frame, left + 8, TOPS[row] + 2, label, color);
            if row < BACK {
                self.draw_text(frame, VALUE_X, TOPS[row] + 2, &values[row], color);
                if enabled {
                    self.arrow(frame, 130, TOPS[row] + 2, false, color);
                    self.arrow(frame, 290, TOPS[row] + 2, true, color);
                }
                if matches!(row, 7 | 8) {
                    let value = if row == 7 {
                        display.crt_curvature
                    } else {
                        display.scanlines
                    };
                    frame.rect(216, TOPS[row] + 5, 64, 3, FOCUS);
                    frame.rect(216, TOPS[row] + 5, i32::from(value) * 64 / 100, 3, color);
                }
            }
        }
        frame.rect(24, 198, 272, 1, FOCUS);
        self.draw_text(
            frame,
            24,
            200,
            if self.tabs_focused {
                "LEFT/RIGHT:TABS DOWN:OPTIONS"
            } else {
                "L/R:CHANGE X/Y:TABS START:BACK"
            },
            WHITE,
        );
        self.draw_text(frame, 24, 212, self.display_help(), MUTED);
    }
}
