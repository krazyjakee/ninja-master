use super::*;
use crate::practice::{Attempt, GhostFrame, Kind, MAX_GHOST_FRAMES, Record, Session};

impl Game {
    pub(crate) fn start_practice(
        &mut self,
        front: crate::front_end::Front,
        kind: Kind,
        ghost: Option<Record>,
    ) {
        front.prepare(self);
        self.start_session(front.options);
        self.run.practice = true;
        self.practice = Some(Session {
            front,
            kind,
            stage: self.stage_id.with_part(0),
            ticks: 0,
            valid: !self.options.invincible && !self.options.infinite,
            frames: Vec::new(),
            ghost: ghost.filter(Record::valid).map(std::rc::Rc::new),
        });
        self.clear_explore_enemies();
        let pose = self.ghost_frame();
        if kind == Kind::TimeTrial {
            self.practice.as_mut().unwrap().frames.push(pose);
        }
    }
    fn ghost_frame(&self) -> GhostFrame {
        let (x, y) = self.player.position();
        GhostFrame(
            self.stage_id.part,
            x,
            y,
            self.player.display_frame() as u16,
            self.player.facing_left,
            self.special
                .as_ref()
                .and_then(|s| s.horse.map(|h| h.frame as u16)),
        )
    }
    pub(super) fn clear_explore_enemies(&mut self) {
        if self.exploring() {
            // The mounted pickup scheduler is an invisible stage controller.
            self.combat
                .enemies
                .retain(|enemy| self.combat.spawns[enemy.spawn].kind == 0x40);
            self.combat.projectiles.retain(|shot| !shot.enemy);
        }
    }
    pub(super) fn exploring(&self) -> bool {
        self.practice
            .as_ref()
            .is_some_and(|s| s.kind == Kind::Explore)
    }
    pub(super) fn record_practice_frame(&mut self) {
        if self.practice.is_none() {
            return;
        }
        self.clear_explore_enemies();
        if self.exploring() && self.death.is_none() {
            let section = &self.sections[self.part()];
            // Mounted stages normally end through their guard's death signal.
            let mounted_exit =
                self.special.is_some() && self.player.position().0 >= section.exit.0 - 24;
            let boss_exit = ModeFlags(section.mode_flags).boss_arena()
                && self.player.position().0 >= self.player.camera_x + crate::WIDTH as i32 - 40;
            if mounted_exit || boss_exit {
                self.requests |= REQUEST_NEXT_SUBSTAGE;
            }
        }
        let pose = self.ghost_frame();
        if let Some(session) = &mut self.practice {
            session.ticks += 1;
            session.valid &= self.stage_id.with_part(0) == session.stage;
            if session.kind == Kind::TimeTrial {
                if session.frames.len() < MAX_GHOST_FRAMES {
                    session.frames.push(pose);
                } else {
                    session.valid = false;
                }
            }
        }
    }
    pub(crate) fn finish_practice(&mut self, completed: bool) -> Option<ModeChange> {
        let session = self.practice.take()?;
        Some(ModeChange::Practice(
            session.front,
            Some(session.kind),
            Some(session.stage),
            Some(Attempt {
                completed,
                valid: session.valid,
                record: Record {
                    ticks: session.ticks,
                    frames: session.frames,
                },
            }),
        ))
    }
    pub(super) fn draw_ghost(&self, frame: &mut Frame, camera: (i32, i32)) {
        let Some(session) = &self.practice else {
            return;
        };
        let Some(ghost) = &session.ghost else { return };
        let Some(pose) = ghost.frames.get(session.ticks as usize) else {
            return;
        };
        if pose.0 != self.stage_id.part {
            return;
        }
        let palette = std::array::from_fn(|i| {
            if i == 0 {
                [0; 3]
            } else {
                [32 + i as u8 * 3, 120 + i as u8 * 6, 160 + i as u8 * 6]
            }
        });
        let lines = [crate::render::PaletteRef::Fixed(&palette); 4];
        if let Some(horse) = pose.5.and_then(|id| self.sprites.frames.get(id as usize)) {
            horse.draw_ghost(frame, &lines, pose.1 - camera.0, pose.2 - camera.1, false);
        }
        if let Some(sprite) = self.sprites.frames.get(pose.3 as usize) {
            sprite.draw(frame, &lines, pose.1 - camera.0, pose.2 - camera.1, pose.4);
        }
    }
    pub(super) fn draw_practice_clock(&self, frame: &mut Frame) {
        if let Some(session) = &self.practice {
            let label = if session.kind == Kind::Explore {
                "EXPLORE".into()
            } else {
                format!("TIME {}", crate::practice::time(session.ticks))
            };
            frame.rect(16, 38, 288, 12, [8, 12, 24]);
            self.font.draw_text(
                frame,
                24,
                40,
                &label,
                crate::text::Style::plain([120, 224, 255], 1),
            );
            if session.kind == Kind::TimeTrial {
                let best = session.ghost.as_ref().map_or("NO GHOST".into(), |g| {
                    format!("BEST {}", crate::practice::time(g.ticks))
                });
                self.font.draw_text(
                    frame,
                    288 - best.len() as i32 * 8,
                    40,
                    &best,
                    crate::text::Style::plain([220, 224, 232], 1),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rom(region: &str) -> Vec<u8> {
        let directory = std::env::var_os("NINJA_MASTER_TEST_ROMS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../rom"));
        crate::rom::regions::prepare(
            std::fs::read(directory.join(format!("shinobi3_{region}.bin"))).unwrap(),
        )
        .unwrap()
    }
    #[test]
    #[ignore = "requires the three user-supplied cartridges"]
    fn explore_removes_all_enemies_in_every_part_including_reloads_and_bosses() {
        for region in ["us", "eu", "jp"] {
            let rom = rom(region);
            let front = crate::front_end::Front::load(&rom).unwrap();
            for id in crate::stage::all_substages(&rom).unwrap() {
                let mut game = Game::load_stage(&rom, id).unwrap();
                game.start_practice(front.clone(), Kind::Explore, None);
                for part in 0..game.sections.len() {
                    game.load_part(part);
                    Mode::tick(&mut game, Input::default());
                    assert!(
                        game.combat
                            .enemies
                            .iter()
                            .all(|e| game.combat.spawns[e.spawn].kind == 0x40),
                        "{region} {id} part {part}"
                    );
                    assert!(game.combat.projectiles.iter().all(|shot| !shot.enemy));
                    game.reload_part();
                    Mode::tick(&mut game, Input::default());
                    assert!(
                        game.combat
                            .enemies
                            .iter()
                            .all(|e| game.combat.spawns[e.spawn].kind == 0x40)
                    );
                    game.draw(&mut Frame::default());
                }
            }
        }
    }
    #[test]
    #[ignore = "requires a user-supplied cartridge"]
    fn actual_stage_clear_unlocks_only_that_stage_and_practice_cannot_unlock_more() {
        let rom = rom("us");
        let mut profile = crate::profile::Profile::default();
        let mut game = Game::load(&rom).unwrap();
        profile.begin(&mut game.run);
        game.requests = REQUEST_NEXT_SUBSTAGE;
        game.track_end();
        profile.observe(&game.run);
        assert!(crate::practice::unlocked(&profile.data, StageId::FOREST));
        assert!(!crate::practice::unlocked(
            &profile.data,
            StageId::new(1, 2, 0)
        ));
        let mut next = game.load_next(StageId::new(1, 2, 0)).unwrap();
        next.run.practice = true;
        next.requests = REQUEST_NEXT_SUBSTAGE;
        next.track_end();
        profile.observe(&next.run);
        assert!(!crate::practice::unlocked(&profile.data, next.stage_id));
    }
    #[test]
    #[ignore = "requires a user-supplied cartridge"]
    fn trial_records_parts_and_lag_but_pause_cannot_advance_the_clock_or_ghost() {
        use crate::input::press;
        let rom = rom("us");
        let mut game = Game::load(&rom).unwrap();
        game.start_practice(
            crate::front_end::Front::load(&rom).unwrap(),
            Kind::TimeTrial,
            None,
        );
        let mut modes = crate::modes::Modes::new(Box::new(game));
        for _ in 0..10 {
            modes.tick(Input::default());
        }
        modes.game_mut().unwrap().lag = 2;
        for _ in 0..2 {
            modes.tick(Input::default());
        }
        assert_eq!(modes.game().unwrap().practice.as_ref().unwrap().ticks, 12);
        modes.tick(press(Button::Start));
        assert_eq!(modes.name(), "Pause");
        let before = modes.game().unwrap().practice.as_ref().unwrap().ticks;
        for _ in 0..100 {
            modes.tick(Input::default());
        }
        assert_eq!(
            modes.game().unwrap().practice.as_ref().unwrap().ticks,
            before
        );
        modes.menu_event(crate::menu::MenuEvent::Back);
        for _ in 0..10 {
            modes.tick(Input::default());
        }
        assert_eq!(modes.name(), "Game");
        assert_eq!(
            modes.game().unwrap().practice.as_ref().unwrap().ticks,
            before
        );
        let game = modes.game_mut().unwrap();
        game.load_part(1);
        modes.tick(Input::default());
        let session = modes.game().unwrap().practice.as_ref().unwrap();
        assert_eq!(session.frames.len(), session.ticks as usize + 1);
        assert_eq!(session.frames.last().unwrap().0, 1);
        modes.game_mut().unwrap().request_next_substage();
        modes.tick(Input::default());
        assert_eq!(modes.name(), "Practice");
        assert!(modes.profile.data.time_trials["1-1"].valid());
        assert!(modes.profile.data.completed_stages.is_empty());
    }
    #[test]
    #[ignore = "requires a user-supplied cartridge"]
    fn empty_boss_arenas_and_end_encounters_can_complete_explore() {
        let rom = rom("us");
        let front = crate::front_end::Front::load(&rom).unwrap();
        for round in 1..=7 {
            let mut game = Game::load_stage(&rom, StageId::new(round, 3, 0)).unwrap();
            game.start_practice(front.clone(), Kind::Explore, None);
            game.player.x = (game.player.camera_x + crate::WIDTH as i32 - 36) * 256;
            game.record_practice_frame();
            assert_ne!(game.requests & REQUEST_NEXT_SUBSTAGE, 0, "round {round}");
            assert!(matches!(
                Mode::tick(&mut game, Input::default()),
                Some(ModeChange::Practice(
                    _,
                    _,
                    _,
                    Some(Attempt {
                        completed: true,
                        ..
                    })
                ))
            ));
        }
        for round in [2, 4] {
            let mut game = Game::load_stage(&rom, StageId::new(round, 1, 0)).unwrap();
            game.start_practice(front.clone(), Kind::Explore, None);
            let mut completed = false;
            for _ in 0..20000 {
                if let Some(ModeChange::Practice(_, _, _, Some(attempt))) =
                    Mode::tick(&mut game, Input::default())
                {
                    completed = attempt.completed;
                    break;
                }
            }
            assert!(
                completed,
                "mounted explore {round}-1 must finish without its guard"
            );
        }
        let mut game = Game::load_stage(&rom, StageId::new(1, 1, 2)).unwrap();
        game.start_practice(front, Kind::Explore, None);
        let section = &game.sections[2];
        game.arena = Some(ArenaLock {
            camera_x: section.exit.0 + 4,
            camera_y: section.exit_camera_y,
            settled: false,
        });
        game.advance_arena(None);
        assert_ne!(game.requests & REQUEST_NEXT_SUBSTAGE, 0);
    }
}
