//! `Mode_Options` (`$86C4`, #44) and the settings it writes.
//!
//! The cartridge keeps the options in the word at `RAM_Start`: `$FF0000`
//! bits 4-5 control type, `Game_Options` (`$FF0001`) bits 2-3 level,
//! 4-6 the shuriken option and bit 7 infinite shurikens. `Options_Level`
//! also ORs `Options_LevelBits` into `$FF0000..$FF0003`; the low byte
//! `$FF0003` indexes `Continue_Counts` (bits 0-2) and the extra-life table
//! `Data_006186` (bits 4-5). Game start reads them all (`Game_InitSession`,
//! `Game_InitLives`, `Game_InitAmmo`, `Func_012514`), and `Player_ReadInput`
//! (`$3AC6`) maps the pad through the control type. `GameOptions` is that
//! state; every value is read from the ROM tables.
//!
//! The screen: `Options_Init` draws OPTION and the seven items, Up/Down
//! moves the cursor (`$FF00F6`), and the item under it reacts
//! (`Options_Items`). Start anywhere, or any button on EXIT, stops the
//! sound and fades back to `Mode_Title` (`Options_Exit`). The sound test
//! plays through the mode layer's `Audio`, like every other request.
//!
//! The backdrop is the title picture as `Options_Init` rebuilds it
//! (`crate::title_art`): dimmed by shadow/highlight except behind a
//! priority window, with the item texts written into plane A in their
//! original colours and the cursor sprite.

use crate::{
    front_end::{Front, Next, Screen},
    input::{Button, Input},
    menu::{Menu, MenuEvent},
    modes::{Audio, Mode, ModeChange},
    render::Frame,
    rom,
    sound_events::{EFFECT_TABLE, MUSIC_TABLE, Table, VOICE_TABLE},
    title_art::TitleArt,
};

/// Per level: spare lives, maximum health (NORMAL 2/16, EASY 9/16, HARD
/// 2/12, EXPERT 2/8).
const LEVEL_LIVES_HEALTH: usize = 0x615e;
/// Per shuriken option: 50 60 70 0 10 20 30 40.
const SHURIKEN_COUNTS: usize = 0x6174;
/// Per `$FF0003 & 7`: 3 3 3 3.
const CONTINUE_COUNTS: usize = 0x617c;
/// `Data_006186`, per `$FF0003` bits 4-5: the extra-life step (BCD
/// thousands) and a word that disables extra lives when nonzero.
const EXTRA_LIVES: usize = 0x6186;
/// `Options_LevelBits`: the long ORed into `RAM_Start` per level.
const LEVEL_BITS: usize = 0x8b62;
/// `Options_LevelOrder` / `Options_LevelFromOrder`: Left/Right cycle the
/// levels in the order NORMAL, HARD, EXPERT, EASY.
const LEVEL_ORDER: usize = 0x8ac8;
const LEVEL_FROM_ORDER: usize = 0x8acc;
/// NORMAL, EASY, HARD, EXPERT, eight characters each.
const LEVEL_NAMES: usize = 0x8b42;
/// `Player_ButtonLayouts`: per control type, the `Player_Input` bits for
/// Start, A, C and B.
const BUTTON_LAYOUTS: usize = 0x3c4c;

pub const CONTROL_SCHEMES: [&str; 6] = [
    "CLASSIC 1",
    "CLASSIC 2",
    "CLASSIC 3",
    "CLASSIC 4",
    "6 BUTTON",
    "DEDICATED",
];
/// `Options_InvincibleSequence`: HE RUNS, JAPONESQUE, SHINOBI WALK, SAKURA,
/// GETUFU.
const INVINCIBLE_SEQUENCE: usize = 0x8daa;
const INVINCIBLE_LENGTH: usize = 5;
/// `Hud_DrawAmmo` shows 999 for infinite shurikens.
pub const INFINITE_AMMO: u16 = 999;

/// The tables are at fixed addresses; a short image reads zeros.
fn byte(rom: &[u8], at: usize) -> u8 {
    rom::byte(rom, at).unwrap_or(0)
}

/// The options word and the secret flags that game start reads. The
/// cartridge keeps them in `$FF0000..$FF001F`, which survives the console's
/// Reset button but not power-on; the port keeps them for the session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameOptions {
    /// 0 NORMAL, 1 EASY, 2 HARD, 3 EXPERT.
    pub level: u8,
    /// Index into `Shuriken_Counts`.
    pub shurikens: u8,
    /// `Game_Options` bit 7: throws cost nothing.
    pub infinite: bool,
    /// Types 0..3 are classic layouts, 4 is six-button, 5 is dedicated actions.
    pub control: u8,
    /// `Cheat_Invincible` = 'R': `Player_ApplyHit` and
    /// `Player_SubtractHealth` skip the damage.
    pub invincible: bool,
}
impl GameOptions {
    /// `Game_InitLives`: spare lives and maximum health.
    pub fn lives_health(&self, rom: &[u8]) -> (u8, u8) {
        let at = LEVEL_LIVES_HEALTH + usize::from(self.level & 3) * 2;
        (byte(rom, at), byte(rom, at + 1))
    }
    /// `Game_InitAmmo`: the starting (and respawn) shuriken count.
    pub fn start_ammo(&self, rom: &[u8]) -> u16 {
        if self.infinite {
            INFINITE_AMMO
        } else {
            u16::from(byte(rom, SHURIKEN_COUNTS + usize::from(self.shurikens & 7)))
        }
    }
    /// `Options_LevelBits`' low byte, the level's `$FF0003`.
    fn level_byte(&self, rom: &[u8]) -> u8 {
        byte(rom, LEVEL_BITS + usize::from(self.level & 3) * 4 + 3)
    }
    /// `Game_InitSession`: continues for a new game.
    pub fn continues(&self, rom: &[u8]) -> u8 {
        byte(rom, CONTINUE_COUNTS + usize::from(self.level_byte(rom) & 7))
    }
    /// `Func_012514`: points between extra lives (the first comes at one
    /// step), or `None` where the table disables them (EXPERT).
    pub fn extra_life_step(&self, rom: &[u8]) -> Option<u32> {
        let at = EXTRA_LIVES + usize::from((self.level_byte(rom) >> 4) & 3) * 4;
        let [step, disabled] = rom::words(rom, at).unwrap_or_default();
        let step = crate::combat::Combat::score(step) * 1000;
        (disabled == 0 && step != 0).then_some(step)
    }
    /// `Options_LevelNames`.
    pub fn level_name(&self, rom: &[u8]) -> String {
        let at = LEVEL_NAMES + usize::from(self.level & 3) * 8;
        rom.get(at..at + 8)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    }
}

/// `Player_ReadInput` (`$3AC6`) through `Player_ButtonLayouts`: the pad's
/// A, B and C (native `Ninjutsu`, `Attack`, `Jump`) become the actions of
/// the control type. Type 0 keeps them; directions and Start pass through.
pub fn remap(rom: &[u8], control: u8, input: Input) -> Input {
    if control >= 4 {
        // Six-button: A guard, B melee, C jump, X/Y shuriken, Z ninjutsu.
        // Dedicated: A/Z ninjutsu, B melee, C jump, X shuriken, Y guard.
        let guard = if control == 4 {
            Button::Ninjutsu
        } else {
            Button::Y
        };
        let ranged = Button::X.mask() | if control == 4 { Button::Y.mask() } else { 0 };
        let cast = Button::Z.mask()
            | if control == 5 {
                Button::Ninjutsu.mask()
            } else {
                0
            };
        let actions = Button::Ninjutsu.mask() | Button::Attack.mask();
        return Input {
            held: (input.held & !actions)
                | if input.held(guard) {
                    Button::Attack.mask()
                } else {
                    0
                },
            pressed: (input.pressed & !actions)
                | if input.pressed & (ranged | Button::Attack.mask()) != 0 {
                    Button::Attack.mask()
                } else {
                    0
                }
                | if input.pressed & cast != 0 {
                    Button::Ninjutsu.mask()
                } else {
                    0
                },
        };
    }
    if control & 3 == 0 {
        return input;
    }
    let layout = BUTTON_LAYOUTS + usize::from(control & 3) * 4;
    let action = |bits: u8| match bits {
        0x80 => Button::Start.mask(),
        0x40 => Button::Ninjutsu.mask(),
        0x20 => Button::Jump.mask(),
        0x10 => Button::Attack.mask(),
        _ => 0,
    };
    // Layout bytes in the order the routine shifts the pad byte out:
    // Start (bit 7), A (6), C (5), B (4).
    let pads = [
        (Button::Start, 0),
        (Button::Ninjutsu, 1),
        (Button::Jump, 2),
        (Button::Attack, 3),
    ];
    let buttons = pads.iter().fold(0, |mask, (pad, _)| mask | pad.mask());
    let map = |mask: u16| {
        pads.iter().fold(mask & !buttons, |out, &(pad, i)| {
            if mask & pad.mask() != 0 {
                out | action(byte(rom, layout + i))
            } else {
                out
            }
        })
    };
    Input {
        held: map(input.held),
        pressed: map(input.pressed),
    }
}

/// Items in `Options_Items` order.
const ITEMS: usize = 7;
const LEVEL: usize = 0;
const SHURIKINS: usize = 1;
const CONTROL: usize = 2;
const MUSIC: usize = 3;
const SE: usize = 4;
const VOICE: usize = 5;
const EXIT: usize = 6;
/// Sound test ranges (`$1D`, `$42`, `$16` records).
const MUSIC_COUNT: i32 = 0x1d;
const EFFECT_COUNT: i32 = 0x42;
const VOICE_COUNT: i32 = 0x16;
/// `Options_Shurikens`: frames on a zero count, without Left/Right, that
/// turn on infinite shurikens when S.E. 2 is selected; voice `$0C`
/// (KATANA SET) confirms.
pub const SECRET_TICKS: u16 = 0xec;
const SECRET_EFFECT: u16 = 2;
const SECRET_VOICE: u8 = 0x0c;
/// Effect `$0E` confirms invincibility.
const INVINCIBLE_EFFECT: u8 = 0x0e;
/// `Func_008FAE`: a direction held this long auto-repeats every 4 frames.
const REPEAT_DELAY: u16 = 0x20;
/// Driver commands: `Func_004AB4` stops everything, `Func_004A88` fades
/// the music, `Func_004ABE` stops the effects.
const STOP_ALL: u8 = 0xff;
const FADE_MUSIC: u8 = 0xfc;
const STOP_EFFECTS: u8 = 0xfd;

/// `Mode_Options`.
pub struct OptionsScreen {
    screen: Screen,
    /// `$FF00F6` / 4.
    pub cursor: usize,
    viewport: Menu,
    pointer_input: Input,
    /// `$FF00F8`, `$FF00FA`, `$FF00FC`: sound test selections (1 on entry).
    pub music: u16,
    pub effect: u16,
    pub voice: u16,
    /// `$FF0100`: frames a direction has been held.
    repeat: u16,
    /// `$FF00FE`: the infinite-shuriken timer.
    secret: u16,
    /// `$FF9C12`: progress through the invincibility sequence; `None` after
    /// a wrong track (`-1`).
    sequence: Option<usize>,
    /// The backdrop `Options_Init` builds over the title's VRAM, with its
    /// palette in CRAM (the screen fades it in from black).
    art: Option<TitleArt>,
}
impl OptionsScreen {
    pub fn new(front: Front) -> Self {
        let mut screen = Screen::new(front);
        // Options_Init: Func_004AB4 stops the sound.
        screen.audio.push(STOP_ALL);
        screen.front.music = None;
        let art = TitleArt::options(&screen.front.rom).ok().map(|mut art| {
            art.vdp.cram = art.target;
            art
        });
        Self {
            art,
            screen,
            cursor: 0,
            viewport: Menu::new(48, 6, 22),
            pointer_input: Input::default(),
            music: 1,
            effect: 1,
            voice: 1,
            repeat: 0,
            secret: 0,
            sequence: Some(0),
        }
    }
    pub fn options(&self) -> GameOptions {
        self.screen.front.options
    }
    fn rom(&self) -> &[u8] {
        &self.screen.front.rom
    }
    /// `Func_008FAE`: Left/Right presses, auto-repeating while held.
    fn repeat_x(&mut self, input: Input) -> i32 {
        let directions = [Button::Left, Button::Right, Button::Up, Button::Down];
        if !directions.iter().any(|&button| input.held(button)) {
            self.repeat = 0;
        }
        self.repeat = self.repeat.saturating_add(1);
        let (right, left) = if self.repeat >= REPEAT_DELAY && self.screen.ticks.is_multiple_of(4) {
            (input.held(Button::Right), input.held(Button::Left))
        } else {
            (input.pressed(Button::Right), input.pressed(Button::Left))
        };
        i32::from(right) - i32::from(left)
    }
    /// Step a sound test selection, wrapping and skipping hidden records.
    fn step_sound(&self, table: usize, count: i32, current: u16, delta: i32) -> u16 {
        let mut value = i32::from(current);
        if delta == 0 {
            return current;
        }
        for _ in 0..count {
            value += delta;
            if value < 0 {
                value = count - 1;
            }
            if value >= count {
                value = 0;
            }
            if byte(self.rom(), table + value as usize * 16 + 1) == 0 {
                break;
            }
        }
        value as u16
    }
    /// `Options_Menu` and the item under the cursor (`Options_Items`).
    fn menu(&mut self, input: Input) {
        let vertical =
            i32::from(input.pressed(Button::Down)) - i32::from(input.pressed(Button::Up));
        if vertical != 0 {
            self.viewport.selected = self.cursor;
            self.viewport.step(vertical, ITEMS);
            self.cursor = self.viewport.selected;
        }
        // $FF9C10: A +1, else B -1, else Left/Right.
        let delta = if input.pressed(Button::Ninjutsu) {
            1
        } else if input.pressed(Button::Attack) {
            -1
        } else {
            i32::from(input.pressed(Button::Right)) - i32::from(input.pressed(Button::Left))
        };
        let rom = self.screen.front.rom.clone();
        let options = &mut self.screen.front.options;
        match self.cursor {
            LEVEL if delta != 0 => {
                let order = i32::from(byte(&rom, LEVEL_ORDER + usize::from(options.level & 3)));
                let order = (order + delta).rem_euclid(4) as usize;
                options.level = byte(&rom, LEVEL_FROM_ORDER + order) & 3;
            }
            SHURIKINS => {
                let count = byte(&rom, SHURIKEN_COUNTS + usize::from(options.shurikens & 7));
                if delta == 0 && count == 0 {
                    self.secret = self.secret.saturating_add(1);
                    if self.secret == SECRET_TICKS {
                        if self.effect == SECRET_EFFECT {
                            options.infinite = true;
                            self.screen
                                .audio
                                .request(Table::Voice.command(&rom, SECRET_VOICE).ok());
                        } else {
                            options.infinite = false;
                            self.secret = 0;
                        }
                    }
                } else {
                    options.infinite = false;
                    self.secret = 0;
                }
                let options = &mut self.screen.front.options;
                options.shurikens = (i32::from(options.shurikens) + delta).rem_euclid(8) as u8;
            }
            CONTROL => {
                options.control = (i32::from(options.control) + delta).rem_euclid(6) as u8;
            }
            MUSIC => {
                if input.pressed(Button::Attack) {
                    let command = Table::Music.command(&rom, self.music as u8).ok();
                    self.screen.audio.request(command);
                    self.check_sequence(command);
                } else if input.pressed(Button::Jump) {
                    self.screen.audio.push(STOP_ALL);
                } else if input.pressed(Button::Ninjutsu) {
                    self.screen.audio.push(FADE_MUSIC);
                }
                let step = self.repeat_x(input);
                self.music = self.step_sound(MUSIC_TABLE, MUSIC_COUNT, self.music, step);
            }
            SE => {
                if input.pressed(Button::Attack) {
                    self.screen
                        .audio
                        .request(Table::Effect.command(&rom, self.effect as u8).ok());
                } else if input.pressed(Button::Jump) {
                    self.screen.audio.push(STOP_EFFECTS);
                }
                let step = self.repeat_x(input);
                self.effect = self.step_sound(EFFECT_TABLE, EFFECT_COUNT, self.effect, step);
            }
            VOICE => {
                if input.pressed(Button::Attack) {
                    self.screen
                        .audio
                        .request(Table::Voice.command(&rom, self.voice as u8).ok());
                }
                let step = self.repeat_x(input);
                self.voice = self.step_sound(VOICE_TABLE, VOICE_COUNT, self.voice, step);
            }
            EXIT if [
                Button::Ninjutsu,
                Button::Attack,
                Button::Jump,
                Button::Start,
            ]
            .iter()
            .any(|&button| input.pressed(button)) =>
            {
                self.exit();
            }
            _ => {}
        }
    }
    /// `Options_Music` with B: the invincibility sequence advances on the
    /// right track, is abandoned on a wrong one, and completes with effect
    /// `$0E`.
    fn check_sequence(&mut self, command: Option<u8>) {
        let Some(step) = self.sequence else {
            return;
        };
        let expected = byte(self.rom(), INVINCIBLE_SEQUENCE + step);
        self.sequence = None;
        if command != Some(expected) {
            return;
        }
        if step + 1 < INVINCIBLE_LENGTH {
            self.sequence = Some(step + 1);
        } else {
            self.screen.front.options.invincible = true;
            let command = Table::Effect.command(self.rom(), INVINCIBLE_EFFECT).ok();
            self.screen.audio.request(command);
        }
    }
    /// `Options_Exit`: stop the sound and fade back to the title.
    fn exit(&mut self) {
        if !self.screen.leaving() {
            self.screen.audio.push(STOP_ALL);
            self.screen.leave(Next::Title);
        }
    }
    fn text(&self, address: usize, length: usize) -> String {
        self.rom()
            .get(address..address + length)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    }
    /// A sound test record's name (`loc_008F96`: 14 characters).
    fn sound_name(&self, table: usize, index: u16) -> String {
        self.text(table + usize::from(index) * 16 + 2, 14)
    }
}
impl Mode for OptionsScreen {
    fn tick(&mut self, mut input: Input) -> Option<ModeChange> {
        input.pressed |= std::mem::take(&mut self.pointer_input).pressed;
        if let Some(change) = self.screen.step() {
            return Some(change);
        }
        if self.screen.leaving() {
            return None;
        }
        // Mode_Options: Start in any state goes to Options_Exit.
        if input.pressed(Button::Start) {
            self.exit();
            return None;
        }
        if self.screen.faded_in() {
            self.menu(input);
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        if let Some(art) = &self.art {
            art.vdp.render(frame, self.screen.fade);
        } else {
            frame.clear([0; 3]);
        }
        let font = &self.screen.front.font;
        frame.rect(16, 16, 288, 28, [8, 12, 24]);
        font.draw_text(
            frame,
            24,
            24,
            "OPTIONS",
            crate::text::Style::plain([255, 210, 100], 1),
        );
        let options = self.options();
        let ammo = if options.infinite {
            "INFINITE".into()
        } else {
            options.start_ammo(self.rom()).to_string()
        };
        let rows = vec![
            format!("LEVEL       < {} >", options.level_name(self.rom()).trim()),
            format!("SHURIKENS   < {ammo} >"),
            format!(
                "CONTROLS    < {} >",
                CONTROL_SCHEMES[usize::from(options.control.min(5))]
            ),
            format!(
                "MUSIC <{}> {}",
                self.music,
                self.sound_name(MUSIC_TABLE, self.music).trim()
            ),
            format!(
                "S.E.  <{}> {}",
                self.effect,
                self.sound_name(EFFECT_TABLE, self.effect).trim()
            ),
            format!(
                "VOICE <{}> {}",
                self.voice,
                self.sound_name(VOICE_TABLE, self.voice).trim()
            ),
            "BACK".into(),
        ];
        self.viewport.draw(frame, font, &rows);
        frame.rect(16, 192, 288, 32, [8, 12, 24]);
        let style = crate::text::Style::plain([220, 224, 232], 1);
        if self.cursor == CONTROL && options.control >= 4 {
            let label = if options.control == 4 {
                "A GUARD B MELEE C JUMP"
            } else {
                "A JITSU B MELEE C JUMP"
            };
            font.draw_text(frame, 24, 196, label, style);
            font.draw_text(
                frame,
                24,
                208,
                if options.control == 4 {
                    "X/Y THROW Z JITSU MODE CYCLE"
                } else {
                    "X THROW Y GUARD Z JITSU"
                },
                style,
            );
        } else {
            font.draw_text(frame, 24, 196, "LEFT/RIGHT: CHANGE  START: BACK", style);
            font.draw_text(frame, 24, 208, "F1: REMAP  SOUND: B PLAY C STOP", style);
        }
    }
    fn options(&self) -> Option<GameOptions> {
        Some(self.screen.front.options)
    }
    fn set_options(&mut self, options: GameOptions) {
        self.screen.front.options = options;
    }
    fn menu_event(&mut self, event: MenuEvent) {
        if !self.screen.faded_in() || self.screen.leaving() {
            return;
        }
        match event {
            MenuEvent::Back => self.exit(),
            MenuEvent::Scroll(delta) => {
                self.viewport.scroll(delta, ITEMS);
                self.cursor = self.viewport.selected;
            }
            MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) => {
                if let Some(row) = self.viewport.hit(x, y, ITEMS) {
                    self.cursor = row;
                    self.viewport.selected = row;
                    if let MenuEvent::Click(_, _, alternate) = event {
                        let button = match row {
                            EXIT => Button::Start,
                            MUSIC | SE | VOICE if x < 112 => {
                                if alternate {
                                    Button::Jump
                                } else {
                                    Button::Attack
                                }
                            }
                            _ => {
                                if x < 180 || alternate {
                                    Button::Left
                                } else {
                                    Button::Right
                                }
                            }
                        };
                        self.pointer_input.pressed |= button.mask();
                    }
                }
            }
        }
    }
    fn name(&self) -> &'static str {
        "Options"
    }
    fn take_audio(&mut self) -> Audio {
        std::mem::take(&mut self.screen.audio)
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;
    use crate::input::press;

    #[test]
    fn dedicated_actions_keep_guard_melee_throw_and_cast_independent() {
        for scheme in [4, 5] {
            let guard = if scheme == 4 {
                Button::Ninjutsu
            } else {
                Button::Y
            };
            let guarded = remap(&[], scheme, press(guard));
            assert!(guarded.held(Button::Attack));
            assert!(!guarded.pressed(Button::Attack));
            assert!(!guarded.pressed(Button::Ninjutsu));
            for attack in [Button::Attack, Button::X] {
                let action = remap(&[], scheme, press(attack));
                assert!(action.pressed(Button::Attack));
                assert!(!action.held(Button::Attack));
                assert!(!action.pressed(Button::Ninjutsu));
            }
            assert!(remap(&[], scheme, press(Button::Z)).pressed(Button::Ninjutsu));
            for button in [
                Button::Left,
                Button::Right,
                Button::Up,
                Button::Down,
                Button::Jump,
                Button::Start,
                Button::Mode,
            ] {
                let mapped = remap(&[], scheme, press(button));
                assert!(mapped.pressed(button));
                assert!(mapped.held(button));
            }
        }
        assert!(remap(&[], 4, press(Button::Y)).pressed(Button::Attack));
        assert!(remap(&[], 5, press(Button::Ninjutsu)).pressed(Button::Ninjutsu));
        assert!(remap(&[], 0, press(Button::Attack)).held(Button::Attack));
    }
}
