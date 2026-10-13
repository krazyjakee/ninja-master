//! Type `$CC`, `EnemyCC` (`$19CD6`): rises out of the floor and grabs.
use crate::{
    combat::{Enemy, Spawn},
    enemies::{Behavior, Ctx, HitResponse},
    game::Terrain,
    rom::{Result, byte, long, signed, word, word_i},
};

/// `$2C`: home Y; `$35`: height (pose 0..7); `$2E`: wait; `$30`/`$32`: grab
/// offsets.
const GRAB_HOME: usize = 0;
const HEIGHT: usize = 1;
const GRAB_WAIT: usize = 2;
const GRAB_X: usize = 3;
const GRAB_Y: usize = 4;
const ESCAPE_GATE: usize = 5;

/// `$CC`: hidden in the floor, it grows (`$1FD`..`$204`), and when the
/// player is within 48 px it shoots up 40 px (2.5 px/tick) shrinking,
/// waits up to 120 ticks, and sinks back at 0.5 px/tick. A player within
/// 32 x 16 px of the point 40 px above it is grabbed: it rises again,
/// grows, and sinks at 0.22 px/tick while the player is held (the ROM pulls
/// the player along and drains a point of health every 64 ticks until
/// directions or jump are pressed repeatedly). Its body never touches the player
/// (`Func_019F3A` has no `Object_CollidePlayer`), so the `$A` drain is the
/// only damage. Any weapon hit kills it (debris burst). `Object_SetTileBase`
/// (`$19D00`, `$2300`) takes the spawn records' clear flag bit 14, so it is
/// a low-priority sprite: plane tiles with the priority bit cover it.
pub(crate) struct Grabber {
    pose_first: u16,
    death: u16,
    hurt: u16,
    wake: i32,
    sound: u8,
    lunge_vy: i32,
    rise: i32,
    top_wait: i32,
    sink_vy: i32,
    grab_vy: i32,
    hold_vy: i32,
    escape_wait: i32,
    window: (i32, i32),
    full: i32,
    decay: u16,
    shock_period: u16,
}
impl Grabber {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        Ok(Self {
            pose_first: word(rom, 0x19ce8)?,
            death: word(rom, 0x19d16)?,
            hurt: word(rom, 0x19d1c)?,
            wake: word_i(rom, 0x19d64)?,
            sound: byte(rom, 0x19d6d)?,
            lunge_vy: long(rom, 0x19d76)?,
            rise: signed(rom, 0x19d9c)?,
            top_wait: word_i(rom, 0x19db6)?,
            sink_vy: long(rom, 0x19dec)?,
            grab_vy: long(rom, 0x19e38)?,
            hold_vy: long(rom, 0x19e76)?,
            escape_wait: word_i(rom, 0x19ec0)?,
            window: (word_i(rom, 0x19f0a)?, word_i(rom, 0x19f22)?),
            full: i32::from(byte(rom, 0x19fb5)?),
            decay: word(rom, 0x19fe6)?,
            shock_period: word(rom, 0x1a03e)?,
        })
    }
    /// `Func_019FAA`: grow one pose; true while not yet full.
    fn grow(&self, e: &mut Enemy) -> bool {
        e.vars[HEIGHT] += 1;
        if e.vars[HEIGHT] < self.full {
            return true;
        }
        e.vars[HEIGHT] = self.full;
        false
    }
    /// `Func_019EF8`: the player within the window above the object.
    fn in_window(&self, cx: &Ctx, e: &Enemy) -> bool {
        cx.player_dx(e.x) < self.window.0 && (e.y + self.rise - cx.player.1).abs() < self.window.1
    }
    fn grab(&self, cx: &Ctx, e: &mut Enemy) -> bool {
        if cx.player_state.is_some_and(crate::movement::dying) || !self.in_window(cx, e) {
            return false;
        }
        e.vars[GRAB_X] = e.x - cx.player.0;
        e.vars[GRAB_Y] = e.y + self.rise - cx.player.1;
        e.routine = 0x30;
        true
    }
    /// `$19F56`: draw Joe toward the claw by one pixel per axis, writing
    /// the player position and stopping his motion on every held update.
    fn pull(&self, cx: &mut Ctx, e: &mut Enemy) -> bool {
        e.vars[GRAB_X] -= e.vars[GRAB_X].signum();
        e.vars[GRAB_Y] -= e.vars[GRAB_Y].signum();
        let at = (e.x - e.vars[GRAB_X], e.y + self.rise - e.vars[GRAB_Y]);
        if let Some(body) = &mut cx.player_body {
            body.x += at.0 - cx.player_at.0;
            body.y += at.1 - cx.player_at.1;
        }
        cx.player = at;
        cx.player_at = at;
        cx.signal(crate::combat::Signal::HoldPlayer(at));
        e.vars[GRAB_X] == 0 && e.vars[GRAB_Y] == 0
    }
    /// `$19FDE`: three initial presses unlock the struggle; subsequent
    /// direction or jump edges grow the claw toward release. Its height
    /// decays every sixteen updates, so holding a button does not escape.
    fn escape(&self, cx: &Ctx, e: &mut Enemy) -> bool {
        use crate::input::Button;
        if e.timer & self.decay == 0 {
            e.vars[HEIGHT] = (e.vars[HEIGHT] - 1).max(0);
        }
        let mask = Button::Left.mask()
            | Button::Right.mask()
            | Button::Up.mask()
            | Button::Down.mask()
            | Button::Jump.mask();
        if cx.player_input.pressed & mask == 0 {
            return false;
        }
        if e.vars[ESCAPE_GATE] > 0 {
            e.vars[ESCAPE_GATE] -= 1;
            return false;
        }
        e.vars[HEIGHT] += 1;
        e.vars[HEIGHT] >= self.full
    }
}
impl Behavior for Grabber {
    fn label(&self, _kind: u16) -> String {
        "EnemyCC $19CD6".into()
    }
    fn hit(&self, e: &Enemy, _kind: u16) -> HitResponse {
        // Func_019F3A in these routines: Object_LoadHitbox, Object_CollideL3
        // and Object_ApplyDamage. It never calls Object_CollidePlayer, so
        // the body does not touch the player (only the $A drain hurts).
        if matches!(
            e.routine,
            8 | 0x0c | 0x10 | 0x14 | 0x18 | 0x1c | 0x30 | 0x34 | 0x38 | 0x3c
        ) {
            HitResponse::WeaponsOnly
        } else {
            HitResponse::Pass
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, _terrain: &dyn Terrain) {
        // $19CD6: nothing runs while off screen.
        if e.offscreen && e.routine != 0x24 {
            return;
        }
        e.mapping = self.pose_first + (e.vars[HEIGHT] & 7) as u16;
        e.timer = e.timer.wrapping_add(1);
        if cx.process_hit(e).is_some() {
            if e.routine == self.death {
                // $19EDE: score (central), Func_036EEE, free.
                cx.burst(e.x, e.y);
                Ctx::free(e);
            }
            return;
        }
        loop {
            match e.routine {
                0 => {
                    // $19CFA
                    e.timer = 0;
                    e.motion.stop();
                    e.vars[GRAB_HOME] = e.y;
                    e.death_routine = self.death;
                    e.hurt_routine = self.hurt;
                    e.vars[HEIGHT] = 0;
                    e.routine = 4;
                }
                4 | 0x20 => {
                    // $19D30 / $19E1E: grow to full height.
                    if !self.grow(e) {
                        e.routine = 8;
                    }
                    return;
                }
                8 => {
                    // $19D3E
                    if e.vars[GRAB_HOME] < e.y {
                        e.y = e.vars[GRAB_HOME];
                        e.motion.vy = 0;
                    }
                    if cx.player_dx(e.x) >= self.wake {
                        return;
                    }
                    e.routine = 0x0c;
                }
                0x0c => {
                    // $19D6C
                    cx.visible_sound(e.x, e.y, self.sound);
                    e.motion.vy = self.lunge_vy;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $19D80: rise 40 px shrinking; grab on the way.
                    if self.grab(cx, e) {
                        return;
                    }
                    if e.timer & 1 == 0 {
                        e.vars[HEIGHT] = (e.vars[HEIGHT] - 1).max(0);
                    }
                    let top = e.vars[GRAB_HOME] + self.rise;
                    if e.y >= top {
                        return;
                    }
                    e.y = top;
                    e.motion.vy = 0;
                    e.vars[GRAB_WAIT] = self.top_wait;
                    e.routine = 0x14;
                }
                0x14 => {
                    // $19DBA
                    if self.grab(cx, e) {
                        return;
                    }
                    if cx.player_dx(e.x) < self.wake {
                        e.vars[GRAB_WAIT] -= 1;
                        if e.vars[GRAB_WAIT] >= 0 {
                            return;
                        }
                    }
                    e.routine = 0x18;
                }
                0x18 => {
                    // $19DEA
                    e.motion.vy = self.sink_vy;
                    e.routine = 0x1c;
                }
                0x1c => {
                    // $19DF6
                    if self.grab(cx, e) {
                        return;
                    }
                    if e.vars[GRAB_HOME] >= e.y {
                        return;
                    }
                    e.y = e.vars[GRAB_HOME];
                    e.motion.vy = 0;
                    e.routine = 0x20;
                    return;
                }
                0x30 => {
                    // $19E2E: grabbed; rise again.
                    e.motion.vy = self.grab_vy;
                    e.routine = 0x34;
                }
                0x34 => {
                    // $19E42: grow; then sink with the player.
                    self.grow(e);
                    if self.pull(cx, e) {
                        e.motion.vy = self.hold_vy;
                        e.vars[ESCAPE_GATE] = 3;
                        e.routine = 0x38;
                        continue;
                    }
                    let top = e.vars[GRAB_HOME] + self.rise;
                    if e.y < top {
                        e.y = top;
                        e.motion.vy = 0;
                    }
                    return;
                }
                0x38 => {
                    // $19E8C: held until repeated input edges free Joe.
                    if cx.player_state.is_some_and(crate::movement::dying) || self.escape(cx, e) {
                        e.vars[GRAB_WAIT] = self.escape_wait;
                        e.routine = 0x3c;
                        e.vars[GRAB_WAIT] -= 1; // $19EBE falls into $19EC8.
                        return;
                    }
                    self.pull(cx, e);
                    if e.timer & self.shock_period == 0 {
                        // Func_01A038: plr_event $A drains a health point.
                        crate::bosses::boss3::shock(cx, 1);
                    }
                    if e.vars[GRAB_HOME] < e.y {
                        e.y = e.vars[GRAB_HOME];
                        e.motion.vy = 0;
                    }
                    return;
                }
                0x3c => {
                    // $19EC8
                    e.vars[GRAB_WAIT] -= 1;
                    if e.vars[GRAB_WAIT] < 0 {
                        e.routine = 8;
                    }
                    return;
                }
                r if r == self.death => {
                    cx.burst(e.x, e.y);
                    Ctx::free(e);
                    return;
                }
                _ => {
                    // $19EF0: hurt resumes.
                    e.routine = e.saved_routine;
                    return;
                }
            }
        }
    }
}
