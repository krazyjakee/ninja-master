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
    modes::{Audio, Mode, ModeChange},
    render::Frame,
    rom,
    sound_events::{EFFECT_TABLE, MUSIC_TABLE, Table, VOICE_TABLE},
    title_art::TitleArt,
    vdp::PLANE_ROW_BYTES,
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
    /// Types 0..3 are the ROM layouts; 4 enables the secret six-button mode.
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
    if control == 4 {
        // $22954 / $229C2: Z casts, B/X/Y attack, C jumps, A holds guard.
        // Keep X/Y/Mode so attack selection and cycling can read them.
        let extra = Button::X.mask() | Button::Y.mask();
        let actions = Button::Ninjutsu.mask() | Button::Attack.mask();
        return Input {
            held: (input.held & !actions)
                | if input.held(Button::Ninjutsu) {
                    Button::Attack.mask()
                } else {
                    0
                },
            pressed: (input.pressed & !actions)
                | if input.pressed & (extra | Button::Attack.mask()) != 0 {
                    Button::Attack.mask()
                } else {
                    0
                }
                | if input.pressed(Button::Z) {
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
/// `Data_008A2E`: each item's offset from `Data_00C28E` (VRAM `$C28E`,
/// plane A row 5, column 7) in 16-byte steps.
const ITEM_ROWS: usize = 0x8a2e;
const ITEM_BASE: usize = 0xc28e;
/// Item labels (address, characters): `Options_TextLevel` ... `_TextExit`.
const LABELS: [(usize, usize); ITEMS] = [
    (0x8b3c, 5),
    (0x8c6a, 9),
    (0x8d30, 12),
    (0x8e28, 5),
    (0x8eb0, 4),
    (0x8f28, 5),
    (0x8f5e, 4),
];
/// `Options_TextTitle` "OPTION" at VRAM `$C2A0` (row 5, column 16), and
/// the S (`$E013` at `$C2AC`) `Options_Init` adds unless `Options_Secret`.
const TITLE_TEXT: (usize, usize) = (0x899a, 6);
const TITLE_VRAM: usize = 0xc2a0;
const TITLE_S: (usize, u16) = (0xc2ac, 0xe013);
/// Item attributes: `Func_0089A0` draws with `$A000` (line 1, colour 11 =
/// `$666` grey), the item under the cursor with `$E000` (line 3, colour 11),
/// both with priority, so the text stays lit under shadow/highlight.
const NORMAL: u16 = 0xa000;
const SELECTED: u16 = 0xe000;
/// `Data_008D4C`: NINJITSU, ATTACK, JUMP (8 characters each), selected
/// per control type and button by the offsets in `Data_008D3C`.
const CONTROL_NAMES: usize = 0x8d4c;
const CONTROL_ORDER: usize = 0x8d3c;
/// Font tiles for the control letters ('A' = 1) and the colon, and the
/// two tiles of the infinity sign (`Func_008BFA`).
const COLON_TILE: u16 = 0x2b;
const INFINITY_TILES: [u16; 2] = [0x29, 0x2f];
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
            self.cursor = (self.cursor as i32 + vertical).rem_euclid(ITEMS as i32) as usize;
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
                options.control = (i32::from(options.control) + delta).rem_euclid(5) as u8;
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
    /// An item's plane A address: `Data_00C28E` + 16 * `Data_008A2E[item]`,
    /// plus `bytes`.
    fn address(&self, item: usize, bytes: usize) -> usize {
        ITEM_BASE + usize::from(byte(self.rom(), ITEM_ROWS + item)) * 16 + bytes
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
            self.menu(input);
        }
        None
    }
    fn draw(&self, frame: &mut Frame) {
        let Some(art) = &self.art else {
            frame.clear([0; 3]);
            return;
        };
        // The cartridge rewrites plane A cells as items change; the port
        // writes every item into a copy of the backdrop each frame.
        let mut art = art.clone();
        let front = &self.screen.front;
        let rom = &front.rom;
        let options = front.options;
        art.write_text(
            TITLE_VRAM,
            self.text(TITLE_TEXT.0, TITLE_TEXT.1).as_bytes(),
            SELECTED,
        );
        // Without the six-button probe's secret, "OPTION" gains its S.
        art.vdp.write_word(TITLE_S.0, TITLE_S.1);
        for (item, &(address, length)) in LABELS.iter().enumerate() {
            // Func_0089A0 draws every item with $A000; the item under the
            // cursor redraws itself with $E000.
            let attribute = if item == self.cursor {
                SELECTED
            } else {
                NORMAL
            };
            let mut put = |at: usize, text: &str| art.write_text(at, text.as_bytes(), attribute);
            put(self.address(item, 0), &self.text(address, length));
            match item {
                LEVEL => put(self.address(item, 0x1c), &options.level_name(rom)),
                SHURIKINS if options.infinite => {
                    for (i, tile) in INFINITY_TILES.iter().enumerate() {
                        art.vdp
                            .write_word(self.address(item, 0x1c) + i * 2, tile | attribute);
                    }
                }
                SHURIKINS => {
                    // Func_004456: two BCD digits, leading zero kept.
                    let count = byte(rom, SHURIKEN_COUNTS + usize::from(options.shurikens));
                    put(self.address(item, 0x1c), &format!("{count:02}"));
                }
                CONTROL => {
                    let at = self.address(item, 0x18);
                    let label = if options.control == 4 {
                        "6".to_owned()
                    } else {
                        char::from(b'1' + options.control).to_string()
                    };
                    put(at, &label);
                    // Three lines, two rows apart: A:, B:, C: and the action.
                    for line in 0..3u16 {
                        let row = at + 4 + usize::from(line) * 2 * PLANE_ROW_BYTES;
                        art.vdp.write_word(row, (1 + line) | attribute);
                        art.vdp.write_word(row + 2, COLON_TILE | attribute);
                        let order = CONTROL_ORDER
                            + usize::from(options.control & 3) * 4
                            + usize::from(line);
                        let name = if options.control == 4 {
                            ["GUARD   ", "SWORD   ", "JUMP    "][usize::from(line)].to_owned()
                        } else {
                            self.text(CONTROL_NAMES + usize::from(byte(rom, order)), 8)
                        };
                        art.write_text(row + 4, name.as_bytes(), attribute);
                    }
                }
                MUSIC | SE | VOICE => {
                    let (table, index) = match item {
                        MUSIC => (MUSIC_TABLE, self.music),
                        SE => (EFFECT_TABLE, self.effect),
                        _ => (VOICE_TABLE, self.voice),
                    };
                    let name = self.sound_name(table, index);
                    put(self.address(item, 0x16), &name);
                }
                _ => {}
            }
        }
        // Options_Menu places the cursor sprite from its first frame on.
        if self.screen.faded_in() || self.screen.leaving() {
            art.options_cursor(rom, self.cursor);
        }
        art.vdp.render(frame, self.screen.fade);
    }
    fn name(&self) -> &'static str {
        "Options"
    }
    fn take_audio(&mut self) -> Audio {
        std::mem::take(&mut self.screen.audio)
    }
}
