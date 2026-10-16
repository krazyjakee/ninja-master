//! `Mode_Ending` (`$10436`, #44): the epilogue, the staff roll and TO BE
//! CONTINUED!, installed by `Mode_Game` after 7-3 (`$B1A0`).
//!
//! - `Ending_S00..S14`: music `$0F` (SAKURA) over the cliff scene, the
//!   falling wing and a white flash, drawn from the ROM planes and mappings.
//! - `Ending_S18..S20`: the epilogue, page by page from `Data_01147A`
//!   (text at `$1129E`, 34 columns, rows 24 pixels apart). Each page is
//!   written below the screen and crawls up at `Ending_ScrollSpeeds`
//!   (`$3000` / 65536 pixels a frame) under the scene, whose raster split
//!   is at line 160 (`Func_0109EA`: H-interrupt at `$A0`); then 300 frames.
//! - `Ending_S24..S30`: fade, music `$1B`, 120 frames, then the staff roll
//!   (`Ending_BuildStaffRoll`: 19-character lines from `$10E70`, each
//!   followed by a repeat count, `$FE` or a negative word ends) scrolls a
//!   quarter pixel a frame with a new line every 64 frames
//!   (`Func_010B74`), until the last line is out.
//! - `Ending_S34..S44`: the sunset palette, 3600 frames with TO BE
//!   CONTINUED! on screen, the rider leaves as the music fades (`$06`), a
//!   fade and 60 frames, then `Mode_SegaLogo` (`$107DA`).
//!
//! The staff roll uses the original sunset planes and rider. Native addition: Start
//! during the 3600-frame hold moves on to the rider's exit.

mod cliff;
mod scene;

use crate::{
    front_end::{Front, Next, WHITE, go},
    game::Game,
    input::{Button, Input},
    modes::{Audio, Mode, ModeChange},
    render::{Fade, Frame, md_color},
    rom::{pointer, word, words},
    sound_events::Table,
    text::Style,
};

/// `Data_01147A`: the text base, then per page (offset, width - 1,
/// rows - 1, frames), ending with a negative offset.
const PAGES: usize = 0x1147a;
/// `Ending_ScrollSpeeds` (NTSC entry).
const SCROLL_SPEED: usize = 0x11296;
/// `Ending_ScrollStep`: `$4000` a frame (16.16).
const ROLL_SPEED: u32 = 0x4000;
const STAFF_ROLL: usize = 0x10e70;
const STAFF_COLUMNS: usize = 19;
/// Epilogue rows are drawn with a `$180`-byte step: three plane rows.
const PAGE_ROW_STEP: i32 = 24;
/// Plane A below the raster split holds the text: it starts at VRAM
/// `$CE04` (row 28, column 2) and shows under line 160.
const PAGE_TOP: i32 = 28 * 8;
const PAGE_LEFT: i32 = 2 * 8;
const SPLIT_LINE: i32 = 160;
/// Staff roll: lines at column 10 (`Data_00C014`), 16 pixels apart; a new
/// line shows at screen line 208 (`$FF00FC` starts at `$130`).
const ROLL_LINE_TICKS: u32 = 64;
/// Music: SAKURA (`Ending_S00`), `$1B` (`Ending_S24`), FADE OUT
/// (`Ending_S3C`).
const PROLOGUE_MUSIC: u8 = 0x0f;
const ROLL_MUSIC: u8 = 0x1b;
const FADE_MUSIC: u8 = 0x06;
/// `Data_0103BA`, installed by `Ending_S00 -> $B7A2` and retained for the
/// entire ending: X=1/X=0 pairs mask sprite lines 0..15 and 160..223.
const MASK_SPRITES: usize = 6;
fn sprite_masks(rom: &[u8]) -> crate::rom::Result<[[u16; 4]; MASK_SPRITES]> {
    let mut masks = [[0; 4]; MASK_SPRITES];
    for (i, record) in masks.iter_mut().enumerate() {
        *record = words(rom, 0x103ba + i * 8)?;
    }
    Ok(masks)
}
/// Cliff pan, falling wing, flash and relighting before the epilogue.
pub const PROLOGUE_TICKS: u32 = cliff::TICKS;
/// `Ending_S1C` -> `S20`: `$12C` frames after the last page.
pub const EPILOGUE_REST: u32 = 0x12c;
/// `Ending_S24`: `$78` frames before the roll fades in.
pub const ROLL_WAIT: u32 = 0x78;
/// `Ending_S34`: the sunset palette, one step every fourth frame.
pub const SUNSET_TICKS: u32 = 16 * 4;
/// `Ending_S38`: `$E10` frames.
pub const REST_TICKS: u32 = 0xe10;
/// `Ending_S3C`: the rider moves from X `$70` to `$110` at half a pixel.
pub const RIDER_TICKS: u32 = (0x110 - 0x70) * 2;
/// `Ending_S44`: `$3C` frames after the fade.
pub const FINAL_TICKS: u32 = 0x3c;

/// One epilogue page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub lines: Vec<String>,
    /// JP epilogue characters index the cartridge's 16 x 16 font.
    pub glyphs: Option<Vec<Vec<u8>>>,
    /// Frames before the next page (`$FF00F8`).
    pub hold: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Prologue,
    /// `Ending_S18`/`S1C`: page index.
    Epilogue(usize),
    /// `Ending_S20`.
    EpilogueRest,
    /// `Ending_S24`.
    RollFadeOut,
    /// `Ending_S28`.
    RollWait,
    /// `Ending_S2C`/`S30`.
    Roll,
    /// `Ending_S34`.
    Sunset,
    /// `Ending_S38`.
    Rest,
    /// `Ending_S3C`.
    RiderLeaves,
    /// `Ending_S40`.
    FadeOut,
    /// `Ending_S44`.
    Final,
}

pub struct Ending {
    front: Front,
    pub pages: Vec<Page>,
    pub roll: Vec<String>,
    pub step: Step,
    fade: Fade,
    /// Frames in the current step.
    ticks: u32,
    /// Epilogue text scroll (16.16 pixels), reset for each page.
    scroll: u32,
    /// Staff roll frames scrolled.
    rolled: u32,
    rider_ticks: u32,
    speed: u32,
    epilogue_rest: u32,
    audio: Audio,
    scene: Option<scene::Scene>,
    cliff: Option<cliff::Cliff>,
    cliff_age: u32,
}
impl Ending {
    pub fn new(game: &Game) -> Self {
        let front = Front::from_game(game);
        let rom = &front.rom;
        let pages = pages(rom);
        let roll = staff_roll(rom);
        let japanese =
            crate::rom::regions::identify(rom).ok() == Some(crate::rom::regions::Region::Japan);
        let speed_at = if japanese {
            0x100000 + 0x10d3e
        } else {
            SCROLL_SPEED
        };
        let speed = u32::from(word(rom, speed_at).unwrap_or(0x3000));
        let mut audio = Audio::default();
        audio.request(Table::Music.command(rom, PROLOGUE_MUSIC).ok());
        let cliff = cliff::Cliff::load(rom).ok();
        let epilogue_rest = if japanese {
            u32::from(word(rom, 0x100000 + 0x10d46).unwrap_or(464))
        } else {
            EPILOGUE_REST
        };
        Self {
            front,
            pages,
            roll,
            step: Step::Prologue,
            fade: Fade::In(0),
            ticks: 0,
            scroll: 0,
            rolled: 0,
            rider_ticks: 0,
            speed,
            epilogue_rest,
            audio,
            scene: None,
            cliff,
            cliff_age: 0,
        }
    }
    fn begin(&mut self, step: Step) {
        self.step = step;
        self.ticks = 0;
        match step {
            Step::Epilogue(_) => self.scroll = 0, // Func_010BEC
            Step::RollFadeOut | Step::FadeOut => self.fade = Fade::Out(0),
            Step::RollWait => {
                self.music(ROLL_MUSIC);
                self.scene = scene::Scene::load(&self.front.rom, &self.roll).ok();
            }
            Step::Roll => self.fade = Fade::In(0),
            _ => {}
        }
    }
    fn music(&mut self, index: u8) {
        self.audio
            .request(Table::Music.command(&self.front.rom, index).ok());
    }
    /// The page on screen during the epilogue and its rest.
    fn page(&self) -> Option<&Page> {
        match self.step {
            Step::Epilogue(page) => self.pages.get(page),
            Step::EpilogueRest => self.pages.last(),
            _ => None,
        }
    }
    /// Staff roll lines shown so far and the pixels scrolled.
    fn roll_state(&self) -> (usize, u32) {
        let shown = (self.rolled / ROLL_LINE_TICKS) as usize + 1;
        (shown.min(self.roll.len()), (self.rolled * ROLL_SPEED) >> 16)
    }
}
impl Mode for Ending {
    fn options(&self) -> Option<crate::options::GameOptions> {
        Some(self.front.options)
    }
    fn set_options(&mut self, options: crate::options::GameOptions) {
        self.front.options = options;
    }
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        self.ticks += 1;
        let fade_period = match self.step {
            Step::Roll => 2,
            Step::FadeOut => 4,
            _ => 1,
        };
        if self.ticks.is_multiple_of(fade_period) {
            self.fade = self.fade.advance();
        }
        if matches!(self.step, Step::RiderLeaves | Step::FadeOut | Step::Final) {
            self.rider_ticks += 1;
        }
        if self.scene.is_none()
            && let Some(cliff) = &mut self.cliff
        {
            self.cliff_age += 1;
            cliff.tick(self.cliff_age);
        }
        match self.step {
            Step::Prologue => {
                if self.ticks >= PROLOGUE_TICKS {
                    self.begin(Step::Epilogue(0));
                }
            }
            Step::Epilogue(page) => {
                self.scroll += self.speed; // Ending_Scroll
                let hold = self.pages.get(page).map_or(0, |page| page.hold);
                if self.ticks >= hold {
                    if page + 1 < self.pages.len() {
                        self.begin(Step::Epilogue(page + 1));
                    } else {
                        self.step = Step::EpilogueRest;
                        self.ticks = 0;
                    }
                }
            }
            Step::EpilogueRest => {
                if self.ticks >= self.epilogue_rest {
                    self.begin(Step::RollFadeOut);
                }
            }
            Step::RollFadeOut => {
                if self.fade.finished() {
                    self.begin(Step::RollWait);
                }
            }
            Step::RollWait => {
                if self.ticks >= ROLL_WAIT {
                    self.begin(Step::Roll);
                }
            }
            Step::Roll => {
                // Ending_S2C fades in first; S30 scrolls.
                if self.fade.finished() {
                    self.rolled += 1;
                    if (self.rolled / ROLL_LINE_TICKS) as usize + 1 >= self.roll.len() {
                        self.begin(Step::Sunset);
                    }
                }
            }
            Step::Sunset => {
                if self.ticks >= SUNSET_TICKS {
                    self.begin(Step::Rest);
                }
            }
            Step::Rest => {
                if self.ticks >= REST_TICKS || input.pressed(Button::Start) {
                    self.begin(Step::RiderLeaves);
                }
            }
            Step::RiderLeaves => {
                if self.ticks >= RIDER_TICKS {
                    self.music(FADE_MUSIC);
                    self.begin(Step::FadeOut);
                }
            }
            Step::FadeOut => {
                if self.fade.finished() {
                    self.begin(Step::Final);
                }
            }
            Step::Final => {
                if self.ticks >= FINAL_TICKS {
                    return Some(go(&self.front, Next::SegaLogo));
                }
            }
        }
        let (shown, scrolled) = self.roll_state();
        if let Some(scene) = &mut self.scene {
            let x = 0x70 + self.rider_ticks as i32 / 2;
            scene.tick(
                shown,
                scrolled,
                matches!(
                    self.step,
                    Step::Sunset | Step::Rest | Step::RiderLeaves | Step::FadeOut
                ),
                x,
            );
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        frame.clear([0; 3]);
        if self.scene.is_none()
            && let Some(cliff) = &self.cliff
        {
            cliff.vdp.render(frame, Fade::Out(0));
        }
        let font = &self.front.font;
        let style = Style::plain(md_color(WHITE), 1);
        if let Some(page) = self.page() {
            let scroll = (self.scroll >> 16) as i32;
            if let Some(lines) = &page.glyphs {
                for (row, glyphs) in lines.iter().enumerate() {
                    let y = PAGE_TOP + row as i32 * PAGE_ROW_STEP - scroll;
                    if (SPLIT_LINE - 16..crate::HEIGHT as i32).contains(&y) {
                        for (col, &glyph) in glyphs.iter().enumerate() {
                            let source = 0x100000 + 0x3abda + usize::from(glyph) * 32;
                            for dy in 0..16 {
                                let pixels = word(&self.front.rom, source + dy * 2).unwrap_or(0);
                                for dx in 0..16 {
                                    if pixels & (0x8000 >> dx) != 0 {
                                        frame.pixel(
                                            40 + col as i32 * 16 + dx,
                                            y + dy as i32,
                                            md_color(WHITE),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                for (row, line) in page.lines.iter().enumerate() {
                    let y = PAGE_TOP + row as i32 * PAGE_ROW_STEP - scroll;
                    if (SPLIT_LINE - 8..crate::HEIGHT as i32).contains(&y) {
                        font.draw_text(frame, PAGE_LEFT, y, line, style);
                    }
                }
            }
            // Preserve the cliff above the original line-160 raster split.
            if let Some(cliff) = &self.cliff {
                let mut upper = Frame::default();
                cliff.vdp.render(&mut upper, Fade::Out(0));
                let rgb = upper.rgb();
                for y in 0..SPLIT_LINE as usize {
                    for x in 0..crate::WIDTH {
                        let at = (y * crate::WIDTH + x) * 3;
                        frame.pixel(x as i32, y as i32, [rgb[at], rgb[at + 1], rgb[at + 2]]);
                    }
                }
            }
        }
        if let Some(scene) = &self.scene {
            scene.vdp.render(frame, self.fade);
            return;
        }
        frame.fade(self.fade);
    }
    fn name(&self) -> &'static str {
        "Ending"
    }
    fn take_audio(&mut self) -> Audio {
        std::mem::take(&mut self.audio)
    }
}

/// `Data_01147A` expanded: `Func_01155E` draws `rows` rows of `width`
/// characters from the text base plus the page offset.
pub fn pages(rom: &[u8]) -> Vec<Page> {
    if crate::rom::regions::identify(rom).ok() == Some(crate::rom::regions::Region::Japan) {
        return japanese_pages(rom);
    }
    let Ok(base) = pointer(rom, PAGES) else {
        return Vec::new();
    };
    let mut pages = Vec::new();
    let mut at = PAGES + 4;
    while let Ok([offset, width, rows, hold]) = words::<4>(rom, at) {
        if offset & 0x8000 != 0 {
            break;
        }
        let (width, rows) = (usize::from(width) + 1, usize::from(rows) + 1);
        let start = base + usize::from(offset);
        let lines = (0..rows)
            .filter_map(|row| rom.get(start + row * width..start + (row + 1) * width))
            .map(|line| String::from_utf8_lossy(line).trim_end().to_string())
            .collect();
        pages.push(Page {
            lines,
            glyphs: None,
            hold: u32::from(hold),
        });
        at += 8;
    }
    pages
}

/// `Ending_BuildStaffRoll` (`$10B02`): each 19-character line is written
/// once plus its repeat count (blank lines repeat), until a `$FE` count or
/// a negative word where the next line would start.
pub fn staff_roll(rom: &[u8]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut at = STAFF_ROLL;
    while let Some(text) = rom.get(at..at + STAFF_COLUMNS) {
        let text = String::from_utf8_lossy(text).into_owned();
        let Some(&repeat) = rom.get(at + STAFF_COLUMNS) else {
            break;
        };
        at += STAFF_COLUMNS + 1;
        if repeat == 0xfe {
            lines.push(text);
            break;
        }
        for _ in 0..=repeat {
            lines.push(text.clone());
        }
        if rom.get(at).is_none_or(|&b| b & 0x80 != 0) {
            break;
        }
    }
    lines
}

/// JP uses zero-terminated glyph rows and FE/FF page/end markers, rather
/// than the overseas revision's ASCII page descriptor table.
fn japanese_pages(rom: &[u8]) -> Vec<Page> {
    let mut at = 0x100000 + 0x10d4a;
    let hold = u32::from(word(rom, 0x100000 + 0x10d42).unwrap_or(720));
    let mut pages = Vec::new();
    let mut lines = Vec::new();
    while let Some(&byte) = rom.get(at) {
        if byte == 0xfe || byte == 0xff {
            pages.push(Page {
                lines: Vec::new(),
                glyphs: Some(std::mem::take(&mut lines)),
                hold,
            });
            if byte == 0xff {
                break;
            }
            at += 2;
            continue;
        }
        let start = at;
        while rom.get(at).is_some_and(|&byte| byte != 0) {
            at += 1;
        }
        if at == rom.len() || at - start > 20 || lines.len() >= 16 || pages.len() >= 16 {
            break;
        }
        lines.push(rom[start..at].to_vec());
        at += 1;
    }
    pages
}
