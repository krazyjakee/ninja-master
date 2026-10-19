//! The main-loop mode layer (#48).
//!
//! The cartridge's main loop (`$508`) calls the handler in `Main_Handler`
//! (`$FF002A`) once per frame; each handler steps its own state word
//! (`Mode_Routine`, `Mode_SubRoutine`, ...). Temporary modes save the
//! interrupted handler in `Main_HandlerSaved` (`$FF002E`) and restore it
//! value, `Modes` holds the current one, and a temporary mode owns the mode
//! it suspended, handing it back through `ModeChange::Replace`. Gameplay
//! (`Game`) is one mode; pause, the round screens, game over, the ending
//! and the front end (`crate::front_end`) are others.

use crate::{
    game::Game,
    input::{Button, Input},
    render::Frame,
    sound_events::{Route, route},
};

/// Sound commands a mode requested, in request order: the main loop feeds
/// them to `SoundPlayer::play_command` as game code appends them to
/// `Sound_Queue`. `music`, `effects` and `driver` view them as
/// `SndDrv_ProcessCommand` groups them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Audio {
    pub commands: Vec<u8>,
}
impl Audio {
    pub fn push(&mut self, command: u8) {
        self.commands.push(command);
    }
    /// `push` a command a table lookup found.
    pub fn request(&mut self, command: Option<u8>) {
        if let Some(command) = command {
            self.push(command);
        }
    }
    /// Append requests made after this one's.
    pub fn extend(&mut self, later: Audio) {
        self.commands.extend(later.commands);
    }
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
    /// The last music command (`$81..$9C`) requested.
    pub fn music(&self) -> Option<u8> {
        self.routed(Route::Music).last()
    }
    /// Effect commands (`$9D..$D8`).
    pub fn effects(&self) -> Vec<u8> {
        self.routed(Route::Effect).collect()
    }
    /// Voice and control commands (`$D9..$FF`).
    pub fn driver(&self) -> Vec<u8> {
        self.routed(Route::Driver).collect()
    }
    fn routed(&self, group: Route) -> impl DoubleEndedIterator<Item = u8> + '_ {
        self.commands
            .iter()
            .copied()
            .filter(move |&command| route(command) == group)
    }
}

/// What a mode asks the main loop to do after its update.
pub enum ModeChange {
    /// Install `mode` (`move.l #Mode_*,(Main_Handler).l`); a temporary mode
    /// returns the mode it suspended this way (the `Main_HandlerSaved` copy).
    Replace(Box<dyn Mode>),
    /// Save the current mode (`Main_HandlerSaved`) inside the temporary mode
    /// the function builds, as `Game_CheckPause` (`$B7F4`) and `Mode_Game`
    /// state `$20` (`$B172`) do.
    Suspend(fn(Box<dyn Mode>) -> Box<dyn Mode>),
    /// Leave the program: the title menu's native QUIT item.
    Quit,
    NewGame(crate::front_end::Front),
    Continue(crate::front_end::Front),
    Profile(crate::front_end::Front, crate::profile::ScreenKind),
    Practice(
        crate::front_end::Front,
        Option<crate::practice::Kind>,
        Option<crate::stage::StageId>,
        Option<crate::practice::Attempt>,
    ),
    StartPractice(
        crate::front_end::Front,
        crate::practice::Kind,
        crate::stage::StageId,
    ),
}

/// One `$FF002A` handler.
pub trait Mode {
    fn tick(&mut self, input: Input) -> Option<ModeChange>;
    fn draw(&self, frame: &mut Frame);
    fn name(&self) -> &'static str;
    fn menu_event(&mut self, _: crate::menu::MenuEvent) {}
    fn menu_context(&mut self, _: bool, _: &str) {}
    fn display_context(&mut self, _: crate::settings::DisplaySettings, _: Option<&str>) {}
    fn take_display_settings(&mut self) -> Option<crate::settings::DisplaySettings> {
        None
    }
    fn checkpoint_requested(&self) -> bool {
        false
    }
    fn options(&self) -> Option<crate::options::GameOptions> {
        self.game().map(|game| game.options)
    }
    fn set_options(&mut self, options: crate::options::GameOptions) {
        if let Some(game) = self.game_mut() {
            game.options = options;
        }
    }
    /// Sound requests since the last call, including a suspended mode's.
    fn take_audio(&mut self) -> Audio {
        Audio::default()
    }
    /// The gameplay this mode runs or holds suspended.
    fn game(&self) -> Option<&Game> {
        None
    }
    fn game_mut(&mut self) -> Option<&mut Game> {
        None
    }
    /// Native: the music level the sound player should use while this mode
    /// runs (1.0 = as the driver plays it).
    fn music_gain(&self) -> f64 {
        1.0
    }
    /// The sound driver's `$FFF803` communication byte (music sequence
    /// command `$E2`). Gameplay reads it (`Game::sound_communication`);
    /// the intro waits for the SHINOBI music's markers.
    fn sound_communication(&mut self, value: u8) {
        if let Some(game) = self.game_mut() {
            game.sound_communication(value);
        }
    }
}

/// Stand-in while `Modes` moves the current mode into a temporary one.
struct Idle;
impl Mode for Idle {
    fn tick(&mut self, _: Input) -> Option<ModeChange> {
        None
    }
    fn draw(&self, _: &mut Frame) {}
    fn name(&self) -> &'static str {
        "Idle"
    }
}

/// `Main_Handler` and the main loop's call into it.
pub struct Modes {
    current: Box<dyn Mode>,
    /// Requests of modes that were replaced before the caller took them.
    audio: Audio,
    quit: bool,
    progress: crate::progress::Progress,
    pub(crate) history: Option<crate::devtools::replay::History>,
    pub profile: crate::profile::Profile,
    profile_font: Option<crate::text::Font>,
}
impl Modes {
    pub fn new(mode: Box<dyn Mode>) -> Self {
        Self {
            current: mode,
            audio: Audio::default(),
            quit: false,
            progress: crate::progress::Progress::default(),
            history: None,
            profile: crate::profile::Profile::default(),
            profile_font: None,
        }
    }
    /// One main-loop frame (`$508`: `JSR (A6)` with A6 = `Main_Handler`).
    pub fn tick(&mut self, mut input: Input) {
        if self.quit {
            return;
        }
        if matches!(self.current.name(), "Profile" | "Options" | "Practice") {
            self.profile.tick_without_notices();
        } else {
            self.profile.tick();
        }
        if let Some(game) = self.current.game() {
            self.profile.observe(&game.run);
        }
        self.record(crate::devtools::replay::Event::Tick(input));
        if input.pressed(Button::Mode)
            && matches!(
                self.current.name(),
                "Title" | "Options" | "Pause" | "GameOver" | "Profile" | "Practice"
            )
        {
            self.current.menu_event(crate::menu::MenuEvent::Back);
            input.pressed &= !Button::Mode.mask();
            input.held &= !Button::Mode.mask();
        }
        let status = if self.current.name() == "Profile" {
            &self.profile.status
        } else {
            &self.progress.status
        };
        self.current.menu_context(self.progress.available(), status);
        let change = self.current.tick(input);
        if let Some(game) = self.current.game() {
            self.profile.observe(&game.run);
        }
        // Save after installing modes that still own gameplay. A transition
        // to the title has already consumed the game in a replay, so its
        // final tick cannot be used as a resumable history.
        let checkpoint = match &change {
            Some(ModeChange::Replace(mode)) => mode.game().is_some(),
            Some(ModeChange::Suspend(_)) => true,
            None => self.current.checkpoint_requested(),
            _ => false,
        };
        match change {
            None => {}
            Some(ModeChange::Replace(mode)) => {
                let mut old = std::mem::replace(&mut self.current, mode);
                self.audio.extend(old.take_audio());
                if self.current.name() == "Ending"
                    && let Err(error) = self.progress.clear()
                {
                    self.progress.status = error;
                }
            }
            Some(ModeChange::Suspend(build)) => {
                let suspended = std::mem::replace(&mut self.current, Box::new(Idle));
                self.current = build(suspended);
            }
            Some(ModeChange::Profile(front, kind)) => {
                self.profile.flush();
                self.replace(Box::new(crate::profile::ProfileScreen::new(
                    front,
                    kind,
                    self.profile.data.clone(),
                    self.profile.status.clone(),
                )));
            }
            Some(ModeChange::Practice(front, kind, stage, attempt)) => {
                self.history = None;
                let mut message = String::new();
                if let Some(attempt) = attempt {
                    let ticks = attempt.record.ticks;
                    message = if !attempt.completed {
                        "ATTEMPT ENDED. SELECT A STAGE".into()
                    } else if kind == Some(crate::practice::Kind::Explore) {
                        "STAGE COMPLETE".into()
                    } else if !attempt.valid {
                        "MODIFIED RUN - TIME NOT SAVED".into()
                    } else if stage.is_some_and(|id| self.profile.record_trial(id, attempt)) {
                        format!("NEW BEST! {}", crate::practice::time(ticks))
                    } else {
                        format!("FINISHED IN {}", crate::practice::time(ticks))
                    };
                }
                self.profile.flush();
                if !self.profile.status.is_empty() {
                    message = self.profile.status.clone();
                }
                self.replace(Box::new(crate::practice::PracticeScreen::new(
                    front,
                    self.profile.data.clone(),
                    kind,
                    stage,
                    message,
                )));
            }
            Some(ModeChange::StartPractice(front, kind, stage)) => {
                // Recheck the unlock at the boundary; menu navigation cannot bypass it.
                let allowed = stage.part == 0
                    && crate::stage::all_substages(&front.rom)
                        .is_ok_and(|stages| stages.contains(&stage))
                    && crate::practice::unlocked(&self.profile.data, stage);
                if !allowed {
                    self.replace(Box::new(crate::practice::PracticeScreen::new(
                        front,
                        self.profile.data.clone(),
                        Some(kind),
                        Some(stage),
                        "COMPLETE THIS STAGE IN NORMAL GAME".into(),
                    )));
                } else {
                    match Game::load_stage(&front.rom, stage) {
                        Ok(mut game) => {
                            let ghost = if kind == crate::practice::Kind::TimeTrial {
                                self.profile
                                    .data
                                    .time_trials
                                    .get(&crate::practice::stage_key(stage))
                                    .cloned()
                            } else {
                                None
                            };
                            game.start_practice(front, kind, ghost);
                            self.profile.begin(&mut game.run);
                            self.history = None;
                            self.replace(Box::new(game));
                        }
                        Err(error) => self.replace(Box::new(crate::practice::PracticeScreen::new(
                            front,
                            self.profile.data.clone(),
                            Some(kind),
                            Some(stage),
                            error.to_ascii_uppercase(),
                        ))),
                    }
                }
            }
            Some(ModeChange::Quit) => {
                self.profile.flush();
                self.quit = true;
            }
            Some(ModeChange::NewGame(front)) => match front.new_game() {
                Ok(game) => {
                    if let Err(error) = self.progress.clear() {
                        self.progress.status = format!("Cannot replace saved run: {error}");
                        self.replace(Box::new(crate::front_end::Title::menu(front)));
                        return;
                    }
                    self.start_run(&front, game, false);
                }
                Err(error) => {
                    self.progress.status = error;
                    self.replace(Box::new(crate::front_end::Title::menu(front)));
                }
            },
            Some(ModeChange::Continue(front)) => match self.progress.restore(&front.rom) {
                Ok((mut game, mut history)) => {
                    if game.run.id == 0 {
                        game.run.practice = true;
                        self.profile.begin(&mut game.run);
                        self.profile
                            .data
                            .credited
                            .insert(game.run.id, game.run.stats.clone());
                        history
                            .events
                            .push(crate::devtools::replay::Event::RunIdentity(game.run.id));
                    }
                    game.run.practice |= self.profile.data.tainted.contains(&game.run.id);
                    self.history = Some(history);
                    if game.run.practice {
                        self.record(crate::devtools::replay::Event::Practice);
                    }
                    game.restart_music();
                    self.replace(game);
                    self.record(crate::devtools::replay::Event::Resume);
                }
                Err(error) => {
                    // Histories replay only on the build that recorded them.
                    // After a gameplay change, restart the saved substage as
                    // a new run that keeps the old run's eligibility.
                    let restart = self.progress.saved_run().and_then(|(stage, id, practice)| {
                        let mut game = Game::load_stage(&front.rom, stage.with_part(0)).ok()?;
                        front.prepare(&mut game);
                        game.start_session(front.options);
                        let practice = practice
                            || id == 0
                            || self.profile.data.tainted.contains(&id)
                            || game.run.practice;
                        Some((game, practice))
                    });
                    if let Some((game, practice)) = restart {
                        self.start_run(&front, game, practice);
                    } else {
                        self.progress.invalidate();
                        self.progress.status = format!("Continue unavailable: {error}");
                        self.replace(Box::new(crate::front_end::Title::menu(front)));
                    }
                }
            },
        }
        if checkpoint {
            self.checkpoint();
        }
    }
    pub fn menu_event(&mut self, event: crate::menu::MenuEvent) {
        self.record(crate::devtools::replay::Event::Menu(event));
        self.current.menu_event(event);
    }
    pub fn options(&self) -> Option<crate::options::GameOptions> {
        self.current.options()
    }
    pub fn display_context(
        &mut self,
        settings: crate::settings::DisplaySettings,
        error: Option<&str>,
    ) {
        self.current.display_context(settings, error);
    }
    pub fn take_display_settings(&mut self) -> Option<crate::settings::DisplaySettings> {
        self.current.take_display_settings()
    }
    pub fn set_options(&mut self, options: crate::options::GameOptions) {
        if self.game().is_some() {
            self.record(crate::devtools::replay::Event::Options(options));
        }
        if self.game().is_some_and(|g| {
            g.options.level != options.level
                || g.options.shurikens != options.shurikens
                || g.options.infinite != options.infinite
                || g.options.invincible != options.invincible
        }) {
            self.mark_practice();
        }
        self.current.set_options(options);
    }
    pub fn enable_progress(&mut self, path: Option<std::path::PathBuf>, rom: &[u8]) {
        self.profile_font = crate::text::Font::load(rom).ok();
        self.profile =
            crate::profile::Profile::load(path.as_ref().map(|p| p.with_file_name("profile.json")));
        self.progress = crate::progress::Progress::load(path, rom);
        if self.current.name() != "Demo"
            && let Some(game) = self.current.game_mut()
            && game.run.id == 0
        {
            self.profile.begin(&mut game.run);
            game.run.level = game.options.level;
            game.run.initial_ammo = game.options.start_ammo(rom);
            game.run.round_start_ammo = game.combat.hud.ammo;
            game.run.practice = true; // Direct stage starts are practice; New Game is eligible.
            let id = game.run.id;
            self.record(crate::devtools::replay::Event::RunIdentity(id));
            self.record(crate::devtools::replay::Event::Practice);
            self.profile.flush();
        }
    }
    pub fn checkpoint(&mut self) {
        if let Some(game) = self.current.game() {
            self.profile.observe(&game.run);
        }
        self.profile.flush();
        if self.current.name() == "Demo" || self.game().is_some_and(|g| g.practice.is_some()) {
            return;
        }
        if self
            .game()
            .is_some_and(|game| game.game_over() && game.continues == 0)
        {
            if let Err(error) = self.progress.clear() {
                self.progress.status = error;
            }
            return;
        }
        if let Some(state) = crate::devtools::SaveState::capture(self, 0, "Continue".into()) {
            self.progress.save(
                state,
                self.game()
                    .map(|g| crate::rom::regions::checksum(g.rom()))
                    .unwrap_or(0),
            );
        }
    }
    /// The desktop view affects object spawning and removal, so retain width
    /// changes alongside the input history used by disk saves.
    pub fn set_view_width(&mut self, width: usize) {
        let Some(game) = self.game_mut() else {
            return;
        };
        let before = (game.combat.view_width, game.combat.world_bounds);
        game.set_view_width(width);
        let after = (game.combat.view_width, game.combat.world_bounds);
        if before != after {
            self.record(crate::devtools::replay::Event::ViewWidth(after.0 as u16));
        }
    }
    /// Native dev tools: install `mode` now, dropping the current one (a
    /// save-state load or a warp).
    pub fn replace(&mut self, mode: Box<dyn Mode>) {
        let mut old = std::mem::replace(&mut self.current, mode);
        old.take_audio();
    }
    pub fn draw(&self, frame: &mut Frame) {
        self.current.draw(frame);
        // Keep requirements, exact statistics and option controls unobscured.
        // The notification queue resumes after leaving these screens.
        if matches!(self.current.name(), "Profile" | "Options" | "Practice") {
            return;
        }
        if let (Some(&index), Some(font)) = (
            self.profile.notices.front(),
            self.current
                .game()
                .map(|g| g.font())
                .or(self.profile_font.as_ref()),
        ) {
            let name = crate::profile::CATALOG[index].name.to_ascii_uppercase();
            frame.rect(16, 184, 288, 32, [8, 12, 24]);
            font.draw_text(
                frame,
                24,
                188,
                "LOCAL ACHIEVEMENT UNLOCKED",
                crate::text::Style::plain([255, 210, 100], 1),
            );
            font.draw_text(
                frame,
                24,
                202,
                &name,
                crate::text::Style::plain([220, 224, 232], 1),
            );
        }
    }
    pub fn name(&self) -> &'static str {
        self.current.name()
    }
    pub fn take_audio(&mut self) -> Audio {
        let mut audio = std::mem::take(&mut self.audio);
        audio.extend(self.current.take_audio());
        audio
    }
    pub fn game(&self) -> Option<&Game> {
        self.current.game()
    }
    pub(crate) fn loaded_profile_run(&mut self) {
        if let Some(game) = self.current.game_mut()
            && game.run.id == 0
        {
            self.profile.begin(&mut game.run);
            self.profile
                .data
                .credited
                .insert(game.run.id, game.run.stats.clone());
            let id = game.run.id;
            self.record(crate::devtools::replay::Event::RunIdentity(id));
        }
        self.mark_practice();
    }
    pub fn mark_practice(&mut self) {
        if let Some(game) = self.current.game_mut() {
            if let Some(session) = &mut game.practice {
                session.valid = false;
            }
            if game.run.practice {
                return;
            }
            self.profile.taint(&mut game.run);
            self.record(crate::devtools::replay::Event::Practice);
        }
    }
    pub fn game_mut(&mut self) -> Option<&mut Game> {
        self.current.game_mut()
    }
    /// The current mode's music level (`Mode::music_gain`).
    pub fn music_gain(&self) -> f64 {
        self.current.music_gain()
    }
    /// Deliver the cartridge sound driver's `$FFF803` communication byte to
    /// the current mode (`Mode::sound_communication`).
    pub fn sound_communication(&mut self, value: u8) {
        self.record(crate::devtools::replay::Event::Communication(value));
        self.current.sound_communication(value);
    }
    /// Begin a profile run with a session the replay rebuilds from `game`'s
    /// stage, then show its round card and save it for CONTINUE.
    fn start_run(&mut self, front: &crate::front_end::Front, mut game: Game, practice: bool) {
        use crate::devtools::replay::{Event, History};
        self.profile.begin(&mut game.run);
        let mut events = vec![
            Event::Session(front.options, front.high_score, front.demo_counter),
            Event::RunIdentity(game.run.id),
        ];
        if practice {
            self.profile.taint(&mut game.run);
            events.push(Event::Practice);
        }
        self.profile.observe(&game.run);
        self.profile.flush();
        self.history = Some(History {
            boot: false,
            stage: game.stage_id,
            effects: front.effects,
            events,
        });
        self.replace(crate::round_screens::StageClear::round_card(Box::new(game)));
        self.checkpoint();
    }
    pub(crate) fn record(&mut self, event: crate::devtools::replay::Event) {
        if let Some(history) = &mut self.history {
            history.events.push(event);
        }
    }
    /// The player chose QUIT; the desktop front end should close.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }
}

/// The gameplay a temporary mode holds in `Main_HandlerSaved`.
pub fn held_game(saved: &Option<Box<dyn Mode>>) -> Option<&Game> {
    saved.as_deref()?.game()
}
pub fn held_game_mut(saved: &mut Option<Box<dyn Mode>>) -> Option<&mut Game> {
    saved.as_deref_mut()?.game_mut()
}
/// A temporary mode's `Mode::take_audio`: the suspended mode's requests,
/// then its own.
pub fn take_held_audio(saved: &mut Option<Box<dyn Mode>>, own: &mut Audio) -> Audio {
    let mut audio = saved
        .as_deref_mut()
        .map(|mode| mode.take_audio())
        .unwrap_or_default();
    audio.extend(std::mem::take(own));
    audio
}

/// A/B/C/Start newly pressed: the skip test of the round screens
/// (`Func_00BDD6`, `RoundClear_S0C`: `Input_Port1` & `$F0`).
pub fn any_button(input: Input) -> bool {
    [
        Button::Attack,
        Button::Jump,
        Button::Ninjutsu,
        Button::Start,
    ]
    .iter()
    .any(|&button| input.pressed(button))
}

#[cfg(test)]
mod tests;
