//! `Mode_Story` (`$7F56`, #65): the story text scrolling up over the moonlit
//! "Story").
//!
//! `Story_S00` loads the moon picture into plane B and the palette;
//! `Story_S08` scrolls plane A a pixel every third frame and writes the next
//! of the 25 lines of `$8326` every 16 pixels; `Story_S0C` scrolls on every
//! fourth frame for `$C9` more pixels, clearing the plane rows it will bring
//! up; `Story_S10` fades out and installs the intro's second pass. Start
//! (`Func_008302`) fades out to the title from any state.
//!
//! The port repeats those writes into a `FrontVdp`. `Story_S00`'s load
//! takes nine frames in the cartridge (the call starts at frame 445 of the
//! power-on run and returns at 453) and `Story_S10`'s last call three
//! (2466..2468); the port holds for the same frames.

use crate::{
    front_end::{Front, Next, go},
    front_vdp::{FONT_ART, FrontVdp, art, text_cells},
    input::{Button, Input},
    modes::{Audio, Mode, ModeChange},
    render::{FADE_STEPS, Fade, Frame},
    rom::{Result, byte, word},
};

/// `Story_S00`: the moon picture is entry 0 of the `$80000` + `$24` table,
/// loaded at tile `$34D` whatever its header says.
const MOON_TABLE: usize = 0x24;
const MOON_TILE: usize = 0x34d;
/// `Data_04952C`: width - 1, height - 1, then LZ words, drawn with base
/// `$434D` (line 2) at plane B `$E800` (row 16).
const MOON_MAP: usize = 0x4952c;
const MOON_MAP_VRAM: usize = 0xe800;
const MOON_MAP_BASE: u16 = 0x434d;
/// Plane B scrolled down 128 pixels, onto the map's rows.
const MOON_VSCROLL: i16 = 0x80;
/// Palette set `$42`, with colour 0 = `$200` and colour 11 = `$EEE`.
const PALETTE: usize = 0x42;
const BACKDROP: u16 = 0x0200;
const TEXT_COLOR: u16 = 0x0eee;
/// `$FF16E8` = 3: the fade out steps every fourth frame.
const FADE_MASK: u16 = 3;
/// `$8326`: 25 lines (`$19`), each a length byte n and n + 1 characters.
pub const TEXT: usize = 0x8326;
pub const LINES: u16 = 0x19;
/// The first line goes to plane A offset `$E84` (row 29, column 2); each
/// next line two rows further (`$100`), wrapping in the plane (`$1FFF`).
const FIRST_LINE: u16 = 0x0e84;
const LINE_STEP: u16 = 0x100;
const PLANE_MASK: u16 = 0x1fff;
/// `Story_S08` writes a line every 16 scroll steps; the first at once.
const LINE_PIXELS: i16 = 0x10;
const FIRST_COUNT: i16 = 0x1f;
/// `Story_S0C`: `$C8` + 1 more steps; every `$18` it clears three rows,
/// `$180` bytes ahead of the last line written.
const TAIL: i16 = 0xc8;
const CLEAR_EVERY: u16 = 0x18;
const CLEAR_STEP: u16 = 0x180;
/// Frames the cartridge spends in `Story_S00`'s call and `Story_S10`'s
/// last call after the one they start in (measured, Genesis Plus GX).
const LOAD_HOLD: u32 = 8;
const LEAVE_HOLD: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// `Story_S00`: load.
    Load,
    /// `Story_S08`: scroll and write the lines.
    Lines,
    /// `Story_S0C`: scroll the rest, clearing rows.
    Tail,
    /// `Story_S10`: fade out to the intro's second pass.
    FadeOut,
    /// `Story_S14`: Start; fade out to the title.
    Skip,
}

/// The story screen's VDP state and its handler's counters.
#[derive(Clone)]
pub struct StoryArt {
    pub v: FrontVdp,
    state: State,
    /// `$FF00F6`: scroll steps since the last line (`Story_S08`), steps
    /// left (`Story_S0C`).
    count: i16,
    /// `$FF00F8`: lines written (`Story_S08`), steps since the last clear
    /// (`Story_S0C`).
    lines: u16,
    /// `$FF0100`: plane A offset of the next line.
    line_at: u16,
    /// `$FF00FC`: the next line's ROM address.
    text: usize,
    japanese: bool,
}
impl StoryArt {
    /// The VRAM the story finds: the font `SegaLogo_S00` loaded (colour 1
    /// on 0, tiles 0..`$39`); the intro cleared plane A, the sprite list and
    /// the palette before installing the story. `frame_counter` is the
    /// count before the story's first frame.
    pub fn new(rom: &[u8], frame_counter: u16) -> Result<Self> {
        let mut v = FrontVdp::new(frame_counter);
        v.one_bpp_tiles(rom, FONT_ART, 0, 1)?;
        let japanese =
            crate::rom::regions::identify(rom).ok() == Some(crate::rom::regions::Region::Japan);
        Ok(Self {
            japanese,
            v,
            state: State::Load,
            count: 0,
            lines: 0,
            line_at: 0,
            text: TEXT,
        })
    }

    /// `Story_S00`.
    fn load(&mut self, rom: &[u8]) -> Result<()> {
        let v = &mut self.v;
        v.kosinski_tiles_at(rom, art(rom, MOON_TABLE, 0)?, MOON_TILE)?;
        let width = usize::from(word(rom, MOON_MAP)?) + 1;
        let height = usize::from(word(rom, MOON_MAP + 2)?) + 1;
        v.lz_words_to(
            rom,
            MOON_MAP + 4,
            MOON_MAP_VRAM,
            MOON_MAP_BASE,
            width,
            height,
        )?;
        v.vscroll[1] = MOON_VSCROLL;
        v.target_set(rom, PALETTE)?;
        v.palette_set(rom, PALETTE)?;
        v.target[0] = BACKDROP;
        v.palette[0] = BACKDROP;
        v.target[11] = TEXT_COLOR;
        v.palette[11] = TEXT_COLOR;
        v.fade_mask = FADE_MASK;
        self.lines = 0;
        self.line_at = FIRST_LINE;
        self.count = if self.japanese { 0x2f } else { FIRST_COUNT };
        self.text = if self.japanese {
            0x100000 + 0x81cc
        } else {
            TEXT
        };
        if self.japanese {
            let mut pixels: Vec<u8> = Vec::with_capacity(160 * 128);
            // JP story glyphs are 16 x 16, one bit per pixel. Expand to
            // four hardware tiles in column order, below the moon's art.
            for glyph in 0..160 {
                for col in 0..2 {
                    for row in 0..2 {
                        for y in 0..8 {
                            let bits =
                                word(rom, 0x100000 + 0x3abda + glyph * 32 + (row * 8 + y) * 2)?;
                            for pair in 0..4 {
                                let first = col * 8 + pair * 2;
                                let color = |x: usize| -> u8 {
                                    if bits & (0x8000u16 >> x) != 0u16 {
                                        11
                                    } else {
                                        0
                                    }
                                };
                                pixels.push(color(first) << 4 | color(first + 1));
                            }
                        }
                    }
                }
            }
            v.vdp.write_tiles(1, &pixels);
        }
        Ok(())
    }

    /// Plane A's vertical scroll, one pixel more.
    fn scroll(&mut self) {
        self.v.vscroll[0] = self.v.vscroll[0].wrapping_add(1);
    }

    /// `Story_S08`, on frames where `Frame_Counter` divides by 3.
    fn lines_step(&mut self, rom: &[u8]) -> Result<()> {
        if self.japanese {
            return self.japanese_lines_step(rom);
        }
        if !self.v.frame_counter.is_multiple_of(3) {
            return Ok(());
        }
        self.scroll();
        if self.count >= LINE_PIXELS {
            self.count = 0;
            self.lines += 1;
            // Func_004100: length byte n, then n + 1 characters, each through
            // the font map with priority, line 0.
            let length = usize::from(byte(rom, self.text)?);
            let characters = rom
                .get(self.text + 1..self.text + 2 + length)
                .ok_or("story text truncated")?;
            let cells: Vec<u16> = text_cells(rom, characters)?
                .iter()
                .map(|&tile| tile | 0x8000)
                .collect();
            self.v
                .vdp
                .write_map(0xc000 + usize::from(self.line_at), cells.len(), 0, &cells);
            self.text += 2 + length;
            self.line_at = (self.line_at + LINE_STEP) & PLANE_MASK;
        }
        self.count += 1;
        if self.lines == LINES {
            self.count = TAIL;
            self.lines = CLEAR_EVERY;
            self.state = State::Tail;
        }
        Ok(())
    }

    fn japanese_lines_step(&mut self, rom: &[u8]) -> Result<()> {
        if self.v.frame_counter & 1 == 0 {
            // The JP handler's shared timer advances on both frame phases;
            // only its scroll and row writes run on odd frames.
            self.count += 1;
            let at = 0xc180 + usize::from((self.line_at + 0x180) & PLANE_MASK);
            for row in 0..3 {
                for cell in 0..80 {
                    self.v.vdp.write_word(at + row * 128 + cell * 2, 0x8000);
                }
            }
            return Ok(());
        }
        self.scroll();
        if self.count >= 0x30 {
            self.count = 0;
            self.lines += 1;
            let mut column = 0;
            while byte(rom, self.text)? != 0 {
                let glyph = byte(rom, self.text)?;
                if glyph >= 160 || column >= 20 {
                    return Err("invalid Japanese story row".into());
                }
                let tile = 1 + u16::from(glyph) * 4;
                let at = 0xc000 + usize::from(self.line_at) + column * 4;
                self.v.vdp.write_word(at, 0x8000 | tile);
                self.v.vdp.write_word(at + 2, 0x8000 | (tile + 2));
                self.v.vdp.write_word(at + 128, 0x8000 | (tile + 1));
                self.v.vdp.write_word(at + 130, 0x8000 | (tile + 3));
                self.text += 1;
                column += 1;
            }
            self.text += 1;
            self.line_at = (self.line_at + 0x180) & PLANE_MASK;
        }
        self.count += 1;
        if self.lines == 21 {
            self.count = TAIL;
            self.lines = CLEAR_EVERY;
            self.state = State::Tail;
        }
        Ok(())
    }

    /// `Story_S0C`, on frames where `Frame_Counter & 3` is 3. True once
    /// done.
    fn tail_step(&mut self) -> bool {
        if (self.japanese && self.v.frame_counter & 1 == 0)
            || (!self.japanese && self.v.frame_counter & 3 != 3)
        {
            return false;
        }
        if self.lines >= CLEAR_EVERY {
            self.lines = 0;
            self.line_at = (self.line_at + CLEAR_STEP) & PLANE_MASK;
            // Three rows of 80 priority cells from the row start: each row's
            // writes run 16 cells into the next.
            let at = 0xc000 + usize::from(self.line_at);
            for row in 0..3 {
                for cell in 0..80 {
                    self.v.vdp.write_word(at + row * 0x80 + cell * 2, 0x8000);
                }
            }
        }
        self.scroll();
        self.lines += 1;
        self.count -= 1;
        self.count < 0
    }
}

/// `Mode_Story` (`$7F56`).
pub struct Story {
    front: Front,
    art: Option<StoryArt>,
    /// Frames left in a multi-frame handler call.
    hold: u32,
    /// The handler installed when the hold ends.
    next: Option<Next>,
}
impl Story {
    pub fn new(front: Front) -> Self {
        let art = StoryArt::new(&front.rom, front.frame_counter).ok();
        Self {
            front,
            art,
            hold: 0,
            next: None,
        }
    }

    fn leave(&mut self, next: Next) -> ModeChange {
        if let Some(art) = &self.art {
            self.front.frame_counter = art.v.frame_counter;
        }
        self.front.fade_mask = 0;
        go(&self.front, next)
    }
}
impl Mode for Story {
    fn options(&self) -> Option<crate::options::GameOptions> {
        Some(self.front.options)
    }
    fn set_options(&mut self, options: crate::options::GameOptions) {
        self.front.options = options;
    }
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        let Some(art) = self.art.as_mut() else {
            return Some(go(&self.front, Next::Title));
        };
        if self.hold > 0 {
            art.v.lag(true);
            self.hold -= 1;
            return match (self.hold, self.next) {
                (0, Some(next)) => Some(self.leave(next)),
                _ => None,
            };
        }
        art.v.vblank();
        // Func_008302: Start newly pressed.
        if input.pressed(Button::Start) && art.state != State::Skip {
            art.state = State::Skip;
        }
        let rom = self.front.rom.clone();
        match art.state {
            State::Load => {
                if art.load(&rom).is_err() {
                    return Some(go(&self.front, Next::Title));
                }
                art.state = State::Lines;
                self.hold = LOAD_HOLD;
            }
            State::Lines => {
                if art.lines_step(&rom).is_err() {
                    return Some(go(&self.front, Next::Title));
                }
            }
            State::Tail => {
                if art.tail_step() {
                    art.count = 0;
                    art.state = State::FadeOut;
                }
            }
            State::FadeOut => {
                if art.v.frame_counter & 1 != 0 {
                    art.scroll();
                }
                if !art.v.fade_out() {
                    // Story_ModeIntro: the second pass starts at its state 4
                    // with the planes cleared.
                    art.v.fade_mask = 0;
                    art.v.clear_plane_a();
                    art.v.clear_plane_b();
                    self.hold = LEAVE_HOLD;
                    self.next = Some(Next::Intro(true));
                }
            }
            State::Skip => {
                art.v.fade_mask = 0;
                if !art.v.fade_out() {
                    return Some(self.leave(Next::Title));
                }
            }
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        match &self.art {
            Some(art) => art.v.vdp.render(frame, Fade::In(FADE_STEPS)),
            None => frame.clear([0; 3]),
        }
    }
    fn name(&self) -> &'static str {
        "Story"
    }
    fn take_audio(&mut self) -> Audio {
        Audio::default()
    }
}
