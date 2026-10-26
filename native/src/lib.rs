//! Small platform-independent foundation for reconstructing the game.
//! Play mode uses recovered ROM art, ordinary movement, combat and sound,
//! run through the `$FF002A` mode layer (`modes`): gameplay, pause and the
//! round transition screens.
//! The separate demo remains original test content.
#![forbid(unsafe_code)]
// The crate is not published: its docs are read with
// `--document-private-items`, where links to private items resolve.
#![allow(rustdoc::private_intra_doc_links)]

pub mod art_pack;
pub mod assets;
pub mod attract;
pub mod audio_output;
pub mod audio_pack;
pub mod bosses;
pub mod combat;
pub mod content;
pub mod death;
pub mod demo;
pub mod devtools;
pub mod ending;
pub mod enemies;
pub mod front_end;
pub mod front_vdp;
pub mod game;
pub mod game_over;
pub mod hud;
pub mod input;
pub mod intro;
pub mod menu;
pub mod modes;
pub mod movement;
pub mod ninjutsu;
pub mod ninjutsu_fx;
pub mod options;
pub mod pack;
pub mod pause;
pub mod practice;
pub mod profile;
mod progress;
mod raster;
pub mod render;
pub mod rom;
pub mod round_screens;
pub mod sega_logo;
pub mod settings;
pub mod sound;
pub mod sound_events;
pub mod special;
pub mod stage;
pub mod stage_objects;
pub mod story;

pub mod text;
pub mod timing;
pub mod title_art;
pub mod vdp;
pub mod viewer;

pub const WIDTH: usize = 320;
/// Nearest even pixel width to 16:9 at the cartridge's 224-line height.
pub const WIDE_WIDTH: usize = 398;
pub const HEIGHT: usize = 224;
/// Provisional demo cadence. Original NTSC timing still needs verification.
pub const TICK_HZ: u32 = 60;
