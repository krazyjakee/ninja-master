//! Disk saves reconstruct gameplay from its deterministic input/action history.
//! No game structs or ROM assets are serialized. Audio communication is recorded
//! explicitly so music-dependent transitions replay without synthesizing PCM.
use crate::{
    front_end::{Front, SegaLogo},
    game::Game,
    input::Input,
    modes::Modes,
    stage::StageId,
};

#[derive(Clone)]
pub(crate) struct History {
    pub boot: bool,
    pub stage: StageId,
    pub effects: [bool; 4],
    pub events: Vec<Event>,
}

#[derive(Clone, Copy)]
pub(crate) enum Event {
    Tick(Input),
    Communication(u8),
    Warp(StageId),
    Skip,
    Refill,
    Powered(bool),
    Invincible(bool),
    Infinite(bool),
    Locks(bool, bool),
    Resume,
    ViewWidth(u16),
    Menu(crate::menu::MenuEvent),
    Options(crate::options::GameOptions),
    Session(crate::options::GameOptions, u32, u16),
}

impl History {
    pub fn restore(&self, rom: &[u8]) -> Result<Box<Game>, String> {
        let mut modes = if self.boot {
            let mut front = Front::load(rom)?;
            front.effects = self.effects;
            Modes::new(Box::new(SegaLogo::new(front)))
        } else {
            let mut game = Game::load_stage(rom, self.stage)?;
            game.set_effects(self.effects);
            Modes::new(Box::new(game))
        };
        let mut tools = super::DevTools::new(rom);
        let mut view_width = None;
        for event in &self.events {
            match *event {
                Event::Menu(event) => modes.menu_event(event),
                Event::Options(options) => modes.set_options(options),
                Event::Session(options, high_score, demo_counter) => {
                    let game = modes.game_mut().ok_or("session has no gameplay")?;
                    game.start_session(options);
                    game.high_score = high_score;
                    game.demo_counter = demo_counter;
                    let game = game.clone();
                    modes.replace(crate::round_screens::StageClear::round_card(Box::new(game)));
                }
                Event::Tick(input) => {
                    if let Some(width) = view_width {
                        modes.set_view_width(width);
                    }
                    modes.tick(input);
                    modes.take_audio();
                }
                Event::Communication(value) => modes.sound_communication(value),
                Event::Warp(id) => tools.warp(&mut modes, id),
                Event::Skip => tools.skip_substage(&mut modes),
                Event::Refill => tools.refill(&mut modes),
                Event::Powered(value) => tools.set_powered(&mut modes, value),
                Event::Invincible(value) => tools.set_invincible(&mut modes, value),
                Event::Infinite(value) => tools.set_infinite(&mut modes, value),
                Event::Locks(lives, health) => {
                    tools.infinite_lives = lives;
                    tools.lock_health = health;
                    tools.after_tick(&mut modes);
                }
                Event::Resume => {
                    let mut game = modes.game().ok_or("save replay lost gameplay")?.clone();
                    game.restart_music();
                    modes.replace(Box::new(game));
                }
                Event::ViewWidth(width) => {
                    view_width = Some(usize::from(width));
                    modes.set_view_width(usize::from(width));
                }
            }
        }
        modes
            .game()
            .map(|g| Box::new(g.clone()))
            .ok_or_else(|| format!("save replay has no gameplay (ended in {})", modes.name()))
    }

    /// Older desktop recordings omitted the display width, although it changes
    /// object entrances and removal. Try the two desktop widths and retain the
    /// successful choice in the history so saving after a load stays replayable.
    pub fn restore_checked(
        &self,
        rom: &[u8],
        stage: StageId,
        checksum: u32,
    ) -> Result<(Box<Game>, Self), String> {
        let legacy = !self
            .events
            .iter()
            .any(|event| matches!(event, Event::ViewWidth(_)));
        let widths = [None, Some(crate::WIDTH), Some(crate::WIDE_WIDTH)];
        let mut replay_error = None;
        let mut replayed_gameplay = false;
        for width in widths.into_iter().take(if legacy { 3 } else { 1 }) {
            let mut history = self.clone();
            if let Some(width) = width {
                history.events.insert(0, Event::ViewWidth(width as u16));
            }
            let game = match history.restore(rom) {
                Ok(game) => game,
                Err(error) if legacy => {
                    replay_error = Some(error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            replayed_gameplay = true;
            let mut frame = crate::render::Frame::default();
            game.draw(&mut frame);
            if game.stage_id == stage && crate::assets::crc32(frame.rgb()) == checksum {
                return Ok((game, history));
            }
        }
        if !replayed_gameplay && let Some(error) = replay_error {
            return Err(format!("{error}; game left unchanged"));
        }
        Err("save replay differs from this build; game left unchanged".into())
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        out.extend([
            u8::from(self.boot),
            self.stage.round,
            self.stage.substage,
            self.stage.part,
        ]);
        out.extend(self.effects.map(u8::from));
        out.extend((self.events.len() as u32).to_le_bytes());
        for event in &self.events {
            match *event {
                Event::Menu(event) => {
                    use crate::menu::MenuEvent;
                    out.push(13);
                    let (tag, x, y) = match event {
                        MenuEvent::Hover(x, y) => (0, x, y),
                        MenuEvent::Click(x, y, false) => (1, x, y),
                        MenuEvent::Click(x, y, true) => (2, x, y),
                        MenuEvent::Scroll(delta) => (3, delta, 0),
                        MenuEvent::Back => (4, 0, 0),
                    };
                    out.push(tag);
                    out.extend(x.to_le_bytes());
                    out.extend(y.to_le_bytes());
                }
                Event::Options(options) => {
                    out.push(11);
                    encode_options(options, out);
                }
                Event::Session(options, high_score, demo_counter) => {
                    out.push(12);
                    encode_options(options, out);
                    out.extend(high_score.to_le_bytes());
                    out.extend(demo_counter.to_le_bytes());
                }
                Event::Tick(input) => {
                    out.push(0);
                    out.extend(input.held.to_le_bytes());
                    out.extend(input.pressed.to_le_bytes());
                }
                Event::Communication(v) => out.extend([1, v]),
                Event::Warp(id) => out.extend([2, id.round, id.substage, id.part]),
                Event::Skip => out.push(3),
                Event::Refill => out.push(4),
                Event::Powered(v) => out.extend([5, u8::from(v)]),
                Event::Invincible(v) => out.extend([6, u8::from(v)]),
                Event::Infinite(v) => out.extend([7, u8::from(v)]),
                Event::Locks(a, b) => out.extend([8, u8::from(a), u8::from(b)]),
                Event::Resume => out.push(9),
                Event::ViewWidth(width) => {
                    out.push(10);
                    out.extend(width.to_le_bytes());
                }
            }
        }
    }

    pub fn decode(reader: &mut Reader<'_>) -> Result<Self, String> {
        let boot = reader.boolean()?;
        let stage = reader.stage()?;
        let effects = [
            reader.boolean()?,
            reader.boolean()?,
            reader.boolean()?,
            reader.boolean()?,
        ];
        let count = reader.u32()? as usize;
        if count > 2_000_000 || count > reader.bytes.len() {
            return Err("save history is too large or truncated".into());
        }
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            events.push(match reader.byte()? {
                0 => {
                    let held = reader.u16()?;
                    let pressed = reader.u16()?;
                    if (held | pressed) & !crate::input::Button::ALL_MASK != 0 {
                        return Err("invalid saved input".into());
                    }
                    Event::Tick(Input { held, pressed })
                }
                1 => Event::Communication(reader.byte()?),
                2 => Event::Warp(reader.stage()?),
                3 => Event::Skip,
                4 => Event::Refill,
                5 => Event::Powered(reader.boolean()?),
                6 => Event::Invincible(reader.boolean()?),
                7 => Event::Infinite(reader.boolean()?),
                8 => Event::Locks(reader.boolean()?, reader.boolean()?),
                9 => Event::Resume,
                10 => {
                    let width = reader.u16()?;
                    if usize::from(width) < crate::WIDTH {
                        return Err("invalid saved display width".into());
                    }
                    Event::ViewWidth(width)
                }
                11 => Event::Options(decode_options(reader)?),
                12 => Event::Session(decode_options(reader)?, reader.u32()?, reader.u16()?),
                13 => {
                    use crate::menu::MenuEvent;
                    let tag = reader.byte()?;
                    let x = reader.u32()? as i32;
                    let y = reader.u32()? as i32;
                    Event::Menu(match tag {
                        0 => MenuEvent::Hover(x, y),
                        1 => MenuEvent::Click(x, y, false),
                        2 => MenuEvent::Click(x, y, true),
                        3 => MenuEvent::Scroll(x),
                        4 => MenuEvent::Back,
                        _ => return Err("invalid saved menu input".into()),
                    })
                }
                _ => return Err("unknown saved action".into()),
            });
        }
        Ok(Self {
            boot,
            stage,
            effects,
            events,
        })
    }
}

pub(crate) struct Reader<'a> {
    pub bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if self.bytes.len() < count {
            return Err("truncated save".into());
        }
        let (value, rest) = self.bytes.split_at(count);
        self.bytes = rest;
        Ok(value)
    }
    pub fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    pub fn boolean(&mut self) -> Result<bool, String> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("invalid saved boolean".into()),
        }
    }
    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn stage(&mut self) -> Result<StageId, String> {
        let id = StageId::new(self.byte()?, self.byte()?, self.byte()?);
        if !(1..=7).contains(&id.round) || !(1..=3).contains(&id.substage) || id.part > 31 {
            return Err("invalid saved stage".into());
        }
        Ok(id)
    }
}

fn encode_options(options: crate::options::GameOptions, out: &mut Vec<u8>) {
    out.extend([
        options.level,
        options.shurikens,
        u8::from(options.infinite),
        options.control,
        u8::from(options.invincible),
    ]);
}
fn decode_options(reader: &mut Reader<'_>) -> Result<crate::options::GameOptions, String> {
    let options = crate::options::GameOptions {
        level: reader.byte()?,
        shurikens: reader.byte()?,
        infinite: reader.boolean()?,
        control: reader.byte()?,
        invincible: reader.boolean()?,
    };
    if options.level > 3 || options.shurikens > 7 || options.control > 5 {
        return Err("invalid saved options".into());
    }
    Ok(options)
}
