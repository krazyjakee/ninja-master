//! Type `$E0`, `EnemyE0` (`$1A54C`): the chain-and-sickle ninja.
use crate::{
    combat::{Effect, EffectKind, Enemy, Motion, Projectile, Spawn},
    enemies::{Behavior, Ctx, HitResponse, shot_damage},
    game::Terrain,
    rom::{Result, byte, long, pointer, signed, word, word_i},
};

/// `$2C` script index / countdown, `$2E` step duration, `$34` steps to
/// replay, script selection, chain thrown (`$4C` valid).
const STEP: usize = 0;
const DURATION: usize = 1;
const REPLAY: usize = 2;
const SCRIPT: usize = 3;
const CHAIN: usize = 4;

/// `$E0`: a kusarigama fighter. Within 200 px it waits 61 ticks, then plays
/// one of four pose scripts (`Data_01A4EE`, by spawn flag bits 0 and 8): a
/// chain throw (list-4 `$0A`, 5.5 px/tick decelerating back to the hand,
/// damage 2) caught and replayed backwards, or within 32 px a close slash
/// (list-4 `$35`, 11 ticks). Shurikens arriving while it stands (`$224`,
/// `$228`) are deflected. Two hits; the second throws it back, leaving
/// the `$231` piece, and the debris burst on landing.
pub(crate) struct Kusarigama {
    stand: u16,
    dead: u16,
    scripts: [Vec<(u16, u16)>; 4],
    range: i32,
    close: i32,
    wait: i32,
    leash: i32,
    catch: i32,
    chain_heights: [i32; 2],
    chain_reach: i32,
    chain_speed: i32,
    chain_brake: i32,
    chain_frames: [[(u16, bool); 4]; 2],
    chain_damage: u8,
    chain_sound: u8,
    slash_mapping: u16,
    slash_damage: u8,
    death_vy: i32,
    death_gravity: i32,
    death_vx: i32,
    piece: (u16, i32, i32, i32, i32),
}
impl Kusarigama {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let script = |i: usize| -> Result<Vec<(u16, u16)>> {
            let p = pointer(rom, 0x1a4ee + i * 4)?;
            let mut out = Vec::new();
            for j in 0..16 {
                let frame = word(rom, p + j * 4)?;
                if frame & 0x8000 != 0 {
                    return Ok(out);
                }
                out.push((frame, word(rom, p + j * 4 + 2)?));
            }
            Err("unterminated $E0 script".into())
        };
        let frames = |set: usize| -> Result<[(u16, bool); 4]> {
            let mut out = [(0, false); 4];
            for (i, frame) in out.iter_mut().enumerate() {
                let p = 0x1a8a4 + set * 16 + i * 4;
                *frame = (word(rom, p)?, word(rom, p + 2)? & 0x800 != 0);
            }
            Ok(out)
        };
        Ok(Self {
            stand: word(rom, 0x1a59e)?,
            dead: word(rom, 0x1a4fe)?,
            scripts: [script(0)?, script(1)?, script(2)?, script(3)?],
            range: word_i(rom, 0x1a5ac)?,
            close: word_i(rom, 0x1a5ec)?,
            wait: word_i(rom, 0x1a5b2)?,
            leash: word_i(rom, 0x1a674)?,
            catch: word_i(rom, 0x1a69e)?,
            chain_heights: [signed(rom, 0x1a808)?, signed(rom, 0x1a80a)?],
            chain_reach: word_i(rom, 0x1a80c)?,
            chain_speed: long(rom, 0x1a84c)?,
            chain_brake: signed(rom, 0x1a852)?,
            chain_frames: [frames(0)?, frames(1)?],
            chain_damage: shot_damage(rom, 0x0a)?,
            chain_sound: byte(rom, 0x1a897)?,
            slash_mapping: word(rom, 0x1ac56)?,
            slash_damage: shot_damage(rom, 0x35)?,
            death_vy: long(rom, 0x1a754)?,
            death_gravity: word_i(rom, 0x1a75c)?,
            death_vx: long(rom, 0x1a76a)?,
            // Data_01A500: mapping, dx, dy, vy, gravity (Func_015FE8).
            piece: (
                word(rom, 0x1a500)?,
                signed(rom, 0x1a502)?,
                signed(rom, 0x1a504)?,
                signed(rom, 0x1a506)? << 16,
                word_i(rom, 0x1a508)?,
            ),
        })
    }
    /// `.loc_01A5B0`: 61 ticks before the next script.
    fn rest(&self, e: &mut Enemy) {
        e.vars[STEP] = self.wait;
        e.routine = 8;
    }
    /// `Func_01A7B6`: spawn bits 4/5 flip; both set face the player.
    fn face(cx: &Ctx, e: &mut Enemy, spawn: &Spawn) {
        let flips = spawn.flags & 0x30;
        e.facing_left = if flips == 0x30 {
            !cx.player_left(e.x)
        } else {
            flips & 0x10 != 0
        };
    }
    /// `Func_01A98E`: while standing (`$224`/`$228`), a player shuriken in
    /// the 18 x 56 box on the facing side is deflected (`$368EC`, effect 8).
    fn deflect(&self, cx: &mut Ctx, e: &Enemy) {
        if e.mapping != self.stand && e.mapping != self.scripts[0][4].0 {
            return;
        }
        let centre = e.x + if e.facing_left { 16 } else { -16 };
        let left = cx.player_left(e.x);
        let mut deflected = Vec::new();
        for shot in cx
            .projectiles
            .iter_mut()
            .filter(|shot| !shot.enemy && shot.alive)
        {
            let d = shot.x - centre;
            if (d < 0) == left && d.abs() <= 18 && shot.y < e.y && e.y - shot.y <= 56 {
                shot.alive = false;
                deflected.push((shot.x, shot.y));
            }
        }
        for (x, y) in deflected {
            let first = cx.shared.hit_seq[0];
            cx.effect(Effect::new(EffectKind::HitSpark, x, y, first));
            cx.visible_sound(x, y, 8);
        }
    }
    /// `Func_01A80E`: the chain, thrown from 32 px ahead at the script height.
    fn throw_chain(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.vars[CHAIN] = 0;
        let y = e.y + self.chain_heights[usize::from(spawn.flags & 1 != 0)];
        // The art faces left: an unflipped thrower throws left.
        let right = e.facing_left;
        let sign = if right { 1 } else { -1 };
        let x = e.x + sign * self.chain_reach;
        let base = cx.base(0xe0);
        let frames = self.chain_frames[usize::from(!right)];
        let mut chain = Projectile::new(x, y, 0, true, frames[0].0, self.chain_damage)
            .with_motion(Motion {
                vx: sign * self.chain_speed,
                ax: sign * self.chain_brake,
                ..Motion::default()
            })
            .with_art(base, false)
            .unset_frame();
        chain.passes_terrain = true;
        chain.owner = Some(cx.current);
        chain.home = Some(x);
        chain.frames = Some(frames);
        if cx.shoot(chain) {
            e.vars[CHAIN] = 1;
            cx.visible_sound(e.x, e.y, self.chain_sound);
        }
    }
    /// `Func_01AC12`: the close slash, 8 px/tick for 11 ticks, 32 px up.
    fn slash(&self, cx: &mut Ctx, e: &Enemy) {
        let mut slash = Projectile::new(
            e.x,
            e.y - 32,
            if e.facing_left { 8 } else { -8 },
            true,
            self.slash_mapping,
            self.slash_damage,
        );
        slash.passes_terrain = true;
        slash.life = 11;
        cx.shoot(slash);
    }
    fn chain<'c>(cx: &'c Ctx) -> Option<&'c Projectile> {
        let me = cx.current;
        cx.projectiles
            .iter()
            .find(|shot| shot.owner == Some(me) && shot.home.is_some() && shot.alive)
    }
}
impl Behavior for Kusarigama {
    fn label(&self, _kind: u16) -> String {
        "EnemyE0 $1A54C".into()
    }
    fn initial_mapping(&self, _kind: u16) -> u16 {
        0
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, terrain: &dyn Terrain) {
        // $1A54C: nothing at all runs while off screen.
        if e.offscreen {
            return;
        }
        e.timer = e.timer.wrapping_add(1);
        self.deflect(cx, e);
        cx.process_hit(e);
        Self::face(cx, e, spawn);
        loop {
            match e.routine {
                0 => {
                    // $1A570
                    e.timer = 0;
                    e.motion.stop();
                    e.death_routine = 0x28;
                    e.hurt_routine = 0x30;
                    e.vars[CHAIN] = 0;
                    e.routine = 4;
                    return;
                }
                4 => {
                    // $1A59C: stand until the player is within 200 px.
                    e.mapping = self.stand;
                    if cx.player_dx(e.x) >= self.range {
                        return;
                    }
                    self.rest(e);
                }
                8 => {
                    // $1A5BC: pick the script; close range or bit 8 slash.
                    e.vars[STEP] -= 1;
                    if e.vars[STEP] >= 0 {
                        return;
                    }
                    e.vars[SCRIPT] =
                        i32::from(spawn.flags & 1) + 2 * i32::from((spawn.flags >> 8) & 1);
                    e.vars[STEP] = 0;
                    e.routine = 0x0c;
                    if spawn.flags & 0x100 != 0 || cx.player_dx(e.x) < self.close {
                        e.routine = 0x20;
                    }
                    return;
                }
                0x0c => {
                    // $1A5F8: next script step; the throw waits for the chain.
                    let script = &self.scripts[e.vars[SCRIPT] as usize & 3];
                    let Some(&(frame, duration)) = script.get(e.vars[STEP] as usize) else {
                        e.routine = 0x14;
                        return;
                    };
                    e.mapping = frame;
                    e.vars[DURATION] = i32::from(duration & 0x7fff);
                    if duration & 0x8000 != 0 {
                        self.throw_chain(cx, e, spawn);
                        e.routine = 0x14;
                    } else {
                        e.vars[REPLAY] = 3;
                        e.routine = 0x10;
                    }
                    return;
                }
                0x10 => {
                    // $1A648
                    e.vars[DURATION] -= 1;
                    if e.vars[DURATION] >= 0 {
                        return;
                    }
                    e.vars[STEP] += 1;
                    e.routine = 0x0c;
                    return;
                }
                0x14 => {
                    // $1A662: until the chain is back within 64 px (or gone).
                    if (cx.camera.0 + 0xa0 - e.x).abs() >= self.leash {
                        e.routine = 4;
                        return;
                    }
                    if e.vars[CHAIN] == 0 {
                        return;
                    }
                    if let Some(chain) = Self::chain(cx) {
                        let outbound = (chain.motion.vx < 0) != (chain.motion.ax < 0);
                        if outbound || (e.x - chain.x).abs() >= self.catch {
                            return;
                        }
                    }
                    e.vars[STEP] = e.vars[REPLAY];
                    e.routine = 0x18;
                    return;
                }
                0x18 => {
                    // $1A6AE: the script backwards, 4 ticks a pose.
                    e.vars[STEP] -= 1;
                    if e.vars[STEP] < 0 {
                        e.vars[STEP] = self.wait;
                        e.routine = 8;
                        return;
                    }
                    let script = &self.scripts[e.vars[SCRIPT] as usize & 3];
                    if let Some(&(frame, _)) = script.get(e.vars[STEP] as usize) {
                        e.mapping = frame;
                    }
                    e.vars[DURATION] = 2;
                    e.routine = 0x1c;
                    return;
                }
                0x1c => {
                    // $1A6DE
                    e.vars[DURATION] -= 1;
                    if e.vars[DURATION] >= 0 {
                        return;
                    }
                    e.routine = 0x18;
                    return;
                }
                0x20 => {
                    // $1A6EC: script D with the slash.
                    let step = e.vars[STEP] as usize;
                    e.vars[STEP] += 1;
                    let Some(&(frame, duration)) = self.scripts[3].get(step) else {
                        self.rest(e);
                        continue;
                    };
                    e.mapping = frame;
                    e.vars[DURATION] = i32::from(duration & 0x7fff);
                    if duration & 0x8000 != 0 {
                        self.slash(cx, e);
                    }
                    e.routine = 0x24;
                    return;
                }
                0x24 => {
                    // $1A71C
                    e.vars[DURATION] -= 1;
                    if e.vars[DURATION] >= 0 {
                        return;
                    }
                    e.routine = 0x20;
                    return;
                }
                0x28 => {
                    // $1A73A: the chain explodes; thrown back with the piece.
                    if e.vars[CHAIN] != 0 {
                        let me = cx.current;
                        let mut blast_at = None;
                        for shot in cx.projectiles.iter_mut() {
                            if shot.owner == Some(me) && shot.home.is_some() && shot.alive {
                                shot.alive = false;
                                blast_at = Some((shot.x, shot.y));
                            }
                        }
                        if let Some((x, y)) = blast_at {
                            let blast = cx.shared.blasts[0];
                            cx.explode(x, y, blast);
                        }
                    }
                    e.motion.vy = self.death_vy;
                    e.motion.gravity = self.death_gravity;
                    e.mapping = self.dead;
                    e.motion.vx = if cx.player_left(e.x) {
                        self.death_vx
                    } else {
                        -self.death_vx
                    };
                    e.routine = 0x2c;
                    let (mapping, dx, dy, vy, gravity) = self.piece;
                    let mut piece = Effect::new(EffectKind::Pieces, e.x + dx, e.y + dy, mapping);
                    piece.base = cx.base(0xe0);
                    piece.flip = e.facing_left;
                    piece.motion = Motion {
                        vx: e.motion.vx >> 1,
                        vy,
                        gravity,
                        ..Motion::default()
                    };
                    piece.counter = 600;
                    cx.effect(piece);
                    return;
                }
                0x2c => {
                    // $1A790: burst on landing.
                    if terrain.solid_at(e.x, e.y) {
                        cx.burst(e.x, e.y);
                        Ctx::free(e);
                    }
                    return;
                }
                _ => {
                    // $1A7AE ($30/$34): a non-lethal hit costs this tick only.
                    e.routine = e.saved_routine;
                    return;
                }
            }
        }
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        if enemy.offscreen || enemy.routine == 0 {
            HitResponse::Pass
        } else {
            HitResponse::Accept
        }
    }
}
