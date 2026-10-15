use ninja_master_native::{
    input::Button,
    settings::{Bindings, Settings},
};
use sdl2::{
    controller::{Axis, Button as PadButton, GameController},
    keyboard::Scancode,
};
use std::collections::HashSet;

/// Bindings resolved to SDL values; indexed by `Button as usize`.
pub(super) struct InputMap {
    keys: [Vec<Scancode>; Button::COUNT],
    pads: [Vec<PadButton>; Button::COUNT],
}

impl InputMap {
    /// Unknown names are reported and dropped. A button whose configured
    /// names are all unknown falls back to its default binding.
    pub(super) fn new(settings: &Settings) -> Self {
        fn resolve<T>(
            bindings: &Bindings,
            defaults: &Bindings,
            device: &str,
            parse: impl Fn(&str) -> Option<T>,
        ) -> [Vec<T>; Button::COUNT] {
            std::array::from_fn(|index| {
                let button = Button::ALL[index];
                let names = bindings.get(button);
                let mut resolved = Vec::new();
                for name in names {
                    match parse(name.as_str()) {
                        Some(value) => resolved.push(value),
                        None => eprintln!("settings: unknown {device} input {name:?} ignored"),
                    }
                }
                if resolved.is_empty() && !names.is_empty() {
                    resolved = defaults
                        .get(button)
                        .iter()
                        .filter_map(|name| parse(name.as_str()))
                        .collect();
                }
                resolved
            })
        }
        Self {
            keys: resolve(
                &settings.keyboard,
                &Bindings::default_keyboard(),
                "keyboard",
                Scancode::from_name,
            ),
            pads: resolve(
                &settings.gamepad,
                &Bindings::default_gamepad(),
                "gamepad",
                PadButton::from_string,
            ),
        }
    }

    fn keyboard_mask(&self, keys: &HashSet<Scancode>) -> u16 {
        Button::ALL.iter().fold(0, |mask, button| {
            mask | if self.keys[*button as usize]
                .iter()
                .any(|key| keys.contains(key))
            {
                button.mask()
            } else {
                0
            }
        })
    }

    fn controller_mask(&self, pad: &GameController) -> u16 {
        let mut mask = Button::ALL.iter().fold(0, |mask, button| {
            mask | if self.pads[*button as usize]
                .iter()
                .any(|binding| pad.button(*binding))
            {
                button.mask()
            } else {
                0
            }
        });
        // The left stick always mirrors the D-pad; it is not rebindable.
        let x = pad.axis(Axis::LeftX);
        let y = pad.axis(Axis::LeftY);
        for (active, button) in [
            (x < -16_000, Button::Left),
            (x > 16_000, Button::Right),
            (y < -16_000, Button::Up),
            (y > 16_000, Button::Down),
        ] {
            if active {
                mask |= button.mask();
            }
        }
        mask
    }

    pub(super) fn mask(
        &self,
        keys: &HashSet<Scancode>,
        pads: &[GameController],
        controller: &str,
    ) -> u16 {
        pads.iter()
            .filter(|pad| controller.is_empty() || pad.name() == controller)
            .fold(self.keyboard_mask(keys), |mask, pad| {
                mask | self.controller_mask(pad)
            })
    }
}

/// Front-end keys that are never bound to pad controls.
pub(super) const RESERVED_KEYS: [Scancode; 11] = [
    Scancode::Escape,
    Scancode::Backspace,
    Scancode::F1,
    Scancode::F2,
    Scancode::F5,
    Scancode::F6,
    Scancode::F7,
    Scancode::F8,
    Scancode::F9,
    Scancode::F10,
    Scancode::F11,
];
