//! Mech head and hands: parent commands, flame sweep and orb attacks.
use super::*;

fn timed(rom: &[u8], p: usize) -> Result<Vec<(u16, u16)>> {
    let mut out = Vec::new();
    for i in 0..16 {
        let last = word(rom, p + i * 4)?;
        if last == 0xffff {
            return Ok(out);
        }
        out.push((last, word(rom, p + i * 4 + 2)?));
    }
    Err("unterminated animation list".into())
}
/// The frame of a `(last timer, frame)` list at timer `t`.
fn frame_at(list: &[(u16, u16)], t: i32) -> Option<u16> {
    list.iter()
        .find(|(last, _)| i32::from(*last) >= t)
        .map(|(_, frame)| *frame)
}

// Type $7C, Enemy7C ($30FAC): parts A (head) and B (hands).

/// Part scratch: the last command seen, the command timer, the sweep
/// angle and leg, the flame counter.
const SEEN: usize = 0;
const PT: usize = 1;
const ANGLE: usize = 2;
const LEG: usize = 3;
const SHOTS: usize = 4;
/// Facing is independent of the serial: bit 8 belongs to command 16.
const COMMAND_FLIP: usize = 5;

/// `$7C`: part A (routines `$00`-`$2C`, the head at the core position:
/// idle `$2BE`, roar `$3058C`, the flame sweep, destroyed `$30536`) or
/// part B (routines `$30`-`$54`, invisible, 40 px below it: the orbs and
/// the two hands).
pub(crate) struct MechPart {
    idle: u16,
    roar: Vec<(u16, u16)>,
    down: Vec<(u16, u16)>,
    /// `$31156`: (angle limit, frame) of the sweep.
    sweep: Vec<(u16, u16)>,
    flame: Vec<(u16, u16)>,
    flame_damage: u8,
    fire: u8,
    boom: u8,
    /// `$31782`: (hold, dx, dy) of the three orbs.
    orbs: [(u16, i32, i32); 3],
    orb_frames: [u16; 2],
    orb_damage: u8,
    /// `Data_01F68E` and `Data_034A00`, for the orbs' aim (`$3167C`).
    slopes: [u8; 64],
    circle: Vec<(i32, i32)>,
    power_shot: u8,
    hands: [(u16, i32); 2],
    /// `obj_tile` of the head and hands records (`Data_030F56`/`$30F5E`).
    /// `Func_031418` copies the player's bit 15 over it each frame; the
    /// player is always high here (`$30654`), so the records' bit stands.
    tiles: [u16; 2],
    /// `obj_tile` of the flame and orb records (`Data_0315DC`/`$31794`).
    flame_tile: u16,
    orb_tile: u16,
}
impl MechPart {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let mut sweep = Vec::new();
        for i in 0..6 {
            sweep.push((word(rom, 0x31156 + i * 4)?, word(rom, 0x31158 + i * 4)?));
        }
        let orb = |p: usize| -> Result<(u16, i32, i32)> {
            Ok((word(rom, p)?, signed(rom, p + 2)?, signed(rom, p + 4)?))
        };
        Ok(Self {
            idle: word(rom, 0x3058a)?,
            roar: timed(rom, 0x3058c)?,
            down: timed(rom, 0x30536)?,
            sweep,
            flame: timed(rom, 0x30572)?,
            // Record $315DC: damage 3; record $31794: damage 1.
            flame_damage: 3,
            fire: Table::Effect.command(rom, 0x2e)?,
            boom: Table::Effect.command(rom, 0x33)?,
            orbs: [orb(0x31782)?, orb(0x31788)?, orb(0x3178e)?],
            orb_frames: [0x2ac, 0x2ad],
            orb_damage: 1,
            slopes: slopes(rom)?,
            circle: half_turn(rom)?,
            power_shot: Table::Effect.command(rom, 0x30)?,
            hands: [(0x2c5, 0x48), (0x2c6, -0x38)],
            tiles: [word(rom, 0x30f56 + 4)?, word(rom, 0x30f5e + 4)?],
            flame_tile: word(rom, 0x315dc + 4)?,
            orb_tile: word(rom, 0x31794 + 4)?,
        })
    }
    fn head(&self, cx: &mut Ctx, e: &mut Enemy, command: i32) {
        let t = e.vars[PT];
        e.vars[PT] += 1;
        match command {
            HEAD_ROAR | HEAD_FLAME if t < ROAR_TICKS - i32::from(command == HEAD_FLAME) => {
                e.mapping = frame_at(&self.roar, t).unwrap_or(self.idle);
            }
            HEAD_FLAME => {
                if t == ROAR_TICKS - 1 {
                    // Leg 0: toward $40 (unflipped) or $1C0, then across,
                    // then back to $100.
                    e.vars[ANGLE] = 0x100;
                    e.vars[LEG] = 0;
                    cx.play(self.fire);
                }
                self.sweep(cx, e);
            }
            HEAD_DOWN => {
                if t == 0 {
                    cx.play(self.boom);
                }
                e.mapping = frame_at(&self.down, t).unwrap_or(0);
            }
            _ => e.mapping = self.idle,
        }
    }
    /// `$311A0`: the jaw sweeps 180 degrees, a flame every fourth tick on
    /// the outward legs (`$314F2`).
    fn sweep(&self, cx: &mut Ctx, e: &mut Enemy) {
        let tick = e.vars[PT] - 1 - (ROAR_TICKS - 1);
        let Some((angle, emits)) = sweep_pose(tick, e.vars[COMMAND_FLIP] != 0) else {
            e.vars[LEG] = 3;
            e.mapping = self.idle;
            return;
        };
        e.vars[ANGLE] = angle;
        e.vars[LEG] = if emits { 0 } else { 2 };
        e.mapping = self
            .sweep
            .iter()
            .find(|(limit, _)| angle <= i32::from(*limit))
            .map_or(self.idle, |(_, frame)| *frame);
        e.facing_left = angle < 0x100;
        if emits {
            if e.vars[SHOTS] & 3 == 0 {
                let (c, s) = cx.shared.sine.polar(angle as u16);
                let mut flame = Projectile::new(e.x, e.y - 0x20, 0, true, 0, self.flame_damage)
                    .with_motion(Motion {
                        vx: c * 5,
                        vy: (s - (s >> 2)) * 5,
                        ..Motion::default()
                    })
                    .box_after();
                flame.base = 0x4000;
                flame.passes_terrain = true;
                flame.owner = Some(cx.current);
                flame.life = 46;
                flame.mech = Some(MechShot::Flame);
                cx.shoot_tiled_capped(flame, self.flame_tile, 16);
            }
            e.vars[SHOTS] += 1;
        }
    }
    /// `$30572` frames of the flames, and their end at Y $1C8.
    fn steer_flames(&self, cx: &mut Ctx) {
        let mut puffs = Vec::new();
        for shot in cx
            .projectiles
            .iter_mut()
            .filter(|p| p.owner == Some(cx.current) && p.alive)
        {
            if let Some(frame) = frame_at(&self.flame, i32::from(shot.age)) {
                shot.mapping = frame;
            } else {
                shot.alive = false;
                continue;
            }
            if shot.y >= 0x1c8 {
                shot.alive = false;
                puffs.push((shot.x, shot.y + 8));
            }
        }
        for at in puffs {
            smoke(cx, at, (0, -0x4000), 0, 4);
        }
    }
    /// `$3167C`: 4 px/tick at the player's chest, `Func_01F5DE`'s
    /// 128-step aim through `Func_01F820`, then doubled twice.
    fn orb_velocity(&self, orb: (i32, i32), player: (i32, i32)) -> (i32, i32) {
        let angle = aim_angle(&self.slopes, orb, (player.0, player.1 - 0x10));
        let (vx, vy) = unit_vector(&self.circle, angle);
        (vx << 2, vy << 2)
    }
    /// Part B: three orbs held, then fired at the player (`$3172C`).
    fn hands(&self, cx: &mut Ctx, e: &mut Enemy, fire: bool, terrain: &dyn Terrain) {
        if fire {
            for (i, &(_, dx, dy)) in self.orbs.iter().enumerate() {
                let shift = if e.facing_left { 2 } else { -2 };
                let mut orb =
                    Projectile::new(e.x + shift + dx, e.y + dy, 0, true, self.orb_frames[0], 0)
                        .box_after();
                orb.base = 0x4000;
                orb.passes_terrain = true;
                orb.no_collision = true;
                orb.owner = Some(cx.current);
                orb.life = u16::MAX;
                orb.mech = Some(MechShot::Orb {
                    index: i as u8,
                    launched: None,
                    hit: false,
                });
                cx.shoot_tiled_capped(orb, self.orb_tile, 16);
            }
        }
        let mut launches = 0;
        let mut sparks = Vec::new();
        let mut trails = Vec::new();
        let mut impacts = Vec::new();
        for orb in cx
            .projectiles
            .iter_mut()
            .filter(|p| p.owner == Some(cx.current) && p.alive)
        {
            let Some(MechShot::Orb {
                index,
                mut launched,
                hit,
            }) = orb.mech
            else {
                continue;
            };
            let (hold, dx, dy) = self.orbs[usize::from(index)];
            orb.mapping = self.orb_frames[usize::from(orb.age % 3 == 2)];
            if launched.is_none() {
                orb.x = e.x + if e.facing_left { 2 } else { -2 } + dx;
                orb.y = e.y + dy;
                if (hold - orb.age) & 7 == 0 {
                    sparks.push((orb.x, orb.y));
                }
                if orb.age + 1 >= hold {
                    let (vx, vy) = self.orb_velocity((orb.x, orb.y), cx.player);
                    orb.motion = Motion {
                        vx,
                        vy,
                        ..Motion::default()
                    };
                    orb.damage = self.orb_damage;
                    orb.no_collision = false;
                    launched = Some(orb.age);
                    launches += 1;
                }
            }
            if let Some(start) = launched {
                if (orb.age - start + 1) & 3 == 0 {
                    trails.push((orb.x, orb.y, orb.motion.fx, orb.motion.fy));
                }
                if hit || orb.x >= 0x1c0 || terrain.solid_at(orb.x, orb.y) {
                    orb.alive = false;
                    impacts.push((orb.x, orb.y));
                }
            }
            orb.mech = Some(MechShot::Orb {
                index,
                launched,
                hit,
            });
        }
        for _ in 0..launches {
            cx.play(self.power_shot);
        }
        for at in sparks {
            let before = cx.effects.len();
            cx.charge_sparkle(at, 0);
            if cx.effects.len() > before {
                let spark = cx.effects.last_mut().unwrap();
                spark.kind = EffectKind::MechParticle;
                spark.phase = 1;
                spark.high_priority = true;
            }
        }
        for (x, y, fx, fy) in trails {
            let mut trail = Effect::new(EffectKind::MechParticle, x, y, 0x368);
            trail.phase = 1;
            trail.base = 0x8000;
            trail.high_priority = true;
            trail.motion.fx = fx;
            trail.motion.fy = fy;
            cx.effect(trail);
        }
        for at in impacts {
            cx.visible_sound(at.0, at.1, 0x12);
            particles(cx, at, 4);
        }
        let positions = self.hands.map(|(_, dx)| {
            (
                e.x + if e.facing_left { -dx } else { dx },
                e.y - (HAND_UP - HANDS_UP),
            )
        });
        cx.signal(Signal::MechHands(Some(positions)));
    }
}
impl Behavior for MechPart {
    fn label(&self, _kind: u16) -> String {
        "Enemy7C $30FAC (Boss5 parts)".into()
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        let Some(boss) = cx.parent(e).filter(|boss| boss.live) else {
            Ctx::free(e);
            return;
        };
        let hands = e.routine >= 0x30;
        e.tile = Some(self.tiles[usize::from(hands)]);
        e.x = boss.x;
        e.y = if hands {
            boss.y + CORE_UP - HANDS_UP
        } else {
            boss.y
        };
        let slot = if hands { HANDS_CMD } else { HEAD_CMD };
        let command = boss.vars[slot];
        let fresh = command != e.vars[SEEN];
        if fresh {
            e.vars[SEEN] = command;
            e.vars[COMMAND_FLIP] = i32::from(boss.facing_left);
            e.vars[PT] = 0;
        }
        if hands {
            e.mapping = 0;
            e.facing_left = boss.facing_left;
            self.hands(cx, e, fresh && command & 15 == 1, terrain);
        } else {
            // The sweep turns the head itself; otherwise it copies the boss.
            let sweeping = command & 15 == HEAD_FLAME && e.vars[PT] > ROAR_TICKS && e.vars[LEG] < 3;
            if !sweeping {
                e.facing_left = boss.facing_left;
            }
            self.head(cx, e, command & 15);
            self.steer_flames(cx);
        }
    }
    fn hit(&self, _enemy: &Enemy, _kind: u16) -> HitResponse {
        HitResponse::Pass
    }
    fn integrates(&self) -> bool {
        false
    }
}
