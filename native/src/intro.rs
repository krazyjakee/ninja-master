//! `Mode_Intro` (`$FBD6`, #65): the SHINOBI logo after the SEGA logo, and
//!
//! The port repeats the handler's VRAM, CRAM-copy, sprite-table and scroll
//! writes into a `FrontVdp`, state by state, with the cartridge's RAM words
//! (`$FF00F6..$FF00FE`) as counters. The handler runs once per frame, but a
//! few of its calls load art for longer than a frame; the frames such a
//! call spans are ticks where the handler does not run (`Hold`), measured
//! in Genesis Plus GX. The logo waits for the SHINOBI music's `$FFF803`
//! markers; without a sound driver (headless) they arrive at the frames
//! measured in the emulator (`MARKERS`).

use crate::{
    front_end::{Front, Next, go},
    front_vdp::{FrontVdp, art},
    input::{Button, Input},
    modes::{Audio, Mode, ModeChange},
    render::{FADE_STEPS, Fade, Frame},
    rom::{Result, words},
    sound_events::Table,
    title_art::{LIST_COUNT, LIST_FIRST, draw_logo, logo_sprites},
};

/// Handler frames of the two passes (emulator frames 234..444 and
/// 2469..3131, the lag frames included).
pub const INTRO_TICKS: u32 = 211;
pub const REVEAL_TICKS: u32 = 663;
/// `Intro_S10`: `$FF16E8` = 3 for the title's fade in (a step every fourth
/// frame, 3161..3221 in the attract capture; after Start every frame).
pub const TITLE_FADE_MASK: u16 = 3;

/// `Intro_S00`: music `$0D` (SHINOBI); the reveal's voices 7 (MUSASHI
/// KIAI) and 6 (KATANA HIT).
const INTRO_MUSIC: u8 = 0x0d;
const KIAI_VOICE: u8 = 7;
const KATANA_VOICE: u8 = 6;
/// Palette sets (`Func_000900`): the logo, the reveal's wall and the moon.
const LOGO_PALETTE: usize = 2;
const REVEAL_PALETTE: usize = 0x44;
const MOON_PALETTE: usize = 0x45;
/// `Intro_S00` sets fade target entry 47 (`$FF1758`, line 2 colour 15).
const LOGO_TARGET_47: (usize, u16) = (47, 0x0644);
/// `Data_00FD5A`: the 32 colours lines 1-2 fade toward (every other call
/// of `$51..$70`); `Data_00FD3A`: the 16 set in both at call `$71`;
/// `Data_00FD9A`: line 2 colour 13 turns red.
const LOGO_FADE: usize = 0xfd5a;
const LOGO_LINES: usize = 0xfd3a;
const KANJI_RED: usize = 45;
const KANJI_RED_TARGET: u16 = 0x000e;
/// `Data_00FFBC`: the reveal's eight shuriken sprites.
const SHURIKENS: usize = 0xffbc;
const SHURIKEN_COUNT: usize = 8;
/// `Data_0102DC`: nine LZ tiles at `$255` (`Intro_S08`).
const SWORD_TILES: usize = 0x102dc;
/// The reveal's art: entries of the `$80000` table at `+$24`, and the maps
/// drawn with them.
const ART_TABLE: usize = 0x24;
/// Entry 4 (tiles `$40..$5F`): the paper wall, `Data_04992C` (LZ bytes,
/// plane B, 64 x 50 cells).
const WALL_ART: usize = 0x04;
const WALL_MAP: usize = 0x4992c;
/// Entries `$18` (tiles `$80..$9F`, Kosinski) and `$1C` (`$A0..$A8`, LZ):
/// the shurikens.
const SHURIKEN_ART: usize = 0x18;
const SHURIKEN_ART_LZ: usize = 0x1c;
/// Entry `$C` (tiles `$C0..$25C`): Musashi, `Data_049E36` (plane A, 32 x
/// 20), the picture the title shows too.
const MUSASHI_ART: usize = 0x0c;
const MUSASHI_MAP: usize = 0x49e36;
/// Entry `$14` (tiles `$280..$320`): the leap, `Data_04A218` (plane A,
/// 35 x 23).
const LEAP_ART: usize = 0x14;
const LEAP_MAP: usize = 0x4a218;
/// Entry 8 (tiles `$40..$BF`): the moon, `Data_04A414` (plane B, 40 x 56);
/// entry `$10` (tiles `$140..$26E`): Musashi falling with the sword,
/// `Data_04A7F6` (plane A, 21 x 38).
const MOON_ART: usize = 0x08;
const MOON_MAP: usize = 0x4a414;
const FALL_ART: usize = 0x10;
const FALL_MAP: usize = 0x4a7f6;

/// Handler frames (from the pass's first) at which the SHINOBI music's
/// `$FFF803` markers arrive in the attract capture: 1, 2, 3 and 4 at
/// frames 367, 385, 403 and 427 (the logo waits for 4), 6 at 2684 (the
/// reveal ends on it). 5 (frame 1507) arrives during the story.
const MARKERS: [&[(u32, u8)]; 2] = [&[(133, 1), (151, 2), (169, 3), (193, 4)], &[(215, 6)]];

/// Frames a long handler call runs past its own, measured in Genesis Plus
/// GX: the next call starts that many frames later.
/// `Intro_S00`'s loads: frames 234..245.
const LOGO_LOAD: Hold = Hold::new(11, None);
/// The logo's last call clears plane A: 443..444.
const LOGO_CLEAR: Hold = Hold::new(1, None);
/// The reveal's loads and `Intro_S08`'s first call: 2469..2485, the
/// interrupt of frame 2483 lost (`Frame_Counter` does not count it).
const REVEAL_LOAD: Hold = Hold::new(16, Some(14));
/// `Intro_S08`'s leap: 2915..2919.
const LEAP_LOAD: Hold = Hold::new(4, None);
/// Frame 2919, the leap load's last: its plane A map is written (row by
/// row, 35 cells a row) while the frame is drawn. A line shows the cells
/// written by the time it is drawn: 359 at line 150, 9 more a line
/// (measured in the emulator's picture of frame 2920).
const LEAP_TEAR_LINE: i32 = 150;
const LEAP_TEAR_CELLS: i32 = 359;
const LEAP_TEAR_RATE: i32 = 9;
/// `Intro_S0C`'s moon: 2991..3006, frame 3005's interrupt lost.
const MOON_LOAD: Hold = Hold::new(15, Some(14));
/// `Intro_S10` and `Intro_S14` clear planes A and B: 3129..3131.
const PLANES_CLEAR: Hold = Hold::new(2, None);

/// Frames a handler call spans past its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hold {
    frames: u32,
    /// The frame (1-based within the hold) whose interrupt is lost.
    lost: Option<u32>,
}
impl Hold {
    const fn new(frames: u32, lost: Option<u32>) -> Self {
        Self { frames, lost }
    }
}

/// What the rest of a long call does once it returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum After {
    Nothing,
    /// The reveal's load branches into `Intro_S08` (`bra.w`).
    RevealS08,
    /// `Intro_S08`'s leap: the plane A map and the kiai, as the call ends.
    LeapMap,
    Leave(Next),
}

/// `Mode_Intro`.
pub struct Intro {
    front: Front,
    pub v: FrontVdp,
    second: bool,
    /// `Mode_Routine`: 0, 4, 8, `$C`, `$10`, `$14`.
    routine: u16,
    /// `$FF00F6`, `$FF00F8`, `$FF00FA`, `$FF00FC`, `$FF00FE`.
    f6: u16,
    f8: u16,
    fa: u16,
    fc: u16,
    fe: u16,
    /// `$FFF803`, the sound driver's communication byte.
    pub marker: u8,
    /// `Intro_S00` asked for the SHINOBI music at the end of its load.
    music_pending: bool,
    /// The highest marker the sound driver has sent this pass.
    heard: u8,
    /// Frames since the pass began (handler and lag frames).
    pub frame: u32,
    /// A long call: frames left, the lost interrupt, and what follows.
    hold: u32,
    hold_lost: Option<u32>,
    after: After,
    /// The VDP before a plane map written while the frame was drawn, and
    /// the map's width in cells (frame 2919).
    tear: Option<(Box<crate::vdp::Vdp>, usize)>,
    audio: Audio,
}

impl Intro {
    /// `Mode_Intro` from its first call. The first pass starts at
    /// `Intro_S00`; the pass after the story at `Intro_S04` with
    /// `$FF00F8` = `$100`, `$FF00FA` = `$20` and `$FF00FE` = 1
    /// (`Story_S10`). `front.frame_counter` is `Frame_Counter` before the
    /// first frame's interrupt.
    pub fn new(front: Front, second: bool) -> Self {
        let mut v = FrontVdp::new(front.frame_counter);
        v.fade_mask = front.fade_mask;
        let (routine, f8, fa, fe) = if second {
            (4, 0x100, 0x20, 1)
        } else {
            (0, 0, 0, 0)
        };
        Self {
            front,
            v,
            second,
            routine,
            f6: 0,
            f8,
            fa,
            fc: 0,
            fe,
            marker: 0,
            music_pending: false,
            heard: 0,
            frame: 0,
            hold: 0,
            hold_lost: None,
            after: After::Nothing,
            tear: None,
            audio: Audio::default(),
        }
    }

    fn rom(&self) -> std::rc::Rc<[u8]> {
        self.front.rom.clone()
    }
    fn hold(&mut self, hold: Hold, after: After) {
        self.hold = hold.frames;
        self.hold_lost = hold.lost.map(|lost| hold.frames - lost);
        self.after = after;
    }
    fn music(&mut self, index: u8) {
        let music = Table::Music.command(&self.front.rom, index).ok();
        self.audio.request(music);
        self.front.music = music;
    }
    fn voice(&mut self, index: u8) {
        self.audio
            .request(Table::Voice.command(&self.front.rom, index).ok());
    }
    /// The mode that follows, with `Frame_Counter` and `$FF16E8` handed on.
    fn leave(&mut self, next: Next) -> ModeChange {
        self.front.frame_counter = self.v.frame_counter;
        self.front.fade_mask = self.v.fade_mask;
        self.front.palette = self.v.palette;
        go(&self.front, next)
    }

    /// One handler call: `Func_0102B8` (Start), then the state.
    fn call(&mut self, input: Input) -> Result<()> {
        if input.pressed(Button::Start) && self.routine != 0x14 {
            self.routine = 0x14;
        }
        match self.routine {
            0 => self.s00(),
            4 => self.s04(),
            8 => self.s08(),
            0x0c => self.s0c(),
            0x10 => {
                self.s10();
                Ok(())
            }
            _ => {
                self.s14();
                Ok(())
            }
        }
    }

    /// `Intro_S00`: the logo, in black, `$FF16E8` = 1 and the music.
    fn s00(&mut self) -> Result<()> {
        let rom = self.rom();
        let v = &mut self.v;
        v.hscroll = [0, 0];
        v.vscroll = [0, 0];
        v.target_set(&rom, LOGO_PALETTE)?;
        v.target[0] = 0;
        v.target[LOGO_TARGET_47.0] = LOGO_TARGET_47.1;
        draw_logo(&mut v.vdp, &rom)?;
        v.hscroll[0] = 0x4c;
        v.vscroll[0] = -0x20;
        let sprites = logo_sprites(&rom, 0x20, 0x40)?;
        v.sprites[..sprites.len()].copy_from_slice(&sprites);
        v.fade_mask = 1;
        (self.f6, self.f8, self.fa, self.fc, self.fe) = (1, 0, 0, 1, 0);
        self.music_pending = true;
        self.routine = 4;
        self.hold(LOGO_LOAD, After::Nothing);
        Ok(())
    }

    /// `Intro_S04`: the fade in, the logo's colour changes, the wait for
    /// marker 4, the slide; then the story (first pass) or the reveal's
    /// loads.
    fn s04(&mut self) -> Result<()> {
        if self.f6 != 0 {
            if !self.v.fade_in() {
                self.v.fade_mask = 0;
                self.f6 = 0;
            }
            return Ok(());
        }
        if self.f8 <= 0x80 {
            self.logo_colors()?;
            return Ok(());
        }
        self.f8 = self.f8.wrapping_add(1);
        if self.fc != 0 {
            if self.marker != 4 {
                return Ok(());
            }
            self.marker = 0;
            self.fc = 0;
        }
        self.fa = self.fa.wrapping_add(1);
        if self.fa <= 0x10 {
            // The logo slides right, the kanji and "III" left.
            self.v.hscroll[0] = self.v.hscroll[0].wrapping_add(0x10);
            for sprite in &mut self.v.sprites[LIST_FIRST..LIST_FIRST + LIST_COUNT] {
                sprite[3] = sprite[3].wrapping_sub(0x10);
            }
            return Ok(());
        }
        if self.fe == 0 {
            // $FEA4: black, the sprite list cut, plane A cleared, Mode_Story.
            self.v.palette = [0; 64];
            self.v.palette_dirty = true;
            self.v.sprites[0][0] = 0;
            self.v.sprites[0][1] = 0;
            self.v.clear_plane_a();
            self.v.hscroll[0] = 0;
            self.v.vscroll[0] = 0;
            self.hold(LOGO_CLEAR, After::Leave(Next::Story));
            return Ok(());
        }
        // $FEEE: the reveal, once the story's palette is black.
        if self.v.fade_out() {
            return Ok(());
        }
        self.reveal()
    }

    /// `loc_00FD9C`, calls 0..`$80` after the fade: from `$51` lines 1-2
    /// fade toward `Data_00FD5A` every other call, at `$71` the kanji take
    /// priority and lines 1-2 become `Data_00FD3A`, from `$72` every other
    /// call the kanji and "III" move 4 pixels left and line 2 colour 13
    /// turns red, which it stays after `$7D`.
    fn logo_colors(&mut self) -> Result<()> {
        let rom = self.rom();
        let v = &mut self.v;
        v.palette_dirty = true;
        let d0 = self.f8;
        self.f8 += 1;
        match d0 {
            0..=0x50 => {}
            0x51..=0x70 => {
                if d0 & 1 == 0 {
                    let target: [u16; 32] = words(&rom, LOGO_FADE)?;
                    v.fade_toward(16, &target);
                }
            }
            0x71 => {
                for sprite in &mut v.sprites[LIST_FIRST..LIST_FIRST + 9] {
                    sprite[2] |= 0x8000;
                }
                let lines: [u16; 16] = words(&rom, LOGO_LINES)?;
                v.palette[16..32].copy_from_slice(&lines);
                v.palette[32..48].copy_from_slice(&lines);
            }
            0x72..=0x7d => {
                if d0 & 1 == 0 {
                    v.fade_toward(KANJI_RED, &[KANJI_RED_TARGET]);
                    v.fade_toward(KANJI_RED, &[KANJI_RED_TARGET]);
                    for sprite in &mut v.sprites[LIST_FIRST..LIST_FIRST + LIST_COUNT] {
                        sprite[2] |= 0x8000;
                        sprite[3] = sprite[3].wrapping_sub(4);
                    }
                    // "III" joins the list.
                    let link = &mut v.sprites[14][1];
                    *link = (*link & 0xff00) | 0x0f;
                }
            }
            _ => v.palette[KANJI_RED] = KANJI_RED_TARGET,
        }
        Ok(())
    }

    /// `$FEEE`..`$FFB8`: the paper wall on plane B, the shurikens, Musashi
    /// on plane A, then `Intro_S08` (after the long call returns).
    fn reveal(&mut self) -> Result<()> {
        let rom = self.rom();
        let v = &mut self.v;
        v.vscroll = [-0x40, 0xa0];
        v.hscroll = [0x100, -0xc0];
        v.target_set(&rom, REVEAL_PALETTE)?;
        v.kosinski_tiles(&rom, art(&rom, ART_TABLE, WALL_ART)?)?;
        v.lz_byte_map(&rom, WALL_MAP)?;
        v.kosinski_tiles(&rom, art(&rom, ART_TABLE, SHURIKEN_ART)?)?;
        v.lz_tiles(&rom, art(&rom, ART_TABLE, SHURIKEN_ART_LZ)?)?;
        v.kosinski_tiles(&rom, art(&rom, ART_TABLE, MUSASHI_ART)?)?;
        v.lz_word_map(&rom, MUSASHI_MAP)?;
        for i in 0..SHURIKEN_COUNT {
            v.sprites[i] = words(&rom, SHURIKENS + i * 8)?;
        }
        (self.f6, self.f8, self.fa, self.fc) = (0xc0, 0x20, 0x18, 0x10);
        self.routine = 8;
        self.hold(REVEAL_LOAD, After::RevealS08);
        Ok(())
    }

    /// `Intro_S08`: Musashi fades in while the planes part (a pixel every
    /// even frame), the sword tiles, the leap with the kiai, then the
    /// shurikens fly out.
    fn s08(&mut self) -> Result<()> {
        let rom = self.rom();
        let v = &mut self.v;
        if self.f6 != 0 {
            v.fade_in();
            if v.frame_counter & 1 != 0 {
                return Ok(());
            }
            self.f6 -= 1;
            v.hscroll[0] = v.hscroll[0].wrapping_sub(1);
            v.hscroll[1] = v.hscroll[1].wrapping_add(1);
            return Ok(());
        }
        if self.f8 != 0 {
            self.f8 -= 1;
            if self.f8 == 0 {
                v.lz_tiles(&rom, SWORD_TILES)?;
            }
            return Ok(());
        }
        if self.fc != 0 {
            self.fc -= 1;
            if self.fc == 0 {
                // The tiles go to $280.., which nothing shows yet; the map
                // and the voice follow as the call ends (`leap_map`).
                v.kosinski_tiles(&rom, art(&rom, ART_TABLE, LEAP_ART)?)?;
                v.hscroll[0] = 0x30;
                v.vscroll[0] = 0;
                self.hold(LEAP_LOAD, After::LeapMap);
            }
            return Ok(());
        }
        if self.fa != 0 {
            self.fa -= 1;
            for group in [0, 4] {
                if v.sprites[group][3] >= 0x168 {
                    v.sprites[group][3] = 0x200;
                } else {
                    for sprite in &mut v.sprites[group..group + 4] {
                        sprite[0] = sprite[0].wrapping_add(8);
                        sprite[3] = sprite[3].wrapping_add(0x10);
                    }
                }
            }
            if self.fa & 1 == 0 {
                v.vscroll[0] = v.vscroll[0].wrapping_add(0x10);
            }
            return Ok(());
        }
        v.sprites[0][3] = 1;
        v.sprites[4][3] = 1;
        v.fade_mask = 1;
        (self.f6, self.f8, self.fa, self.fc, self.fe) = (0x30, 0x22, 0x18, 0x20, 0x20);
        self.routine = 0x0c;
        self.s0c()
    }

    /// The end of the leap's load: `Data_04A218` on plane A, drawn while
    /// the frame is (`LEAP_TEAR_CELLS`), then voice 7.
    fn leap_map(&mut self) -> Result<()> {
        let rom = self.rom();
        let before = Box::new(self.v.vdp.clone());
        self.v.lz_word_map(&rom, LEAP_MAP)?;
        let (_, _, width, _) = crate::front_vdp::map_header(&rom, LEAP_MAP)?;
        self.tear = Some((before, width));
        self.voice(KIAI_VOICE);
        Ok(())
    }

    /// `Intro_S0C`: the leap fades out, the moon and Musashi's fall load
    /// and fade in, the katana, then a flash to white.
    fn s0c(&mut self) -> Result<()> {
        let rom = self.rom();
        let v = &mut self.v;
        if self.f6 != 0 {
            self.f6 -= 1;
            if self.f6 != 0 {
                v.vscroll[1] = v.vscroll[1].wrapping_sub(8);
                for sprite in &mut v.sprites[..SHURIKEN_COUNT] {
                    sprite[0] = sprite[0].wrapping_add(0x10);
                }
                v.fade_out();
                return Ok(());
            }
            v.clear_plane_b();
            v.target_set(&rom, MOON_PALETTE)?;
            v.kosinski_tiles(&rom, art(&rom, ART_TABLE, MOON_ART)?)?;
            v.lz_word_map(&rom, MOON_MAP)?;
            v.clear_plane_a();
            v.kosinski_tiles(&rom, art(&rom, ART_TABLE, FALL_ART)?)?;
            v.lz_word_map(&rom, FALL_MAP)?;
            v.hscroll[0] = 0x100;
            v.vscroll[0] = -0xd0;
            v.sprites[0][0] = 0;
            v.sprites[0][1] = 0;
            self.hold(MOON_LOAD, After::Nothing);
            return Ok(());
        }
        v.sprites[0][0] = 0;
        v.sprites[0][1] = 0;
        if self.f8 != 0 {
            self.f8 -= 1;
            v.vscroll[1] = v.vscroll[1].wrapping_sub(8);
            v.fade_in();
            return Ok(());
        }
        if self.fe != 0 {
            self.fe -= 1;
            return Ok(());
        }
        if self.fa != 0 {
            self.fa -= 1;
            if self.fa == 0 {
                self.voice(KATANA_VOICE);
            }
            let v = &mut self.v;
            v.hscroll[0] = 0x58;
            v.vscroll[0] = v.vscroll[0].wrapping_sub(8);
            v.fade_mask = 7;
            return Ok(());
        }
        if self.fc != 0 {
            self.fc -= 1;
            v.vscroll[0] = v.vscroll[0].wrapping_sub(8);
            // Func_00075C four times: every CRAM copy entry a step whiter.
            for line in 0..4 {
                v.brighten(line * 16);
            }
            return Ok(());
        }
        self.routine = 0x10;
        self.s10();
        Ok(())
    }

    /// `Intro_S10`: once the music's marker 6 has come, planes A and B
    /// are cleared and the title follows with `$FF16E8` = 3.
    fn s10(&mut self) {
        if self.marker != 6 {
            return;
        }
        self.v.clear_plane_a();
        self.v.clear_plane_b();
        self.v.fade_mask = TITLE_FADE_MASK;
        self.hold(PLANES_CLEAR, After::Leave(Next::Title));
    }

    /// `Intro_S14` (Start): a fade out every frame, planes A and B
    /// cleared, the title.
    fn s14(&mut self) {
        self.v.fade_mask = 0;
        if self.v.fade_out() {
            return;
        }
        self.v.clear_plane_a();
        self.v.clear_plane_b();
        self.hold(PLANES_CLEAR, After::Leave(Next::Title));
    }

    /// The markers the measured frames bring, unless the sound driver sent
    /// them already.
    fn measured_markers(&mut self, frame: u32) {
        for &(at, value) in MARKERS[usize::from(self.second)] {
            if at == frame && self.heard < value {
                self.marker = value;
            }
        }
    }
}

impl Mode for Intro {
    fn options(&self) -> Option<crate::options::GameOptions> {
        Some(self.front.options)
    }
    fn set_options(&mut self, options: crate::options::GameOptions) {
        self.front.options = options;
    }
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        let frame = self.frame;
        self.frame += 1;
        self.tear = None;
        if self.hold > 0 {
            self.hold -= 1;
            self.v.lag(self.hold_lost != Some(self.hold));
            if self.hold > 0 {
                return None;
            }
            return match std::mem::replace(&mut self.after, After::Nothing) {
                After::Nothing => None,
                After::RevealS08 => {
                    let _ = self.s08();
                    None
                }
                After::LeapMap => {
                    let _ = self.leap_map();
                    None
                }
                After::Leave(next) => Some(self.leave(next)),
            };
        }
        self.v.vblank();
        // Func_004A36 queues the music as the load returns (frame 245); the
        // driver plays its first step two interrupts later (247: markers 4
        // and 6 come 180 and 2436 driver updates after it). The native
        // player plays the first step in the update after the tick that
        // asks, so the request goes out one frame after the load.
        if std::mem::take(&mut self.music_pending) {
            self.music(INTRO_MUSIC);
        }
        self.measured_markers(frame);
        match self.call(input) {
            Ok(()) => None,
            // Art that does not decode: straight to the title.
            Err(_) => Some(self.leave(Next::Title)),
        }
    }
    fn draw(&self, frame: &mut Frame) {
        let vdp = &self.v.vdp;
        vdp.render(frame, Fade::In(FADE_STEPS));
        let Some((before, width)) = &self.tear else {
            return;
        };
        // Plane A cells not yet written when a line was drawn show as
        // before.
        let mut old = Frame::default();
        before.render(&mut old, Fade::In(FADE_STEPS));
        let (plane_w, plane_h) = vdp.plane_size;
        for y in 0..crate::HEIGHT {
            let written =
                (LEAP_TEAR_CELLS + (y as i32 - LEAP_TEAR_LINE) * LEAP_TEAR_RATE).max(0) as usize;
            let py = (y as i32 + i32::from(vdp.vscroll[0])).rem_euclid(plane_h as i32 * 8) as usize;
            for x in 0..crate::WIDTH {
                let px = (x as i32 - i32::from(vdp.hscroll[y][0])).rem_euclid(plane_w as i32 * 8)
                    as usize;
                let (row, column) = (py / 8, px / 8);
                if column < *width && row * width + column >= written {
                    let at = (y * crate::WIDTH + x) * 3;
                    let rgb = &old.rgb()[at..at + 3];
                    frame.pixel(x as i32, y as i32, [rgb[0], rgb[1], rgb[2]]);
                }
            }
        }
    }
    fn name(&self) -> &'static str {
        "Intro"
    }
    fn take_audio(&mut self) -> Audio {
        std::mem::take(&mut self.audio)
    }
    /// `$FFF803`: the SHINOBI music's markers.
    fn sound_communication(&mut self, value: u8) {
        self.marker = value;
        self.heard = self.heard.max(value);
    }
}
