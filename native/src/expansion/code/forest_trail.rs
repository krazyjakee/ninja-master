//! Forest Trail's code: the leaping monk, a new enemy drawn with the
//! komuso's art and poses.
use super::{Code, EnemyType, FIRST_KIND};
use crate::{
    combat::{Enemy, EnemyState, Reaction, Spawn},
    enemies::{Behavior, Ctx, forest::Rules},
    game::Terrain,
    rom::Result,
};
use std::sync::Arc;

pub(super) struct ForestTrail;

/// The leaping monk's type.
const MONK: u16 = FIRST_KIND;
/// The komuso (`$24`), whose art and poses the monk uses.
const KOMUSO: u16 = 0x24;
/// Any non-zero routine lets weapons collide (`Behavior::hit`).
const ALIVE: u16 = 4;
/// Walking speed, 16.16 pixels per tick.
const WALK: i32 = 0x1_0000;
/// Leap launch and gravity, 16.16 pixels per tick.
const LEAP_UP: i32 = -0x4_8000;
const LEAP_ACROSS: i32 = 0x2_0000;
const GRAVITY: i32 = 0x2400;
/// Ticks between leaps, and how near the player must be to leap.
const LEAP_EVERY: u16 = 90;
const LEAP_RANGE: i32 = 144;
/// Ticks a non-lethal hit stuns the monk.
const STUN: u16 = 20;

impl Code for ForestTrail {
    fn enemies(&self, rom: &[u8]) -> Result<Vec<EnemyType>> {
        Ok(vec![EnemyType {
            kind: MONK,
            art: KOMUSO,
            score_bcd: 0x0300,
            contact_damage: 1,
            health_counter: 1,
            behavior: Arc::new(LeapingMonk {
                rules: Rules::load(rom)?,
            }),
        }])
    }
}

/// Walks at the player, leaps at it when near, and takes two hits.
struct LeapingMonk {
    rules: Rules,
}

impl LeapingMonk {
    fn grounded(terrain: &dyn Terrain, enemy: &Enemy) -> bool {
        terrain.solid_at(enemy.x, enemy.y + 8)
    }
    fn face_player(cx: &Ctx, enemy: &mut Enemy) {
        // The komuso's walking poses face left unflipped.
        enemy.facing_left = !cx.player_left(enemy.x);
    }
    fn fall(enemy: &mut Enemy) {
        enemy.motion.gravity = GRAVITY;
        enemy.state = EnemyState::Hurt;
    }
}

impl Behavior for LeapingMonk {
    fn label(&self, _: u16) -> String {
        "Forest Trail leaping monk".into()
    }
    fn initial_mapping(&self, _: u16) -> u16 {
        self.rules.patrol[0]
    }
    fn activate(&self, cx: &mut Ctx, enemy: &mut Enemy, _: &Spawn) {
        enemy.tile = Some(cx.tile_base(KOMUSO));
        enemy.mapping = self.rules.patrol[0];
        enemy.routine = ALIVE;
        enemy.state = EnemyState::Waiting;
        Self::face_player(cx, enemy);
    }
    fn update(&self, cx: &mut Ctx, enemy: &mut Enemy, _: &Spawn, terrain: &dyn Terrain) {
        match enemy.reaction.take() {
            Some(Reaction::Hurt) => {
                cx.spark(enemy, false);
                enemy.flash = 8;
                enemy.timer = STUN;
                enemy.mapping = self.rules.patrol_hit;
                enemy.motion.vx = if cx.player_left(enemy.x) { WALK } else { -WALK };
                enemy.motion.vy = LEAP_UP / 2;
                Self::fall(enemy);
                return;
            }
            Some(Reaction::Deflect) => {
                let sound = cx.shared.deflect_sound;
                cx.play(sound);
                cx.spark(enemy, false);
            }
            _ => {}
        }
        if enemy.y > terrain.height() + 64 {
            Ctx::free(enemy); // Fell into a pit.
            return;
        }
        match enemy.state {
            // Airborne: a leap, a stun or a fall, until it lands.
            EnemyState::Hurt => {
                enemy.timer = enemy.timer.saturating_sub(1);
                if enemy.motion.vy >= 0 && terrain.solid_at(enemy.x, enemy.y) {
                    enemy.y &= !15;
                    enemy.motion.stop();
                    enemy.mapping = self.rules.patrol[0];
                    enemy.state = EnemyState::Waiting;
                    enemy.counter = 0;
                }
            }
            _ => {
                if !Self::grounded(terrain, enemy) {
                    enemy.motion.vx = 0;
                    Self::fall(enemy);
                    return;
                }
                Self::face_player(cx, enemy);
                let toward = if enemy.facing_left { 1 } else { -1 };
                enemy.counter = enemy.counter.wrapping_add(1);
                if enemy.counter >= LEAP_EVERY && cx.player_dx(enemy.x) < LEAP_RANGE {
                    enemy.counter = 0;
                    // The throw pose faces right unflipped, unlike the walk.
                    enemy.mapping = self.rules.patrol[3];
                    enemy.facing_left = cx.player_left(enemy.x);
                    enemy.motion.vx = toward * LEAP_ACROSS;
                    enemy.motion.vy = LEAP_UP;
                    Self::fall(enemy);
                    return;
                }
                // Alternate the two walking poses every eight ticks.
                if enemy.counter.is_multiple_of(8) {
                    let step = usize::from(enemy.counter / 8 % 2);
                    enemy.mapping = self.rules.patrol[step];
                }
                let ahead = enemy.x + toward * 12;
                enemy.motion.vx = if terrain.solid_at(ahead, enemy.y - 8) {
                    0
                } else {
                    toward * WALK
                };
            }
        }
    }
    /// The ordinary blast, as unported types use.
    fn dying(&self, cx: &mut Ctx, enemy: &mut Enemy, _: &Spawn, _: &dyn Terrain) {
        if enemy.reaction.take() == Some(Reaction::Death) {
            if enemy.death_spark {
                cx.spark(enemy, true);
            }
            let blast = cx.shared.blasts[0];
            cx.explode(enemy.x, enemy.y, blast);
            enemy.mapping = 0;
        }
        Ctx::remove(enemy);
    }
}
