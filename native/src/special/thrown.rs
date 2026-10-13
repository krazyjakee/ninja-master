//! The rider thrown off in a surf death: the type-`$2B` record `Player_Die`
//! (`$21AB0`) allocates in list 9 with routine `$7C` for state 62 (4-1),
//! 16 px above the rider, with no pose and `+$28` 0 (docs: the mount
//! wreck). Its poses `$1D`/`$1F` use tile word 0 and so the player's DMA'd
//! patterns at `$2B0`: the flying rider that the death's slot 1 (frame
//! 132) and later slots (frame 182) load, while the player's own frame 182
//! shows only the board left floating (`PlayerAssets::overlay`). The
//! horse's (`$78`, `$26D90`, list 1) bounces instead and has no plumes;
//! it is implemented by `HorseWreck` below.
//!
//! `Func_026CCE` runs after the player in each update and reads the death's
//! cursor (`plr_cursor`) less `$20`, the handler's next cursor:
//!
//! - below `$100` it shows nothing and moves the cursor to `$100`: the
//!   first update, where the stepper has just reached `$20`, skips slot 0,
//!   the music fade at `$20` and seven updates, six of them slowed;
//! - `$100..$1FF`: pose `$1D`, and the player's frame `$B6` (182, the
//!   board) over slot 1's;
//! - from `$200`: pose `$1F`, 16 px higher at exactly `$200`.
//!
//! Past the first, each update drifts it by a quarter of the water's speed
//! (`$FF0190`), sinks it at `$1000` and, while it is not yet hidden, tests
//! the terrain at it (`$3854` for tile word 0). In the water it hides
//! (`obj_flags` bit 11) and throws up two routine-8 type-`$24` plumes in
//! list 1 (`$26D3A`, `$26D64`), 8 px either side, 8 px below the player,
//! with the player's tile word. It is never freed: the part reload ends it.
use crate::{
    combat::{Combat, Effect, EffectKind},
    death::Death,
    game::Terrain,
    movement::Player,
    rom::{Result, byte, long, word},
};

/// `Effect::phase` once the rider is hidden.
const HIDDEN: u8 = 1;

/// The data field of an `addq`/`subq` opcode word (0 stands for 8).
fn quick(opcode: u16) -> i32 {
    match (opcode >> 9) & 7 {
        0 => 8,
        n => i32::from(n),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThrownRider {
    /// `$21ADE`: `subi.w #$10` from the rider's Y.
    rise: i32,
    /// `$26CD2`: `subi.w #$20` from the cursor.
    lead: u16,
    /// `$26CE2`: the cursor it moves the death to.
    jump: u16,
    /// `$26CF0`: the player's frame (the board) over slot 1.
    frame: usize,
    /// `$26CF4`/`$26D04`: its poses before and from `lift.0`.
    poses: [u16; 2],
    /// `$26CF8`/`$26D00`: at exactly this lead it rises this far.
    lift: (u16, i32),
    /// `$26D1A`: its sink rate.
    gravity: i32,
    /// `$26D54`/`$26D7E` and `$26D5E`: the plumes' offsets.
    plume: (i32, i32),
}
impl ThrownRider {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            rise: i32::from(word(rom, 0x21ae0)?),
            lead: word(rom, 0x26cd4)?,
            jump: word(rom, 0x26ce2)?,
            frame: usize::from(word(rom, 0x26cf0)?),
            poses: [
                u16::from(byte(rom, 0x26cf5)?),
                u16::from(byte(rom, 0x26d05)?),
            ],
            lift: (word(rom, 0x26cf8)?, i32::from(word(rom, 0x26d00)?)),
            gravity: long(rom, 0x26d1a)?,
            plume: (quick(word(rom, 0x26d54)?), quick(word(rom, 0x26d5e)?)),
        })
    }
    /// `Player_Die` for a surf death: the list-9 visual record.
    pub(crate) fn allocate(&self, combat: &mut Combat, (x, y): (i32, i32)) {
        let mut thrown = Effect::new(EffectKind::ThrownRider, x, y - self.rise, 0);
        thrown.back = true;
        combat.allocate_back(thrown);
    }
    /// `Func_026CCE` after the death's update (handler and stepper).
    pub(crate) fn update(
        &self,
        combat: &mut Combat,
        player: &mut Player,
        death: &mut Death,
        terrain: &impl Terrain,
    ) {
        let Some(index) = combat
            .effects
            .iter()
            .position(|effect| effect.kind == EffectKind::ThrownRider)
        else {
            return;
        };
        let lead = death.cursor.wrapping_sub(self.lead);
        let mut thrown = combat.effects[index].clone();
        if lead >> 8 == 0 {
            thrown.mapping = 0;
            death.cursor = self.jump;
            player.animator.cursor = self.jump;
            combat.effects[index] = thrown;
            return;
        }
        let mut pose = self.poses[0];
        if lead >> 8 == 1 {
            player.animator.frame = self.frame;
        }
        if lead >= self.lift.0 {
            if lead == self.lift.0 {
                thrown.y -= self.lift.1;
            }
            pose = self.poses[1];
        }
        // `neg.l` / `asr.l #2` of `$FF0190`, added to the 16.16 X.
        let water = -super::WATER_FLOW << 16;
        crate::combat::advance(&mut thrown.x, &mut thrown.motion.fx, -water >> 2);
        thrown.motion.gravity = self.gravity;
        let mut plumes = None;
        if thrown.phase != HIDDEN && terrain.solid_at(thrown.x, thrown.y) {
            thrown.phase = HIDDEN;
            let y = player.position().1 + self.plume.1;
            plumes = Some([(thrown.x - self.plume.0, y), (thrown.x + self.plume.0, y)]);
        }
        // $19D4.
        thrown.motion.step(&mut thrown.x, &mut thrown.y);
        thrown.mapping = if thrown.phase == HIDDEN { 0 } else { pose };
        combat.effects[index] = thrown;
        for at in plumes.into_iter().flatten() {
            combat.pending_plume(at);
        }
    }
}

/// `$26D90`, the list-1 rider thrown from a horse. Unlike the surf rider,
/// this starts immediately, never rewrites the death cursor, and bounces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HorseWreck {
    rise: i32,
    lead: u16,
    change: u16,
    poses: [u16; 2],
    drift: i32,
    gravity: i32,
    floor_mask: u16,
}

impl HorseWreck {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        let pose = u16::from(byte(rom, 0x26d99)?);
        Ok(Self {
            rise: i32::from(word(rom, 0x21ae0)?) + i32::from(word(rom, 0x21af8)?),
            lead: word(rom, 0x26d96)?,
            change: word(rom, 0x26d9c)?,
            poses: [pose, pose + quick(word(rom, 0x26da0)?) as u16],
            // `$1ED4` sets the horse background speed to six pixels.
            drift: -(i32::from(word(rom, 0x1ed6)?) << 16) >> 2,
            gravity: long(rom, 0x26db6)?,
            floor_mask: word(rom, 0x26dce)?,
        })
    }

    pub(crate) fn allocate(&self, combat: &mut Combat, (x, y): (i32, i32)) {
        combat.allocate_front(Effect::new(EffectKind::HorseWreck, x, y - self.rise, 0));
    }

    pub(crate) fn update(&self, combat: &mut Combat, death: &Death, terrain: &impl Terrain) {
        for wreck in &mut combat.effects {
            if wreck.kind != EffectKind::HorseWreck {
                continue;
            }
            let lead = death.cursor.wrapping_sub(self.lead);
            wreck.mapping = self.poses[usize::from(lead >= self.change)];
            crate::combat::advance(&mut wreck.x, &mut wreck.motion.fx, self.drift);
            wreck.motion.gravity = self.gravity;
            if terrain.solid_at(wreck.x, wreck.y) {
                wreck.y = i32::from((wreck.y as u16 & self.floor_mask) as i16);
                wreck.motion.vy = wreck.motion.vy.wrapping_neg();
            }
            // Objects_BuildSprites integrates after the handler's bounce.
            wreck.motion.step(&mut wreck.x, &mut wreck.y);
        }
    }
}

/// `$26E80`, list-9 `$2B` routine `$98`: the separate board underneath a
/// powered slash. It lives exactly while the player remains in state 63.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SlashBoard {
    mapping: u16,
    state: usize,
}

impl SlashBoard {
    pub(crate) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            mapping: word(rom, 0x26e82)?,
            state: usize::from(word(rom, 0x26e98)?),
        })
    }

    pub(crate) fn allocate(&self, combat: &mut Combat, (x, y): (i32, i32)) {
        let mut board = Effect::new(EffectKind::SurfSlashBoard, x, y, 0);
        board.back = true;
        combat.allocate_back(board);
    }

    pub(crate) fn update(&self, combat: &mut Combat, player: &Player) {
        if player.animator.state != self.state {
            combat
                .effects
                .retain(|effect| effect.kind != EffectKind::SurfSlashBoard);
            return;
        }
        for board in &mut combat.effects {
            if board.kind != EffectKind::SurfSlashBoard {
                continue;
            }
            board.mapping = self.mapping;
            // The ROM copies acceleration/velocity/position before sprite
            // construction integrates both records. Native movement has
            // already integrated, so both end at the player's same position.
            (board.x, board.y) = player.position();
            board.motion.fx = (player.x as u16 & 255) << 8;
            board.motion.fy = (player.y as u16 & 255) << 8;
        }
    }
}
