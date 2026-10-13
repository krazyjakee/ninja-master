//! Native gameplay for any round/substage using recovered ROM movement, art,
//! spawns, sections, scroll models, music selection and `$FF0029` transitions.
//! Collision geometry, enemy behavior and streaming remain documented native approximations.
mod camera;
mod doors;
mod markers;
mod ninjutsu;
mod scripted;
mod session;
mod signals;

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

/// Minimal terrain interface keeps simulation usable without SDL or a ROM.
/// `material_at` is the `$3854` query; the predicates classify it the way
pub trait Terrain {
    fn width(&self) -> i32;
    /// Layout height in pixels: 512 for horizontal layouts.
    fn height(&self) -> i32 {
        512
    }
    /// Compatibility query for ordinary objects: `$3854`'s D2 bit 15 for a
    /// sprite with the priority bit clear (`$3906` high word `$55BE`:
    /// materials 1-5, 7, 8, 10, 12 and 14), the test most enemy handlers
    /// make with `BMI`. It is not the player's collision; the player uses
    /// the predicates below. Terrains that only implement this read as
    /// full-solid material 1 through the default `material_at`.
    fn solid_at(&self, x: i32, y: i32) -> bool;
    /// `$3854`'s signed result for an ordinary object on one sprite-priority
    /// layer. The mask at `$3906` selects `$55BE` for low-priority sprites
    /// and `$5ABE` for high-priority sprites.
    fn object_solid_at(&self, x: i32, y: i32, high_priority: bool) -> bool {
        self.material_at(x, y).object_solid(high_priority)
    }
    /// `$3854`: the material code at a pixel after quadrant exclusion.
    fn material_at(&self, x: i32, y: i32) -> Material {
        if self.solid_at(x, y) {
            Material::SOLID
        } else {
            Material::EMPTY
        }
    }
    /// `$22456`: wall scans query the layout directly, without probing or
    /// activating list-6 platforms (`Player_ProbeTerrain`, `$21B20`).
    fn wall_material_at(&self, x: i32, y: i32) -> Material {
        self.material_at(x, y)
    }
    /// `$21B98`: the player probe found a list-6 object at the pixel
    /// (`d2` = `$8200` + its material), not the layout.
    fn object_at(&self, _x: i32, _y: i32) -> bool {
        false
    }
    /// Player floor: lands on and stands on the pixel (`$220EC` mask).
    fn stands_on(&self, x: i32, y: i32) -> bool {
        self.material_at(x, y).stands_on()
    }
    /// Player wall moving in `direction` (`$22490` mask, one-way 9 and 13).
    fn blocks_horizontal(&self, x: i32, y: i32, direction: i32) -> bool {
        self.material_at(x, y).blocks_horizontal(direction)
    }
    /// Stops a rising player's head (`$2259E` mask without 2, 6 and 14).
    fn blocks_ceiling(&self, x: i32, y: i32) -> bool {
        self.material_at(x, y).blocks_ceiling()
    }
    /// A wall the player can cling to while moving in `direction` (`$210F2`).
    fn clingable(&self, x: i32, y: i32, direction: i32) -> bool {
        self.material_at(x, y).clingable(direction)
    }
    /// Spikes (`$21C44..$21C94`).
    fn hurts(&self, x: i32, y: i32) -> bool {
        self.material_at(x, y).hurts()
    }
    /// The `Layout_Buffer` metatile under a pixel (`Func_037606`), for
    /// handlers that test the layout itself; `None` without a layout.
    fn metatile_at(&self, _x: i32, _y: i32) -> Option<u8> {
        None
    }
}
impl Terrain for StageAssets {
    fn width(&self) -> i32 {
        self.data.pixel_size().0
    }
    fn height(&self) -> i32 {
        self.data.pixel_size().1
    }
    fn solid_at(&self, x: i32, y: i32) -> bool {
        StageAssets::solid_at(self, x, y)
    }
    fn material_at(&self, x: i32, y: i32) -> Material {
        Material(self.collision_at(x, y))
    }
    fn metatile_at(&self, x: i32, y: i32) -> Option<u8> {
        self.data.metatile_at_pixel(x, y)
    }
}

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
        self.combat
            .set_player_motion(self.airborne, self.velocity_y);
        let attack = self
            .combat
            .start_player_attack(posture, self.position, facing_left);
        // $231CA..$232D8: every throw, the somersault spread included, sets
        // $FF37EC bit 0 (no SHINOBI bonus).
        if attack == Some(Attack::Throw) || self.combat.hud.ammo < ammo {
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
    /// Round 1-1, part 0.
    pub fn load(rom: &[u8]) -> Result<Self, String> {
        Self::load_stage(rom, StageId::FOREST)
    }
    /// Any round/substage, starting in `id.part`. Every part of the substage
    /// is loaded up front so stream handoffs never stall.
    pub fn load_stage(rom: &[u8], id: StageId) -> Result<Self, String> {
        let sections = crate::stage::sections(rom, id.round_index(), id.substage_index())?;
        let part = id.part_index();
        if part >= sections.len() {
            return Err(format!("{id}: substage has {} parts", sections.len()));
        }
        let ids: Vec<StageId> = (0..sections.len()).map(|p| id.with_part(p as u8)).collect();
        let mut pending_stages = ids
            .iter()
            .map(|&p| StageAssets::load_stage(rom, p).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        let mut pending_combat = ids
            .iter()
            .map(|&p| crate::combat::Combat::load_stage(rom, p).map(Some))
            .collect::<Result<Vec<_>, _>>()?;
        let scroll = ids
            .iter()
            .map(|&p| crate::stage::scroll_model(rom, p))
            .collect::<Result<Vec<_>, _>>()?;
        let bounds = sections
            .iter()
            .map(|section| CameraBounds::for_section(rom, section))
            .collect::<Result<Vec<_>, _>>()?;
        let assets = pending_stages[part].take().unwrap();
        let combat = pending_combat[part].take().unwrap();
        let sprites = PlayerAssets::load(rom)?;
        // `$119F2` selects the stage-table music first. On 2-1, `Player_Init`
        // immediately replaces IDATEN with UMA at `$22CC4`; the driver later
        // raises its communication byte to request IDATEN at `$205D6`.
        let music = if id.round == 2 && id.substage == 1 {
            SoundEvent::HorseStart.command(rom)?
        } else {
            crate::stage::stage_music(rom, id)?
        };
        // $11720/$11754: the part record's player and camera start.
        let (x, y) = sections[part].player_start;
        let mut player = Player::new(x, y);
        player.grounded = Player::occupied(&assets, x, y + 1);
        (player.camera_x, player.camera_y) = sections[part].camera_start;
        let special = crate::special::Special::for_stage(rom, id, &sections[part], &mut player);
        let objects = crate::stage_objects::StageObjects::load(rom, id)?;
        let autoscroll = CameraAutoScroll::new(
            AutoScroll(sections[part].raw[13]),
            (player.camera_x, player.camera_y),
        );
        let mut stage = Viewer::new(assets, View::Scene);
        // The asset preview uses the final mech colours; gameplay begins black.
        if id.round == 5 && id.substage == 3 {
            stage.reload_boss_planes();
        }
        let door = doors::Door::entrance(rom, id, &sections[part])?;
        let mut game = Self {
            stage_id: id,
            complete: false,
            requests: 0,
            part_request: None,
            game_flags: 0,
            pad: BossPad::default(),
            object_signals: Vec::new(),
            transitions: 0,
            sections,
            scroll,
            bounds,
            autoscroll,
            background: BackgroundY::at(0),
            rom: rom.into(),
            pending_stages,
            combat,
            pending_combat,
            cues: Vec::new(),
            music,
            music_request: Some(music),
            player,
            stage,
            sprites,
            cast: None,
            arena: None,
            special,
            markers: markers::Markers::default(),
            objects,
            high_score: 10_000,
            round_flags: RoundFlags::default(),
            fade: None,
            font: Font::load(rom)?,
            hud_renderer: HudRenderer::load(rom)?,
            // Direct play (`--play`) uses the power-on options: NORMAL.
            options: crate::options::GameOptions::default(),
            continues: crate::options::GameOptions::default().continues(rom),
            demo_counter: 0,
            extra_life: crate::options::GameOptions::default()
                .extra_life_step(rom)
                .map(|step| (step, step)),
            death: None,
            lag: 0,
            lost_fade: None,
            part_fade: None,
            door,
        };
        game.enter_markers();
        game.apply_camera_model();
        game.background = BackgroundY::at(game.player.camera_y);
        game.sync_stage_camera();
        Ok(game)
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
        let advancing = self.lag == 0 && self.cast.is_none();
        self.combat.weapons.force_throw = (self.options.control == 4)
            .then_some(input.pressed(Button::X) || input.pressed(Button::Y));
        if advancing
            && self.options.control == 4
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
        let result = self.game_run_frame(input);
        if advancing && self.death.is_none() {
            self.advance_door();
        }
        if std::mem::take(&mut self.combat.art_reloaded) {
            self.lag = self.lag.saturating_add(HIT_LAG + self.extra_hit_lag());
        }
        result
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
        if self.options.infinite {
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
            let ammo = self.combat.hud.ammo;
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
            if events.lethal {
                self.round_flags.damaged = true; // $21E92
            } else if self.combat.hud.ammo < ammo {
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
        self.extend_cues(events.sound_ids.iter().copied());
        self.apply_signals(&events.signals);
        if events.hit.is_some_and(|hit| hit.damage > 0) || events.lethal {
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
        let player_high_priority = self.combat.player_high_priority();
        let mut hooks = GameHooks {
            combat: &mut self.combat,
            cast: &mut self.cast,
            flags: &mut self.round_flags,
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
    /// The rider thrown off in a surf death (`special::thrown`), allocated
    /// at the death, is list 9's last record; its tile word 0 shows the
    /// rider the player's DMA loaded (`PlayerAssets::overlay`).
    fn draw_mounted_effects(
        &self,
        frame: &mut Frame,
        palettes: &Lines<'_>,
        camera: (i32, i32),
        list: SpriteList,
    ) {
        use crate::combat::EffectKind;
        for effect in self.combat.effects.iter().rev().filter(|effect| {
            effect.sprite_list() == list
                && matches!(
                    effect.kind,
                    EffectKind::ThrownRider | EffectKind::HorseWreck | EffectKind::SurfSlashBoard
                )
        }) {
            let pose = if effect.kind == EffectKind::SurfSlashBoard && effect.mapping == 0xb6 {
                Some(&self.sprites.frames[182])
            } else {
                self.sprites.overlay(effect.mapping)
            };
            let Some(pose) = pose else { continue };
            frame.mark_high_sprites(effect.high_priority);
            pose.draw(
                frame,
                palettes,
                effect.x - camera.0,
                effect.y - camera.1,
                false,
            );
            frame.mark_high_sprites(false);
        }
    }
    /// List 2: the player, then the 2-1 horse.
    fn draw_player(&self, frame: &mut Frame, palettes: &Lines<'_>, camera: (i32, i32)) {
        let player_high = self.combat.player_high_priority();
        let pose = &self.sprites.frames[self.player.display_frame()];
        let (x, feet) = self.player.position();
        // Ikazuchi charges: every fourth player update shows the shield.
        let ticks = self
            .cast
            .as_ref()
            .map_or(self.player.ticks, |cast| u64::from(cast.fx.frame()));
        let shield = crate::ninjutsu_fx::shield_mapping(
            self.combat.weapons.shield,
            ticks,
            self.player.animator.state,
        );
        // PlayerState_DeathHidden hides the sprite (42, 54); no death state
        // flickers ($21540). The horse death's rider is its list-1 wreck.
        let state = self.player.animator.state;
        let visible = if dying(state) {
            !death_hidden(state)
        } else {
            self.combat.player_visible(state, self.player.ticks)
        };
        frame.mark_high_sprites(player_high);
        // #85: the 2-1 horse is the next list-2 sprite, so it sits behind
        // the rider. It copies the rider's hide bit, so it flickers with it
        // and vanishes in the death, where the unported wreck `$78` takes
        // its place.
        if let Some(horse) = self.special.as_ref().and_then(|special| special.horse)
            && visible
            && !dying(state)
        {
            self.sprites.frames[horse.frame].draw_on_line(
                frame,
                palettes,
                (x - camera.0, feet - camera.1),
                false,
                crate::special::Horse::PALETTE_LINE,
            );
        }
        if visible
            && !shield.is_some_and(|id| {
                self.combat.shield_art.draw(
                    frame,
                    palettes,
                    id,
                    (x - camera.0, feet - camera.1),
                    self.player.facing_left,
                )
            })
        {
            pose.draw(
                frame,
                palettes,
                x - camera.0,
                feet - camera.1,
                self.player.facing_left,
            );
        }
        frame.mark_high_sprites(false);
    }
    pub fn draw(&self, frame: &mut Frame) {
        self.stage.align_frame(frame);
        let (backdrop, index) = self.stage.backdrop();
        frame.clear_backdrop(backdrop, index);
        // $2386 (4-1): below $40 the player's state leaves the line-200
        // H-interrupt on, and no sprite shows below that line.
        let sprite_end = self.scroll[self.part()]
            .sprite_cut
            .filter(|_| self.player.animator.state < 0x40)
            .map(|line| line + 1);
        // Boss4's H-interrupt reloads CRAM line 1 below line 199 (4-3).
        if let Some(change) = self.stage.cram_change() {
            frame.change_cram_line(change);
        }
        // The sprites make up the frame's sprite table (`Objects_BuildSprites`
        // and `$39A02`, see `render::sat`): recorded back to front, then
        // drawn with their explicit masks and priority.
        frame.begin_sprite_table();
        self.stage.scene_game_cut(frame, sprite_end);
        frame.clip_lines(sprite_end);
        // Object_UpdateVisibility subtracts Stage_VScrollOffset (the 7-1
        // bob) from every object's screen Y, as VSRAM adds it to plane A.
        let camera = (
            self.player.camera_x,
            self.player.camera_y + self.stage.raster.vscroll_offset(),
        );
        let lines = self.stage.lines();
        let (palettes, tiles) = (&lines, &self.stage.assets.tiles);
        // `Objects_BuildSprites` walks lists 1 to 9 (`SpriteList`), each
        // record's pieces in link order: painted from list 9 (the 2-1 kites,
        // behind them all, #87) to list 1, each list last linked first.
        for list in SpriteList::BACK_TO_FRONT {
            match list {
                SpriteList::Player => self.draw_player(frame, palettes, camera),
                SpriteList::PlayerAttacks => {
                    self.combat.draw_list(frame, list, palettes, tiles, camera);
                    // The spread and spent shurikens carry the player's
                    // priority bit (`$22C4C` or a boss's raise): high ones
                    // stay over the high plane cells.
                    let player_high = self.combat.player_high_priority();
                    self.combat
                        .draw_player_attacks(frame, palettes, tiles, camera, player_high);
                    if let Some(cast) = &self.cast {
                        cast.fx.draw(frame, palettes, camera);
                    }
                }
                _ => {
                    if matches!(list, SpriteList::Back | SpriteList::Effects) {
                        self.draw_mounted_effects(frame, palettes, camera, list);
                    }
                    self.combat.draw_list(frame, list, palettes, tiles, camera);
                    if list == SpriteList::Pickups
                        && frame.width() > crate::WIDTH
                        && let Some(special) = self.special.as_ref().filter(|s| s.scrolling)
                    {
                        for (mapping, x, y) in self.objects.pickup_approach(
                            self.combat
                                .approaching_pickups(special.camera_x, special.step),
                            camera.0,
                        ) {
                            self.combat.draw_object(
                                frame,
                                palettes,
                                tiles,
                                mapping,
                                0,
                                (x - camera.0, y - camera.1),
                                false,
                            );
                        }
                    }
                    if list == SpriteList::StageObjects
                        && frame.width() > crate::WIDTH
                        && let Some(special) = self.special.as_ref().filter(|s| s.scrolling)
                    {
                        for (mapping, x, y) in self.objects.mounted_approach(
                            special.camera_x,
                            self.player.camera_y,
                            special.step,
                        ) {
                            self.combat.draw_object(
                                frame,
                                palettes,
                                tiles,
                                mapping,
                                0,
                                (x - camera.0, y - camera.1),
                                false,
                            );
                        }
                    }
                    self.objects
                        .draw_list(frame, &self.combat, list, palettes, tiles, camera);
                }
            }
            if list == SpriteList::EnemyShots {
                self.combat.draw_boss_parts(frame, palettes, tiles, camera);
            }
        }
        let under_planes = frame.sprite_count();
        // Priority sprites of the ambient spawners sit over the planes'
        // high-priority cells.
        self.combat.draw_front(frame, palettes, tiles, camera);
        // List 1 heads the sprite table, and the warning sign's tile word
        // is high priority: it covers every other sprite and both planes.
        for marker in self.markers.visible(self.combat.ticks) {
            self.combat.draw_object(
                frame,
                palettes,
                tiles,
                marker.mapping,
                markers::TILE,
                (marker.x - camera.0, marker.y - camera.1),
                false,
            );
        }
        if let Some(special) = &self.special {
            for position in special.warnings.visible() {
                let position = (
                    position.0 + frame.view_right() - crate::WIDTH as i32,
                    position.1,
                );
                self.combat.draw_object(
                    frame,
                    palettes,
                    tiles,
                    crate::special::warning::MAPPING,
                    crate::special::warning::TILE,
                    position,
                    false,
                );
            }
        }
        frame.clip_lines(None);
        hud::window_masks(frame);
        frame.end_sprite_table();
        frame.draw_sprite_table(0..under_planes);
        // Both planes' high-priority cells cover the low-priority sprites.
        frame.behind_high_sprites(true);
        self.stage.front_game(frame);
        frame.behind_high_sprites(false);
        frame.draw_sprite_table(under_planes..frame.sprite_count());
        self.stage.clear_world_padding(frame);
        frame.center_content();
        let hud = &self.combat.hud;
        self.hud_renderer.draw(
            frame,
            HudValues {
                health: hud.health,
                max_health: hud.max_health,
                lives: hud.lives,
                ammo: hud.ammo,
                ninjutsu_uses: self.combat.weapons.ninjutsu_uses,
                ninjutsu_selected: self.combat.weapons.ninjutsu_selected,
                score_display: hud.score_display,
                infinite_shurikens: self.options.infinite,
            },
        );
        if let Some(fade) = self.fade {
            frame.fade(fade);
        }
        // $B04E: a lost life or a part load fades the frame out.
        if let Some(fade) = self.lost_fade.or(self.part_fade) {
            frame.fade(fade);
        }
    }
}

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
            if self.options.infinite {
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
