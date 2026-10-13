/// Mega Drive 3-button pad: A = ninjutsu, B = attack, C = jump. The bit
/// numbers are part of the input-tape format and must not be renumbered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Button {
    Left,
    Right,
    Up,
    Down,
    /// Pad C.
    Jump,
    /// Pad B.
    Attack,
    Start,
    /// Pad A (bit 7, mask 0x80).
    Ninjutsu,
    /// Six-button pad X, Y, Z and Mode; original tape bits 0..7 stay fixed.
    X,
    Y,
    Z,
    Mode,
}

impl Button {
    pub const COUNT: usize = 12;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Left,
        Self::Right,
        Self::Up,
        Self::Down,
        Self::Jump,
        Self::Attack,
        Self::Start,
        Self::Ninjutsu,
        Self::X,
        Self::Y,
        Self::Z,
        Self::Mode,
    ];
    /// Every defined button bit.
    pub const ALL_MASK: u16 = (1 << Self::COUNT) - 1;

    pub const fn mask(self) -> u16 {
        1 << self as u8
    }
}

#[derive(Clone, Copy, Default)]
pub struct Input {
    pub held: u16,
    pub pressed: u16,
}

impl Input {
    pub fn held(self, button: Button) -> bool {
        self.held & button.mask() != 0
    }

    pub fn pressed(self, button: Button) -> bool {
        self.pressed & button.mask() != 0
    }
}

/// Parse an input tape: one held mask per game tick, decimal or `0x` hex,
/// optionally `MASK*COUNT` for `COUNT` identical ticks. `#` starts a comment.
/// Presses are the held edges, starting from nothing held.
pub fn parse_tape(text: &str) -> Result<Vec<Input>, String> {
    let mut inputs = Vec::new();
    let mut previous = 0;
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (mask, count) = match line.split_once('*') {
            Some((mask, count)) => (
                mask.trim(),
                count
                    .trim()
                    .parse::<usize>()
                    .ok()
                    .filter(|&count| count > 0)
                    .ok_or_else(|| format!("invalid repeat count at line {}", n + 1))?,
            ),
            None => (line, 1),
        };
        let held = match mask.strip_prefix("0x") {
            Some(hex) => u16::from_str_radix(hex, 16),
            None => mask.parse::<u16>(),
        }
        .map_err(|_| format!("invalid button mask at line {}", n + 1))?;
        if held & !Button::ALL_MASK != 0 {
            return Err(format!("unknown button bits at line {}", n + 1));
        }
        for _ in 0..count {
            inputs.push(Input {
                held,
                pressed: held & !previous,
            });
            previous = held;
        }
    }
    Ok(inputs)
}

/// Write held masks as a tape `parse_tape` reads back, one `0xMM*COUNT`
/// line per run of identical ticks.
pub fn format_tape(masks: &[u16]) -> String {
    let mut text = String::new();
    let mut rest = masks;
    while let Some(&mask) = rest.first() {
        let run = rest.iter().take_while(|&&m| m == mask).count();
        if run == 1 {
            text += &format!("0x{mask:02x}\n");
        } else {
            text += &format!("0x{mask:02x}*{run}\n");
        }
        rest = &rest[run..];
    }
    text
}

/// `button` newly pressed (and held), for tests driving modes.
#[cfg(test)]
pub(crate) fn press(button: Button) -> Input {
    Input {
        held: button.mask(),
        pressed: button.mask(),
    }
}

/// Retains short taps until a simulation tick consumes them. Different devices
/// must be combined before calling set_held, so one cannot release another.
#[derive(Default)]
pub struct InputLatch {
    state: Input,
}

impl InputLatch {
    pub fn set_held(&mut self, held: u16) {
        self.state.pressed |= held & !self.state.held;
        self.state.held = held;
    }

    pub fn take_tick(&mut self) -> Input {
        let input = self.state;
        self.state.pressed = 0;
        input
    }

    pub fn clear(&mut self) {
        self.state = Input::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_tap_survives_until_tick_and_is_consumed_once() {
        let mut latch = InputLatch::default();
        latch.set_held(Button::Jump.mask());
        latch.set_held(0);
        let tick = latch.take_tick();
        assert!(tick.pressed(Button::Jump));
        assert!(!tick.held(Button::Jump));
        assert!(!latch.take_tick().pressed(Button::Jump));
    }

    #[test]
    fn button_bits_are_stable() {
        let masks: Vec<u16> = Button::ALL.iter().map(|button| button.mask()).collect();
        assert_eq!(&masks[..8], [1, 2, 4, 8, 16, 32, 64, 128]);
        assert_eq!(&masks[8..], [256, 512, 1024, 2048]);
        assert_eq!(Button::ALL_MASK, 0xfff);
    }

    #[test]
    fn tapes_repeat_masks_and_round_trip() {
        let tape = parse_tape("# c\n2\n0x22*3 # run\n 0 * 2 \n0x10\n").unwrap();
        let held: Vec<u16> = tape.iter().map(|input| input.held).collect();
        assert_eq!(held, [2, 0x22, 0x22, 0x22, 0, 0, 0x10]);
        let pressed: Vec<u16> = tape.iter().map(|input| input.pressed).collect();
        assert_eq!(pressed, [2, 0x20, 0, 0, 0, 0, 0x10]);
        assert_eq!(format_tape(&held), "0x02\n0x22*3\n0x00*2\n0x10\n");
        let again: Vec<u16> = parse_tape(&format_tape(&held))
            .unwrap()
            .iter()
            .map(|input| input.held)
            .collect();
        assert_eq!(again, held);
        assert!(parse_tape("1*0").is_err());
        assert!(parse_tape("0x1000").is_err());
        assert!(parse_tape("x").is_err());
    }

    #[test]
    fn repeated_down_does_not_retrigger() {
        let mut latch = InputLatch::default();
        latch.set_held(Button::Jump.mask());
        latch.take_tick();
        latch.set_held(Button::Jump.mask());
        assert!(!latch.take_tick().pressed(Button::Jump));
    }
}
