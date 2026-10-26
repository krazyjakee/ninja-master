//! SDL-independent asset inspection and shared stage presentation.
mod scene;

use crate::{
    WIDTH,
    assets::StageAssets,
    combat::CramReload,
    input::{Button, Input},
    render::{Color, Frame, cram_palettes},
    stage::{
        BossPlanes, BossRaster, COLUMNS, EVENT_CAMERA_FLOOR, EVENT_CAMERA_X, EVENT_FLAG_TICK,
        EVENT_PALETTE_TICK, EVENT_PART, LineWave, PaletteContext, RasterState,
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    Scene,
    Foreground,
    Background,
    Objects,
    Metatiles,
    Palettes,
}
impl View {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "scene" => Ok(Self::Scene),
            "foreground" => Ok(Self::Foreground),
            "background" => Ok(Self::Background),
            "objects" => Ok(Self::Objects),
            "metatiles" => Ok(Self::Metatiles),
            "palettes" => Ok(Self::Palettes),
            _ => Err(format!("unknown asset view: {value}")),
        }
    }
    fn next(self) -> Self {
        match self {
            Self::Scene => Self::Foreground,
            Self::Foreground => Self::Background,
            Self::Background => Self::Objects,
            Self::Objects => Self::Metatiles,
            Self::Metatiles => Self::Palettes,
            Self::Palettes => Self::Scene,
        }
    }
}

#[derive(Clone)]
pub struct Viewer {
    pub assets: StageAssets,
    pub effects_enabled: [bool; 4], // palette, parallax, priority, vegetation
    pub ticks: u64,
    pub event_ticks: Option<u64>,
    pub view: View,
    pub x: i32,
    pub y: i32,
    /// Gameplay's live horizontal camera limits after boss and stage events.
    /// Asset inspection uses the part record when these are absent.
    pub(crate) camera_x_limits: Option<(i32, i32)>,
    /// `$FFE014` (16.16). Play sets it (`Game`); the viewer keeps it at Y.
    pub background_y: i64,
    /// Difference between the continuous background X and the part-local camera.
    pub(crate) background_x_bias: i32,
    /// Scroll-routine RAM advanced by `advance_effects`.
    pub raster: RasterState,
    /// The boss-drawn planes as the boss leaves them (3-3 Hydra, #96;
    /// 5-3 mech body).
    pub boss_raster: Option<BossRaster>,
    /// Boss2's per-line wave on the stage's plane A while VDP register
    /// `$0B` is 3 (`Func_02B832`).
    pub plane_a_wave: Option<LineWave>,
    pub(crate) final_boss_scroll: Option<(u8, LineWave)>,
    pub(crate) final_boss_ticks: u16,
    /// Object-written foreground column scroll, held until the part reloads.
    pub plane_a_vscroll: Option<[i32; COLUMNS]>,
    /// The H-interrupt's CRAM reload while one is installed (Boss4's
    /// `Func_02EF80`); [`Viewer::cram_change`] draws it.
    pub cram_reload: Option<CramReload>,
    /// VDP register `$07`: `Player_Init` (`$22BC0`) writes `$8700`,
    /// Boss7's intro (`$1FA7E`, `Func_00346A`) `$8720`.
    pub backdrop_entry: u8,
    pub page: usize,
    pub palette: usize,
    /// 0 composite, 1 foreground only, 2 background only.
    pub layer: usize,
    /// Driver commands requested by stage effects (`$D38` lightning).
    sound_cues: Vec<u8>,
}

impl Viewer {
    /// The playable horizontal span; decoded layouts may contain padding
    /// beyond the camera limits, particularly in the narrow tower parts.
    pub(crate) fn world_bounds(&self) -> (i32, i32) {
        let width = self.assets.data.pixel_size().0;
        let (left, right) = self.assets.data.scroll.forced_camera_x.map_or(
            self.camera_x_limits.unwrap_or(self.assets.data.camera.x),
            |x| (x, x),
        );
        (left.max(0), (right + WIDTH as i32).min(width))
    }

    pub(crate) fn align_frame(&self, frame: &mut Frame) {
        let (left, right) = self.world_bounds();
        frame.align_world(self.x - left, right - left);
    }

    pub fn new(mut assets: StageAssets, view: View) -> Self {
        // 5-3's stage header intentionally loads black in lines 1 and 3;
        // The asset preview uses the final colours. Game::load_stage restores
        // black before Boss5 runs the entrance fades.
        if let Some(BossPlanes::Mech {
            body_palette,
            arena_palette,
            ..
        }) = &assets.data.boss_planes
        {
            assets.palette_words[1] = *body_palette;
            assets.palette_words[3] = *arena_palette;
            assets.palettes = cram_palettes(&assets.palette_words);
        }
        let boss_raster = BossRaster::for_stage(&assets.data);
        let palette = match view {
            View::Foreground => 3,
            View::Background => 2,
            _ => 1,
        };
        // Non-forest substages open at their part record's camera start.
        let (x, y) = if assets.data.id.is_forest() {
            (0, 304)
        } else {
            let (x, y) = assets.data.section.camera_start;
            (x.max(0), y.max(0))
        };
        Self {
            assets,
            effects_enabled: [true; 4],
            ticks: 0,
            event_ticks: None,
            view,
            x,
            y,
            camera_x_limits: None,
            background_y: i64::from(y) << 16,
            background_x_bias: 0,
            raster: RasterState::default(),
            boss_raster,
            plane_a_wave: None,
            final_boss_scroll: None,
            final_boss_ticks: 0,
            plane_a_vscroll: None,
            cram_reload: None,
            backdrop_entry: 0,
            page: 0,
            palette,
            layer: 0,
            sound_cues: Vec::new(),
        }
    }
    /// SFX commands raised by the stage's palette routine since the last call.
    pub fn take_sound_cues(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.sound_cues)
    }
    /// The camera minimum X the 1-1 event has raised (`$3714C`), from the
    /// tick that switches the background. `event_ticks` restarts with each
    /// part load, as `$1A64` reloads the part's own limit.
    pub fn camera_floor_x(&self) -> Option<i32> {
        self.event_ticks
            .is_some_and(|t| t >= EVENT_FLAG_TICK)
            .then_some(EVENT_CAMERA_FLOOR)
    }
    /// One frame of the part's recovered effects: the `$99A` palette
    /// routine, the `$390E2` tile animation and, in 1-1, the `$39642` event.
    pub fn advance_effects(&mut self) {
        self.ticks += 1;
        if self.final_boss_scroll.is_some_and(|(mode, _)| mode == 7) {
            self.final_boss_ticks = self.final_boss_ticks.wrapping_add(1);
        }
        let data = &self.assets.data;
        let sine = &self.assets.effects.sine;
        self.raster.step(&data.scroll, data.id.part_index(), sine);
        // $39642: round 1-1 part 2 event at local camera X >= $100, once: its
        // $3714C camera floor keeps the $FF11B8 latch from re-arming.
        if self.assets.effects.event.is_some()
            && self.event_ticks.is_none()
            && self.assets.data.id.part_index() == EVENT_PART
            && self.x >= EVENT_CAMERA_X
        {
            self.event_ticks = Some(0);
        }
        if let Some(tick) = &mut self.event_ticks {
            *tick += 1;
        }
        let event = self.event_ticks;
        let palette = &mut self.assets.effects.palette;
        if event == Some(1) {
            palette.stop_ramp(); // $37108
        }
        if event == Some(EVENT_PALETTE_TICK) {
            palette.start_flash(); // $3721A
        }
        self.refresh_effects(event == Some(EVENT_PALETTE_TICK), false);
    }
    /// `Mode_Pause_Refresh` (`$B4B2`): the `$99A` palette routine and the
    /// `$390E2` tile animation keep running under the pause window, so the
    /// 3-1 tank brains (`$395D4`'s two-frame swap, #94) do not freeze on
    /// their empty frame. The scroll routines, the 1-1 event and `ticks`
    /// hold.
    pub fn advance_paused_effects(&mut self) {
        self.refresh_effects(false, true);
    }
    /// The `$390E2` tile animation and the `$99A` palette routine;
    /// `event_palette` is the 1-1 event's `$371D8` selector load.
    fn refresh_effects(&mut self, event_palette: bool, paused: bool) {
        let event = self.event_ticks;
        let background_x = self.background_x();
        let assets = &mut self.assets;
        if self.effects_enabled[3]
            && let Some(animation) = &mut assets.effects.animation
        {
            let flag = event.is_some_and(|t| t >= EVENT_FLAG_TICK);
            animation.tick(flag, background_x, &mut assets.tiles);
        }
        if self.effects_enabled[0] {
            if event_palette && let Some(event) = &assets.effects.event {
                // $371D8 loads selector 15 into the `$FF16FA` fade targets
                // only; `$371F0` fades just line 2 (`$FFF040`) toward them.
                // Line 1 keeps the `$1300` samurai line and its writes.
                assets.palette_words[2] = event.palettes[2];
            }
            let context = PaletteContext {
                background_x,
                paused,
                part: assets.data.id.part_index(),
                vdp_mode_3: self
                    .final_boss_scroll
                    .map_or(assets.data.flags.vdp_mode_3(), |(mode, _)| mode),
            };
            if let Some(command) = assets
                .effects
                .palette
                .step(&mut assets.palette_words, &context)
            {
                self.sound_cues.push(command);
            }
            assets.palettes = cram_palettes(&assets.palette_words);
        }
    }
    pub fn tick(&mut self, input: Input) -> bool {
        self.advance_effects();
        if input.pressed(Button::Jump) {
            self.view = self.view.next();
            self.page = 0;
            self.palette = match self.view {
                View::Foreground => 3,
                View::Background => 2,
                _ => 1,
            };
        }
        if input.pressed(Button::Start) {
            self.x = 0;
            self.y = 304;
            self.background_y = 304 << 16;
            self.page = 0;
            self.layer = 0;
            self.palette = match self.view {
                View::Foreground => 3,
                View::Background => 2,
                _ => 1,
            };
        }
        if input.pressed(Button::Attack) {
            if self.view == View::Scene {
                self.layer = (self.layer + 1) % 3;
            } else {
                self.palette = (self.palette + 1) % 4;
            }
        }
        if self.view == View::Scene {
            self.x +=
                4 * (i32::from(input.held(Button::Right)) - i32::from(input.held(Button::Left)));
            self.y += 4 * (i32::from(input.held(Button::Down)) - i32::from(input.held(Button::Up)));
            let (width, height) = self.assets.data.pixel_size();
            self.x = self.x.clamp(0, (width - WIDTH as i32).max(0));
            self.y = self.y.clamp(0, (height - 192).max(0));
            self.background_y = i64::from(self.y) << 16;
        } else {
            if input.pressed(Button::Right) {
                self.page = (self.page + 1).min(self.pages() - 1);
            }
            if input.pressed(Button::Left) {
                self.page = self.page.saturating_sub(1);
            }
        }
        false
    }
    fn range(&self) -> std::ops::Range<usize> {
        match self.view {
            View::Foreground => self.assets.foreground_range.clone(),
            View::Background => self.assets.background_range.clone(),
            View::Objects => self.assets.object_range.clone(),
            _ => 0..0,
        }
    }
    pub fn pages(&self) -> usize {
        if self.view == View::Metatiles {
            self.assets.data.metatiles.len().div_ceil(40).max(1)
        } else {
            self.range().len().div_ceil(160).max(1)
        }
    }
    fn sheet(&self, frame: &mut Frame) {
        let range = self.range();
        for (i, tile) in range.skip(self.page * 160).take(160).enumerate() {
            let x = 16 + (i % 16) as i32 * 18;
            let y = 24 + (i / 16) as i32 * 18;
            for py in 0..8 {
                for px in 0..8 {
                    let byte = self.assets.tiles[tile].0[py * 4 + px / 2];
                    let n = if px % 2 == 0 { byte >> 4 } else { byte & 15 };
                    let color = if n == 0 {
                        checker(px as i32, py as i32)
                    } else {
                        self.assets.palettes[self.palette][usize::from(n)]
                    };
                    frame.rect(x + px as i32 * 2, y + py as i32 * 2, 2, 2, color);
                }
            }
        }
    }
    fn metatiles(&self, frame: &mut Frame) {
        // Batch thumbnails so the full scratch frame is resolved only once.
        let mut scratch = Frame::default();
        scratch.clear([22, 29, 37]);
        let page = self
            .assets
            .data
            .metatiles
            .iter()
            .skip(self.page * 40)
            .take(40);
        for (i, attrs) in page.clone().enumerate() {
            for (j, attr) in attrs.iter().enumerate() {
                self.attr(
                    &mut scratch,
                    *attr,
                    (i % 8 * 16 + j % 2 * 8) as i32,
                    (i / 8 * 16 + j / 2 * 8) as i32,
                );
            }
        }
        let rgb = scratch.rgb();
        for (i, _) in page.enumerate() {
            let x = (i % 8) as i32 * 40 + 4;
            let y = (i / 8) as i32 * 38 + 18;
            for py in 0..16 {
                for px in 0..16 {
                    let p = ((i / 8 * 16 + py) * WIDTH + i % 8 * 16 + px) * 3;
                    frame.rect(
                        x + px as i32 * 2,
                        y + py as i32 * 2,
                        2,
                        2,
                        rgb[p..p + 3].try_into().unwrap(),
                    );
                }
            }
            text(
                frame,
                x,
                y + 32,
                &format!("{:02X}", self.page * 40 + i),
                [139, 162, 177],
            );
        }
    }
    fn palettes(&self, frame: &mut Frame) {
        for bank in 0..4 {
            let y = 30 + bank as i32 * 43;
            text(frame, 8, y, &format!("BANK {bank}"), [123, 225, 197]);
            for i in 0..16 {
                let x = 8 + i as i32 * 19;
                frame.rect(x, y + 9, 17, 17, self.assets.palettes[bank][i]);
                text(
                    frame,
                    x,
                    y + 29,
                    &format!("{:03X}", self.assets.palette_words[bank][i]),
                    [176, 190, 201],
                );
            }
        }
    }
    pub fn draw(&self, frame: &mut Frame) {
        frame.center_content();
        frame.clear([13, 20, 27]);
        if self.view == View::Scene {
            self.align_frame(frame);
        }
        match self.view {
            View::Scene => self.scene(frame),
            View::Metatiles => self.metatiles(frame),
            View::Palettes => self.palettes(frame),
            _ => self.sheet(frame),
        }
        if self.view == View::Scene {
            self.clear_world_padding(frame);
        }
        frame.center_content();
        frame.rect(frame.view_left(), 0, frame.width() as i32, 16, [8, 15, 22]);
        frame.rect(
            frame.view_left(),
            208,
            frame.width() as i32,
            16,
            [8, 15, 22],
        );
        let title = match self.view {
            View::Scene if !self.assets.data.id.is_forest() => format!(
                "ROUND {}-{} PART {} / {:?}   X:{} Y:{}   LAYER:{}",
                self.assets.data.id.round,
                self.assets.data.id.substage,
                self.assets.data.id.part,
                self.assets.data.special,
                self.x,
                self.y,
                self.layer
            ),
            View::Scene => format!(
                "ROUND 1-1 / FOREST   X:{} Y:{}   LAYER:{}",
                self.x, self.y, self.layer
            ),
            View::Metatiles => format!(
                "METATILES / PAGE {} OF {} / NATIVE ATTRIBUTES",
                self.page + 1,
                self.pages()
            ),
            View::Palettes => "PALETTES / CURRENT STATE / GENESIS RGB WORDS".into(),
            _ => format!(
                "{:?} / PAGE {} OF {} / PAL {} / FIRST ${:03X}",
                self.view,
                self.page + 1,
                self.pages(),
                self.palette,
                self.range().start + self.page * 160
            ),
        };
        text(frame, 5, 5, &title.to_uppercase(), [123, 225, 197]);
        text(
            frame,
            5,
            212,
            "Z:NEXT VIEW  X:PALETTE/LAYER  ENTER:RESET  ESC:QUIT",
            [176, 190, 201],
        );
        text(
            frame,
            5,
            219,
            if self.view == View::Scene && !self.assets.data.id.is_forest() {
                "ARROWS:PAN  /  STAGE ASSET PREVIEW"
            } else if self.view == View::Scene {
                "ARROWS:PAN  /  FOREST ASSET PREVIEW"
            } else {
                "LEFT/RIGHT:PAGE  /  COLOR INDEX 0 IS TRANSPARENT"
            },
            [105, 131, 147],
        );
    }
}

fn checker(x: i32, y: i32) -> Color {
    if (x / 2 + y / 2) % 2 == 0 {
        [31, 40, 50]
    } else {
        [22, 29, 37]
    }
}

// Tiny code-native inspector font (3x5), not extracted game content.
pub(crate) fn text(frame: &mut Frame, x: i32, y: i32, value: &str, color: Color) {
    for (i, ch) in value.chars().enumerate() {
        for (dy, row) in glyph(ch).iter().enumerate() {
            for dx in 0..3 {
                if row & (4 >> dx) != 0 {
                    frame.pixel(x + i as i32 * 4 + dx, y + dy as i32, color);
                }
            }
        }
    }
}

/// A character's five rows of three pixels (bit 2 leftmost).
pub(crate) fn glyph(ch: char) -> [u8; 5] {
    match ch {
        'A' => [2, 5, 7, 5, 5],
        'B' => [6, 5, 6, 5, 6],
        'C' => [3, 4, 4, 4, 3],
        'D' => [6, 5, 5, 5, 6],
        'E' => [7, 4, 6, 4, 7],
        'F' => [7, 4, 6, 4, 4],
        'G' => [3, 4, 5, 5, 3],
        'H' => [5, 5, 7, 5, 5],
        'I' => [7, 2, 2, 2, 7],
        'J' => [1, 1, 1, 5, 2],
        'K' => [5, 5, 6, 5, 5],
        'L' => [4, 4, 4, 4, 7],
        'M' => [5, 7, 7, 5, 5],
        'N' => [5, 7, 7, 7, 5],
        'O' => [2, 5, 5, 5, 2],
        'P' => [6, 5, 6, 4, 4],
        'Q' => [2, 5, 5, 3, 1],
        'R' => [6, 5, 6, 5, 5],
        'S' => [3, 4, 2, 1, 6],
        'T' => [7, 2, 2, 2, 2],
        'U' => [5, 5, 5, 5, 7],
        'V' => [5, 5, 5, 5, 2],
        'W' => [5, 5, 7, 7, 5],
        'X' => [5, 5, 2, 5, 5],
        'Y' => [5, 5, 2, 2, 2],
        'Z' => [7, 1, 2, 4, 7],
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [6, 1, 2, 4, 7],
        '3' => [6, 1, 2, 1, 6],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 6, 1, 6],
        '6' => [3, 4, 7, 5, 7],
        '7' => [7, 1, 2, 2, 2],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 6],
        ':' => [0, 2, 0, 2, 0],
        '/' => [1, 1, 2, 4, 4],
        '-' => [0, 0, 7, 0, 0],
        '$' => [3, 6, 3, 6, 2],
        _ => [0; 5],
    }
}
