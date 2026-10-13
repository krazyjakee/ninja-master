//! 7-2's scripted door passages (`$397B0`, `$398A8`, `$399AE`).
//! Layout writes update both the picture and the collision, as `$15054` does.
use super::{Game, REQUEST_NEXT_SUBSTAGE, REQUEST_PART_LOAD};
use crate::{
    combat::{Effect, EffectKind, Motion},
    movement::PlayerEvent,
    rom::{Result, Sine, signed, word},
    stage::{Section, StageId, TerrainEdit},
};

#[derive(Clone, Debug)]
pub(super) struct Door {
    pub(super) exit: bool,
    step: u8,
    timer: i32,
    at: (i32, i32),
    request: u8,
}
impl Door {
    pub(super) fn entrance(rom: &[u8], id: StageId, section: &Section) -> Result<Option<Self>> {
        if section.mode_flags & 0x800 == 0 {
            return Ok(None);
        }
        let mut door = Self::new(rom, false, if id.part == 3 { 2 } else { 0 })?;
        // Mode_Game's fade-in runs the player before the entrance handler.
        door.step = 0;
        door.timer = 16;
        Ok(Some(door))
    }
    fn new(rom: &[u8], exit: bool, index: usize) -> Result<Self> {
        let p = 0x3999e + index * 4;
        Ok(Self {
            exit,
            step: 1,
            timer: 0,
            at: (signed(rom, p)?, signed(rom, p + 2)?),
            request: 0,
        })
    }
    pub(super) fn event(&self) -> PlayerEvent {
        if self.step == 0 {
            PlayerEvent::Freeze
        } else if self.exit {
            PlayerEvent::Idle
        } else {
            PlayerEvent::WalkRight
        }
    }
}
impl Game {
    pub(super) fn begin_door_exit(&mut self) {
        let mut door = Door::new(&self.rom, true, if self.part() == 3 { 3 } else { 1 })
            .expect("validated 7-2 door anchors");
        if self.part() + 1 < self.sections.len() {
            self.part_request = Some(self.part() + 1);
            door.request = REQUEST_PART_LOAD;
        } else {
            door.request = REQUEST_NEXT_SUBSTAGE;
        }
        self.door = Some(door);
    }
    pub(super) fn advance_door(&mut self) {
        let Some(mut door) = self.door.take() else {
            return;
        };
        match door.step {
            0 => {
                if door.timer == 16 {
                    // Part 3 starts inside its closed door. The cartridge's
                    // wall correction eases Joe left to anchor - 32. Resolve
                    // that overlap once here so native floor probes cannot
                    // lift him through the doorway during the fade-in.
                    self.player.x = self
                        .player
                        .x
                        .min((door.at.0 - 32) * crate::movement::NATIVE_UNIT);
                }
                door.timer -= 1;
                if door.timer == 0 {
                    door.step = 1;
                }
            }
            1 => {
                door.timer = if door.exit { 60 } else { 30 };
                door.step = 2;
                if !door.exit {
                    door.timer -= 1;
                }
            }
            2 => {
                door.timer -= 1;
                if door.timer < 0 {
                    let table = if door.exit { 0x3971e } else { 0x3969c };
                    for (metatile, dx, dy) in crate::enemies::metatile_list(&self.rom, table)
                        .expect("validated door metatiles")
                    {
                        self.stage.assets.data.apply_edit(TerrainEdit {
                            x: door.at.0 + dx,
                            y: door.at.1 + dy,
                            metatile,
                        });
                    }
                    self.door_fragments(door.at, door.exit);
                    door.step = 3;
                }
            }
            3 => {
                let mut piece = Effect::new(EffectKind::Pieces, door.at.0, door.at.1, 0x3ae);
                piece.base = if door.exit { 0xe800 } else { 0xe000 };
                piece.high_priority = true;
                piece.motion = Motion {
                    vx: if door.exit { -0x28000 } else { 0x28000 },
                    vy: -0x18000,
                    gravity: 0x1800,
                    ..Motion::default()
                };
                piece.counter = 60;
                self.combat.effects.push(piece);
                let sound = crate::sound_events::Table::Effect
                    .command(&self.rom, 0x20)
                    .expect("validated door explosion sound");
                self.extend_cues([sound]);
                door.timer = if door.exit { 90 } else { 30 };
                door.timer -= 1;
                door.step = 4;
            }
            _ => {
                door.timer -= 1;
                if door.timer < 0 {
                    self.requests |= door.request;
                    return;
                }
            }
        }
        self.door = Some(door);
    }
    fn door_fragments(&mut self, at: (i32, i32), exit: bool) {
        let p = if exit { 0x3975c } else { 0x396da };
        let mapping = word(&self.rom, p).expect("validated door mapping");
        let sine = Sine::load(&self.rom).expect("validated sine table");
        // Each original fragment alternates two trajectories; native sprites
        // retain the eight pieces and their ROM positions/velocities.
        for i in 0..8 {
            let p = p + 4 + i * 8;
            let read = |offset| signed(&self.rom, p + offset).unwrap();
            let (cos, sin) = sine.polar(read(4) as u16);
            let shift = read(6) as u32 + 1;
            let mut piece =
                Effect::new(EffectKind::Pieces, at.0 + read(0), at.1 + read(2), mapping);
            piece.base = 0xe000;
            piece.high_priority = true;
            piece.motion = Motion {
                vx: (if at.0 < self.player.position().0 {
                    -cos
                } else {
                    cos
                }) << shift,
                vy: sin << shift,
                gravity: i32::from(self.combat.random_word() & 0xfff) + 0x3000,
                ..Motion::default()
            };
            piece.counter = 120;
            self.combat.effects.push(piece);
        }
    }
}
