//! Lives, deaths, continues and substage transitions.
use super::{
    BackgroundY, CameraAutoScroll, Game, ONE_UP_EFFECT, PIT_CENTRE, REQUEST_LIFE_LOST,
    REQUEST_NEXT_SUBSTAGE, REQUEST_PART_LOAD,
};
use crate::{
    WIDTH,
    assets::StageAssets,
    combat::Combat,
    input::{Button, Input},
    movement::Player,
    stage::{AutoScroll, StageId},
};

impl Game {
    /// The substage `Mode_Game` state `$20` moves to after this one; `None`
    /// past 7-3, where `$B1A0` installs `Mode_Ending`.
    pub fn next_stage(&self) -> Option<StageId> {
        self.source
            .next_substage(&self.rom, self.stage_id)
            .ok()
            .flatten()
    }
    /// Part 0 of `next`, keeping score, lives, health, weapons, the high
    /// score and, inside a round, the bonus flags and a music id that is
    /// already playing (`$4B02`).
    pub fn load_next(&self, next: StageId) -> Result<Self, String> {
        let mut game = Self::load_from(&self.rom, self.source.clone(), next)?;
        game.run = self.run.clone();
        game.practice = self.practice.clone();
        game.run.clear_recorded = false;
        if next.round != self.stage_id.round {
            game.run.round_ticks = 0;
            game.run.round_died = false;
            game.run.round_start_ammo = self.combat.hud.ammo;
        }
        game.run.encounter = None;
        game.run.encounter_finished = false;
        game.combat.inherit_progress(&self.combat);
        game.set_effects(self.stage.effects_enabled);
        game.transitions = self.transitions + 1;
        game.high_score = self.high_score.max(self.combat.hud.score);
        game.options = self.options;
        game.continues = self.continues;
        game.demo_counter = self.demo_counter;
        game.extra_life = self.extra_life;
        if next.round == self.stage_id.round {
            // $F276 clears the flags only after a round's tally.
            game.round_flags = self.round_flags;
            if game.music == self.music {
                game.music_request = None;
            }
        }
        game.sync_shurikens();
        game.clear_explore_enemies();
        Ok(game)
    }
    /// Direct `Mode_Game` state `$20` without the mode layer (tests and
    /// tools that drive `Game::tick` alone): consume the request and swap in
    /// the next substage at once. `Modes` intercepts the request first and
    /// runs the fades, round clear and round card (`StageClear`). Past 7-3
    /// the game is complete.
    pub(super) fn enter_next_substage(&mut self) {
        self.requests &= !REQUEST_NEXT_SUBSTAGE;
        let Some(next) = self.next_stage() else {
            self.complete = true; // Mode_Ending ($B1A0).
            return;
        };
        // Every substage is validated by the headless tests; a load failure
        // ends the session rather than panicking.
        let Ok(mut game) = self.load_next(next) else {
            self.complete = true;
            return;
        };
        if game.music == self.music {
            game.music_request = None; // $4B02: the same id keeps playing.
        }
        *self = game;
    }
    /// The round clear tally (`$125BC` adds each step to `Score`;
    /// `RoundClear_S20` then runs `HighScore_Update` and `Func_01257E`).
    pub fn add_score(&mut self, points: u32) {
        self.combat.hud.score += points;
        self.high_score = self.high_score.max(self.combat.hud.score);
        self.award_extra_life();
        self.track_thresholds();
    }
    /// `Func_01257E`: once per call, a life (and effect `$10`, 1UP) when the
    /// score has reached the next threshold, which then moves up one step.
    pub(super) fn award_extra_life(&mut self) {
        let Some((next, step)) = self.extra_life else {
            return;
        };
        if self.combat.hud.score < next {
            return;
        }
        self.combat.hud.lives = self.combat.hud.lives.saturating_add(1);
        self.extra_life = Some((next.saturating_add(step), step));
        if let Ok(command) = crate::sound_events::Table::Effect.command(&self.rom, ONE_UP_EFFECT) {
            self.extend_cues([command]);
        }
    }
    /// A new game (`Mode_Game` states 0..8): `Game_InitSession` sets the
    /// continues, then `start_lives` as below.
    pub fn start_session(&mut self, options: crate::options::GameOptions) {
        self.run = crate::profile::Run {
            level: options.level,
            practice: options.infinite || options.invincible,
            initial_ammo: options.start_ammo(&self.rom),
            round_start_ammo: options.start_ammo(&self.rom),
            ..Default::default()
        };
        self.options = options;
        self.continues = options.continues(&self.rom);
        self.start_lives();
    }
    /// `Game_InitLives` (score 0, lives and maximum health from
    /// `Level_LivesHealth`), `Game_InitAmmo` (`Shuriken_Counts`) and
    /// `Game_RefillHealth`, with `Func_012514`'s extra-life thresholds.
    pub(super) fn start_lives(&mut self) {
        let (lives, health) = self.options.lives_health(&self.rom);
        let ammo = self.options.start_ammo(&self.rom);
        let hud = &mut self.combat.hud;
        hud.score = 0;
        hud.score_display = 0;
        hud.lives = lives;
        hud.max_health = health;
        hud.health = health;
        hud.ammo = ammo;
        self.combat.game_over = false;
        self.extra_life = self
            .options
            .extra_life_step(&self.rom)
            .map(|step| (step, step));
        self.sync_shurikens();
    }
    /// CONTINUE on the game-over screen: `Mode_Game` state `$28` goes back
    /// to state 4, so the substage restarts from part 0 with the lives,
    /// score (0), shurikens and health of a new game. The continues left,
    /// the high score, the options, the selected ninjutsu and the round's
    /// bonus flags carry over.
    pub fn continue_game(&self) -> Result<Self, String> {
        let mut game = Self::load_from(&self.rom, self.source.clone(), self.stage_id.with_part(0))?;
        game.set_effects(self.stage.effects_enabled);
        game.transitions = self.transitions;
        game.high_score = self.high_score.max(self.combat.hud.score);
        game.options = self.options;
        game.continues = self.continues;
        game.demo_counter = self.demo_counter;
        game.run = self.run.clone();
        game.run.stats.continues += 1;
        game.run.encounter = None;
        game.run.encounter_finished = false;
        game.round_flags = self.round_flags;
        game.combat.weapons.ninjutsu_selected = self.combat.weapons.ninjutsu_selected;
        game.start_lives();
        game.sync_shurikens();
        Ok(game)
    }
    /// `Mode_Game` state `$20` after the fade (`$B096`): `Lives` - 1. With
    /// a life left, state 8 reloads the part; below zero the game is over
    /// and `Mode_GameOver` follows (`$B1AC`). Returns true for game over.
    pub(super) fn lose_life(&mut self) -> bool {
        self.requests &= !REQUEST_LIFE_LOST;
        if self.combat.hud.lives == 0 {
            self.combat.game_over = true;
            return true;
        }
        self.combat.hud.lives -= 1;
        self.reload_part();
        false
    }
    /// `Mode_Game` state 8 (`$AF18`) after a lost life: `Game_InitAmmo` and
    /// `Game_RefillHealth` (health and the ninjutsu stock). `Mode_RoundCard`
    /// returns at once because `Game_Requests` bit 5 is still set
    /// (`RoundCard_Init`, `$BAE0`), so no card is shown. `Mode_StageLoad`
    /// reloads the part the player died in, from its start, and state `$14`
    /// fades it in while play runs. The cartridge's load takes 35-40 black
    /// frames; the native reload is immediate.
    pub(super) fn reload_part(&mut self) {
        // Func_0119F2 clears the other requests with the lost life's.
        self.requests &= !REQUEST_PART_LOAD;
        self.part_request = None;
        // Section-local objects and projectiles restart; later parts' caches
        // stay untouched.
        let id = self.stage_id;
        let routines = self.rules().routines;
        let mut combat = self
            .source
            .combat(&self.rom, id)
            .expect("previously validated section combat");
        combat.inherit_progress(&self.combat);
        self.combat = combat;
        self.combat.new_life();
        // Mode_StageLoad decodes the layout again: burnt floors come back.
        self.stage.assets.data.revert_edits();
        // $1FA7E/$1FBB8 replace the final arena's art, maps and palettes.
        if routines == StageId::new(7, 3, 0) {
            self.stage.assets = self
                .source
                .stage_assets(&self.rom, id)
                .expect("previously validated final arena");
        }
        // It redraws the 3-3 corridor and head (#96) and sets VDP register
        // $0B again (no 2-3 wave).
        self.stage.reload_boss_planes();
        // The reload passes $26EE6: the list-6 mask and objects start over.
        self.objects = self
            .source
            .objects(&self.rom, id)
            .expect("previously validated section objects");
        // Game_InitAmmo selects the option's count.
        self.combat.hud.ammo = self.options.start_ammo(&self.rom);
        let section = &self.sections[self.part()];
        self.player = Player::new(section.player_start.0, section.player_start.1);
        self.player.camera_x = section.camera_start.0;
        self.player.camera_y = section.camera_start.1;
        self.autoscroll = CameraAutoScroll::new(
            AutoScroll(section.raw[13]),
            (self.player.camera_x, self.player.camera_y),
        );
        self.player.grounded = Player::occupied(
            &self.stage.assets,
            section.player_start.0,
            section.player_start.1 + 1,
        );
        // $11A08/$399D4 reset the door script after every stage reload.
        self.door = super::doors::Door::entrance(&self.rom, routines, section)
            .expect("validated door script");
        self.run.encounter = None;
        self.run.encounter_finished = false;
        self.cast = None;
        self.arena = None;
        self.death = None;
        self.lag = 0;
        self.lost_fade = None;
        self.special = crate::special::Special::for_stage(
            &self.rom,
            routines,
            &self.sections[self.part()],
            &mut self.player,
        );
        self.clear_explore_enemies();
        self.enter_markers();
        self.background = BackgroundY::at(self.player.camera_y);
        self.sync_stage_camera();
        self.fade_in();
        // Func_0119F2 requests the stage music again ($4AC8), so a
        // camera-lock theme gives way; the capture shows the stage track
        // restarted after the death fade.
        if let Ok(command) = self.source.music(&self.rom, id) {
            self.music = command;
            self.music_request = Some(command);
        }
    }
    /// The art, layout and enemies of `part` as a fresh load finds them: the
    /// copy preloaded by `load_stage` when the part has not been entered
    /// yet, otherwise decoded again from the ROM (6-2's switches revisit
    /// parts).
    pub(super) fn part_state(&mut self, part: usize) -> (StageAssets, Combat) {
        let id = self.stage_id.with_part(part as u8);
        let assets = self.pending_stages[part].take().unwrap_or_else(|| {
            self.source
                .stage_assets(&self.rom, id)
                .expect("previously validated section")
        });
        let mut combat = self.pending_combat[part].take().unwrap_or_else(|| {
            self.source
                .combat(&self.rom, id)
                .expect("previously validated section combat")
        });
        if self.exploring() {
            combat
                .enemies
                .retain(|e| combat.spawns[e.spawn].kind == 0x40);
        }
        (assets, combat)
    }
    /// `Mode_Game` state `$20` for `REQUEST_PART_LOAD` after its fade out:
    /// consume the request and load the part.
    pub(super) fn enter_requested_part(&mut self) {
        self.requests &= !REQUEST_PART_LOAD;
        if let Some(part) = self.part_request.take() {
            self.load_part(part);
        }
    }
    /// A part load inside the substage (`$B0AA` or `$B118`, then `$B136`,
    /// `$AF38` and `Mode_StageLoad`). Unlike a lost life it takes no life
    /// and skips `Game_InitAmmo` and `Game_RefillHealth`: lives, score,
    /// health, shurikens, the ninjutsu stock and the round's bonus flags
    /// carry over. `Func_011A28` saves the player's words `$28..$31` and
    /// `Func_011A4A` puts them back over `Player_Init`, masked (`$EAC080`)
    /// and with state 1: power-up (`plr_ctrl` bit 7), the Ikazuchi shield
    /// (`plr_shield`) and Fushin's jump (`plr_flags` bit 1) are kept, the
    /// immunity counter is not. The player and camera start at the part
    /// record's start (`$11720`), not at a streamed position. The list-6
    /// objects restart unless the part shares the previous list
    /// (`$26EE6`, [`crate::stage_objects::StageObjects::enter_part`]). The
    /// cartridge spends about 50 black frames loading; native loads at once
    /// and fades in.
    pub(super) fn load_part(&mut self, part: usize) {
        let id = self.stage_id.with_part(part as u8);
        let routines = self.source.rules(id).routines;
        let (assets, mut combat) = self.part_state(part);
        combat.inherit_progress(&self.combat);
        combat.invulnerability = 0;
        self.combat = combat;
        self.stage_id = id;
        self.stage.assets = assets;
        self.stage.background_x_bias = 0;
        self.stage.event_ticks = None;
        // Player_Init writes VDP register $07 = 0 ($22BC0).
        self.stage.backdrop_entry = 0;
        self.source
            .enter_part_objects(&mut self.objects, &self.rom, id)
            .expect("previously validated section objects");
        let section = &self.sections[part];
        let fushin = self.player.fushin;
        self.player = Player::new(section.player_start.0, section.player_start.1);
        self.player.fushin = fushin;
        (self.player.camera_x, self.player.camera_y) = section.camera_start;
        self.autoscroll = CameraAutoScroll::new(
            AutoScroll(section.raw[13]),
            (self.player.camera_x, self.player.camera_y),
        );
        self.player.grounded = Player::occupied(
            &self.stage.assets,
            section.player_start.0,
            section.player_start.1 + 1,
        );
        self.run.encounter = None;
        self.run.encounter_finished = false;
        self.cast = None;
        self.arena = None;
        self.death = None;
        self.lag = 0;
        self.lost_fade = None;
        self.part_fade = None;
        self.door = super::doors::Door::entrance(&self.rom, routines, section)
            .expect("validated door script");
        self.special = crate::special::Special::for_stage(
            &self.rom,
            routines,
            &self.sections[part],
            &mut self.player,
        );
        self.enter_markers();
        self.apply_camera_model();
        // $11982: the background Y starts at the camera.
        self.background = BackgroundY::at(self.player.camera_y);
        self.sync_stage_camera();
        self.fade_in();
        // Func_0119F2 asks for the part's music ($4AC8); an id already
        // playing continues ($4B02).
        if let Ok(command) = self.source.music(&self.rom, id) {
            self.request_music(command);
        }
    }
    /// `Object_UpdateVisibility` (`$17A4`) sets `obj_flags` bit 6 when the
    /// player is within 256 px of the screen centre horizontally and at
    /// least 256 px below its vertical centre (camera Y + `$70`, with
    /// `Stage_VScrollOffset` 0). `Player_ApplyEvent` reads it on the next
    /// update.
    pub(super) fn below_screen(&self) -> bool {
        let (x, feet) = self.player.position();
        (self.player.camera_x + WIDTH as i32 / 2 - x).abs() < 256
            && feet - (self.player.camera_y + PIT_CENTRE) >= 256
    }
    /// Start `PlayerState_Death` in `state`.
    pub(super) fn start_death(&mut self, state: usize) {
        if self.death.is_none() {
            self.run.round_died = true;
            self.run.stats.deaths += 1;
        }
        if let Some(e) = &mut self.run.encounter {
            e.damaged = true;
        }
        self.death = Some(crate::death::Death::new(state, self.player.animator.cursor));
    }
    /// `Func_026CCE`: the rider thrown off in a surf death, after the
    /// death's update.
    pub(super) fn update_rider(&mut self) {
        if let (Some(special), Some(death)) = (&self.special, &mut self.death) {
            special.update_rider(
                &mut self.player,
                death,
                &mut self.combat,
                &self.stage.assets,
            );
        }
    }
    /// One `PlayerState_Death` call (after the player's own update on foot):
    /// its cues, the slowdown and the lost life.
    pub(super) fn run_death(&mut self, input: Input) {
        let Some(death) = &mut self.death else {
            return;
        };
        let skip = [Button::Attack, Button::Jump, Button::Ninjutsu]
            .iter()
            .any(|&button| input.pressed(button));
        let slow_lag = death.slow_lag();
        let update = death.update(self.combat.hud.lives, skip);
        for sound in update.sounds {
            self.cue(sound);
        }
        if update.slow {
            self.lag = slow_lag;
        }
        if update.life_lost {
            self.requests |= REQUEST_LIFE_LOST;
        }
    }
}
