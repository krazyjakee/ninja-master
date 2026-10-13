//! The displayed camera: part limits, forced coordinates, the autoscroll
//! accumulators and the `$AC7C` camera lock.
use super::{Game, REQUEST_NEXT_SUBSTAGE, UNIT};
use crate::{
    WIDTH,
    movement::PlayerEvent,
    stage::{AutoScroll, CameraBounds, ModeFlags},
};

#[derive(Clone)]
pub(super) struct ArenaLock {
    pub(super) camera_x: i32,
    pub(super) camera_y: i32,
    pub(super) settled: bool,
}

/// The normal-stage half-pixel camera accumulators of `$1B8E`. Mounted
/// horizontal stages own the same fields in `Special`; this state handles
/// the three vertical climbs whose part record selects the Y axis and the
/// on-foot 4-3 arena, whose `$0007` selects X at speed 0 (Genesis Plus GX:
/// camera X stays 0 although its limits reach 3776).
#[derive(Clone, Copy, Debug)]
pub(super) struct CameraAutoScroll {
    /// `$FF0184`'s 16.16 Y step. `$1B32` sets it on a stage load only.
    velocity: i32,
    /// `$FF0180`'s 16.16 X step, set with it.
    velocity_x: i32,
    /// `$1B8E` dispatches on the live word's axis bits.
    vertical: bool,
    horizontal: bool,
    pub(super) y: i32,
    pub(super) fraction: u16,
    /// `$FF0176`: the X accumulator that `Func_001BF2` copies to Camera_X.
    pub(super) x: i32,
    pub(super) x_fraction: u16,
    /// The 16.16 step this frame's `tick` applied (0 while held).
    pub(super) step: i32,
    active: bool,
    arming: bool,
}
/// `$FFE014`, the background Y that plane B's vertical scroll reads.
#[derive(Clone, Copy, Debug)]
pub(super) struct BackgroundY {
    /// 16.16 position.
    pub(super) y: i64,
    /// The camera Y it last followed.
    camera_y: i32,
}
impl BackgroundY {
    /// Keep the background continuous while the foreground changes origin.
    pub(super) fn rebase(&mut self, before: i32, after: i32) {
        self.y += i64::from(before - self.camera_y) << 16;
        self.camera_y = after;
    }

    /// Part load (`$11982`): the background starts at the camera.
    pub(super) fn at(camera_y: i32) -> Self {
        Self {
            y: i64::from(camera_y) << 16,
            camera_y,
        }
    }
}
impl CameraAutoScroll {
    /// A stage load (`Func_011924`, `$11934`) sets word 13 bit 0 after
    /// reading the part record, so an autoscroll starts held.
    pub(super) fn new(config: AutoScroll, (camera_x, camera_y): (i32, i32)) -> Self {
        let config = AutoScroll(config.0 | 1);
        let immediate = config.starts_immediately();
        let (velocity_x, velocity) = config.velocity();
        Self {
            velocity,
            velocity_x,
            vertical: config.vertical(),
            horizontal: config.horizontal(),
            y: camera_y,
            fraction: 0,
            x: camera_x,
            x_fraction: 0,
            step: 0,
            active: immediate,
            // `$1BC0` releases an immediate scroll after its first held frame.
            arming: immediate,
        }
    }
    /// `SectionChange_S0C` moving into the next part in place: `$11754`
    /// reloads word 13 and the accumulator restarts at the new camera, but
    /// `$1B32` does not run, so the running velocity carries over. 2-2
    /// part 2's `$0828` (bit 0 clear) keeps the lift climb scrolling.
    pub(super) fn section_change(
        self,
        config: AutoScroll,
        (camera_x, camera_y): (i32, i32),
    ) -> Self {
        let held = config.0 & 1 != 0;
        let arming = held && config.0 & 2 != 0;
        Self {
            velocity: self.velocity,
            velocity_x: self.velocity_x,
            vertical: config.0 & 8 != 0,
            horizontal: config.0 & 4 != 0,
            y: camera_y,
            fraction: self.fraction,
            x: camera_x,
            x_fraction: self.x_fraction,
            step: 0,
            active: !held || arming,
            arming,
        }
    }
    pub(super) fn vertical(self) -> bool {
        self.vertical
    }
    pub(super) fn horizontal(self) -> bool {
        self.horizontal
    }
    /// A vertical scroll that `$1B8E` applies: word 13 bit 0 is clear.
    fn running(self) -> bool {
        self.vertical && self.active
    }
    /// The same for the X axis (4-3).
    fn running_x(self) -> bool {
        self.horizontal && self.active
    }
    pub(super) fn tick(&mut self) {
        self.step = 0;
        if !self.vertical && !self.horizontal {
            return;
        }
        if self.arming {
            self.arming = false;
        } else if self.active {
            if self.vertical {
                self.step = self.velocity;
                crate::combat::advance(&mut self.y, &mut self.fraction, self.step);
            }
            if self.horizontal {
                crate::combat::advance(&mut self.x, &mut self.x_fraction, self.velocity_x);
            }
        }
    }
    pub(super) fn release(&mut self) {
        if self.vertical || self.horizontal {
            self.active = true;
            self.arming = false;
        }
    }
}

impl Game {
    /// Encounters limited to one screen keep their original composition as
    /// the camera moves, rather than exposing padded level columns. Wider
    /// boss rooms use their live world limits.
    pub(super) fn view_camera_limits(&self) -> (i32, i32) {
        if let Some(arena) = &self.arena {
            return (arena.camera_x, arena.camera_x);
        }
        if ModeFlags(self.sections[self.part()].mode_flags).boss_arena()
            && (self.scroll[self.part()].screen_clamp || self.combat.view_screen_clamped)
        {
            return (self.player.camera_x, self.player.camera_x);
        }
        self.camera_bounds().x
    }

    /// The live `$FF0148` limits: the part's `$1A64` limits with the floor
    /// a stage event raised since the part loaded (`$3714C`, the 1-1 cave).
    pub(super) fn camera_bounds(&self) -> CameraBounds {
        let mut bounds = self.bounds[self.part()];
        if let Some(floor) = self.stage.camera_floor_x() {
            bounds.x.0 = bounds.x.0.max(floor);
        }
        bounds
    }
    /// `$342E`, run by `$1D4C` when the part's runtime flags have bit 6:
    /// the player's X word within camera X + 32 ..= + 288 and its Y word
    /// no less than camera Y + 16 (skipped while that is negative). Only
    /// the integer words are written; fractions stay.
    pub(super) fn clamp_player_to_screen(&mut self) {
        let player = &mut self.player;
        let clamp_word = |fixed: &mut i32, pixels: i32| {
            *fixed = pixels * UNIT + fixed.rem_euclid(UNIT);
        };
        let (x, y) = player.position();
        let left = player.camera_x + 32;
        if x <= left {
            clamp_word(&mut player.x, left);
        } else if x > left + 256 {
            clamp_word(&mut player.x, left + 256);
        }
        let top = player.camera_y + 16;
        if top >= 0 && y <= top {
            clamp_word(&mut player.y, top);
        }
    }
    /// Displayed camera: the scroll routine's forced coordinates (for example
    /// `$1E18` pins 1-1 to Y 288), otherwise the follow camera within the
    /// part's live `$FF0148` limits.
    pub(super) fn apply_camera_model(&mut self) {
        let bounds = self.camera_bounds();
        let model = &self.scroll[self.part()];
        self.player.camera_x = match model.forced_camera_x {
            Some(x) => x,
            None => self
                .player
                .camera_x
                .clamp(bounds.x.0, bounds.x.1.max(bounds.x.0)),
        };
        self.player.camera_y = match model.forced_camera_y {
            Some(y) => y,
            None => self
                .player
                .camera_y
                .clamp(bounds.y.0, bounds.y.1.max(bounds.y.0)),
        };
        if self.autoscroll.running() {
            self.player.camera_y = self
                .autoscroll
                .y
                .clamp(bounds.y.0, bounds.y.1.max(bounds.y.0));
        } else if self.autoscroll.vertical() {
            // $1BD4: while bit 0 holds the scroll, the follow camera moves
            // and the accumulator copies it (7-1 before the lift is boarded).
            self.autoscroll.y = self.player.camera_y;
            self.autoscroll.fraction = 0;
        }
        if self.autoscroll.running_x() {
            self.player.camera_x = self
                .autoscroll
                .x
                .clamp(bounds.x.0, bounds.x.1.max(bounds.x.0));
        } else if self.autoscroll.horizontal() {
            self.autoscroll.x = self.player.camera_x;
            self.autoscroll.x_fraction = 0;
        }
    }
    /// Show the camera on the stage and move the background Y with it.
    pub(super) fn sync_stage_camera(&mut self) {
        self.stage.camera_x_limits = Some(self.view_camera_limits());
        let camera_y = self.player.camera_y;
        self.background.y += if self.autoscroll.running() {
            // $1BF2/$1C0E: a climb moves $FFE014 twice as far as the
            // camera; $1CD4 puts it back while the camera is at a limit.
            if camera_y == self.autoscroll.y {
                2 * i64::from(self.autoscroll.step)
            } else {
                0
            }
        } else {
            // $22772: the follow camera moves both by the same amount.
            i64::from(camera_y - self.background.camera_y) << 16
        };
        self.background.camera_y = camera_y;
        self.stage.x = self.player.camera_x;
        self.stage.y = camera_y;
        self.stage.background_y = self.background.y;
    }
    /// A moving arena can catch an idle player before its encounter issues
    /// a walk event. Give that grounded screen-clamp movement a walking pose.
    /// Object events (including walk-out and damage) retain precedence.
    pub(super) fn arena_player_event(&self, event: Option<PlayerEvent>) -> Option<PlayerEvent> {
        event.or_else(|| {
            let arena = self.arena.as_ref()?;
            let exit = self.sections[self.part()].exit.0;
            (arena.camera_x <= exit
                && self.player.grounded
                && self.player.position().0 <= arena.camera_x + 4 + 32)
                .then_some(PlayerEvent::WalkRight)
        })
    }
    /// The `$AC7C` camera lock: the arena scrolls into place, holds the
    /// player inside it and ends the substage once it is cleared.
    /// `event` is this update's `plr_event`: code 3 walks Joe out of it.
    pub(super) fn advance_arena(&mut self, event: Option<PlayerEvent>) {
        let Some(arena) = &mut self.arena else {
            return;
        };
        let section = &self.sections[self.stage_id.part_index()];
        // $AC88/$ACFC: X advances four until strictly beyond exit X.
        // The Y accumulator moves one toward the section's target ($AD2A).
        if arena.camera_x <= section.exit.0 {
            arena.camera_x += 4;
        }
        arena.camera_y += (section.exit_camera_y - arena.camera_y).signum();
        arena.settled = arena.camera_x > section.exit.0 && arena.camera_y == section.exit_camera_y;
        self.player.camera_x = arena.camera_x;
        self.player.camera_y = arena.camera_y;
        if !arena.settled || event != Some(PlayerEvent::WalkRight) {
            self.player.x = self.player.x.clamp(
                (arena.camera_x + 32) * UNIT,
                (arena.camera_x + WIDTH as i32 - 32) * UNIT,
            );
        }
        // $1BB1C removes B4 behind camera-32. Native cleanup also removes
        // actors left outside the locked arena; it never awards kill score.
        // Defeating the remaining actors stands in for the encounter's own
        // $FF0029 bit 1 request ($1C026); it is a native gate.
        for enemy in &mut self.combat.enemies {
            // Enemy10 hides at the origin until Camera_Mode >= 8 and then
            // deliberately dashes offscreen; its handler owns its lifetime.
            if self.combat.spawns[enemy.spawn].kind != 0x10 && enemy.x < arena.camera_x - 32 {
                enemy.state = crate::combat::EnemyState::Defeated;
            }
        }
        if arena.settled && self.combat.enemies.iter().all(|enemy| !enemy.live()) {
            self.requests |= REQUEST_NEXT_SUBSTAGE;
        }
    }
}
