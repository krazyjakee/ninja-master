//! Type `$18`, `Enemy18` (`$13F3A`): the 4-1 end guard.
use super::{Mines, count_down, escort::Escort, nudge_x, nudge_y, settle, splash};
use crate::{
    combat::{Blast, Enemy, Motion, PaletteOp, Projectile, Signal, Spawn},
    enemies::{Behavior, Ctx, HitResponse, set_whole, shot_damage, stop},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
    sound_events::Table,
};

/// `$2A` timer, `$2C` flash (negative: hittable), `$30` bob angle, `$32`
/// sway angle, `$34` repeat count, `$4E` bomb countdown, `$4C` the bob's
/// spray countdown (its low word; the long held the flame part); the missile's
/// countdown and ceiling (list-4 `$38`). `Enemy::phase` is the missile's
/// routine, `step_x`/`launch_y` its target.
const G_TIMER: usize = 0;
const G_FLASH: usize = 1;
const G_BOB: usize = 2;
const G_SWAY: usize = 3;
const G_COUNT: usize = 4;
const G_NEXT: usize = 5;
const G_MISSILE_TIMER: usize = 6;
const G_MISSILE_CEILING: usize = 7;
const G_SPRAY: usize = 8;
/// A queued weapon met the shield before this update changed the pose.
const G_SHIELDED_STRIKE: usize = 9;

/// A list-1 wake splash (`Func_0176DC`) with `+$28` = `life` at X + `dx`
/// and the fixed Y `y`, the water line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spray {
    life: i32,
    dx: i32,
    y: i32,
}
impl Spray {
    /// `moveq #life,d7` at `life`, `move.w #y,$24(a6)` at `y`, `addi.w
    /// #dx,d0` at `dx` (the immediates' addresses).
    fn load(rom: &[u8], life: usize, y: usize, dx: usize) -> Result<Self> {
        Ok(Self {
            life: i32::from(byte(rom, life)?),
            dx: signed(rom, dx)?,
            y: word_i(rom, y)?,
        })
    }
    fn throw(self, cx: &mut Ctx, e: &Enemy) {
        splash(cx, (e.x + self.dx, self.y), self.life, false);
    }
}

/// `$18`: starts the HASSOU! music (the stage's autoscroll stops,
/// `crate::special`), idles in from the left edge, flies off, drops in
/// from the top right onto the water and fights there: bobbing
/// (`Func_0143AE`) and swaying (`Func_0143EE`) it throws three bombs, runs
/// off right, returns to camera + $100 and fires two missiles that climb,
/// wait 30 ticks and split into three aimed fragments, then runs off right
/// again and drifts back. Spray flies up from the water in front of it
/// (list-1 wake splashes, [`Spray`]): every fourth tick of the glide down
/// and of the death, every sixth bob. Hits are taken only from routine `$10` and not
/// during the seven flash ticks after each hit; pose `$188` (driving
/// off) spends weapons without damage. The death runs nine explosions,
/// hides it, plays STAGE CLEAR and requests the next substage.
pub(crate) struct Guard {
    mines: Mines,
    /// Its list-9 jet-skis and glide flame.
    escort: Escort,
    /// `Data_013742`, copied over palette line 1 at the entry (`$13F86`),
    /// and `Data_013736` over colours 2..7 of it at the drop (`$14090`).
    palettes: ([u16; 16], [u16; 6]),
    /// Func_013B68 indexes Data_0136CE by the remaining health counter.
    health_rows: [u8; 32],
    health_palettes: [[u16; 6]; 7],
    flash_ticks: i32,
    flash_color: u16,
    /// `Func_0143AE`: the bob's spray, `$4C` reloaded with `bob_period`.
    bob_spray: Spray,
    bob_period: i32,
    /// `Func_01440E`: the glide's and death's spray, on ticks with no bit
    /// of `glide_mask` set; `Func_0143FE` runs it in the routines `dying`.
    glide_spray: Spray,
    glide_mask: u16,
    dying: (u16, u16),
    music: u8,
    clear_music: u8,
    voice: u8,
    enter: (i32, i32, u16),
    creep: i32,
    idle_ticks: (i32, i32),
    leave_accel: (i32, i32),
    fly_poses: [u16; 2],
    drop: (i32, i32, i32, i32, u16),
    drop_line: i32,
    glide: (i32, i32, i32),
    land_accel: i32,
    water: i32,
    land_sound: u8,
    ride: u16,
    bombs: i32,
    next: (i32, i32),
    run: (u16, i32),
    rest: i32,
    home: i32,
    aim: (u16, u16, i32, i32, i32),
    drift: (i32, i32),
    bomb_offset: i32,
    bomb_launch: (i32, i32, i32),
    bomb_sound: u8,
    missile: u16,
    fragment: u16,
    missile_offset: (i32, i32),
    missile_speed: i32,
    missile_damage: u8,
    missile_sound: u8,
    death_vx: i32,
    blast_ticks: i32,
    hide_sound: u8,
    request_delay: i32,
    music_delay: i32,
}
/// `N` palette words at `at`.
fn colours<const N: usize>(rom: &[u8], at: usize) -> Result<[u16; N]> {
    let mut out = [0; N];
    for (i, colour) in out.iter_mut().enumerate() {
        *colour = word(rom, at + i * 2)?;
    }
    Ok(out)
}
impl Guard {
    pub(super) fn load(rom: &[u8], mines: Mines) -> Result<Self> {
        let mut health_rows = [0; 32];
        for (i, row) in health_rows.iter_mut().enumerate() {
            *row = byte(rom, 0x136ce + i)?;
        }
        let mut health_palettes = [[0; 6]; 7];
        for (i, palette) in health_palettes.iter_mut().enumerate() {
            *palette = colours(rom, 0x136ee + i * 12)?;
        }
        Ok(Self {
            mines,
            health_rows,
            health_palettes,
            flash_ticks: word_i(rom, 0x13b5c)?,
            flash_color: word(rom, 0x13f46)?,
            escort: Escort::load(rom)?,
            palettes: (colours(rom, 0x13742)?, colours(rom, 0x13736)?),
            bob_spray: Spray::load(rom, 0x143bb, 0x143c6, 0x143d0)?,
            bob_period: word_i(rom, 0x143b6)?,
            glide_spray: Spray::load(rom, 0x14419, 0x14424, 0x1442e)?,
            glide_mask: word(rom, 0x14414)?,
            dying: (word(rom, 0x14404)?, word(rom, 0x1440a)?),
            music: Table::Music.command_at(rom, 0x13ff7)?,
            clear_music: Table::Music.command_at(rom, 0x1438b)?,
            voice: Table::Voice.command_at(rom, 0x141e1)?,
            enter: (
                signed(rom, 0x13fb8)?,
                signed(rom, 0x13fc4)?,
                word(rom, 0x13fcc)?,
            ),
            creep: word_i(rom, 0x13fd8)?,
            idle_ticks: (word_i(rom, 0x13fde)?, word_i(rom, 0x14008)?),
            leave_accel: (signed(rom, 0x1401e)?, signed(rom, 0x14024)?),
            fly_poses: [word(rom, 0x1402e)?, word(rom, 0x1403a)?],
            drop: (
                signed(rom, 0x1405c)?,
                signed(rom, 0x14074)? << 16,
                signed(rom, 0x1407a)? << 16,
                signed(rom, 0x14080)?,
                word(rom, 0x14086)?,
            ),
            drop_line: word_i(rom, 0x140b0)? + signed(rom, 0x140b4)?,
            glide: (
                signed(rom, 0x140ca)?,
                signed(rom, 0x140d0)?,
                word_i(rom, 0x140d6)?,
            ),
            land_accel: signed(rom, 0x140f6)?,
            water: word_i(rom, 0x14104)?,
            land_sound: byte(rom, 0x14123)?,
            ride: word(rom, 0x14130)?,
            bombs: word_i(rom, 0x14136)?,
            next: (word_i(rom, 0x1443e)?, word_i(rom, 0x14442)?),
            run: (word(rom, 0x1416e)?, signed(rom, 0x14174)?),
            rest: word_i(rom, 0x14196)?,
            home: word_i(rom, 0x141bc)?,
            aim: (
                word(rom, 0x141dc)?,
                word(rom, 0x1420e)?,
                word_i(rom, 0x141d0)?,
                word_i(rom, 0x14208)?,
                word_i(rom, 0x1425e)?,
            ),
            drift: (word_i(rom, 0x142c0)?, word_i(rom, 0x142c4)?),
            bomb_offset: signed(rom, 0x1448c)?,
            bomb_launch: (
                signed(rom, 0x14498)? << 16,
                signed(rom, 0x1449e)? << 16,
                signed(rom, 0x144a4)?,
            ),
            bomb_sound: byte(rom, 0x144c9)?,
            missile: word(rom, 0x146e8)?,
            fragment: word(rom, 0x147e2)?,
            missile_offset: (signed(rom, 0x146c6)?, signed(rom, 0x146ca)?),
            missile_speed: signed(rom, 0x146d6)? << 16,
            missile_damage: shot_damage(rom, 0x38)?,
            missile_sound: byte(rom, 0x146f9)?,
            death_vx: long(rom, 0x1430c)?,
            blast_ticks: word_i(rom, 0x14314)?,
            hide_sound: byte(rom, 0x1436b)?,
            request_delay: word_i(rom, 0x14382)?,
            music_delay: word_i(rom, 0x14362)?,
        })
    }
    /// `Func_0143AE`: bob on the water, throwing up spray when `$4C` runs
    /// out (every sixth call).
    fn bob(&self, cx: &mut Ctx, e: &mut Enemy) {
        if !count_down(&mut e.vars[G_SPRAY]) {
            e.vars[G_SPRAY] = self.bob_period;
            self.bob_spray.throw(cx, e);
        }
        e.vars[G_BOB] = (e.vars[G_BOB] + 0x13) & 0xffff;
        let d = cx.shared.sine.polar(e.vars[G_BOB] as u16).0 >> 1;
        nudge_y(e, d);
    }
    /// `Func_01440E`: spray on the ticks (`+$28`) with no `glide_mask` bit.
    fn glide_spray(&self, cx: &mut Ctx, e: &Enemy) {
        if e.timer & self.glide_mask == 0 {
            self.glide_spray.throw(cx, e);
        }
    }
    /// `Func_0143EE`: sway, halved (`asr.l #1`) by the missile routines.
    fn sway(cx: &Ctx, e: &mut Enemy, shift: u32) {
        e.vars[G_SWAY] = (e.vars[G_SWAY] + 4) & 0xffff;
        let d = cx.shared.sine.polar(e.vars[G_SWAY] as u16).0 >> shift;
        nudge_x(e, d);
    }
    /// `Func_014436`: the next bomb in 63..126 ticks.
    fn next_bomb(&self, cx: &mut Ctx, e: &mut Enemy) {
        e.vars[G_NEXT] = (i32::from(cx.random()) & self.next.0) + self.next.1;
    }
    /// `Func_014464`: a list-4 `$2C` bomb lobbed from 32 px up.
    fn bomb(&self, cx: &mut Ctx, e: &Enemy) {
        let fraction = i32::from(cx.random());
        let (vx, vy, gravity) = self.bomb_launch;
        let motion = Motion {
            vx: vx | fraction,
            vy: vy | fraction,
            gravity,
            ..Motion::default()
        };
        if self.mines.throw(cx, e.x, e.y + self.bomb_offset, motion) {
            cx.effect_sound(self.bomb_sound);
        }
    }
    /// `Func_0146A8`: the list-4 `$38` missile, climbing up-left.
    fn missile(&self, cx: &mut Ctx, e: &mut Enemy) {
        let (x, y) = (e.x + self.missile_offset.0, e.y + self.missile_offset.1);
        let mut missile = Projectile::new(x, y, 0, true, self.missile, self.missile_damage)
            .with_motion(Motion {
                vx: self.missile_speed,
                vy: self.missile_speed,
                ..Motion::default()
            });
        missile.owner = Some(cx.current);
        missile.passes_terrain = true;
        missile.life = u16::MAX;
        if cx.shoot(missile) {
            cx.effect_sound(self.missile_sound);
            // $14738: the climb ends 32..95 px below the camera top.
            e.phase = 1;
            e.vars[G_MISSILE_CEILING] = (i32::from(cx.random()) & 0x3f) + 0x20;
        }
    }
    /// `Func_01471A` for this guard's missile: climb to its ceiling, lock
    /// the player's position, wait 30 ticks, then split into three
    /// fragments 56/1024 turn apart, centred on the target, 4 px/tick.
    fn steer_missile(&self, cx: &mut Ctx, e: &mut Enemy) {
        let me = cx.current;
        let Some(index) = cx
            .projectiles
            .iter()
            .position(|shot| shot.owner == Some(me) && shot.alive && shot.mapping == self.missile)
        else {
            e.phase = 0;
            return;
        };
        match e.phase {
            1 => {
                // $1474E
                let ceiling = cx.camera.1 + e.vars[G_MISSILE_CEILING];
                let shot = &mut cx.projectiles[index];
                if shot.y > ceiling {
                    return;
                }
                shot.y = ceiling;
                shot.motion = Motion::default();
                (e.step_x, e.launch_y) = cx.player;
                e.vars[G_MISSILE_TIMER] = 0x1e;
                e.phase = 2;
            }
            _ => {
                // $14788 / $147BA
                if count_down(&mut e.vars[G_MISSILE_TIMER]) {
                    return;
                }
                let shot = &mut cx.projectiles[index];
                shot.alive = false;
                let (x, y) = (shot.x, shot.y);
                let heading =
                    i32::from(cx.shared.angle_to((x, y), (e.step_x, e.launch_y))) * 4 - 0x38;
                for i in 0..3 {
                    let (cos, sin) = cx.shared.sine.polar((heading + i * 0x38) as u16);
                    let mut fragment =
                        Projectile::new(x, y, 0, true, self.fragment, self.missile_damage)
                            .with_motion(Motion {
                                vx: cos << 2,
                                vy: sin << 2,
                                ..Motion::default()
                            });
                    fragment.owner = Some(me);
                    cx.shoot(fragment);
                }
                // Func_036FF2 as the missile bursts.
                let blast = cx.shared.blasts[0];
                cx.explode(x, y, blast);
                e.phase = 0;
            }
        }
    }
    /// `$142EE`: its missiles burst and its airborne bombs vanish with it.
    fn recall(&self, cx: &mut Ctx) {
        let me = cx.current;
        let mut bursts = Vec::new();
        for shot in cx.projectiles.iter_mut() {
            if shot.owner != Some(me) || !shot.alive {
                continue;
            }
            if shot.mapping == self.missile || shot.mapping == self.fragment {
                bursts.push((shot.x, shot.y));
                shot.alive = false;
            } else if shot.mapping == self.mines.bomb {
                shot.alive = false;
            }
        }
        for (x, y) in bursts {
            let blast = cx.shared.blasts[0];
            cx.explode(x, y, blast);
        }
    }
}
impl Behavior for Guard {
    fn label(&self, _kind: u16) -> String {
        "Enemy18 $13F3A".into()
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        // Func_01444A: from routine $10, while $2C is negative.
        if enemy.routine < 0x10 || enemy.vars[G_FLASH] >= 0 {
            HitResponse::Pass
        } else if enemy.mapping == self.run.0 {
            // Func_01293C clears the weapon's collision bit; shurikens
            // enter the four-step turning routine instead of being spent.
            HitResponse::Reflect
        } else if enemy.vars[G_SHIELDED_STRIKE] != 0 {
            HitResponse::Ignore
        } else {
            HitResponse::Accept
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        // $1444A checks the displayed pose before the routine changes it.
        // Native applies queued weapon hits after the handler has run.
        e.vars[G_SHIELDED_STRIKE] = i32::from(e.strike.is_some() && e.mapping == self.run.0);
        settle(cx, &self.mines);
        cx.process_hit(e);
        if (0x54..=0x64).contains(&e.routine) {
            cx.player_event(crate::movement::PlayerEvent::Freeze);
        }
        self.update_palette(cx, e);
        // Func_0143FE: the death throws up spray (Func_01440E).
        if (self.dying.0..self.dying.1).contains(&e.routine) {
            self.glide_spray(cx, e);
        }
        if e.phase != 0 {
            self.steer_missile(cx, e);
        }
        self.handle(cx, e);
        // List 9 after list 5: its parts, with its position before its
        // motion step.
        self.escort.drive(cx, e);
    }
}
impl Guard {
    /// Func_013B68: six white updates, then the six health colours.
    /// The shared helper skips routine $2C and leaves colour 8 white.
    fn update_palette(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.routine == 0x2c || e.vars[G_FLASH] < 0 {
            return;
        }
        if e.vars[G_FLASH] > 0 {
            for index in 2..=8 {
                cx.palette_op(PaletteOp::Color(1, index, self.flash_color));
            }
        } else {
            let counter = usize::from(e.health.saturating_sub(1)).min(31);
            let row = usize::from(self.health_rows[counter]);
            for (i, &colour) in self.health_palettes[row].iter().enumerate() {
                cx.palette_op(PaletteOp::Color(1, 2 + i as u8, colour));
            }
        }
        e.vars[G_FLASH] -= 1;
    }
    /// The routine's handler (`loc_013ED2`).
    fn handle(&self, cx: &mut Ctx, e: &mut Enemy) {
        loop {
            match e.routine {
                0 => {
                    // $13F6E: HASSOU!, creeping in from the left edge.
                    // Func_01DD9C leaves d0 = 0 for `move.l d0,$38(a1)`
                    // with a1 = $FF0148: the autoscroll stops.
                    stop(e);
                    cx.signal(Signal::StopAutoscroll);
                    e.hurt_routine = 0x50;
                    e.death_routine = 0x54;
                    e.vars[G_FLASH] = -1;
                    e.x = cx.camera.0 + self.enter.0;
                    e.y = cx.camera.1 + self.enter.1;
                    e.mapping = self.enter.2;
                    e.tile = Some(0);
                    e.motion.vx = self.creep;
                    e.vars[G_TIMER] = self.idle_ticks.0;
                    e.routine = 4;
                    // $13F86: its colours (the jet-skis' too) on line 1,
                    // uploaded with `Sys_Flags` bit 1.
                    cx.signal(Signal::Palette(PaletteOp::Load(1, self.palettes.0)));
                    self.escort.enter(cx, e);
                    cx.play(self.music);
                    return;
                }
                4 => {
                    // $14000
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.vars[G_TIMER] = self.idle_ticks.1;
                    e.motion.vx = set_whole(e.motion.vx, 0);
                    e.routine = 8;
                }
                8 => {
                    // $14016: off up and to the right.
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.motion.ax = self.leave_accel.0;
                    e.motion.gravity = self.leave_accel.1;
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $1402C: then it drops in from the top right.
                    e.mapping = self.fly_poses[usize::from(e.timer & 1 == 0)];
                    if !e.offscreen {
                        return;
                    }
                    stop(e);
                    // Object_SetTileBase($10, $2300), then `ori.w
                    // #$8000,8(a0)`: on the surfer's priority layer.
                    e.tile = Some(cx.tile_base(0x10) | 0x8000);
                    let (dx, vx, vy, gravity, mapping) = self.drop;
                    e.x = cx.camera.0 + dx;
                    e.y = cx.camera.1;
                    e.motion.vx = vx;
                    e.motion.vy = vy;
                    e.motion.gravity = gravity;
                    e.mapping = mapping;
                    // $14090: colours 2..7 of line 1 for the drop.
                    for (i, &colour) in self.palettes.1.iter().enumerate() {
                        cx.signal(Signal::Palette(PaletteOp::Color(1, 2 + i as u8, colour)));
                    }
                    e.routine = 0x10;
                    return;
                }
                0x10 => {
                    // $140AE
                    if e.y <= self.drop_line {
                        return;
                    }
                    self.escort.glide(cx, e);
                    let (ax, ay, ticks) = self.glide;
                    e.motion.ax = ax;
                    e.motion.gravity = ay;
                    e.vars[G_TIMER] = ticks;
                    e.routine = 0x14;
                }
                0x14 => {
                    // $140DE
                    self.glide_spray(cx, e);
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    Escort::land(cx);
                    e.motion.gravity = self.land_accel;
                    e.routine = 0x18;
                }
                0x18 => {
                    // $140FE: on the water.
                    if e.y < self.water {
                        return;
                    }
                    stop(e);
                    // `move.l d0,$4C(a0)`: the flame's link goes, so the
                    // first bob sprays.
                    e.vars[G_SPRAY] = 0;
                    e.y = self.water;
                    e.vars[G_TIMER] = 2;
                    e.routine = 0x1c;
                    cx.effect_sound(self.land_sound);
                    return;
                }
                0x1c => e.routine = 0x20,
                0x20 => {
                    // $1412E
                    e.mapping = self.ride;
                    e.vars[G_COUNT] = self.bombs;
                    self.next_bomb(cx, e);
                    e.routine = 0x24;
                }
                0x24 => {
                    // $14144: three bombs, then off to the right.
                    self.bob(cx, e);
                    Self::sway(cx, e, 0);
                    if count_down(&mut e.vars[G_NEXT]) {
                        return;
                    }
                    self.next_bomb(cx, e);
                    self.bomb(cx, e);
                    if count_down(&mut e.vars[G_COUNT]) {
                        return;
                    }
                    e.mapping = self.run.0;
                    e.motion.ax = self.run.1;
                    e.routine = 0x28;
                    return;
                }
                0x28 | 0x44 => {
                    // $1417E / $14292
                    self.bob(cx, e);
                    if !e.offscreen {
                        return;
                    }
                    e.motion.vx = 0;
                    e.motion.ax = 0;
                    e.vars[G_TIMER] = self.rest;
                    e.routine += 4;
                }
                0x2c => {
                    // $1419E
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.motion.vx = set_whole(e.motion.vx, -1);
                    e.routine = 0x30;
                    return;
                }
                0x30 => {
                    // $141B0: back to camera + $100.
                    self.bob(cx, e);
                    if cx.camera.0 + self.home < e.x {
                        return;
                    }
                    e.motion.vx = set_whole(e.motion.vx, 0);
                    e.routine = 0x34;
                    e.vars[G_TIMER] = self.aim.2;
                    e.vars[G_COUNT] = 1;
                    e.mapping = self.aim.0;
                    cx.play(self.voice);
                    return;
                }
                0x34 => {
                    // $141EE
                    self.bob(cx, e);
                    Self::sway(cx, e, 1);
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.vars[G_TIMER] = self.aim.3;
                    e.mapping = self.aim.1;
                    e.routine = 0x38;
                }
                0x38 => {
                    // $14216
                    self.bob(cx, e);
                    Self::sway(cx, e, 1);
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    self.missile(cx, e);
                    e.vars[G_TIMER] = 0x20;
                    e.routine = 0x3c;
                }
                0x3c => {
                    // $1423E
                    self.bob(cx, e);
                    Self::sway(cx, e, 1);
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.mapping = self.aim.0;
                    e.vars[G_TIMER] = self.aim.4;
                    e.routine = 0x40;
                }
                0x40 => {
                    // $14266: a second missile, then off to the right.
                    self.bob(cx, e);
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.vars[G_TIMER] = self.aim.3;
                    if count_down(&mut e.vars[G_COUNT]) {
                        e.routine = 0x34;
                        continue;
                    }
                    e.mapping = self.run.0;
                    e.motion.ax = self.run.1;
                    e.routine = 0x44;
                    return;
                }
                0x48 => {
                    // $142B2
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.vars[G_TIMER] = (i32::from(cx.random()) & self.drift.0) + self.drift.1;
                    e.motion.vx = set_whole(e.motion.vx, -1);
                    e.routine = 0x4c;
                }
                0x4c => {
                    // $142D4: drift left, then bomb again.
                    self.bob(cx, e);
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.motion.vx = set_whole(e.motion.vx, 0);
                    e.routine = 0x20;
                    return;
                }
                0x50 => {
                    // loc_013B5A: six white ticks, then hittable again.
                    e.vars[G_FLASH] = self.flash_ticks;
                    e.routine = e.saved_routine;
                    return;
                }
                0x54 => {
                    // $142EE
                    self.recall(cx);
                    e.phase = 0;
                    e.routine = 0x58;
                }
                0x58 => {
                    // $142FE
                    e.motion.ax = 0;
                    e.motion.vx = self.death_vx;
                    e.vars[G_TIMER] = self.blast_ticks;
                    e.routine = 0x5c;
                }
                0x5c => {
                    // $1431C: nine explosions, eight ticks apart.
                    if e.timer & 7 != 0 {
                        return;
                    }
                    let big = cx.shared.blasts[1];
                    let blast = Blast {
                        sound: cx.shared.effects.command(1).unwrap_or(big.sound),
                        ..big
                    };
                    cx.explode(e.x, e.y, blast);
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    stop(e);
                    e.mapping = 0;
                    e.tile = Some(0);
                    e.y = 0;
                    e.vars[G_TIMER] = self.music_delay;
                    e.routine = 0x60;
                    cx.effect_sound(self.hide_sound);
                    return;
                }
                0x60 => {
                    // $14372: STAGE CLEAR.
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    e.vars[G_TIMER] = self.request_delay;
                    e.routine = 0x64;
                    cx.play(self.clear_music);
                    return;
                }
                _ => {
                    // $14392: $FF0029 bit 1.
                    if count_down(&mut e.vars[G_TIMER]) {
                        return;
                    }
                    cx.next_substage();
                    Ctx::remove(e);
                    return;
                }
            }
        }
    }
}
