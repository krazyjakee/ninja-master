//! The attract demo (#44): `Mode_DemoStart` (`$7C76`) and `Mode_Demo`
//! (`$7C90`) playing the cartridge's recorded input tapes through `Game`.
//!
//! `Mode_DemoStart` counts `Demo_Counter` (done by `front_end::go`); its
//! bit 0 picks the stage from `Demo_StartTable` (`$7C6C`) and the tape from
//! `Demo_TapeSets` (`$649E2`). `Demo_Init` starts a session with the
//! options in force, then 99 shurikens; `Demo_Load` selects Kariu. The
//! stage fades in with the pad idle (`Demo_FadeIn`), then `Demo_Play` feeds
//! the tape to `Game_RunFrame` with control type 0 until a life is lost or
//! 30 one-second ticks pass. Start on the real pad, or the end, fades out to
//! `Mode_SegaLogo` (`Demo_ModeStartPressed`, `Demo_ModeTimeout`).

use crate::{
    front_end::{Front, Next, go},
    game::{Game, REQUEST_LIFE_LOST, REQUEST_NEXT_SUBSTAGE},
    input::{Button, Input},
    modes::{Audio, Mode, ModeChange},
    render::{Fade, Frame},
    rom::pointer,
    stage::StageId,
};

/// Per demo: round, substage and part indices, then a pad byte.
const START_TABLE: usize = 0x7c6c;
/// `Demo_NinjutsuSelect` (`move.b` of `$7C74` = 1): Kariu.
const NINJUTSU_SELECT: usize = 0x7c74;
/// `Demo_TapeSets`: a pointer (by `Sys_Flags` bit 0; both are the same) to
/// `Demo_Tapes`, one tape pointer per demo.
const TAPE_SETS: usize = 0x649e2;
/// `Demo_Init`: `move.w #$63,(Ammo)`.
pub const DEMO_AMMO: u16 = 99;
/// `Demo_FadeIn` sets 30 one-second ticks (`$FF00F6` = `$1E`).
pub const DEMO_TICKS: u32 = 30 * 60;

/// A pad byte (Up, Down, Left, Right, B, C, A, Start = bits 0-7) as the
/// native buttons it stands for; Start is the real pad's, not the tape's.
pub fn pad_buttons(pad: u8) -> u16 {
    [
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
        Button::Attack,
        Button::Jump,
        Button::Ninjutsu,
    ]
    .iter()
    .enumerate()
    .filter(|&(bit, _)| pad & (1u8 << bit) != 0)
    .fold(0, |mask, (_, button)| mask | button.mask())
}

/// A recorded tape: `(pad byte, count)` pairs read on from the tape's
/// address, as `Demo_PlayTick` does (the forest demo runs past its own tape
/// into the following bytes).
#[derive(Clone, Debug)]
pub struct Tape {
    at: usize,
    pad: u8,
    count: i16,
    previous: u16,
}
impl Tape {
    /// `Demo_TapePointer` and `Demo_PlayStart`: the first pair is loaded.
    pub fn new(rom: &[u8], demo: usize) -> Result<Self, String> {
        let tapes = pointer(rom, TAPE_SETS)?;
        let at = pointer(rom, tapes + (demo & 1) * 4)?;
        let pair = rom
            .get(at..at + 2)
            .ok_or_else(|| format!("demo tape ${at:05X} outside ROM"))?;
        Ok(Self {
            at: at + 2,
            pad: pair[0],
            count: i16::from(pair[1]),
            previous: 0,
        })
    }
    /// `Demo_PlayTick`: count down, load the next pair on underflow, and
    /// read the pad byte like a real pad (held, and pressed on its edges).
    pub fn next(&mut self, rom: &[u8]) -> Input {
        self.count -= 1;
        if self.count < 0 {
            self.pad = rom.get(self.at).copied().unwrap_or(0);
            self.count = i16::from(rom.get(self.at + 1).copied().unwrap_or(0));
            self.at += 2;
        }
        let held = pad_buttons(self.pad);
        let input = Input {
            held,
            pressed: held & !self.previous,
        };
        self.previous = held;
        input
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// `Demo_FadeIn`: play frames with an idle pad until the fade ends.
    FadeIn,
    /// `Demo_Play`.
    Play,
    /// `Demo_FadeOutTimeout` / `Demo_FadeOutStart`: the frame holds.
    FadeOut,
}

/// `Mode_Demo`.
pub struct Demo {
    front: Front,
    game: Box<Game>,
    tape: Tape,
    pub step: Step,
    fade: Fade,
    /// `Demo_Play` frames so far.
    pub ticks: u32,
}
impl Demo {
    /// `Demo_Init` and `Demo_Load` for demo `Demo_Counter & 1`.
    pub fn new(front: Front) -> Result<Self, String> {
        let demo = usize::from(front.demo_counter & 1);
        let rom = &front.rom;
        let at = START_TABLE + demo * 4;
        let start = rom.get(at..at + 3).ok_or("Demo_StartTable outside ROM")?;
        let stage = StageId::new(start[0] + 1, start[1] + 1, start[2]);
        let mut game = Game::load_stage(rom, stage)?;
        front.prepare(&mut game);
        game.start_session(front.options);
        game.set_ammo(DEMO_AMMO);
        game.set_ninjutsu(usize::from(rom.get(NINJUTSU_SELECT).copied().unwrap_or(0)));
        game.fade_in();
        let tape = Tape::new(rom, demo)?;
        Ok(Self {
            front,
            game: Box::new(game),
            tape,
            step: Step::FadeIn,
            fade: Fade::Out(0),
            ticks: 0,
        })
    }
    fn leave(&mut self) {
        self.step = Step::FadeOut;
        self.fade = Fade::Out(0);
        // HighScore_Update ran in the demo's frames too.
        self.front.high_score = self.front.high_score.max(self.game.high_score);
    }
}
impl Mode for Demo {
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        // Demo_CheckStart: Start on the real pad ends the demo.
        if self.step != Step::FadeOut && input.pressed(Button::Start) {
            self.leave();
            return None;
        }
        match self.step {
            Step::FadeIn => {
                self.game.tick(Input::default());
                if !self.game.fading() {
                    self.step = Step::Play;
                }
            }
            Step::Play => {
                self.ticks += 1;
                // Game_RunFrame with control type 0: the tape is not remapped.
                let pad = self.tape.next(&self.front.rom);
                self.game.tick(pad);
                // $7DB6: Game_Requests bits 0-1, a lost life (the death's
                // slot $2B) or the next substage.
                let ended = self.ticks >= DEMO_TICKS
                    || self.game.requests & (REQUEST_LIFE_LOST | REQUEST_NEXT_SUBSTAGE) != 0;
                if ended {
                    self.leave();
                }
            }
            Step::FadeOut => {
                self.fade = self.fade.advance();
                if self.fade.finished() {
                    return Some(go(&self.front, Next::SegaLogo));
                }
            }
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        self.game.draw(frame);
        frame.fade(self.fade);
    }
    fn name(&self) -> &'static str {
        "Demo"
    }
    fn take_audio(&mut self) -> Audio {
        Mode::take_audio(self.game.as_mut())
    }
    fn game(&self) -> Option<&Game> {
        Some(self.game.as_ref())
    }
    fn game_mut(&mut self) -> Option<&mut Game> {
        Some(self.game.as_mut())
    }
}
