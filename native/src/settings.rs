//! Persisted native settings: display scale/fullscreen, volume and bindings.
//!
//! The file is plain INI-style `key = value` text. Binding values are device
//! names understood by the desktop front end (SDL scancode names for the
//! keyboard, SDL game-controller button names for pads); this module only
//! stores them, so it stays independent of SDL and is testable headless.

use crate::input::Button;
mod display;
pub use display::DisplaySettings;
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

/// Environment variable that overrides the settings file path.
pub const CONFIG_ENV: &str = "NINJA_MASTER_CONFIG";
pub const MIN_SCALE: u32 = 1;
pub const MAX_SCALE: u32 = 8;
pub const DEFAULT_SCALE: u32 = 3;
pub const MAX_VOLUME: u8 = 100;

/// Mega Drive pad controls in rebinding-prompt order: button, config key, label.
pub const PAD_CONTROLS: [(Button, &str, &str); Button::COUNT] = [
    (Button::Up, "up", "Up"),
    (Button::Down, "down", "Down"),
    (Button::Left, "left", "Left"),
    (Button::Right, "right", "Right"),
    (Button::Ninjutsu, "a", "A (ninjutsu)"),
    (Button::Attack, "b", "B (attack)"),
    (Button::Jump, "c", "C (jump)"),
    (Button::Start, "start", "Start"),
    (Button::X, "x", "X (shuriken)"),
    (Button::Y, "y", "Y (shuriken)"),
    (Button::Z, "z", "Z (ninjutsu)"),
    (Button::Mode, "mode", "Mode (cycle ninjutsu)"),
];

/// Per-button lists of device input names, indexed by `Button as usize`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    slots: [Vec<String>; Button::COUNT],
}

impl Bindings {
    fn from_table(table: &[(Button, &[&str])]) -> Self {
        let mut bindings = Self {
            slots: std::array::from_fn(|_| Vec::new()),
        };
        for (button, names) in table {
            bindings.slots[*button as usize] = names.iter().map(|name| name.to_string()).collect();
        }
        bindings
    }

    /// Keyboard defaults: arrows for the D-pad, the A/S/D row (and Z/X/C row)
    /// for pad A/B/C in left-to-right order, Space as an extra jump, Enter for Start.
    pub fn default_keyboard() -> Self {
        Self::from_table(&[
            (Button::Up, &["Up"]),
            (Button::Down, &["Down"]),
            (Button::Left, &["Left"]),
            (Button::Right, &["Right"]),
            (Button::Ninjutsu, &["A", "Z"]),
            (Button::Attack, &["S", "X"]),
            (Button::Jump, &["D", "C", "Space"]),
            (Button::Start, &["Return"]),
            (Button::X, &["Q"]),
            (Button::Y, &["W"]),
            (Button::Z, &["E"]),
            (Button::Mode, &["Tab"]),
        ])
    }

    /// Controller defaults (SDL positional names): west/south/east face buttons
    /// form the Mega Drive A/B/C row, so B (attack) sits on the bottom button.
    pub fn default_gamepad() -> Self {
        Self::from_table(&[
            (Button::Up, &["dpup"]),
            (Button::Down, &["dpdown"]),
            (Button::Left, &["dpleft"]),
            (Button::Right, &["dpright"]),
            (Button::Ninjutsu, &["x"]),
            (Button::Attack, &["a"]),
            (Button::Jump, &["b"]),
            (Button::Start, &["start"]),
            (Button::X, &["leftshoulder"]),
            (Button::Y, &["rightshoulder"]),
            (Button::Z, &["y"]),
            (Button::Mode, &["back"]),
        ])
    }

    pub fn get(&self, button: Button) -> &[String] {
        &self.slots[button as usize]
    }

    pub fn set(&mut self, button: Button, names: Vec<String>) {
        self.slots[button as usize] = names;
    }

    /// Bind `name` to exactly `button`, removing it from every other button.
    pub fn assign(&mut self, button: Button, name: &str) {
        for slot in &mut self.slots {
            slot.retain(|existing| !existing.eq_ignore_ascii_case(name));
        }
        self.slots[button as usize] = vec![name.to_string()];
    }
}

/// An entry this version does not understand, preserved across saves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub section: String,
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Empty uses original art; otherwise a library entry or legacy pack path.
    pub art_pack: String,
    pub game: crate::options::GameOptions,
    /// Initial window size as a multiple of the selected frame size.
    pub scale: u32,
    pub fullscreen: bool,
    /// Expand gameplay to fit the window's width without stretching pixels.
    pub widescreen: bool,
    /// 0: native pixels, 1: 4:3, 2: fill the window.
    pub aspect: usize,
    pub integer_scale: bool,
    /// Interpolate scenery waves instead of stepping between pixel rows.
    pub smooth_wave_effects: bool,
    /// Presentation-only darkening of alternating source rows, 0..=100.
    pub scanlines: u8,
    /// GPU CRT presentation with phosphor beams, bloom and curved glass.
    pub crt: bool,
    /// Fisheye curvature, 0..=100; 25 gives a subtle rounded screen.
    pub crt_curvature: u8,
    /// Master volume, 0..=100 percent.
    pub volume: u8,
    pub muted: bool,
    /// Empty accepts every connected controller; otherwise its SDL name.
    pub controller: String,
    pub keyboard: Bindings,
    pub gamepad: Bindings,
    pub extra: Vec<Entry>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            art_pack: String::new(),
            game: crate::options::GameOptions::default(),
            scale: DEFAULT_SCALE,
            fullscreen: false,
            widescreen: true,
            aspect: 0,
            integer_scale: true,
            smooth_wave_effects: true,
            scanlines: 0,
            crt: false,
            crt_curvature: 25,
            volume: MAX_VOLUME,
            muted: false,
            controller: String::new(),
            keyboard: Bindings::default_keyboard(),
            gamepad: Bindings::default_gamepad(),
            extra: Vec::new(),
        }
    }
}

enum Applied {
    Ok,
    Invalid,
    Unknown,
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_list(value: &str) -> Vec<String> {
    if value.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn write_list(names: &[String]) -> String {
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

impl Settings {
    /// Initial window width. The desktop adapts the game view when resized.
    pub fn frame_width(&self) -> usize {
        if self.widescreen {
            crate::WIDE_WIDTH
        } else {
            crate::WIDTH
        }
    }

    /// Linear gain for the master volume.
    pub fn gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            f32::from(self.volume.min(MAX_VOLUME)) / f32::from(MAX_VOLUME)
        }
    }

    fn apply(&mut self, section: &str, key: &str, value: &str) -> Applied {
        match (section, key) {
            ("game", "level" | "shurikens" | "control") => match value.parse::<u8>() {
                Ok(value)
                    if value
                        <= match key {
                            "level" => 3,
                            "shurikens" => crate::options::DISABLED_SHURIKENS,
                            _ => 5,
                        } =>
                {
                    match key {
                        "level" => self.game.level = value,
                        "shurikens" => self.game.shurikens = value,
                        _ => self.game.control = value,
                    }
                    Applied::Ok
                }
                _ => Applied::Invalid,
            },
            ("game", "infinite") => match parse_bool(value) {
                Some(value) => {
                    self.game.infinite = value;
                    Applied::Ok
                }
                None => Applied::Invalid,
            },
            ("art", "pack") => {
                self.art_pack = value.to_owned();
                Applied::Ok
            }
            ("display", "aspect") => match value.parse::<usize>() {
                Ok(aspect) if aspect <= 2 => {
                    self.aspect = aspect;
                    Applied::Ok
                }
                _ => Applied::Invalid,
            },
            ("display", "integer_scale" | "widescreen" | "smooth_wave_effects" | "crt")
            | ("audio", "muted") => match parse_bool(value) {
                Some(enabled) => {
                    if key == "widescreen" {
                        self.widescreen = enabled;
                    } else if key == "smooth_wave_effects" {
                        self.smooth_wave_effects = enabled;
                    } else if key == "crt" {
                        self.crt = enabled;
                    } else if section == "display" {
                        self.integer_scale = enabled;
                    } else {
                        self.muted = enabled;
                    }
                    Applied::Ok
                }
                None => Applied::Invalid,
            },
            ("display", "scanlines" | "crt_curvature") => match value.parse::<u8>() {
                Ok(strength) if strength <= 100 => {
                    if key == "crt_curvature" {
                        self.crt_curvature = strength;
                    } else {
                        self.scanlines = strength;
                    }
                    Applied::Ok
                }
                _ => Applied::Invalid,
            },
            ("input", "controller") => {
                self.controller = value.to_string();
                Applied::Ok
            }
            ("display", "scale") => match value.parse::<u32>() {
                Ok(scale) if (MIN_SCALE..=MAX_SCALE).contains(&scale) => {
                    self.scale = scale;
                    Applied::Ok
                }
                _ => Applied::Invalid,
            },
            ("display", "fullscreen") => match parse_bool(value) {
                Some(fullscreen) => {
                    self.fullscreen = fullscreen;
                    Applied::Ok
                }
                None => Applied::Invalid,
            },
            ("audio", "volume") => match value.parse::<u8>() {
                Ok(volume) if volume <= MAX_VOLUME => {
                    self.volume = volume;
                    Applied::Ok
                }
                _ => Applied::Invalid,
            },
            ("keyboard" | "gamepad", _) => {
                let Some((button, _, _)) = PAD_CONTROLS.iter().find(|(_, name, _)| *name == key)
                else {
                    return Applied::Unknown;
                };
                let bindings = if section == "keyboard" {
                    &mut self.keyboard
                } else {
                    &mut self.gamepad
                };
                bindings.set(*button, parse_list(value));
                Applied::Ok
            }
            _ => Applied::Unknown,
        }
    }

    /// Parse settings text. Invalid values keep their defaults and produce a
    /// warning; unknown keys and sections are kept in `extra`.
    pub fn parse(text: &str) -> (Self, Vec<String>) {
        let mut settings = Self::default();
        let mut warnings = Vec::new();
        let mut section = String::new();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
            {
                section = name.trim().to_ascii_lowercase();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                warnings.push(format!("settings line {}: expected key = value", n + 1));
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = value.trim();
            match settings.apply(&section, &key, value) {
                Applied::Ok => {}
                Applied::Invalid => warnings.push(format!(
                    "settings line {}: invalid {section}.{key} value {value:?}; using default",
                    n + 1
                )),
                Applied::Unknown => settings.extra.push(Entry {
                    section: section.clone(),
                    key,
                    value: value.to_string(),
                }),
            }
        }
        (settings, warnings)
    }

    /// Serialize to the settings file format.
    pub fn to_ini(&self) -> String {
        let mut out = String::from(
            "# Ninja Master native settings. Unknown entries are preserved.\n\
             # Bindings are comma-separated SDL key names ([keyboard]) or SDL\n\
             # controller button names ([gamepad]); \"none\" leaves a button unbound.\n\
             # Pad A = ninjutsu, B = attack, C = jump.\n",
        );
        let extras = |out: &mut String, section: &str| {
            for entry in self.extra.iter().filter(|entry| entry.section == section) {
                out.push_str(&format!("{} = {}\n", entry.key, entry.value));
            }
        };
        extras(&mut out, "");
        out.push_str(&format!(
            "\n[display]\nscale = {}\nfullscreen = {}\nwidescreen = {}\naspect = {}\ninteger_scale = {}\nsmooth_wave_effects = {}\nscanlines = {}\ncrt = {}\ncrt_curvature = {}\n",
            self.scale, self.fullscreen, self.widescreen, self.aspect, self.integer_scale, self.smooth_wave_effects, self.scanlines, self.crt, self.crt_curvature
        ));
        extras(&mut out, "display");
        out.push_str(&format!("\n[art]\npack = {}\n", self.art_pack));
        extras(&mut out, "art");
        out.push_str(&format!(
            "\n[audio]\nvolume = {}\nmuted = {}\n",
            self.volume, self.muted
        ));
        extras(&mut out, "audio");
        out.push_str(&format!("\n[input]\ncontroller = {}\n", self.controller));
        extras(&mut out, "input");
        out.push_str(&format!(
            "\n[game]\nlevel = {}\nshurikens = {}\ncontrol = {}\ninfinite = {}\n",
            self.game.level, self.game.shurikens, self.game.control, self.game.infinite
        ));
        extras(&mut out, "game");
        for (section, bindings) in [("keyboard", &self.keyboard), ("gamepad", &self.gamepad)] {
            out.push_str(&format!("\n[{section}]\n"));
            for (button, key, _) in PAD_CONTROLS {
                out.push_str(&format!("{key} = {}\n", write_list(bindings.get(button))));
            }
            extras(&mut out, section);
        }
        let known = [
            "", "display", "art", "audio", "input", "game", "keyboard", "gamepad",
        ];
        let mut others: Vec<&str> = Vec::new();
        for entry in &self.extra {
            if !known.contains(&entry.section.as_str()) && !others.contains(&entry.section.as_str())
            {
                others.push(&entry.section);
            }
        }
        for section in others {
            out.push_str(&format!("\n[{section}]\n"));
            extras(&mut out, section);
        }
        out
    }

    /// Load from `path`; a missing file yields defaults.
    pub fn load(path: &Path) -> io::Result<(Self, Vec<String>)> {
        match fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok((Self::default(), Vec::new()))
            }
            Err(error) => Err(error),
        }
    }

    /// Write to `path`, creating its directory, via a temporary file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            fs::create_dir_all(dir)?;
        }
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        fs::write(&temporary, self.to_ini())?;
        fs::rename(&temporary, path)
    }
}

/// Settings path: `$NINJA_MASTER_CONFIG`, else `%APPDATA%\ninja-master\settings.ini`
/// on Windows, `~/Library/Application Support/ninja-master/settings.ini` on macOS,
/// or `$XDG_CONFIG_HOME/ninja-master/settings.ini` (default `~/.config`).
pub fn config_path() -> Option<PathBuf> {
    config_path_from(|name| std::env::var_os(name))
}

fn config_path_from(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |name: &str| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    if let Some(path) = var(CONFIG_ENV) {
        return Some(path);
    }
    let dir = if cfg!(windows) {
        var("APPDATA")?
    } else if cfg!(target_os = "macos") {
        var("HOME")?.join("Library").join("Application Support")
    } else {
        var("XDG_CONFIG_HOME")
            .filter(|path| path.is_absolute())
            .or_else(|| var("HOME").map(|home| home.join(".config")))?
    };
    Some(dir.join("ninja-master").join("settings.ini"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Keyboard,
    Gamepad,
}

/// In-game rebinding sequence: prompts each pad control in `PAD_CONTROLS`
/// order and binds the first key or pad button pressed for it.
#[derive(Clone, Debug)]
pub struct Rebinder {
    step: usize,
    control: u8,
    keyboard: Bindings,
    gamepad: Bindings,
}

impl Rebinder {
    pub fn new(settings: &Settings) -> Self {
        Self {
            step: 0,
            control: settings.game.control,
            keyboard: settings.keyboard.clone(),
            gamepad: settings.gamepad.clone(),
        }
    }

    /// `(1-based step, total, label)` for the control awaiting input.
    pub fn prompt(&self) -> Option<(usize, usize, &'static str)> {
        PAD_CONTROLS.get(self.step).map(|(button, _, _)| {
            (
                self.step + 1,
                PAD_CONTROLS.len(),
                control_label(*button, self.control),
            )
        })
    }

    pub fn is_done(&self) -> bool {
        self.step >= PAD_CONTROLS.len()
    }

    /// Bind the named input on `device` to the current control and advance.
    pub fn assign(&mut self, device: Device, name: &str) {
        if let Some((button, _, _)) = PAD_CONTROLS.get(self.step) {
            match device {
                Device::Keyboard => self.keyboard.assign(*button, name),
                Device::Gamepad => self.gamepad.assign(*button, name),
            }
            self.step += 1;
        }
    }

    /// Keep the current control's existing bindings and advance.
    pub fn skip(&mut self) {
        self.step = (self.step + 1).min(PAD_CONTROLS.len());
    }

    pub fn apply(self, settings: &mut Settings) {
        settings.keyboard = self.keyboard;
        settings.gamepad = self.gamepad;
    }
}

/// Names of physical pad inputs and their actions in the selected scheme.
pub fn control_label(button: Button, control: u8) -> &'static str {
    match (control, button) {
        (1, Button::Attack) => "B (jump)",
        (1, Button::Jump) => "C (attack)",
        (2, Button::Ninjutsu) => "A (attack)",
        (2, Button::Jump) => "C (ninjutsu)",
        (2, Button::Attack) => "B (jump)",
        (3, Button::Ninjutsu) => "A (jump)",
        (3, Button::Jump) => "C (ninjutsu)",
        (4, Button::Ninjutsu) => "A (guard)",
        (4 | 5, Button::Attack) => "B (melee)",
        (5, Button::Y) => "Y (guard)",
        (_, Button::Y) => "Y (shuriken)",
        _ => PAD_CONTROLS
            .iter()
            .find(|(b, _, _)| *b == button)
            .map_or("", |(_, _, label)| label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[String]) -> Vec<&str> {
        list.iter().map(String::as_str).collect()
    }

    #[test]
    fn game_choices_and_dedicated_bindings_persist_with_legacy_defaults() {
        assert_eq!(
            Settings::parse("[display]\nscale = 2").0.game,
            crate::options::GameOptions::default()
        );
        for control in 0..=5 {
            let mut settings = Settings {
                game: crate::options::GameOptions {
                    level: 3,
                    shurikens: crate::options::DISABLED_SHURIKENS,
                    infinite: true,
                    control,
                    invincible: false,
                },
                ..Settings::default()
            };
            settings.keyboard.assign(Button::Y, "G");
            settings.gamepad.assign(Button::Y, "leftshoulder");
            assert_eq!(Settings::parse(&settings.to_ini()).0, settings);
        }
        let (settings, warnings) = Settings::parse("[game]\ncontrol = 6\nlevel = 4\nshurikens = 9");
        assert_eq!(warnings.len(), 3);
        assert_eq!(settings.game, crate::options::GameOptions::default());
        assert_eq!(control_label(Button::Ninjutsu, 4), "A (guard)");
        assert_eq!(control_label(Button::Y, 5), "Y (guard)");
    }

    #[test]
    fn defaults_follow_mega_drive_pad_layout() {
        let settings = Settings::default();
        let keys = &settings.keyboard;
        assert_eq!(names(keys.get(Button::Up)), ["Up"]);
        assert_eq!(names(keys.get(Button::Down)), ["Down"]);
        assert_eq!(names(keys.get(Button::Left)), ["Left"]);
        assert_eq!(names(keys.get(Button::Right)), ["Right"]);
        assert_eq!(names(keys.get(Button::Ninjutsu)), ["A", "Z"]);
        assert_eq!(names(keys.get(Button::Attack)), ["S", "X"]);
        assert_eq!(names(keys.get(Button::Jump)), ["D", "C", "Space"]);
        assert_eq!(names(keys.get(Button::Start)), ["Return"]);
        let pad = &settings.gamepad;
        assert_eq!(names(pad.get(Button::Ninjutsu)), ["x"]);
        assert_eq!(names(pad.get(Button::Attack)), ["a"]);
        assert_eq!(names(pad.get(Button::Jump)), ["b"]);
        assert_eq!(names(pad.get(Button::Start)), ["start"]);
        assert_eq!(names(pad.get(Button::Left)), ["dpleft"]);
        // Prompt order walks the pad: D-pad, then A, B, C, Start.
        let order: Vec<&str> = PAD_CONTROLS.iter().map(|(_, key, _)| *key).collect();
        assert_eq!(
            order,
            [
                "up", "down", "left", "right", "a", "b", "c", "start", "x", "y", "z", "mode"
            ]
        );
        assert_eq!(settings.scale, DEFAULT_SCALE);
        assert!(!settings.fullscreen);
        assert_eq!(settings.volume, 100);
    }

    #[test]
    fn default_and_custom_settings_round_trip() {
        let defaults = Settings::default();
        let (parsed, warnings) = Settings::parse(&defaults.to_ini());
        assert!(warnings.is_empty());
        assert_eq!(parsed, defaults);

        let mut custom = Settings {
            art_pack: "/tmp/My artwork/pack.json".into(),
            scale: 5,
            fullscreen: true,
            widescreen: false,
            aspect: 1,
            integer_scale: false,
            smooth_wave_effects: false,
            scanlines: 45,
            crt: true,
            crt_curvature: 35,
            volume: 40,
            muted: true,
            controller: "Test controller".into(),
            ..Settings::default()
        };
        custom
            .keyboard
            .set(Button::Jump, vec!["Left Shift".into(), "K".into()]);
        custom.keyboard.set(Button::Start, Vec::new());
        custom
            .gamepad
            .set(Button::Ninjutsu, vec!["leftshoulder".into()]);
        custom.extra.push(Entry {
            section: "display".into(),
            key: "filter".into(),
            value: "crt".into(),
        });
        custom.extra.push(Entry {
            section: "future".into(),
            key: "stage_select".into(),
            value: "1-2".into(),
        });
        let text = custom.to_ini();
        let (parsed, warnings) = Settings::parse(&text);
        assert!(warnings.is_empty());
        assert_eq!(parsed, custom);
        assert_eq!(parsed.to_ini(), text);
        assert_eq!(parsed.gain(), 0.0);
    }

    #[test]
    fn presentation_settings_reject_invalid_values() {
        let (settings, warnings) = Settings::parse(
            "[display]\naspect = 3\nscanlines = 101\ninteger_scale = perhaps\ncrt = perhaps\ncrt_curvature = 101\n[audio]\nmuted = nope\n",
        );
        assert_eq!(settings, Settings::default());
        assert_eq!(warnings.len(), 6);
    }

    #[test]
    fn widescreen_is_opt_out_in_existing_and_new_settings() {
        let (old, warnings) = Settings::parse("[display]\nscale = 2\n");
        assert!(warnings.is_empty());
        assert!(old.widescreen);
        assert_eq!(old.frame_width(), crate::WIDE_WIDTH);
        let (off, warnings) = Settings::parse("[display]\nwidescreen = off\n");
        assert!(warnings.is_empty());
        assert!(!off.widescreen);
        assert_eq!(off.frame_width(), crate::WIDTH);
        let (invalid, warnings) = Settings::parse("[display]\nwidescreen = perhaps\n");
        assert!(invalid.widescreen);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn smooth_waves_default_on_and_persist_an_opt_out() {
        let (old, warnings) = Settings::parse("[display]\nscale = 2\n");
        assert!(warnings.is_empty());
        assert!(old.smooth_wave_effects);
        let (off, warnings) = Settings::parse("[display]\nsmooth_wave_effects = off\n");
        assert!(warnings.is_empty());
        assert!(!off.smooth_wave_effects);
        assert_eq!(Settings::parse(&off.to_ini()).0, off);
        let (invalid, warnings) = Settings::parse("[display]\nsmooth_wave_effects = perhaps\n");
        assert!(invalid.smooth_wave_effects);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn unknown_entries_are_preserved_not_applied() {
        let text = "orphan = 1\n[display]\nscale = 2\nvsync = on\n\
                    [keyboard]\nturbo = Q\n[mystery]\nanswer = 42\n";
        let (settings, warnings) = Settings::parse(text);
        assert!(warnings.is_empty());
        assert_eq!(settings.scale, 2);
        assert_eq!(settings.keyboard, Bindings::default_keyboard());
        let kept: Vec<(&str, &str, &str)> = settings
            .extra
            .iter()
            .map(|entry| {
                (
                    entry.section.as_str(),
                    entry.key.as_str(),
                    entry.value.as_str(),
                )
            })
            .collect();
        assert_eq!(
            kept,
            [
                ("", "orphan", "1"),
                ("display", "vsync", "on"),
                ("keyboard", "turbo", "Q"),
                ("mystery", "answer", "42"),
            ]
        );
        let written = settings.to_ini();
        assert!(written.contains("vsync = on"));
        assert!(written.contains("[mystery]\nanswer = 42"));
        assert_eq!(Settings::parse(&written).0, settings);
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let text = "[display]\nscale = 0\nfullscreen = maybe\n[audio]\nvolume = 250\n\
                    this line has no separator\n[display]\nscale = huge\n";
        let (settings, warnings) = Settings::parse(text);
        assert_eq!(warnings.len(), 5);
        assert_eq!(settings.scale, DEFAULT_SCALE);
        assert!(!settings.fullscreen);
        assert_eq!(settings.volume, MAX_VOLUME);
        assert!(settings.extra.is_empty());
    }

    #[test]
    fn binding_lists_parse_case_insensitive_keys_and_none() {
        let text = "[KEYBOARD]\nC = d ,  , Space\nSTART = None\n[gamepad]\na = \n";
        let (settings, warnings) = Settings::parse(text);
        assert!(warnings.is_empty());
        assert_eq!(names(settings.keyboard.get(Button::Jump)), ["d", "Space"]);
        assert!(settings.keyboard.get(Button::Start).is_empty());
        assert!(settings.gamepad.get(Button::Ninjutsu).is_empty());
    }

    #[test]
    fn rebinding_sequence_moves_conflicting_names() {
        let mut settings = Settings::default();
        let mut rebinder = Rebinder::new(&settings);
        assert_eq!(rebinder.prompt(), Some((1, 12, "Up")));
        rebinder.assign(Device::Keyboard, "W");
        rebinder.skip(); // Down keeps its default.
        rebinder.assign(Device::Gamepad, "leftshoulder");
        rebinder.skip();
        assert_eq!(rebinder.prompt(), Some((5, 12, "A (ninjutsu)")));
        rebinder.assign(Device::Keyboard, "Space"); // Taken from C (jump).
        rebinder.skip();
        rebinder.skip();
        rebinder.skip();
        assert_eq!(rebinder.prompt(), Some((9, 12, "X (shuriken)")));
        for _ in 0..4 {
            rebinder.skip();
        }
        assert!(rebinder.is_done());
        assert_eq!(rebinder.prompt(), None);
        rebinder.assign(Device::Keyboard, "Q"); // Ignored after completion.
        rebinder.apply(&mut settings);
        assert_eq!(names(settings.keyboard.get(Button::Up)), ["W"]);
        assert_eq!(names(settings.keyboard.get(Button::Down)), ["Down"]);
        assert_eq!(names(settings.gamepad.get(Button::Left)), ["leftshoulder"]);
        assert_eq!(names(settings.keyboard.get(Button::Left)), ["Left"]);
        assert_eq!(names(settings.keyboard.get(Button::Ninjutsu)), ["Space"]);
        assert_eq!(names(settings.keyboard.get(Button::Jump)), ["D", "C"]);
        assert_eq!(Settings::parse(&settings.to_ini()).0, settings);
    }

    #[test]
    fn config_path_prefers_override_then_platform_directory() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(*value))
            }
        };
        assert_eq!(
            config_path_from(env(&[
                (CONFIG_ENV, "/tmp/custom.ini"),
                ("HOME", "/home/joe")
            ])),
            Some(PathBuf::from("/tmp/custom.ini"))
        );
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert_eq!(
                config_path_from(env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/joe")])),
                Some(PathBuf::from("/xdg/ninja-master/settings.ini"))
            );
            assert_eq!(
                config_path_from(env(&[
                    (CONFIG_ENV, ""),
                    ("XDG_CONFIG_HOME", "relative"),
                    ("HOME", "/home/joe"),
                ])),
                Some(PathBuf::from("/home/joe/.config/ninja-master/settings.ini"))
            );
            assert_eq!(config_path_from(env(&[])), None);
        }
    }

    #[test]
    fn save_and_load_round_trip_through_a_file() {
        let dir =
            std::env::temp_dir().join(format!("ninja-master-settings-{}", std::process::id()));
        let path = dir.join("nested").join("settings.ini");
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(Settings::load(&path).unwrap().0, Settings::default());
        let settings = Settings {
            scale: 2,
            volume: 0,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let (loaded, warnings) = Settings::load(&path).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(loaded, settings);
        let _ = fs::remove_dir_all(&dir);
    }
}
