use super::*;

/// `Mode_Game` (`$AEDA`) in the mode layer.
impl Mode for Game {
    /// State `$1C` runs `Game_RunFrame` until `Game_Requests` asks for
    /// more (`$B010`). Bit 1, the next substage, goes to the transition
    /// sequence. Bit 0, a lost life, enters state `$20`: its fade out
    /// (`$B04E`) takes its first step in the same frame and one per frame
    /// after, then `$B096` takes the life. State 8 reloads the part, or with
    /// no life left `Mode_GameOver` (`$B1BC`) follows with the game
    /// suspended in it. Bit 3, a part load (a switch or a reload exit),
    /// runs the same fade with the world held, then loads the part
    /// (`$B118`/`$B0AA`, `Game::load_part`) and fades it in during play.
    ///
    /// `Game_CheckPause` (`$B7B2`) ends each frame: Start installs
    /// `Mode_Pause`, except during a ninjutsu (mode `$22A96`) or a death
    /// (`Game_Flags` bit 1). The pad goes through the options' control type
    /// first (`Player_ReadInput`, `$3AC6`).
    fn tick(&mut self, input: Input) -> Option<ModeChange> {
        if self.requests & REQUEST_NEXT_SUBSTAGE != 0 && !self.complete {
            return Some(ModeChange::Suspend(StageClear::suspend));
        }
        if let Some(fade) = self.lost_fade {
            if !fade.finished() {
                self.lost_fade = Some(fade.advance());
                return None;
            }
            self.lost_fade = None;
            return self
                .lose_life()
                .then_some(ModeChange::Suspend(crate::game_over::GameOver::suspend));
        }
        if let Some(fade) = self.part_fade {
            if !fade.finished() {
                self.part_fade = Some(fade.advance());
                return None;
            }
            self.part_fade = None;
            self.enter_requested_part();
            return None;
        }
        if self.combat.game_over {
            return Some(ModeChange::Suspend(crate::game_over::GameOver::suspend));
        }
        if self.requests & REQUEST_LIFE_LOST == 0 {
            let input = crate::options::remap(&self.rom, self.options.control, input);
            // No Game_CheckPause in a lag frame, nor in Ninjutsu_Finish's
            // update (mode $22A96), which sets the lag that follows it.
            let lagging = self.lag > 0;
            Game::tick(self, input);
            if self.options.infinite && !self.options.shurikens_disabled() {
                self.combat.hud.ammo = crate::options::INFINITE_AMMO;
            }
            if self.requests & (REQUEST_LIFE_LOST | REQUEST_PART_LOAD) == REQUEST_PART_LOAD {
                // State $20's fade out takes its first step in this frame.
                self.part_fade = Some(Fade::Out(1));
                return None;
            }
            if self.requests & REQUEST_LIFE_LOST == 0 {
                return (input.pressed(Button::Start)
                    && self.cast.is_none()
                    && self.death.is_none()
                    && self.door.is_none()
                    && !lagging
                    && self.lag == 0)
                    .then_some(ModeChange::Suspend(Pause::suspend));
            }
        }
        self.lost_fade = Some(Fade::Out(1));
        None
    }
    fn draw(&self, frame: &mut Frame) {
        Game::draw(self, frame);
    }
    fn name(&self) -> &'static str {
        "Game"
    }
    fn take_audio(&mut self) -> Audio {
        // A music change goes first: starting music stops effects, so the
        // effects requested alongside it still play.
        let mut commands: Vec<u8> = self.take_music().into_iter().collect();
        commands.append(&mut self.cues);
        Audio { commands }
    }
    fn game(&self) -> Option<&Game> {
        Some(self)
    }
    fn game_mut(&mut self) -> Option<&mut Game> {
        Some(self)
    }
}
