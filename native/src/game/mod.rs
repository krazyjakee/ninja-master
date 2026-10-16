//! Native gameplay for any round/substage using recovered ROM movement, art,
//! spawns, sections, scroll models, music selection and `$FF0029` transitions.
//! Collision geometry, enemy behavior and streaming remain documented native approximations.
mod draw;
mod loading;
mod mode;
mod terrain;
pub use terrain::Terrain;
mod camera;
mod doors;
mod markers;
mod ninjutsu;
mod scripted;
mod session;
mod signals;
mod tracking;

pub use scripted::scripted_input;

use crate::{
    assets::{PlayerAssets, StageAssets},
    combat::SpriteList,
    hud::{self, Renderer as HudRenderer, Values as HudValues},
    input::{Button, Input},
    modes::{Audio, Mode, ModeChange},
    movement::{
        Action, AnimationInput, Attack, AttackPosture, Material, NATIVE_UNIT as UNIT, Player,
        PlayerEvents, PlayerHooks, death_hidden, dying,
    },
    pause::Pause,
    render::{Fade, Frame, Lines, cram_palettes, fade_step},
    round_screens::{Bonuses, RoundFlags, StageClear},
    sound_events::{Route, SoundEvent},
    stage::{AutoScroll, CameraBounds, ModeFlags, ScrollModel, StageId},
    text::Font,
    viewer::{View, Viewer},
};
use camera::{ArenaLock, BackgroundY, CameraAutoScroll};

/// A boss's hold on the player's controls for one update: Boss2's
/// `$FF0029` bit 7 and `plr_ctrl` bit 11.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BossPad {
    /// [`Signal::SwapLeftRight`](crate::combat::Signal::SwapLeftRight).
    swap: bool,
    /// [`Signal::BlockNinjutsu`](crate::combat::Signal::BlockNinjutsu).
    ninjutsu_blocked: bool,
    /// Left/Right held as the player last read them (`Player_Input`).
    directions: u16,
    /// The last read was swapped: this read's presses follow from it.
    swapped: bool,
}
impl BossPad {
    /// `Input_ReadPort` (`$3996`): with the swap a held Left or Right
    /// becomes the other and both become neither (`eori.b #$C` of the
    /// nonzero pair); new presses are the edges of the swapped bits.
    fn read(&mut self, input: Input) -> Input {
        let pair = Button::Left.mask() | Button::Right.mask();
        let raw = input.held & pair;
        if !self.swap && !self.swapped {
            self.directions = raw;
            return input;
        }
        let held = if self.swap && raw != 0 {
            raw ^ pair
        } else {
            raw
        };
        let previous = std::mem::replace(&mut self.directions, held);
        self.swapped = self.swap;
        Input {
            held: (input.held & !pair) | held,
            pressed: (input.pressed & !pair) | (held & !previous),
        }
    }
}

/// The button-handler parts that combat and the ninjutsu timeline resolve.
struct GameHooks<'a> {
    combat: &'a mut crate::combat::Combat,
    cast: &'a mut Option<crate::ninjutsu::Cast>,
    flags: &'a mut RoundFlags,
    run: &'a mut crate::profile::Run,
    position: (i32, i32),
    /// `plr_ctrl` bit 5 and the Y velocity for an airborne throw's height.
    airborne: bool,
    velocity_y: i32,
    /// `plr_ctrl` bit 11 from a boss ([`Signal::BlockNinjutsu`]).
    ///
    /// [`Signal::BlockNinjutsu`]: crate::combat::Signal::BlockNinjutsu
    ninjutsu_blocked: bool,
}
impl PlayerHooks for GameHooks<'_> {
    fn attack(&mut self, posture: AttackPosture, facing_left: bool) -> Option<Attack> {
        let ammo = self.combat.hud.ammo;
        let spread_count = self.combat.weapons.shots.len();
        self.combat
            .set_player_motion(self.airborne, self.velocity_y);
        let attack = self
            .combat
            .start_player_attack(posture, self.position, facing_left);
        // $231CA..$232D8: every throw, the somersault spread included, sets
        // $FF37EC bit 0 (no SHINOBI bonus).
        if attack == Some(Attack::Throw)
            || self.combat.hud.ammo < ammo
            || self.combat.weapons.shots.len() > spread_count
        {
            self.flags.thrown = true;
        }
        attack
    }
    fn ninjutsu(&mut self, _state: usize) -> Option<usize> {
        // $22E8E: andi.l #$A0800,plr_event.
        if self.ninjutsu_blocked {
            return None;
        }
        let index = self.combat.begin_ninjutsu()?;
        let kind = crate::ninjutsu::Kind::from_index(index)?;
        *self.cast = Some(crate::ninjutsu::Cast::new(kind));
        // $22EC8: $FF37EC bit 1 (no NO NINJITSU bonus).
        self.flags.ninjutsu = true;
        Some(index)
    }
    fn damage(&mut self, damage: u8) -> bool {
        self.flags.damaged |= damage > 0;
        if damage > 0
            && let Some(e) = &mut self.run.encounter
        {
            e.damaged = true;
        }
        self.combat.subtract_event_health(damage)
    }
}

/// `$FF0029` (`Game_Requests`) bit 0: the life is lost. `PlayerState_Death`
/// sets it in cursor slot `$2B` or on the A/B/C skip (`$20564`, a word
/// `ori.w #1,(Game_Flags)`); `Mode_Game` state `$1C` then enters state `$20`,
/// which fades out and takes the life (`$B096`). `Func_0119F2` clears it
/// when the part reloads.
pub const REQUEST_LIFE_LOST: u8 = 1;
/// `$FF0029` (`Game_Requests`) bit 1: enter the next substage. The final
/// stream exit sets it (`$AC72`), as do encounter objects (for example the
/// `$B4/$B8` handler at `$1C026`); `Mode_Game` state `$20` (`$B010..$B148`)
/// consumes it.
pub const REQUEST_NEXT_SUBSTAGE: u8 = 2;
/// `$FF0029` bit 3: load another part of this substage from its part-record
/// start, keeping lives, score, health and shurikens. A type-5 switch sets
/// it with the target in `$FF37AE` (`$2958E`); `Mode_Game` `$B118` copies
/// that into `Part_Index`, and `$B136` sets the bit again for the load
/// after a reload exit (`$B0AA`, [`ModeFlags::reload_exit`]) so that
/// `Func_011A4A` restores the saved player words. `Func_0119F2` clears it.
pub const REQUEST_PART_LOAD: u8 = 8;

#[derive(Clone)]
pub struct Game {
    pub run: crate::profile::Run,
    /// Round/substage (1-based) and part being played (`$FF37A6/$FF37A8/$FF37AA`).
    pub stage_id: StageId,
    /// Set past the last round, where the cartridge starts `Mode_Ending`.
    pub complete: bool,
    /// `$FF0029` mirror; `REQUEST_LIFE_LOST`, `REQUEST_NEXT_SUBSTAGE` and
    /// `REQUEST_PART_LOAD` are modelled.
    pub requests: u8,
    /// The part `REQUEST_PART_LOAD` enters: `$FF37AE` for a switch,
    /// `Part_Index` + 1 for a reload exit.
    pub part_request: Option<usize>,
    /// `$FF0028` (`Game_Flags`) bits raised by native encounter handlers.
    pub game_flags: u16,
    /// What a boss did to the player's pad and controls this update.
    pad: BossPad,
    /// List-5 signals for the stage objects' update (items to drop, layout
    /// writes), held from the combat pass until `tick_objects`.
    object_signals: Vec<crate::combat::Signal>,
    /// Substage transitions taken in this session.
    pub transitions: usize,
    sections: Vec<crate::stage::Section>,
    /// `$1D5A` scroll model and `$1A64` camera limits of each part.
    scroll: Vec<ScrollModel>,
    bounds: Vec<CameraBounds>,
    /// Part-record word 13 while a normal vertical climb is held or running.
    autoscroll: CameraAutoScroll,
    background: BackgroundY,
    /// Shared, so clones (dev-tools save states, the tape search) stay small.
    rom: std::sync::Arc<[u8]>,
    pending_stages: Vec<Option<StageAssets>>,
    pub(crate) combat: crate::combat::Combat,
    pending_combat: Vec<Option<crate::combat::Combat>>,
    /// Effect, voice (external DAC), fade and stop commands (`$9D..$FF`)
    /// in request order.
    cues: Vec<u8>,
    /// Music command playing, and a change not yet taken by the sound player.
    music: u8,
    music_request: Option<u8>,
    pub player: Player,
    stage: Viewer,
    sprites: PlayerAssets,
    /// Running ninjutsu: the rest of the game waits in mode `$22A96`.
    cast: Option<crate::ninjutsu::Cast>,
    arena: Option<ArenaLock>,
    /// Horse (2-1) and surf (4-1) riding, issue #43 (`crate::special`).
    special: Option<crate::special::Special>,
    /// The list-1 "GO" markers (`Func_03771A`).
    markers: markers::Markers,
    /// The `$26F42` stage objects (list 6) and pickups (list 7), issue #60.
    pub(crate) objects: crate::stage_objects::StageObjects,
    /// `High_Score` (`$FF0018`): 10000 at power-on (`HighScore_Init`),
    /// raised from the score every play frame (`$124CA`).
    pub high_score: u32,
    /// `$FF37EC`: what the round clear's bonuses read; kept across the
    /// substages of a round.
    pub round_flags: RoundFlags,
    /// Palette fade over the whole frame: in after a load or a respawn.
    fade: Option<Fade>,
    font: Font,
    hud_renderer: HudRenderer,
    /// The options screen's settings (`Game_Options`, `$FF0000..$FF0003`).
    pub options: crate::options::GameOptions,
    /// `Continues`, from `Continue_Counts` by level (`Game_InitSession`).
    pub continues: u8,
    /// `Demo_Counter` (`$FF001E`), carried so the attract cycle keeps
    /// alternating between its two demos after a game.
    pub demo_counter: u16,
    /// `$FF37C0` and `$FF37BC` (`Func_012514`): the score of the next extra
    /// life and the step between them; `None` on EXPERT.
    extra_life: Option<(u32, u32)>,
    /// The running `PlayerState_Death` (#58, `crate::death`).
    pub death: Option<crate::death::Death>,
    /// Frames without an update (lag frames): a slowed death update
    /// (`$20540`), a ninjutsu's art load or its end (`crate::ninjutsu`).
    /// Nothing runs; the interrupt's music and uploads go on.
    lag: u8,
    /// `Mode_Game` state `$20` after a lost life: the fade out (`$B04E`).
    lost_fade: Option<Fade>,
    /// `Mode_Game` state `$20` before a part load: the same fade out.
    part_fade: Option<Fade>,
    /// The 7-2 door entrance/exit (`$397B0`/`$398A8`).
    door: Option<doors::Door>,
}
/// `Object_UpdateVisibility`: the screen's vertical centre below camera Y.
/// Frames without an update after a hit that deals damage:
/// `Player_ApplyHit` calls `Player_PowerDown` (`$21DC8`), whose
/// `Func_0046DA` unpacks Joe's ordinary art into VRAM, and the update ends
/// after the next vertical interrupt. Genesis Plus GX, the 21 round tapes:
/// 36 of 37 such hits lost one frame, 3 of them (7-3 boss fight) two, one
pub const HIT_LAG: u8 = 1;
const PIT_CENTRE: i32 = 0x70;
/// `Func_012566`: effect `$10` (1UP) with each extra life.
const ONE_UP_EFFECT: u8 = 0x10;
impl Game {
    /// Match object entrances and removal bounds to the active display.
    pub fn set_view_width(&mut self, width: usize) {
        self.combat.view_width = width.max(crate::WIDTH);
        self.combat.world_bounds = self.stage.world_bounds();
    }

    /// Index of the part being played.
    pub fn part(&self) -> usize {
        self.stage_id.part_index()
    }
    /// `Mode_StageLoad`: `Objects_InitWorkspace` empties list 1, then
    /// `Func_03771A` places the part's marker.
    pub(super) fn enter_markers(&mut self) {
        self.markers.clear();
        self.add_part_marker();
    }
    /// `Func_03771A` for the current part; boss arenas have none.
    fn add_part_marker(&mut self) {
        let arena = ModeFlags(self.sections[self.part()].mode_flags).boss_arena();
        if let Ok(Some(marker)) = markers::Markers::entry(&self.rom, self.stage_id, arena) {
            self.markers.add(marker);
        }
    }
    pub fn set_effects(&mut self, enabled: [bool; 4]) {
        self.stage.effects_enabled = enabled;
    }
    pub fn report(&self) -> String {
        self.stage.assets.report()
            + &self.sprites.report()
            + &self.combat.report()
            + &self.objects.report()
    }
    /// The requested SFX commands (`$9D..$D8`), in order.
    pub fn take_sound_cues(&mut self) -> Vec<u8> {
        self.take_cues(Route::Effect)
    }
    /// The other requested driver commands, in order: `$4A66` voices
    /// (external DAC samples `$DA..$FB`), the death fade `$FC` and SE STOP `$FD`.
    pub fn take_driver_cues(&mut self) -> Vec<u8> {
        self.take_cues(Route::Driver)
    }
    fn take_cues(&mut self, group: Route) -> Vec<u8> {
        let (taken, kept) = std::mem::take(&mut self.cues)
            .into_iter()
            .partition(|&command| crate::sound_events::route(command) == group);
        self.cues = kept;
        taken
    }
    /// Music command (`$81..$9C`) selected by the current stage.
    pub fn music(&self) -> u8 {
        self.music
    }
    /// A music change for the sound player: the stage-load selection, the
    /// camera-lock theme or a restart.
    pub fn take_music(&mut self) -> Option<u8> {
        self.music_request.take()
    }
    fn request_music(&mut self, command: u8) {
        if command != self.music {
            self.music = command;
            self.music_request = Some(command);
        }
    }

    /// `$205AE`: the player handler consumes the communication byte written
    /// by music sequence command `$E2`. UMA uses it to hand 2-1 back to
    /// IDATEN. The sound player calls this between platform-independent game
    /// ticks; no SDL state reaches the game.
    pub fn sound_communication(&mut self, value: u8) {
        if value != 0
            && let Ok(command) = SoundEvent::HorseTransition.command(&self.rom)
        {
            self.request_music(command);
        }
    }
    /// Raise the `$FF0029` bit 1 request as a finished encounter does. The
    /// next tick enters the next substage (tests and the dev tools).
    pub fn request_next_substage(&mut self) {
        self.requests |= REQUEST_NEXT_SUBSTAGE;
    }
    /// The horse/surf riding state of a mounted part (#43), else `None`.
    pub fn special(&self) -> Option<&crate::special::Special> {
        self.special.as_ref()
    }
    pub fn rom(&self) -> &[u8] {
        &self.rom
    }
    /// The plane-map font (`crate::text`), shared with the front-end screens.
    pub fn font(&self) -> &Font {
        &self.font
    }
    pub fn score(&self) -> u32 {
        self.combat.hud.score
    }
    /// Read-only view of the enemies, bosses, shots and HUD, for tools such
    /// as the full-game tape search (`native/examples/full_game_tape.rs`).
    pub fn combat(&self) -> &crate::combat::Combat {
        &self.combat
    }
    /// Spare lives (`Lives`, `$FF37E0`).
    pub fn lives(&self) -> u8 {
        self.combat.hud.lives
    }
    /// Health and its maximum (`Health`, `Health_Max`).
    pub fn health(&self) -> (u8, u8) {
        (self.combat.hud.health, self.combat.hud.max_health)
    }
    /// Shurikens left (`Ammo`).
    pub fn ammo(&self) -> u16 {
        self.combat.hud.ammo
    }
    /// `Demo_Init` gives the attract demo 99 shurikens.
    pub fn set_ammo(&mut self, ammo: u16) {
        self.combat.hud.ammo = ammo;
    }
    /// The last life is lost (`Lives` went negative at `$B096`).
    pub fn game_over(&self) -> bool {
        self.combat.game_over
    }
    /// The stage effects switched on (`--disable-effect`).
    pub fn effects(&self) -> [bool; 4] {
        self.stage.effects_enabled
    }
    /// Ask for the stage music again: after the round card, whose own music
    /// replaced it.
    pub fn restart_music(&mut self) {
        self.music_request = Some(self.music);
    }
    /// The last life's death reached slot `$2B`: the next `Mode_Game`
    /// update starts the fade to `Mode_GameOver`.
    /// The round clear bonuses for the round so far (`Func_00F5E2`).
    pub fn bonuses(&self) -> Bonuses {
        let hud = &self.combat.hud;
        Bonuses::new(
            self.round_flags,
            self.combat.weapons.ninjutsu_uses,
            hud.health,
            hud.max_health,
        )
    }
    /// Start a fade in (`Mode_Game` state `$14` after a load); play runs.
    pub fn fade_in(&mut self) {
        self.fade = Some(Fade::In(0));
    }
    pub fn fading(&self) -> bool {
        self.fade.is_some()
    }
    /// Combat updates so far; they stop while another mode holds the game.
    /// The stage view: its live palette and boss planes.
    /// One frame: a `Game_RunFrame` (`game_run_frame`), or none while the
    /// last one's work still holds the CPU (`lag`). A player hit's art
    /// reload (`Player_PowerDown`) makes the frames after it lag too.
    pub fn tick(&mut self, input: Input) -> bool {
        self.track_begin();
        let advancing = self.lag == 0 && self.cast.is_none();
        self.combat.weapons.force_throw = (self.options.control >= 4).then_some(
            input.pressed(Button::X) || (self.options.control == 4 && input.pressed(Button::Y)),
        );
        if advancing
            && self.options.control >= 4
            && self.death.is_none()
            && !self.player.silence_sounds
            && self.door.is_none()
            && input.pressed(Button::Mode)
            && ![Button::Ninjutsu, Button::Attack, Button::Jump]
                .iter()
                .any(|&b| input.pressed(b))
        {
            let next = (self.combat.weapons.ninjutsu_selected + 1) & 3;
            if self.select_ninjutsu(usize::from(next)) {
                self.cue(SoundEvent::NinjutsuSelect);
            }
        }
        self.sync_shurikens();
        let result = self.game_run_frame(input);
        self.sync_shurikens();
        if advancing && self.death.is_none() {
            self.advance_door();
        }
        if std::mem::take(&mut self.combat.art_reloaded) {
            self.lag = self.lag.saturating_add(HIT_LAG + self.extra_hit_lag());
        }
        self.track_end();
        result
    }
    pub(super) fn sync_shurikens(&mut self) {
        let disabled = self.options.shurikens_disabled();
        self.combat.shurikens_disabled = disabled;
        self.combat.infinite_shurikens = self.options.infinite && !disabled;
        self.objects.shurikens_disabled = disabled;
        if disabled {
            self.combat.hud.ammo = 0;
            self.objects.pickups.retain(|pickup| pickup.kind > 1);
        } else if self.options.infinite {
            self.combat.hud.ammo = crate::options::INFINITE_AMMO;
        }
    }
    /// The three two-frame 7-3 stalls in the emulator capture all occur
    /// while the grounded `$300` boss pose and at least six independent
    /// list-4 shots are live. The other four damaging hits cost one frame.
    fn extra_hit_lag(&self) -> u8 {
        if self.stage_id != StageId::new(7, 3, 0) {
            return 0;
        }
        let grounded = self.combat.enemies.iter().any(|enemy| {
            self.combat.spawns[enemy.spawn].kind == crate::bosses::BOSS_KIND
                && enemy.mapping == 0x300
        });
        let shots = self
            .combat
            .projectiles
            .iter()
            .filter(|shot| {
                shot.enemy && shot.alive && shot.life != crate::enemies::round7::ATTACHED
            })
            .count();
        u8::from(grounded && shots >= 6)
    }
    /// One `Game_RunFrame` (`$B1EE`). Stage presentation updates before
    /// player motion; enemies run after the player and camera on foot and
    /// mounted, as in the ROM. Start belongs to `Game_CheckPause`, which the
    /// `Mode` implementation runs: it never reaches the player here.
    fn game_run_frame(&mut self, input: Input) -> bool {
        let start = Button::Start.mask();
        let input = Input {
            held: input.held & !start,
            pressed: input.pressed & !start,
        };
        // A slowed death update ($20540) still holds the CPU: the frame
        // shows the last one and nothing else runs.
        if self.lag > 0 {
            self.lag -= 1;
            return false;
        }
        if let Some(fade) = self.fade {
            let fade = fade.advance();
            self.fade = (!fade.finished()).then_some(fade);
        }
        self.markers.tick(self.requests);
        self.combat.hud.update_score_display(); // Score_Display, +10 per frame.
        self.high_score = self.high_score.max(self.combat.hud.score); // $124CA
        self.award_extra_life(); // Func_0124C4 -> Func_01257E
        if self.options.infinite && !self.options.shurikens_disabled() {
            // Game_Options bit 7: throws never use up Ammo; the HUD shows 999.
            self.combat.hud.ammo = crate::options::INFINITE_AMMO;
        }
        if self.requests & REQUEST_NEXT_SUBSTAGE != 0 && !self.complete {
            self.enter_next_substage();
            return false;
        }
        // Direct Mode_Game state $20 without the mode layer (tests, tools
        // and the attract demo, which ends first): take the life at once.
        if self.requests & REQUEST_LIFE_LOST != 0 && !self.combat.game_over {
            self.lose_life();
            return false;
        }
        // Direct Mode_Game state $20 for a part load: load it at once.
        if self.requests & REQUEST_PART_LOAD != 0 {
            self.enter_requested_part();
            return false;
        }
        if self.complete || self.combat.game_over {
            return false;
        }
        self.combat.set_invincible(self.options.invincible);
        self.combat.level = self.options.level & 3;
        self.combat
            .set_item_box_targets(self.objects.intact_item_boxes());
        self.stage.advance_effects();
        // Stage palette routines request SFX too (`$D38` 2-2 lightning, `$A5`).
        let effect_cues = self.stage.take_sound_cues();
        self.extend_cues(effect_cues);
        // Ninjutsu_Finish on foot: the player's update below, then only the
        // camera (`$219F8`); mode `$22A96` runs no enemies or exits.
        let finishing = self.cast.is_some();
        if finishing && !self.tick_ninjutsu() {
            return false;
        }
        // #43: horse (2-1) and surf (4-1) autoscroll riding, crate::special.
        if let Some(special) = &mut self.special {
            if self.death.is_some() {
                let events =
                    special.tick_dying(&mut self.player, &mut self.combat, &self.stage.assets);
                self.apply_signals(&events.signals);
                self.extend_cues(events.sounds);
                self.run_death(input);
                self.update_enemies(false);
                self.tick_objects(0);
                self.update_attacks();
                self.update_rider();
                self.update_mounted_board();
                self.sync_stage_camera();
                return false;
            }
            self.combat
                .sample_attack_proximity(self.player.position(), self.player.facing_left);
            let ammo = self.combat.hud.ammo;
            let spread_count = self.combat.weapons.shots.len();
            let events = special.tick(
                input,
                &mut self.player,
                &mut self.combat,
                &self.stage.assets,
            );
            if let Some(kind) = events.ninjutsu {
                self.cast = Some(crate::ninjutsu::Cast::new(kind));
                self.round_flags.ninjutsu = true;
            }
            let death_state = match special.mount {
                crate::special::Mount::Horse => crate::movement::state::HORSE_DEATH,
                crate::special::Mount::Surfboard => crate::movement::state::SURF_DEATH,
            };
            self.apply_signals(&events.signals);
            if events.damaged {
                if let Some(e) = &mut self.run.encounter {
                    e.damaged = true;
                }
                self.round_flags.damaged = true; // $21E92
            } else if self.combat.hud.ammo < ammo || self.combat.weapons.shots.len() > spread_count
            {
                self.round_flags.thrown = true;
            }
            self.extend_cues(events.sounds);
            if events.lethal {
                // Player_Die ($21AB0, $21AEA); this update's stepper runs.
                self.player.animator.die(death_state, 0);
                self.start_death(death_state);
                self.player.animator.tick(&AnimationInput::default());
                self.run_death(input);
                if let Some(special) = &self.special {
                    special.throw_rider(&self.player, &mut self.combat);
                }
            }
            self.combat.weapons.proximity = None;
            self.update_enemies(input.held(Button::Down));
            self.tick_objects(0);
            self.update_attacks();
            self.update_rider();
            self.update_mounted_board();
            self.sync_stage_camera();
            return false;
        }
        // `$1B8E` runs independently of the player's follow-camera code.
        // `apply_camera_model` restores this accumulator after that code.
        self.autoscroll.tick();
        // $291C (7-2) moves the background Y on its own every frame.
        self.background.y += i64::from(self.scroll[self.part()].background_drift) << 16;
        // Player_ApplyEvent ($1FD7C): below the screen on the last update,
        // the pit death (43) with Health 0. The death's plr_flags bit 3
        // skips this test afterwards.
        if self.death.is_none() && self.below_screen() {
            self.combat.die();
            self.player.fall_into_pit();
            self.start_death(crate::movement::state::PIT_DEATH);
        }
        // The technique is chosen in the pause window (crate::pause).
        let raw = self.sections[self.part()].raw;
        self.player.scroll_flags = crate::stage::RuntimeFlags::from_record(raw[4], raw[7]).0;
        let down = input.held(Button::Down);
        let animator = &self.player.animator;
        self.combat
            .set_player_pose(animator.state, animator.cursor, animator.frame, down);
        self.combat.set_player_anchors(Some(self.player.anchors()));
        let spike_shocked = self.stage_id.round_index() == 6 && self.stage_id.substage_index() == 1;
        let spikes = self
            .player
            .spikes
            .then(|| crate::combat::PlayerHit::spikes(self.player.head_spikes, spike_shocked));
        // The start of Player_Update: what the last object pass left.
        let events = self.combat.check_player(
            self.player.position(),
            self.player.facing_left,
            (self.player.camera_x, self.player.camera_y),
            spikes,
        );
        self.run.hits(&events.hits);
        self.extend_cues(events.sound_ids.iter().copied());
        self.apply_signals(&events.signals);
        if events.hit.is_some_and(|hit| hit.damage > 0) || events.lethal {
            if let Some(e) = &mut self.run.encounter {
                e.damaged = true;
            }
            self.round_flags.damaged = true; // $20400, $21E92
        }
        if events.lethal {
            self.player.die();
            self.start_death(crate::movement::state::DEATH);
        }
        // plr_status bit 7 as Player_ApplyHit left it: a hit taken above
        // already powered down (`$21DC8`).
        self.player.powered = self.combat.weapons.powered;
        let input = self.pad.read(input);
        self.combat.player_input = input;
        let player_position = self.player.position();
        let player_event = self
            .door
            .as_ref()
            .map(doors::Door::event)
            .or_else(|| self.arena_player_event(events.player_event));
        self.combat
            .sample_attack_proximity(player_position, self.player.facing_left);
        let player_high_priority = self.combat.player_high_priority();
        let mut hooks = GameHooks {
            combat: &mut self.combat,
            cast: &mut self.cast,
            flags: &mut self.round_flags,
            run: &mut self.run,
            position: player_position,
            airborne: !self.player.grounded,
            velocity_y: self.player.velocity_y,
            ninjutsu_blocked: self.pad.ninjutsu_blocked,
        };
        // Player_ProbeTerrain ($21B20) also finds the list-6 objects.
        let terrain = crate::stage_objects::ObjectTerrain::new(
            &self.stage.assets,
            &self.objects,
            player_high_priority,
        );
        self.player.tick_with(
            input,
            &terrain,
            PlayerEvents {
                event: player_event,
                hit: events.hit,
                blocked: events.blocked,
                bounce: events.bounce,
            },
            &mut hooks,
        );
        let touched = terrain.touched();
        self.combat.weapons.proximity = None;
        self.combat.mech_hand_contact = self.objects.mech_hands_touched(touched);
        if let Some(ticks) = self.player.immunity_hold {
            self.combat.invulnerability = ticks;
        }
        if self.player.immunity_step {
            self.combat.invulnerability += 1;
        }
        if self.player.animator.state == crate::movement::state::THROWN_DEATH
            && self.death.is_none()
        {
            self.start_death(crate::movement::state::THROWN_DEATH);
        }
        self.run_death(input);
        let sounds = self.combat.take_player_sounds();
        self.extend_cues(sounds);
        // #63: material-15 splashes, allocated as the player's update found them.
        for splash in std::mem::take(&mut self.player.splashes) {
            let sound = self.combat.splash(splash);
            self.extend_cues(sound);
        }
        if self.player.wall_kicked && !self.player.silence_sounds {
            self.cue(SoundEvent::WallKick); // $213C4: player sound 7
            self.combat
                .spawn_player_dust((player_position.0, player_position.1 - 16));
        }
        if !self.player.silence_sounds
            && (self.player.jump_requested || self.player.action == Some(Action::Somersault))
        {
            self.cue(SoundEvent::Jump); // $23464 and $210B6: effect 5
        }
        let animator = &self.player.animator;
        if animator.state == crate::movement::state::COMBO && animator.cursor == 0x980 {
            self.combat.start_combo_cut(); // the handler saw cursor $0900
        }
        self.apply_camera_model();
        // $1D4C: parts with runtime flags bit 6 (1-1 part 2, 3-1 part 2, 5-1
        // part 3) clamp the player to the screen every frame, and so does the
        // 3-3 scroll routine itself. The native camera lock keeps its own
        // clamp (`advance_arena`).
        if self.arena.is_none()
            && (crate::stage::RuntimeFlags(self.player.scroll_flags).screen_clamp()
                || self.scroll[self.part()].screen_clamp)
        {
            self.clamp_player_to_screen();
        }
        if finishing {
            // $2193E: after Player_Update's own countdown, and one more for
            // the next update, which does not count it down: `Player_Freeze`
            // sets plr_flags bit 4, and `$1FD6C` clears it and skips the
            // damage entry there.
            self.combat.invulnerability = crate::ninjutsu::PROTECTION_TICKS + 1;
            self.sync_stage_camera();
            return false;
        }
        if self.death.is_some() {
            // Native: a dying player neither takes an exit nor locks the
            // camera. The enemies and stage objects keep running.
            self.update_enemies(down);
            self.tick_objects(touched);
            self.update_attacks();
            self.sync_stage_camera();
            return false;
        }

        let part_before = self.part();
        let mut kept_objects = false;
        // $ABC0 proximity selects a stream handoff or $AC7C camera lock.
        // Final-part proximity is not a victory condition. $AB94 skips the
        // test in boss arenas ($FF39F7 bit 0), which end through their boss.
        let section = &self.sections[self.part()];
        let reached = self.arena.is_none()
            && self.door.is_none()
            && !ModeFlags(section.mode_flags).boss_arena()
            && section.exit_reached(self.player.position(), section.mode_flags as u8);
        let (kind, lock_target) = (section.exit_kind(), section.raw[12]);
        let reload = ModeFlags(section.mode_flags).reload_exit();
        if reached {
            if section.mode_flags & 0x400 != 0 {
                self.begin_door_exit();
            } else if kind == crate::stage::ExitKind::CameraLock {
                self.arena = Some(ArenaLock {
                    camera_x: self.player.camera_x,
                    camera_y: self.player.camera_y,
                    settled: false,
                });
                // $ACAC..$ACCA: with a Y target the lock plays record 4.
                if lock_target != 0
                    && let Ok(command) = crate::stage::camera_lock_music(&self.rom)
                {
                    self.request_music(command);
                }
            } else if reload && self.part() + 1 < self.sections.len() {
                // $AC2E, then Mode_Game $B0AA: fade out and load the next
                // part from its record start.
                self.request_part(self.part() + 1);
            } else if self.part() + 1 < self.sections.len() {
                // Account for this tick's follow/autoscroll before rebasing.
                self.sync_stage_camera();
                self.stage_id = self.stage_id.with_part(self.stage_id.part + 1);
                let (assets, mut combat) = self.part_state(self.stage_id.part_index());
                let next = &self.sections[self.part()];
                let previous_camera_x = self.player.camera_x;
                let previous_camera_y = self.player.camera_y;
                (self.player.x, self.player.y) =
                    next.handoff_player_fixed8((self.player.x, self.player.y));
                (self.player.camera_x, self.player.camera_y) =
                    next.handoff_camera_pixels((self.player.camera_x, self.player.camera_y));
                // $11CDE rebases the foreground camera but preserves both
                // background accumulators ($FFE010/$FFE014).
                self.stage.background_x_bias += previous_camera_x - self.player.camera_x;
                self.autoscroll = self.autoscroll.section_change(
                    AutoScroll(next.raw[13]),
                    (self.player.camera_x, self.player.camera_y),
                );
                self.background
                    .rebase(previous_camera_y, self.player.camera_y);
                self.player.spawn = self.sections[self.part()].player_start;
                self.stage.assets = assets;
                combat.inherit_progress(&self.combat);
                self.combat = combat;
                self.stage.event_ticks = None;
                kept_objects = self
                    .objects
                    .enter_part(&self.rom, self.stage_id)
                    .expect("previously validated section objects");
                // SectionChange_S30 adds the new part's marker to list 1.
                self.add_part_marker();
            } else {
                // $AC4E..$AC72: the part counter wraps and requests the next
                // substage; after a reload exit $B0AA wraps it into $B0C6.
                self.requests |= REQUEST_NEXT_SUBSTAGE;
            }
        }
        self.advance_arena(events.player_event);
        self.update_enemies(down);
        // $26F42 and $35D94 run after the player and the camera. The probe
        // found the previous part's objects when the part just changed; a
        // kept list (the lift climbs) still holds them.
        let touched = if self.part() == part_before || kept_objects {
            touched
        } else {
            0
        };
        self.tick_objects(touched);
        self.update_attacks();
        self.sync_stage_camera();
        false
    }
    /// `Game_RunFrame` runs `Func_00915A`, the bosses, the enemies and the
    /// player's attacks after `Player_Update` and the camera (`Func_001A4C`):
    /// they see this update's player and camera, as the sprites drawn from
    /// them do. Their hits reach the player on the next update.
    fn update_enemies(&mut self, down: bool) {
        self.stage.camera_x_limits = Some(self.view_camera_limits());
        self.combat.world_bounds = self.stage.world_bounds();
        self.combat.camera_locked = self.arena.as_ref().is_some_and(|arena| arena.settled);
        self.combat
            .set_player_motion(!self.player.grounded, self.player.velocity_y);
        let animator = &self.player.animator;
        self.combat
            .set_player_pose(animator.state, animator.cursor, animator.frame, down);
        self.combat.set_player_anchors(Some(self.player.anchors()));
        let hidden = !self
            .combat
            .player_visible(animator.state, self.player.ticks);
        self.combat.set_player_hidden(hidden);
        // Boss2 clears `$FF0029` bit 7 at the top of its update (`$2B284`).
        self.pad.swap = false;
        self.pad.ninjutsu_blocked = false;
        let events = self.combat.update_objects(
            self.player.position(),
            self.player.facing_left,
            (self.player.camera_x, self.player.camera_y),
            &self.stage.assets,
            true,
        );
        if let Some(special) = &mut self.special {
            special.after_objects(&events);
        }
        self.run.hits(&events.hits);
        self.extend_cues(events.sound_ids);
        // A boss's death also requests the next substage (`$FF0029` bit 1, #52).
        self.apply_signals(&events.signals);
    }
    /// `PlayerAttacks_Update` follows the list-6 objects, which met the
    /// attacks where the last update left them.
    fn update_attacks(&mut self) {
        let events = self.combat.update_attacks(
            self.player.position(),
            self.player.facing_left,
            (self.player.camera_x, self.player.camera_y),
            &self.stage.assets,
        );
        self.run.hits(&events.hits);
        self.extend_cues(events.sound_ids);
        // The list-1 pass that ends it (the 1-1 gate's push).
        self.apply_signals(&events.signals);
    }
    fn update_mounted_board(&mut self) {
        if let Some(special) = &self.special {
            special.update_board(&self.player, &mut self.combat);
        }
    }
    /// One update of the stage objects and pickups (`crate::stage_objects`).
    fn tick_objects(&mut self, touched: u32) {
        use crate::combat::Signal;
        let signals = std::mem::take(&mut self.object_signals);
        for signal in &signals {
            if let Signal::MechHands(hands) = signal {
                self.objects.place_mech_hands(*hands, &self.combat);
            }
        }
        let drops = signals.iter().filter_map(|signal| match *signal {
            Signal::Drop(drop) => Some(drop),
            _ => None,
        });
        self.objects.drop_pickups(drops);
        // These overlaps become damage on the next combat update, so retain
        // the frame chosen by this update's player step with them.
        self.combat.set_player_frame(self.player.animator.frame);
        let camera = (self.player.camera_x, self.player.camera_y);
        let events = self.objects.tick(&mut crate::stage_objects::Context {
            combat: &mut self.combat,
            player: &mut self.player,
            camera,
            touched,
            terrain: &self.stage.assets,
            animation_step: (self.stage.assets.effects.animation.as_ref())
                .map_or(0, |animation| animation.step()),
        });
        // Layout_Buffer writes from list 5 and the stage objects.
        let edits = signals.iter().filter_map(|signal| match *signal {
            Signal::Terrain(edit) => Some(edit),
            _ => None,
        });
        for edit in edits.chain(events.terrain_edits.iter().copied()) {
            self.stage.assets.data.apply_edit(edit);
        }
        if events.release_autoscroll {
            self.autoscroll.release();
        }
        if events.ramp
            && let Some(special) = &mut self.special
        {
            special.queue_ramp();
        }
        self.apply_palette_ops(&events.palette_ops);
        self.run.stats.pickups += events.collected.len() as u64;
        self.track_pickups(&events.collected_at);
        let crushed = events.crushed;
        self.extend_cues(events.sounds);
        if crushed && self.death.is_none() {
            self.start_death(crate::movement::state::DEATH);
        }
        if let Some(param) = events.switch {
            self.run_switch(param);
        }
    }
    /// `$29552..$2959E`, the pressed type-5 switch: `Func_0295A0` takes
    /// palette line 0 one step toward black each update. The update in
    /// which no colour changes any more raises the request: parameter 0
    /// the next substage (`$2956C`), otherwise a load of part parameter - 1
    /// (`$2958E`). Parameter 1 would load part 7 instead of part 0 when
    /// `$FF380C & $3F` is `$3F` (`$2957A`), but only type-`$E` objects set
    /// those bits, no record has that type and no switch has parameter 1.
    fn run_switch(&mut self, param: u16) {
        let line = &mut self.stage.assets.palette_words[0];
        let mut changed = false;
        for color in line.iter_mut() {
            let faded = fade_step(*color, 0);
            changed |= faded != *color;
            *color = faded;
        }
        if changed {
            self.stage.assets.palettes = cram_palettes(&self.stage.assets.palette_words);
            return;
        }
        match param {
            0 => self.requests |= REQUEST_NEXT_SUBSTAGE,
            part => self.request_part(usize::from(part) - 1),
        }
    }
    /// Raise `REQUEST_PART_LOAD` for `part` (`$FF37AE`, `Part_Index`).
    fn request_part(&mut self, part: usize) {
        if part < self.sections.len() {
            self.part_request = Some(part);
            self.requests |= REQUEST_PART_LOAD;
        }
    }
    /// `Mode_Pause_Refresh` (`$B4B2`), run by `Mode_Pause` every frame: the
    /// stage's palette routine and tile animation go on while the world
    /// holds (see [`Viewer::advance_paused_effects`]).
    pub(crate) fn refresh_paused(&mut self) {
        self.stage.advance_paused_effects();
        let effect_cues = self.stage.take_sound_cues();
        self.extend_cues(effect_cues);
    }
    /// Request an event's ROM command (see `crate::sound_events`).
    fn cue(&mut self, event: SoundEvent) {
        if let Ok(command) = event.command(&self.rom) {
            self.extend_cues([command]);
        }
    }
    /// Queue requested commands: music to `take_music`, everything else in
    /// request order (`take_sound_cues`, `take_driver_cues`). While `PlayerState_Death` runs, `Sys_Flags2`
    /// bits 5-6 drop the `$4A4E` effects and `$4A66` voices; music and the
    /// fade still pass.
    fn extend_cues(&mut self, commands: impl IntoIterator<Item = u8>) {
        let muted = self.death.is_some() && self.requests & REQUEST_LIFE_LOST == 0;
        for command in commands {
            if muted && matches!(command, 0x9d..=0xd8 | 0xda..=0xfb) {
                continue;
            }
            if crate::sound_events::route(command) == Route::Music {
                self.request_music(command);
                continue;
            }
            // Bounded while no caller takes them.
            if self.cues.len() >= 256 {
                self.cues.remove(0);
            }
            self.cues.push(command);
        }
    }
}

#[cfg(test)]
mod roadmap_tests;
