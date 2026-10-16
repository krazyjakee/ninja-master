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
//! The screen keeps the cartridge's font, backdrop, option order and sound
//! test commands. A fixed native layout gives every control scheme room for
//! its diagram, with readable panels and matching mouse targets. Start or
//! Back stops the sound and fades back to the title.

use crate::{
    front_end::{Front, Next, Screen},
    input::{Button, Input},
    menu::MenuEvent,
    modes::{Audio, Mode, ModeChange},
    render::{Color, FADE_STEPS, Fade, Frame},
    rom,
    settings::DisplaySettings,
    sound_events::{EFFECT_TABLE, MUSIC_TABLE, Table, VOICE_TABLE},
    text::Style,
    title_art::TitleArt,
};

mod display;

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
/// Native option stored after the eight original ammunition counts.
pub const DISABLED_SHURIKENS: u8 = 8;
/// Display order: Disabled, 0, 10, 20, 30, 40, 50, 60, 70.
/// Keep the stored ROM indices stable for settings and saved runs.
const SHURIKEN_ORDER: [u8; 9] = [DISABLED_SHURIKENS, 3, 4, 5, 6, 7, 0, 1, 2];

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
    /// Index into `Shuriken_Counts`, or `DISABLED_SHURIKENS` for a melee-only run.
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
    pub fn shurikens_disabled(&self) -> bool {
        self.shurikens == DISABLED_SHURIKENS
    }
    /// `Game_InitLives`: spare lives and maximum health.
    pub fn lives_health(&self, rom: &[u8]) -> (u8, u8) {
        let at = LEVEL_LIVES_HEALTH + usize::from(self.level & 3) * 2;
        (byte(rom, at), byte(rom, at + 1))
    }
    /// `Game_InitAmmo`: the starting (and respawn) shuriken count.
    pub fn start_ammo(&self, rom: &[u8]) -> u16 {
        if self.shurikens_disabled() {
            0
        } else if self.infinite {
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
const LABELS: [&str; ITEMS] = [
    "LEVEL",
    "SHURIKENS",
    "CONTROLS",
    "MUSIC",
    "S.E.",
    "VOICE",
    "BACK",
];
const ROW_TOPS: [i32; ITEMS] = [32, 48, 64, 132, 148, 164, 180];
const ROW_HEIGHT: i32 = 16;
const VALUE_X: i32 = 144;
const VALUE_MIDDLE: i32 = 216;
const WHITE: Color = [220, 224, 232];
const MUTED: Color = [164, 176, 196];
const GOLD: Color = [255, 210, 100];
const PANEL: Color = [8, 12, 24];
const FOCUS: Color = [40, 50, 76];
/// `Data_008D4C`: NINJITSU, ATTACK, JUMP (8 characters each), selected
/// per control type and button by the offsets in `Data_008D3C`.
const CONTROL_NAMES: usize = 0x8d4c;
const CONTROL_ORDER: usize = 0x8d3c;
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
    display_page: bool,
    tabs_focused: bool,
    display_cursor: usize,
    display: DisplaySettings,
    display_changed: bool,
    display_available: bool,
    crt_error: Option<String>,
    /// `$FF00F8`, `$FF00FA`, `$FF00FC`: sound test selections (1 on entry).
    pub music: u16,
    pub effect: u16,
    /// Driver effects absent from the original sound-test record table.
    extra_effects: Vec<u8>,
    pub voice: u16,
    /// `$FF0100`: frames a direction has been held.
    repeat: u16,
    repeat_directions: u16,
    /// Last requested sound-test action; avoids implying a short sample loops.
    feedback: String,
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
        // The original list omits two real driver effects ($AB and $D7 in
        // every retail ROM). Append commands absent from all 66 records;
        // keep their identities numeric because the ROM supplies no names.
        let extra_effects = (0x9d..=0xd8)
            .filter(|command| {
                !(0..EFFECT_COUNT).any(|index| {
                    byte(&screen.front.rom, EFFECT_TABLE + index as usize * 16) == *command
                })
            })
            .collect();
        Self {
            art,
            screen,
            extra_effects,
            cursor: 0,
            display_page: false,
            tabs_focused: false,
            display_cursor: 0,
            display: DisplaySettings::default(),
            display_changed: false,
            display_available: false,
            crt_error: None,
            music: 1,
            effect: 1,
            voice: 1,
            repeat: 0,
            repeat_directions: 0,
            feedback: String::new(),
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
    /// Repeat only an unchanged held direction, after an initial delay.
    fn repeat_input(&mut self, mut input: Input) -> Input {
        let mask =
            Button::Left.mask() | Button::Right.mask() | Button::Up.mask() | Button::Down.mask();
        let directions = input.held & mask;
        if directions == 0 || directions != self.repeat_directions || input.pressed & mask != 0 {
            self.repeat = 0;
        } else {
            self.repeat = self.repeat.saturating_add(1);
            if self.repeat >= REPEAT_DELAY {
                let mut repeated = directions;
                // Repeating toggles would rapidly resize the window or flash
                // the presentation. Only numeric/choice values repeat.
                if self.display_page
                    && !self.tabs_focused
                    && matches!(self.display_cursor, 0 | 2 | 4 | 5 | 6)
                {
                    repeated &= !(Button::Left.mask() | Button::Right.mask());
                }
                input.pressed |= repeated;
                self.repeat = REPEAT_DELAY - 4;
            }
        }
        self.repeat_directions = directions;
        input
    }
    fn select(&mut self, row: usize) {
        if row != self.cursor {
            self.secret = 0;
            self.feedback.clear();
        }
        self.cursor = row;
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
            if (table == EFFECT_TABLE && value >= EFFECT_COUNT)
                || byte(self.rom(), table + value as usize * 16 + 1) == 0
            {
                break;
            }
        }
        value as u16
    }
    /// `Options_Menu` and the item under the cursor (`Options_Items`).
    fn menu(&mut self, input: Input) {
        if input.pressed(Button::X) || input.pressed(Button::Y) {
            self.change_page(input.pressed(Button::Y));
            return;
        }
        let vertical =
            i32::from(input.pressed(Button::Down)) - i32::from(input.pressed(Button::Up));
        if self.tabs_focused {
            if input.pressed(Button::Left) || input.pressed(Button::Right) {
                self.change_page(input.pressed(Button::Right));
            }
            if vertical != 0 || input.pressed(Button::Attack) || input.pressed(Button::Jump) {
                self.tabs_focused = false;
                if self.display_page {
                    self.display_cursor = if vertical < 0 { 10 } else { 0 };
                } else {
                    self.select(if vertical < 0 { EXIT } else { LEVEL });
                }
            }
            return;
        }
        if self.display_page {
            self.display_menu(input);
            return;
        }
        if vertical != 0 {
            let next = self.cursor as i32 + vertical;
            if !(0..ITEMS as i32).contains(&next) {
                self.tabs_focused = true;
                self.secret = 0;
            } else {
                self.select(next as usize);
            }
            // Moving between rows must not change a value on the same frame.
            return;
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
                if delta == 0 && count == 0 && !options.shurikens_disabled() {
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
                let order = SHURIKEN_ORDER
                    .iter()
                    .position(|&index| index == options.shurikens)
                    .unwrap_or(0);
                let order = (order as i32 + delta).rem_euclid(SHURIKEN_ORDER.len() as i32);
                options.shurikens = SHURIKEN_ORDER[order as usize];
            }
            CONTROL => {
                options.control = (i32::from(options.control) + delta).rem_euclid(6) as u8;
            }
            MUSIC => {
                if input.pressed(Button::Attack) {
                    let command = Table::Music.command(&rom, self.music as u8).ok();
                    self.screen.audio.request(command);
                    self.check_sequence(command);
                    self.feedback = format!("LAST PLAYED: MUSIC {:02}", self.music);
                } else if input.pressed(Button::Jump) {
                    self.screen.audio.push(STOP_ALL);
                    self.feedback = "SOUND TEST STOPPED".into();
                } else if input.pressed(Button::Ninjutsu) {
                    self.screen.audio.push(FADE_MUSIC);
                    self.feedback = "MUSIC FADING OUT".into();
                }
                let step = i32::from(input.pressed(Button::Right))
                    - i32::from(input.pressed(Button::Left));
                self.music = self.step_sound(MUSIC_TABLE, MUSIC_COUNT, self.music, step);
            }
            SE => {
                if input.pressed(Button::Attack) {
                    let command = self.effect_command();
                    self.screen.audio.request(command);
                    self.feedback = format!("LAST PLAYED: S.E. {:02}", self.effect);
                } else if input.pressed(Button::Jump) {
                    self.screen.audio.push(STOP_EFFECTS);
                    self.feedback = "EFFECTS AND VOICES STOPPED".into();
                }
                let step = i32::from(input.pressed(Button::Right))
                    - i32::from(input.pressed(Button::Left));
                self.effect = self.step_sound(
                    EFFECT_TABLE,
                    EFFECT_COUNT + self.extra_effects.len() as i32,
                    self.effect,
                    step,
                );
            }
            VOICE => {
                if input.pressed(Button::Attack) {
                    self.screen
                        .audio
                        .request(Table::Voice.command(&rom, self.voice as u8).ok());
                    self.feedback = format!("LAST PLAYED: VOICE {:02}", self.voice);
                } else if input.pressed(Button::Jump) {
                    self.screen.audio.push(STOP_EFFECTS);
                    self.feedback = "EFFECTS AND VOICES STOPPED".into();
                }
                let step = i32::from(input.pressed(Button::Right))
                    - i32::from(input.pressed(Button::Left));
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
    /// The expanded diagram is part of Controls for hover, but is read-only.
    fn hit(&self, x: i32, y: i32) -> Option<usize> {
        if !(24..304).contains(&x) {
            return None;
        }
        (0..ITEMS).find(|&item| {
            let height = if item == CONTROL { 64 } else { ROW_HEIGHT };
            (ROW_TOPS[item]..ROW_TOPS[item] + height).contains(&y)
        })
    }
    fn draw_text(&self, frame: &mut Frame, x: i32, y: i32, text: &str, color: Color) {
        let font = &self.screen.front.font;
        for (i, byte) in text.bytes().enumerate() {
            let x = x + i as i32 * 8;
            if byte == b'%' {
                // The cartridge map has no percentage sign.
                frame.rect(x, y, 2, 2, color);
                frame.rect(x + 5, y + 5, 2, 2, color);
                for step in 0..7 {
                    frame.pixel(x + 6 - step, y + step, color);
                }
            } else {
                font.draw_tile(frame, font.tile(byte), x, y, Style::plain(color, 1));
            }
        }
    }
    fn arrow(&self, frame: &mut Frame, x: i32, y: i32, right: bool, color: Color) {
        for line in 0..4 {
            frame.rect(
                x + if right { line } else { 3 - line },
                y + line,
                1,
                8 - line * 2,
                color,
            );
        }
    }
    fn control_diagram(&self) -> [String; 7] {
        let control = self.options().control;
        if control < 4 {
            let action = |line| {
                let order = CONTROL_ORDER + usize::from(control) * 4 + line;
                self.text(CONTROL_NAMES + usize::from(byte(self.rom(), order)), 8)
                    .trim()
                    .to_string()
            };
            [
                format!("A:{}", action(0)),
                format!("B:{}", action(1)),
                format!("C:{}", action(2)),
                "START:PAUSE".into(),
                String::new(),
                String::new(),
                String::new(),
            ]
        } else {
            [
                if control == 4 {
                    "A:GUARD"
                } else {
                    "A:NINJUTSU"
                }
                .into(),
                "B:MELEE".into(),
                "C:JUMP".into(),
                "X:THROW".into(),
                if control == 4 { "Y:THROW" } else { "Y:GUARD" }.into(),
                "Z:NINJUTSU".into(),
                "MODE:CYCLE  START:PAUSE".into(),
            ]
        }
    }
    fn text(&self, address: usize, length: usize) -> String {
        self.rom()
            .get(address..address + length)
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default()
    }
    fn effect_command(&self) -> Option<u8> {
        if self.effect < EFFECT_COUNT as u16 {
            Table::Effect.command(self.rom(), self.effect as u8).ok()
        } else {
            self.extra_effects
                .get(usize::from(self.effect) - EFFECT_COUNT as usize)
                .copied()
        }
    }
    /// A sound test record's name (`loc_008F96`: 14 characters).
    fn sound_name(&self, table: usize, index: u16) -> String {
        if table == EFFECT_TABLE && index >= EFFECT_COUNT as u16 {
            return self
                .extra_effects
                .get(usize::from(index) - EFFECT_COUNT as usize)
                .map_or_else(String::new, |command| format!("SFX ${command:02X}"));
        }
        self.text(table + usize::from(index) * 16 + 2, 14)
    }
}
impl Mode for OptionsScreen {
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
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
            let input = self.repeat_input(input);
            self.menu(input);
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        frame.center_content();
        if let Some(art) = &self.art {
            art.vdp.render(frame, Fade::In(FADE_STEPS));
        } else {
            frame.clear([0; 3]);
        }
        // Keep the ROM artwork at the edges, with solid backing for every glyph.
        frame.rect(16, 8, 288, 212, PANEL);
        self.draw_text(frame, 24, 16, "OPTIONS", GOLD);
        self.draw_tabs(frame);
        frame.rect(24, 29, 272, 1, FOCUS);
        if self.display_page {
            self.draw_display(frame);
            frame.fade(self.screen.fade);
            return;
        }
        let options = self.options();
        let ammo = if options.shurikens_disabled() {
            "DISABLE".into()
        } else if options.infinite {
            "INFINITE".into()
        } else {
            options.start_ammo(self.rom()).to_string()
        };
        let values = [
            options.level_name(self.rom()).trim().to_string(),
            ammo,
            CONTROL_SCHEMES[usize::from(options.control.min(5))].into(),
            format!(
                "{:02} {}",
                self.music,
                self.sound_name(MUSIC_TABLE, self.music).trim()
            ),
            format!(
                "{:02} {}",
                self.effect,
                self.sound_name(EFFECT_TABLE, self.effect).trim()
            ),
            format!(
                "{:02} {}",
                self.voice,
                self.sound_name(VOICE_TABLE, self.voice).trim()
            ),
            String::new(),
        ];
        for item in 0..ITEMS {
            let top = ROW_TOPS[item];
            let selected = item == self.cursor && !self.tabs_focused;
            if selected {
                frame.rect(24, top, 280, ROW_HEIGHT, FOCUS);
                frame.rect(24, top + 2, 2, ROW_HEIGHT - 4, GOLD);
            }
            let color = if selected { GOLD } else { WHITE };
            self.draw_text(frame, 32, top + 4, LABELS[item], color);
            if item != EXIT {
                self.arrow(frame, 130, top + 4, false, color);
                self.draw_text(frame, VALUE_X, top + 4, &values[item], color);
                self.arrow(frame, 290, top + 4, true, color);
            }
        }
        frame.rect(24, 82, 280, 46, [16, 22, 38]);
        let diagram = self.control_diagram();
        for (i, action) in diagram.iter().take(6).enumerate() {
            self.draw_text(
                frame,
                40 + (i / 3) as i32 * 128,
                84 + (i % 3) as i32 * 12,
                action,
                MUTED,
            );
        }
        self.draw_text(frame, 40, 120, &diagram[6], MUTED);
        frame.rect(24, 196, 272, 1, FOCUS);
        let sound = matches!(self.cursor, MUSIC | SE | VOICE);
        let help = if self.tabs_focused {
            "LEFT/RIGHT:TABS DOWN:OPTIONS"
        } else if sound {
            "L/R:SELECT B:PLAY C/RMB:STOP"
        } else if self.cursor == EXIT {
            "ENTER / START: BACK"
        } else {
            "L/R:CHANGE X/Y:TABS START:BACK"
        };
        self.draw_text(frame, 24, 200, help, WHITE);
        let detail = if sound && !self.feedback.is_empty() {
            self.feedback.as_str()
        } else {
            match self.cursor {
                LEVEL | EXIT => "APPLIES TO YOUR NEXT NEW GAME",
                SHURIKINS if options.shurikens_disabled() => "MELEE ONLY / NO AMMO PICKUPS",
                SHURIKINS if !options.infinite && options.start_ammo(self.rom()) == 0 => {
                    "START EMPTY / AMMO PICKUPS ON"
                }
                SHURIKINS => "STARTING AND RESPAWN AMMO",
                CONTROL => "F1: CONTROLS / REBIND INPUTS",
                MUSIC => "A:FADE  CLICK MUSIC:PLAY",
                SE => "CLICK S.E.:PLAY  START:BACK",
                VOICE => "CLICK VOICE:PLAY START:BACK",
                _ => "",
            }
        };
        self.draw_text(frame, 24, 212, detail, MUTED);
        // Fade the entire UI together, including the fallback without artwork.
        frame.fade(self.screen.fade);
    }
    fn options(&self) -> Option<GameOptions> {
        Some(self.screen.front.options)
    }
    fn set_options(&mut self, options: GameOptions) {
        self.secret = 0;
        self.screen.front.options = options;
    }
    fn display_context(&mut self, settings: DisplaySettings, error: Option<&str>) {
        if !self.display_changed {
            self.display = settings;
        }
        self.display_available = true;
        self.crt_error = error.map(str::to_owned);
    }
    fn take_display_settings(&mut self) -> Option<DisplaySettings> {
        std::mem::take(&mut self.display_changed).then_some(self.display)
    }
    fn menu_event(&mut self, event: MenuEvent) {
        if matches!(event, MenuEvent::Back) {
            self.exit();
            return;
        }
        if !self.screen.faded_in() || self.screen.leaving() {
            return;
        }
        if let MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) = event
            && (12..28).contains(&y)
            && ((112..176).contains(&x) || (184..296).contains(&x))
        {
            self.tabs_focused = true;
            self.secret = 0;
            self.repeat = 0;
            if matches!(event, MenuEvent::Click(_, _, false)) {
                self.change_page(x >= 184);
            }
            return;
        }
        if self.display_page {
            self.display_event(event);
            return;
        }
        match event {
            MenuEvent::Back => self.exit(),
            MenuEvent::Scroll(delta) => {
                self.tabs_focused = false;
                self.repeat = 0;
                self.select(
                    (self.cursor as i64 + i64::from(delta)).clamp(0, ITEMS as i64 - 1) as usize,
                );
            }
            MenuEvent::Hover(x, y) | MenuEvent::Click(x, y, _) => {
                if let Some(row) = self.hit(x, y) {
                    self.tabs_focused = false;
                    self.repeat = 0;
                    self.select(row);
                    if let MenuEvent::Click(_, _, alternate) = event {
                        if y >= ROW_TOPS[row] + ROW_HEIGHT {
                            return;
                        }
                        let button = match row {
                            EXIT => Button::Start,
                            MUSIC | SE | VOICE if alternate => Button::Jump,
                            MUSIC | SE | VOICE if x < 120 => Button::Attack,
                            _ if x < VALUE_MIDDLE || alternate => Button::Left,
                            _ => Button::Right,
                        };
                        // Apply the click to its own row now. Subsequent mouse
                        // motion or another click can arrive before the next tick.
                        self.menu(Input {
                            held: 0,
                            pressed: button.mask(),
                        });
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
mod screen_tests;

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

#[cfg(test)]
mod sound_list_tests {
    use super::*;
    #[test]
    #[ignore = "requires the three user-supplied cartridges"]
    fn sound_test_adds_both_unlisted_driver_effects_without_retriggering() {
        let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
        for region in ["us", "eu", "jp"] {
            let rom = crate::rom::regions::prepare(
                std::fs::read(directory.join(format!("shinobi3_{region}.bin"))).unwrap(),
            )
            .unwrap();
            let mut options = OptionsScreen::new(Front::load(&rom).unwrap());
            for _ in 0..100 {
                options.tick(Input::default());
            }
            options.take_audio();
            assert_eq!(options.extra_effects, [0xab, 0xd7]);
            options.cursor = SE;
            options.effect = 65;
            for command in [0xab, 0xd7] {
                options.tick(crate::input::press(Button::Right));
                assert_eq!(options.effect_command(), Some(command));
                options.tick(crate::input::press(Button::Attack));
                assert_eq!(options.take_audio().commands, [command]);
                options.tick(Input {
                    held: Button::Attack.mask(),
                    pressed: 0,
                });
                assert!(options.take_audio().commands.is_empty());
                let mut player = crate::sound::SoundPlayer::new(&rom, 48000).unwrap();
                player.play_command(command);
                let mut samples = [0; 1600];
                let mut audible = false;
                for _ in 0..120 {
                    player.render(&mut samples).unwrap();
                    audible |= samples.iter().any(|&sample| sample != 0);
                }
                assert!(audible, "{region}: ${command:02x}");
            }
            options.tick(crate::input::press(Button::Right));
            assert_eq!(options.effect, 0);
            options.effect = 66;
            options.menu_event(MenuEvent::Click(80, 156, false));
            options.tick(Input::default());
            assert_eq!(options.take_audio().commands, [0xab]);
            options.menu_event(MenuEvent::Click(80, 156, true));
            options.tick(Input::default());
            assert_eq!(options.take_audio().commands, [STOP_EFFECTS]);
        }
    }
}
