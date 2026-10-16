//! Round 1 forest types behind the handler interface: `$14` ninja
//! (`$1490C`), `$24` komuso patrol (`$15CEE`), `$44` hopper (`$1699C`) and the
//! `$B4`/`$B8` samurai (`$1BB1C`/`$1C5F0`). Each handler integrates its own motion.
use super::{
    Behavior, Ctx, HitResponse, Offsets, Roster, aim, frame_at, frame_list, set_fraction,
    shot_damage,
};
use crate::{
    combat::{
        Effect, EffectKind, Enemy, EnemyState, Launch, Motion, PaletteOp, Projectile, Reaction,
        Signal, Spawn, advance, pose_hitbox,
    },
    game::Terrain,
    movement::PlayerEvent,
    rom::{Result, byte, long, signed, word, word_i, words},
    sound_events::SoundEvent,
};
use std::{collections::BTreeSet, sync::Arc};

pub(crate) mod props;
mod rules;
pub(crate) use rules::Rules;

pub(crate) const KINDS: &[u16] = &[0x14, 0x24, 0x44, 0xb4, 0xb8];

/// `obj_routine` of the `$B8` entrance: `$1C672` waits off screen while
/// the player is walked in, `$1C6C8` paces in. Routine 0 is the shared
/// idle/attack loop the native state enum drives (`$1BC14`, `obj_routine` 4).
const LEADER_WAIT: u16 = 0x20;
const LEADER_ENTER: u16 = 0x24;

pub(crate) fn register(rom: &[u8], roster: &mut Roster) -> Result<()> {
    let forest: Arc<dyn Behavior> = Arc::new(Forest {
        rules: Rules::load(rom)?,
        round6_ninjas: super::round4::round_sources(rom, 6, 0x14)?,
    });
    for &kind in KINDS {
        roster.add(kind, forest.clone());
    }
    Ok(())
}

/// `Func_01B9F8`: palette-line-1 colors 0..7 from the row of the samurai's
/// `obj_health` byte (native `health - 1`). The line is global CRAM, so the
/// sickle and anything else on line 1 shows it too.
fn samurai_health_palette(cx: &mut Ctx, enemy: &Enemy) {
    let row = usize::from(enemy.health.saturating_sub(1).min(5));
    let words = cx.shared.samurai_health_palettes[row];
    cx.palette_op(PaletteOp::Words(1, 0, words));
}

/// `Func_01BEC4`: the samurai's white flash over line-1 colors 0..7.
fn samurai_flash(cx: &mut Ctx) {
    let white = cx.shared.flash_color;
    cx.palette_op(PaletteOp::FillFirst(1, 8, white));
}

/// The five forest handlers, sharing one set of recovered tables.
pub(crate) struct Forest {
    pub(crate) rules: Rules,
    round6_ninjas: BTreeSet<usize>,
}

impl Behavior for Forest {
    fn label(&self, kind: u16) -> String {
        match kind {
            0x14 => "Enemy14_Ninja $1490C",
            0x24 => "Enemy24_Komuso $15CEE",
            0x44 => "Enemy44 $1699C",
            0xb4 => "EnemyB4_Samurai $1BB1C",
            _ => "EnemyB8_SamuraiLeader $1C5F0",
        }
        .to_string()
    }
    fn initial_mapping(&self, kind: u16) -> u16 {
        self.rules.idle(kind)
    }
    fn activate(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn) {
        let rules = &self.rules;
        enemy.mapping = rules.idle(spawn.kind);
        enemy.state = match spawn.kind {
            // $14998: spawn flag bit 4 (object +0 bit 2) shows the ninja on its
            // perch; otherwise it hides until the $14A62 leaves announce a drop.
            0x14 if spawn.flags & 0x10 == 0 => EnemyState::Emerging,
            // $169CE: hidden until the player is within range.
            0x44 => EnemyState::Emerging,
            0x24 => {
                // $15D42: walk left with the step at $15CAC and no flip.
                enemy.step_x = rules.patrol_step;
                enemy.facing_left = false;
                EnemyState::Waiting
            }
            0xb4 | 0xb8 => {
                enemy.timer = rules.heavy_idle_ticks;
                if spawn.kind == 0xb4 {
                    // $1BB98: Func_01BEF6 before the idle's Func_01B9F8.
                    let entry = cx.shared.samurai_entry;
                    cx.palette_op(PaletteOp::Words(1, 1, entry));
                }
                if spawn.kind == 0xb8 {
                    // $1C620: off screen at camera + $1C0, with the rock
                    // wall and gate of its arena (`props`); routine $20
                    // runs in the same update.
                    enemy.x = cx.camera.0 + rules.leader_offset;
                    enemy.routine = LEADER_WAIT;
                    for prop in cx.shared.leader_props.spawn() {
                        cx.effect(prop);
                    }
                    self.leader_wait(cx, enemy);
                } else {
                    samurai_health_palette(cx, enemy);
                }
                EnemyState::Waiting
            }
            _ => EnemyState::Waiting,
        };
        Self::shove(cx, enemy, spawn);
    }
    fn update(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        Self::shove(cx, enemy, spawn);
        match enemy.reaction.take() {
            Some(Reaction::Hurt) => {
                self.start_hurt(cx, enemy, spawn);
                return;
            }
            Some(Reaction::Deflect) => {
                let sound = cx.shared.deflect_sound;
                cx.play(sound);
                cx.spark(enemy, false);
            }
            _ => {}
        }
        match spawn.kind {
            0x14 => self.ninja(cx, enemy, spawn, terrain),
            0x24 => self.patrol(cx, enemy, terrain),
            0x44 => self.hopper(cx, enemy, terrain),
            _ => self.heavy(cx, enemy, spawn),
        }
    }
    fn dying(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        if enemy.phase == 0 {
            Self::shove(cx, enemy, spawn);
        }
        // The launch/first pose belongs to the reacting update.
        if enemy.reaction.take() == Some(Reaction::Death) {
            self.start_death(cx, enemy, spawn);
        } else {
            self.dying_step(cx, enemy, spawn, terrain);
        }
    }
    fn hit(&self, enemy: &Enemy, kind: u16) -> HitResponse {
        let heavy = matches!(kind, 0xb4 | 0xb8);
        // The knockback arc ($24: $15EA4, $15ED0) never calls $127C0, so
        // shurikens fly through and the body cannot touch Joe. ($14 and $44
        // have health counter 0 and never reach it.)
        if enemy.state == EnemyState::Hurt && !heavy {
            return HitResponse::Pass;
        }
        // $1C08A: the heavy actor is only damaged after its attack frame and
        // outside the hurt flash; otherwise $368EC deflects the weapon.
        // $1C672/$1C6C8: the entrance only tests body contact ($37244).
        if kind == 0xb8 && matches!(enemy.routine, LEADER_WAIT | LEADER_ENTER) {
            return HitResponse::BodyOnly;
        }
        if heavy && (!enemy.vulnerable || enemy.flash > 0) {
            return HitResponse::Deflect;
        }
        HitResponse::Accept
    }
    fn integrates(&self) -> bool {
        false
    }
}

impl Forest {
    /// `$1BC14`, `$1BD28`, `$1BE78`, `$1BF92`, `$1C672` and `$1C6C8`
    /// call `$37244` before weapon contact or advancing the pose. The
    /// pusher ignores Joe's attack/immunity flags and collision priority.
    fn shove(cx: &mut Ctx, enemy: &Enemy, spawn: &Spawn) {
        if matches!(spawn.kind, 0xb4 | 0xb8) {
            cx.signal(Signal::Shove(pose_hitbox(cx.mappings, enemy)));
        }
    }

    fn start_hurt(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn) {
        let rules = &self.rules;
        cx.spark(enemy, false);
        match spawn.kind {
            0xb4 | 0xb8 => {
                // $1BE42: pose $175, 40 ticks, white palette for the first ten.
                enemy.mapping = rules.heavy_hurt_mapping;
                enemy.timer = rules.heavy_hurt;
                enemy.flash = rules.heavy_flash;
                samurai_flash(cx);
            }
            0x24 => Self::knock(cx, enemy, rules.patrol_knock, rules.patrol_hit),
            0x44 => {
                let fall = cx.shared.fall;
                Self::knock(cx, enemy, fall, rules.hopper[4]);
            }
            _ => Self::knock(cx, enemy, rules.ninja_fall, rules.ninja_death),
        }
        enemy.state = EnemyState::Hurt;
    }
    /// Launch away from the player: +$38 negative (player left) gives +vx.
    fn knock(cx: &Ctx, enemy: &mut Enemy, launch: Launch, mapping: u16) {
        let away = if cx.player.0 < enemy.x { 1 } else { -1 };
        enemy.mapping = mapping;
        enemy.launch_y = enemy.y;
        enemy.motion.vx = away * launch.vx;
        enemy.motion.vy = launch.vy;
        enemy.motion.gravity = launch.gravity;
        enemy.motion.step(&mut enemy.x, &mut enemy.y);
    }
    fn start_death(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn) {
        let rules = &self.rules;
        enemy.phase = 0;
        enemy.counter = 0;
        enemy.timer = 0;
        match spawn.kind {
            0x14 => Self::knock(cx, enemy, rules.ninja_fall, rules.ninja_death),
            0x24 => Self::knock(cx, enemy, rules.patrol_knock, rules.patrol_hit),
            0x44 => {
                let fall = cx.shared.fall;
                Self::knock(cx, enemy, fall, rules.hopper[4]);
            }
            _ => {
                // $1BF12: flash, slide away at 0.5 px and the lethal spark at +/-40.
                enemy.flash = rules.heavy_death_flash;
                samurai_flash(cx);
                enemy.motion.stop();
                enemy.motion.vx = if enemy.facing_left {
                    -rules.heavy_slide
                } else {
                    rules.heavy_slide
                };
                let (x, y) = (
                    enemy.x + if enemy.facing_left { 40 } else { -40 },
                    enemy.y - 8,
                );
                if enemy.death_spark && !enemy.spark_handled {
                    let first = cx.shared.kill_seq[0];
                    cx.effect(Effect::new(EffectKind::KillSpark, x, y, first));
                }
                return;
            }
        }
        if enemy.death_spark {
            cx.spark(enemy, true);
        }
    }
    fn dying_step(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        let rules = &self.rules;
        match spawn.kind {
            0x24 => {
                // $15E82: burst once the arc drops below its launch height.
                if enemy.y > enemy.launch_y {
                    let base = cx.base(0x24);
                    cx.debris(enemy, base);
                    Ctx::remove(enemy);
                    return;
                }
                enemy.motion.step(&mut enemy.x, &mut enemy.y);
            }
            0xb4 | 0xb8 => {
                if enemy.flash == 1 {
                    // $1BFB4: the flash's last tick installs the death colors.
                    let death = cx.shared.samurai_death;
                    cx.palette_op(PaletteOp::Words(1, 1, death));
                }
                let big = usize::from(spawn.kind == 0xb8);
                // Stop inside the opening at the broken rock pile's ROM X.
                // The old camera+$160 target lay past the doorway's floor.
                let props = &cx.shared.leader_props;
                let doorway_x = props
                    .boulders
                    .iter()
                    .map(|&(dx, _, _)| props.boulder_x + dx)
                    .max()
                    .unwrap_or(cx.camera.0 + crate::WIDTH as i32);
                if enemy.phase == 0 {
                    match frame_at(&rules.heavy_death, enemy.counter) {
                        Some((_, mapping, _)) => {
                            enemy.mapping = mapping;
                            enemy.counter += 1;
                            enemy.motion.step(&mut enemy.x, &mut enemy.y);
                        }
                        None => {
                            // $1BFC6: explode, then wait 48 ($B4) or 120 ($B8) ticks.
                            let blast = cx.shared.blasts[big];
                            cx.explode(enemy.x, enemy.y, blast);
                            if big == 1 {
                                // $1BFE6: the finale requests STAGE CLEAR ($98).
                                let cue = cx.shared.cues.get(SoundEvent::StageClear);
                                cx.play(cue);
                            }
                            enemy.phase = 1;
                            enemy.mapping = 0;
                            enemy.timer = rules.heavy_after[big];
                        }
                    }
                } else if enemy.phase == 1 {
                    enemy.timer = enemy.timer.saturating_sub(1);
                    if enemy.timer == 0 {
                        if big == 0 {
                            Ctx::remove(enemy);
                        } else {
                            // Walk through the broken opening after the
                            // original 120-tick clear jingle.
                            enemy.phase = 2;
                        }
                    }
                } else if cx.player.0 < (cx.camera.0 + 0x160).min(doorway_x) {
                    cx.player_event(PlayerEvent::WalkRight);
                } else {
                    cx.next_substage();
                    Ctx::remove(enemy);
                }
            }
            _ => {
                // $12E3E: explode on terrain while falling, or when offscreen.
                let offscreen = cx.offscreen(enemy.x, enemy.y);
                if offscreen || (enemy.motion.vy >= 0 && terrain.solid_at(enemy.x, enemy.y)) {
                    let blast = cx.shared.blasts[0];
                    cx.explode(enemy.x, enemy.y, blast);
                    Ctx::remove(enemy);
                    return;
                }
                enemy.motion.step(&mut enemy.x, &mut enemy.y);
            }
        }
    }
    fn release_star(&self, cx: &mut Ctx, enemy: &Enemy, airborne: bool) {
        // $14C30: projectile ID 4; $1D1CE offsets +/-8,-43, with the table's
        // +16 posture adjustment inactive because this ninja never sets bit 1.
        let direction = if enemy.facing_left { 1 } else { -1 };
        let damage = cx.star_damage;
        let (x, y) = self
            .rules
            .star_offset
            .place(enemy.x, enemy.y, enemy.facing_left, false);
        // `Func_01D1D6` sets the frame after its first `Object_LoadHitbox`.
        let mut star = Projectile::new(x, y, 0, true, 0x35b, damage).unset_frame();
        if airborne {
            // $14C6A -> $15280: Func_009562's direction, multiplied by 3.
            let (vx, vy) = aim((enemy.x, enemy.y), cx.player);
            star = star.with_motion(Motion {
                vx: vx * 3,
                vy: vy * 3,
                ..Motion::default()
            });
        } else {
            star.velocity_x = direction * (self.rules.star_speed >> 16);
        }
        if cx.shoot(star) {
            let sound = cx.throw_sound;
            cx.play(sound);
        }
    }
    fn ninja_throw(&self, cx: &mut Ctx, enemy: &mut Enemy, airborne: bool) {
        let pose = usize::from(airborne) * 2;
        enemy.state = EnemyState::Throwing;
        enemy.phase = 0;
        enemy.counter = u16::from(airborne);
        enemy.mapping = self.rules.ninja_throw[pose];
        enemy.timer = self.rules.ninja_windup;
        self.release_star(cx, enemy, airborne);
    }
    /// `$14B30..$14B44`: round 6 waits 240 ticks instead of 60, giving
    /// the climbing camera time to bring a perched ninja into view.
    fn ninja_wait(&self, spawn: &Spawn) -> u16 {
        if self.round6_ninjas.contains(&spawn.source) {
            self.rules.ninja_wait_round6
        } else {
            self.rules.ninja_wait
        }
    }
    /// Land on terrain while falling: snap to the 16-pixel row ($14B24).
    fn ninja_landed(&self, enemy: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) -> bool {
        if enemy.motion.vy < 0 || !terrain.solid_at(enemy.x, enemy.y) {
            return false;
        }
        enemy.y &= !15;
        enemy.motion.stop();
        enemy.state = EnemyState::Waiting;
        enemy.counter = 1;
        enemy.timer = self.ninja_wait(spawn);
        enemy.mapping = self.rules.ninja_body[2];
        true
    }
    fn ninja(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        let rules = &self.rules;
        let dx = cx.player.0 - enemy.x;
        // $14DD8: flip while the player is to the right (the art faces left).
        enemy.facing_left = dx >= 0;
        match enemy.state {
            EnemyState::Emerging => {
                if enemy.phase == 0 {
                    if dx.abs() < rules.ninja_range {
                        enemy.phase = 1;
                        enemy.counter = rules.ninja_leaves + 1;
                    }
                } else if enemy.age & rules.ninja_leaf_mask == 0 {
                    cx.leaf(enemy.x, enemy.y);
                    enemy.counter = enemy.counter.saturating_sub(1);
                    if enemy.counter == 0 {
                        // $14A78: drop from the canopy with vx $10000.
                        enemy.state = EnemyState::Airborne;
                        enemy.mapping = rules.ninja_body[1];
                        enemy.motion.vx = rules.ninja_drop;
                        enemy.motion.gravity = rules.ninja_gravity;
                    }
                }
            }
            EnemyState::Waiting => {
                enemy.mapping = rules.ninja_body[2];
                if enemy.counter == 0 {
                    // $149DE: |dx| below 192 primes the round's wait.
                    if dx.abs() < rules.ninja_range {
                        enemy.counter = 1;
                        enemy.timer = self.ninja_wait(spawn);
                    }
                } else if enemy.timer == 0 {
                    // $14B6A: subq.w #1,$2A / bpl: the throw comes on the
                    // update that takes the wait below zero, 61 updates
                    // after 60 was set (241 after 240 in round 6).
                    self.ninja_throw(cx, enemy, false);
                } else {
                    enemy.timer -= 1;
                }
            }
            EnemyState::Throwing => {
                let airborne = enemy.counter == 1;
                if airborne {
                    if self.ninja_landed(enemy, spawn, terrain) {
                        return;
                    }
                    enemy.motion.step(&mut enemy.x, &mut enemy.y);
                }
                enemy.timer = enemy.timer.saturating_sub(1);
                if enemy.timer == 0 {
                    let pose = usize::from(airborne) * 2;
                    if enemy.phase == 0 {
                        enemy.phase = 1;
                        enemy.mapping = rules.ninja_throw[pose + 1];
                        enemy.timer = rules.ninja_follow;
                    } else if airborne {
                        enemy.state = EnemyState::Airborne;
                        enemy.mapping = rules.ninja_body[1];
                        enemy.counter = 0;
                    } else {
                        enemy.state = EnemyState::Recovering;
                        enemy.phase = 0;
                        enemy.mapping = rules.ninja_rest;
                        enemy.timer = rules.ninja_rest_ticks;
                    }
                }
            }
            EnemyState::Recovering => {
                enemy.mapping = rules.ninja_rest;
                enemy.timer = enemy.timer.saturating_sub(1);
                if enemy.timer == 0 {
                    if enemy.phase == 0 {
                        enemy.phase = 1;
                        enemy.timer = rules.ninja_pause;
                    } else {
                        // $14BA6: flag bit 0 jumps at -6 plus a random fraction;
                        // otherwise the player's height selects $14BD6/$14BE2.
                        let vy = if spawn.flags & 1 != 0 {
                            set_fraction(rules.ninja_jump[0], cx.random())
                        } else if cx.player.1 < enemy.y {
                            rules.ninja_jump[1]
                        } else {
                            rules.ninja_jump[2]
                        };
                        enemy.state = EnemyState::Airborne;
                        enemy.counter = 1;
                        enemy.mapping = rules.ninja_body[1];
                        enemy.motion.vx = 0;
                        enemy.motion.vy = vy;
                        enemy.motion.gravity = rules.ninja_gravity;
                        enemy.motion.step(&mut enemy.x, &mut enemy.y);
                    }
                }
            }
            EnemyState::Airborne => {
                if self.ninja_landed(enemy, spawn, terrain) {
                    return;
                }
                let before = enemy.motion.vy;
                enemy.motion.gravity = rules.ninja_gravity;
                enemy.motion.step(&mut enemy.x, &mut enemy.y);
                enemy.mapping = rules.ninja_body[1];
                // $14AC2: one throw ($135/$136) when the jump passes its apex.
                if enemy.counter == 1 && before < 0 && enemy.motion.vy >= 0 {
                    self.ninja_throw(cx, enemy, true);
                } else if cx.offscreen(enemy.x, enemy.y) {
                    enemy.state = EnemyState::Defeated;
                }
            }
            EnemyState::Hurt => {
                if self.ninja_landed(enemy, spawn, terrain) {
                    return;
                }
                enemy.motion.step(&mut enemy.x, &mut enemy.y);
            }
            _ => {}
        }
    }
    /// `$15F04`: ground must continue 16 pixels either side; walls stop the step.
    fn patrol_blocked(terrain: &dyn Terrain, x: i32, y: i32) -> bool {
        !terrain.solid_at(x + 16, y + 8)
            || !terrain.solid_at(x - 16, y + 8)
            || terrain.solid_at(x + 16, y - 8)
            || terrain.solid_at(x - 16, y - 8)
    }
    fn patrol_attack(&self, cx: &Ctx, enemy: &mut Enemy) {
        // $15DC0: pose $1B8, flipped while the player is left, 16-tick windup.
        enemy.state = EnemyState::Throwing;
        enemy.phase = 0;
        enemy.mapping = self.rules.patrol[3];
        enemy.facing_left = cx.player.0 < enemy.x;
        enemy.timer = self.rules.patrol_windup;
    }
    fn patrol(&self, cx: &mut Ctx, enemy: &mut Enemy, terrain: &dyn Terrain) {
        let rules = &self.rules;
        match enemy.state {
            EnemyState::Waiting => {
                if enemy.age & rules.patrol_period == 0 {
                    self.patrol_attack(cx, enemy);
                    return;
                }
                if enemy.age & rules.patrol_cadence == 0 {
                    // $15D78: alternate $1B5/$1B6 and step 3.75 px each 16 ticks.
                    enemy.counter ^= 1;
                    enemy.mapping = rules.patrol[usize::from(enemy.counter & 1)];
                    let (x, fraction) = (enemy.x, enemy.motion.fx);
                    advance(&mut enemy.x, &mut enemy.motion.fx, enemy.step_x);
                    if Self::patrol_blocked(terrain, enemy.x, enemy.y) {
                        enemy.x = x;
                        enemy.motion.fx = fraction;
                        enemy.step_x = -enemy.step_x;
                        enemy.facing_left = !enemy.facing_left;
                    }
                }
            }
            EnemyState::Throwing => {
                enemy.timer = enemy.timer.saturating_sub(1);
                if enemy.timer > 0 {
                    return;
                }
                if enemy.phase == 0 {
                    // $15F70: projectile 39 with mapping $1B9 at $15F6C offsets.
                    let (dx, dy) = rules.knife_offset[usize::from(enemy.facing_left)];
                    let speed = rules.knife_speed >> 16;
                    let base = cx.base(0x24);
                    cx.shoot(
                        Projectile::new(
                            enemy.x + dx,
                            enemy.y + dy,
                            if enemy.facing_left { -speed } else { speed },
                            true,
                            rules.patrol[4],
                            rules.knife_damage,
                        )
                        .with_art(base, false),
                    );
                    enemy.phase = 1;
                    enemy.timer = rules.patrol_recover;
                } else {
                    // $15E34/$15D42: back to $1B5, walking left without flip.
                    enemy.state = EnemyState::Waiting;
                    enemy.mapping = rules.patrol[0];
                    enemy.step_x = rules.patrol_step;
                    enemy.facing_left = false;
                }
            }
            EnemyState::Hurt => {
                // $15ED0: no landing test while rising; a wall cancels vx.
                if enemy.motion.vy >= 0 {
                    if terrain.solid_at(enemy.x, enemy.y) {
                        enemy.y &= !15;
                        enemy.motion.stop();
                        self.patrol_attack(cx, enemy);
                        return;
                    }
                    if terrain.solid_at(enemy.x + 16, enemy.y - 8)
                        || terrain.solid_at(enemy.x - 16, enemy.y - 8)
                    {
                        enemy.motion.vx = 0;
                    }
                }
                enemy.motion.step(&mut enemy.x, &mut enemy.y);
                if cx.offscreen(enemy.x, enemy.y) {
                    enemy.state = EnemyState::Defeated;
                }
            }
            _ => {
                enemy.state = EnemyState::Waiting;
            }
        }
    }
    fn hopper_leap(&self, cx: &mut Ctx, enemy: &mut Enemy) {
        let rules = &self.rules;
        // $16BEC: -6 px/tick, gravity $2000 plus a random low part, +/-2 toward the player.
        let toward = if cx.player.0 < enemy.x { -1 } else { 1 };
        enemy.facing_left = cx.player.0 >= enemy.x;
        enemy.motion.vx = set_fraction(toward * rules.hopper_run, cx.random());
        enemy.motion.vy = set_fraction(rules.hopper_leap, cx.random());
        enemy.motion.gravity =
            rules.hopper_gravity | i32::from(cx.random() & rules.hopper_gravity_mask);
        enemy.mapping = rules.hopper[0];
        enemy.counter = 0;
        enemy.state = EnemyState::Airborne;
        enemy.motion.step(&mut enemy.x, &mut enemy.y);
    }
    fn hopper(&self, cx: &mut Ctx, enemy: &mut Enemy, terrain: &dyn Terrain) {
        let rules = &self.rules;
        match enemy.state {
            EnemyState::Emerging => {
                if enemy.phase == 0 {
                    if cx.player_dx(enemy.x) < rules.hopper_range {
                        enemy.phase = 1;
                        enemy.counter = rules.hopper_leaves + 1;
                    }
                } else if enemy.age & rules.hopper_leaf_mask == 0 {
                    cx.leaf(enemy.x, enemy.y);
                    enemy.counter = enemy.counter.saturating_sub(1);
                    if enemy.counter == 0 {
                        // $169FC: appear falling at +3 px/tick with gravity $2000.
                        enemy.facing_left = cx.player.0 >= enemy.x;
                        enemy.mapping = rules.hopper[0];
                        enemy.motion.vx = 0;
                        enemy.motion.vy = rules.hopper_drop;
                        enemy.motion.gravity = rules.hopper_drop_gravity;
                        enemy.state = EnemyState::Airborne;
                    }
                }
            }
            EnemyState::Airborne => {
                // $16A38: pose by vertical speed; the first falling ticks use $1E3.
                let mut mapping = rules.hopper[0];
                if enemy.motion.vy.abs() < rules.hopper_apex {
                    mapping = rules.hopper[2];
                }
                if enemy.motion.vy >= 0 {
                    mapping = rules.hopper[3];
                    enemy.counter = enemy.counter.saturating_add(1);
                    if enemy.counter < rules.hopper_early {
                        mapping = rules.hopper[5];
                    }
                    // $16A78: land when the probe 24 pixels above the feet is solid.
                    if terrain.solid_at(enemy.x, enemy.y + rules.hopper_probe) {
                        enemy.motion.stop();
                        enemy.y &= !15;
                        enemy.mapping = rules.hopper[1];
                        enemy.timer = rules.hopper_crouch + 1;
                        enemy.counter = 0;
                        let (behind, ahead) = rules.hopper_home;
                        if enemy.home_x - enemy.x >= behind || enemy.x - enemy.home_x >= ahead {
                            // $16B68: leave upward and vanish at the apex.
                            enemy.state = EnemyState::Recovering;
                            enemy.mapping = rules.hopper[2];
                            enemy.motion.vy = rules.hopper_leave;
                            enemy.motion.gravity = rules.hopper_gravity;
                        } else {
                            enemy.state = EnemyState::Waiting;
                        }
                        return;
                    }
                }
                enemy.mapping = mapping;
                enemy.motion.step(&mut enemy.x, &mut enemy.y);
                if cx.offscreen(enemy.x, enemy.y) {
                    enemy.state = EnemyState::Defeated;
                }
            }
            EnemyState::Waiting => {
                enemy.mapping = rules.hopper[1];
                enemy.timer = enemy.timer.saturating_sub(1);
                if enemy.timer == 0 {
                    self.hopper_leap(cx, enemy);
                }
            }
            EnemyState::Recovering => {
                if enemy.motion.vy >= 0 {
                    cx.poof(enemy.x, enemy.y - 32);
                    enemy.state = EnemyState::Defeated;
                    return;
                }
                enemy.motion.step(&mut enemy.x, &mut enemy.y);
            }
            EnemyState::Hurt => {
                // $16B44: land anywhere solid, then crouch 20 ticks.
                if enemy.motion.vy >= 0 && terrain.solid_at(enemy.x, enemy.y) {
                    enemy.motion.stop();
                    enemy.mapping = rules.hopper[1];
                    enemy.timer = rules.hopper_recover + 1;
                    enemy.state = EnemyState::Waiting;
                    return;
                }
                enemy.motion.step(&mut enemy.x, &mut enemy.y);
                if cx.offscreen(enemy.x, enemy.y) {
                    enemy.state = EnemyState::Defeated;
                }
            }
            _ => {
                enemy.state = EnemyState::Waiting;
            }
        }
    }
    fn heavy(&self, cx: &mut Ctx, enemy: &mut Enemy, spawn: &Spawn) {
        let rules = &self.rules;
        // $1BB32: flipped while the actor is not right of the player.
        enemy.facing_left = enemy.x <= cx.player.0;
        if enemy.flash == 1 {
            // $1BEA2: the hurt flash ends with the new health row.
            samurai_health_palette(cx, enemy);
        }
        match enemy.state {
            EnemyState::Waiting if spawn.kind == 0xb8 && enemy.routine == LEADER_WAIT => {
                self.leader_wait(cx, enemy);
            }
            EnemyState::Waiting if spawn.kind == 0xb8 && enemy.routine == LEADER_ENTER => {
                // $1C6DE: pace in; below screen X $1AC, $1BBFC starts the
                // idle loop with the health row.
                if self.pace(cx, enemy) < rules.leader_entered {
                    enemy.routine = 0;
                    enemy.counter = 0;
                    enemy.timer = rules.heavy_idle_ticks;
                    samurai_health_palette(cx, enemy);
                }
            }
            EnemyState::Waiting => {
                if spawn.kind == 0xb8 {
                    self.pace(cx, enemy);
                } else {
                    enemy.mapping = rules.heavy_idle;
                }
                if enemy.timer < rules.heavy_charge_ticks {
                    // $1BC52..$1BCAC: point 40 px toward the facing, jittered
                    // back by 0..31 px and upward by half that distance.
                    let jitter = i32::from(cx.random() & 0x1f);
                    let toward = if enemy.facing_left { 1 } else { -1 };
                    let point = (enemy.x + toward * (40 - jitter), enemy.y - 44 - jitter / 2);
                    cx.charge_sparkle(point, 0);
                }
                enemy.timer = enemy.timer.saturating_sub(1);
                if enemy.timer == 0 {
                    enemy.state = EnemyState::Throwing;
                    enemy.counter = 0;
                }
            }
            EnemyState::Throwing => {
                // $1BD28: frames from $1BDA2; the sickle leaves on the last $176 tick.
                match frame_at(&rules.heavy_attack, enemy.counter) {
                    Some((index, mapping, exact)) => {
                        enemy.mapping = mapping;
                        if index < 2 {
                            // $1BD48..$1BD88/$1BDB0: the first two attack
                            // frames converge on a point 16 px toward the
                            // facing, jittered back/up by 0..7 px.
                            let jitter = i32::from(cx.random() & 7);
                            let toward = if enemy.facing_left { 1 } else { -1 };
                            let point = (enemy.x + toward * (16 - jitter), enemy.y - 64 - jitter);
                            cx.charge_sparkle(point, 0);
                        }
                        if exact && index == usize::from(rules.heavy_release) {
                            self.release_sickle(cx, enemy, spawn.kind == 0xb8);
                            enemy.vulnerable = true;
                        }
                        enemy.counter += 1;
                    }
                    None => {
                        enemy.state = EnemyState::Recovering;
                        enemy.timer = rules.heavy_hold;
                    }
                }
            }
            EnemyState::Recovering | EnemyState::Hurt => {
                enemy.timer = enemy.timer.saturating_sub(1);
                if enemy.timer == 0 {
                    // $1BBFC: idle 90 ticks, invulnerable again.
                    enemy.state = EnemyState::Waiting;
                    enemy.timer = rules.heavy_idle_ticks;
                    enemy.counter = 0;
                    enemy.vulnerable = false;
                    enemy.mapping = rules.heavy_idle;
                    samurai_health_palette(cx, enemy);
                }
            }
            _ => {
                enemy.state = EnemyState::Waiting;
                enemy.timer = rules.heavy_idle_ticks;
            }
        }
    }
    /// `Func_01BCAE` for the `$B8` (`$32` set): the `$1BD14` walk cycle and
    /// 0.25 px pacing between screen X 412..428. Returns the screen X
    /// (+128) it tested, `obj_screen_x` of the last drawn frame.
    fn pace(&self, cx: &Ctx, enemy: &mut Enemy) -> i32 {
        let rules = &self.rules;
        match frame_at(&rules.heavy_walk, enemy.counter) {
            Some((_, mapping, _)) => {
                enemy.mapping = mapping;
                enemy.counter += 1;
            }
            None => enemy.counter = 0,
        }
        let screen = enemy.x - cx.camera.0 + 128;
        let (low, high) = rules.heavy_bounds;
        if enemy.phase == 0 {
            advance(&mut enemy.x, &mut enemy.motion.fx, -rules.heavy_walk_speed);
            if screen < low {
                enemy.phase = 1;
            }
        } else {
            advance(&mut enemy.x, &mut enemy.motion.fx, rules.heavy_walk_speed);
            if screen >= high {
                enemy.phase = 0;
            }
        }
        screen
    }
    /// `$1C672`: off screen, the leader walks the player in (`plr_event` 3)
    /// until he passes its X - `$100`; then `$1C6AC` plays SAMURAI ENTRY,
    /// writes the health row and starts pacing in (routine `$24`).
    fn leader_wait(&self, cx: &mut Ctx, enemy: &mut Enemy) {
        if cx.player.0 <= enemy.x - self.rules.leader_approach {
            cx.player_event(PlayerEvent::WalkRight);
            return;
        }
        enemy.routine = LEADER_ENTER;
        enemy.phase = 0;
        let cue = cx.shared.cues.get(SoundEvent::SamuraiEntry);
        cx.play(cue);
        samurai_health_palette(cx, enemy);
    }
    fn release_sickle(&self, cx: &mut Ctx, enemy: &Enemy, leader: bool) {
        // $1C0BC: projectile 50 at X+32-56 (or +32+56 when flipped), +/-8 px/tick.
        // Native uses feet-1 so the terrain test does not remove it on the floor.
        let rules = &self.rules;
        let (origin, reach) = rules.sickle_offset;
        let speed = rules.sickle_speed >> 16;
        let (dx, velocity) = if enemy.facing_left {
            (origin - reach, -speed)
        } else {
            (origin + reach, speed)
        };
        // The sickle's art faces left and uses the thrower's graphics base
        // ($1C628 shares the $B4 base with $B8).
        let base = cx.base(0xb4);
        let mut shot = Projectile::new(
            enemy.x + dx,
            enemy.y - 1,
            velocity,
            true,
            rules.sickle,
            rules.sickle_damage,
        )
        .with_art(base, true);
        if leader {
            // $1C12E: $B8 sets +$32, selecting the returning throw.
            // Object_Integrate adds +$3A00 before each position step.
            shot.x = enemy.x + rules.leader_sickle_offset;
            shot.velocity_x = 0;
            shot.motion = Motion {
                vx: rules.leader_sickle_speed,
                ax: rules.leader_sickle_accel,
                ..Motion::default()
            };
            shot.life = rules.leader_sickle_life;
        }
        if cx.shoot(shot) {
            // $1C19A: SAMURAI SHOT as the thrown weapon starts.
            let cue = cx.shared.cues.get(SoundEvent::SickleThrow);
            cx.play(cue);
        }
    }
}
