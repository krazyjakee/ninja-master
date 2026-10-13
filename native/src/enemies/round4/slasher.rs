//! Type `$E4`, `EnemyE4` (`$1AA48`): the wave slasher.
use super::count_down;
use crate::{
    combat::{Enemy, EnemyState, Projectile, Spawn},
    enemies::{Behavior, Ctx, shot_damage, stop},
    game::Terrain,
    rom::{Result, signed, word, word_i},
};

/// `$2A` steps/timer, `$2C` table index, `$2E`/`$30` step delay and rate,
/// `$32` steps taken, `$34` post X.
const S_COUNT: usize = 0;
const S_INDEX: usize = 1;
const S_DELAY: usize = 2;
const S_RATE: usize = 3;
const S_TAKEN: usize = 4;
const S_HOME: usize = 5;
const S_FLAGS: usize = 6;

/// `$E4`: faces the player; within 160 px it steps toward it (16 steps of
/// `Data_01A9F0`, one every six ticks) until 48 px away, swings through
/// `Data_01AA10` releasing a slash wave (list-4 `$35`, 8 px/tick for 11
/// ticks, 32 px up, or down when hung upside down by spawn flag bit 0),
/// then steps back to its post. Two hits kill (the shared arc).
pub(crate) struct Slasher {
    stand: u16,
    dead: u16,
    advance_steps: [(u16, i32); 4],
    retreat_steps: [(u16, i32); 4],
    swing: Vec<(u16, i32)>,
    range: i32,
    close: i32,
    steps: i32,
    rate: i32,
    pause: i32,
    trigger: (u16, i32),
    slash: u16,
    slash_damage: u8,
}
impl Slasher {
    pub(super) fn load(rom: &[u8]) -> Result<Self> {
        let steps = |p: usize| -> Result<[(u16, i32); 4]> {
            let mut out = [(0, 0); 4];
            for (i, step) in out.iter_mut().enumerate() {
                *step = (word(rom, p + i * 4)?, signed(rom, p + i * 4 + 2)?);
            }
            Ok(out)
        };
        let mut swing = Vec::new();
        for i in 0..4 {
            let entry = (word(rom, 0x1aa10 + i * 4)?, signed(rom, 0x1aa12 + i * 4)?);
            swing.push(entry);
            if entry.1 < 0 {
                break;
            }
        }
        Ok(Self {
            stand: word(rom, 0x1a9e0)?,
            dead: word(rom, 0x1aa20)?,
            advance_steps: steps(0x1a9f0)?,
            retreat_steps: steps(0x1aa00)?,
            swing,
            range: word_i(rom, 0x1aab2)?,
            close: word_i(rom, 0x1aade)?,
            steps: word_i(rom, 0x1aab8)?,
            rate: word_i(rom, 0x1aac4)?,
            pause: word_i(rom, 0x1ab00)?,
            trigger: (word(rom, 0x1ab16)?, word_i(rom, 0x1ab20)?),
            slash: word(rom, 0x1ac56)?,
            slash_damage: shot_damage(rom, 0x35)?,
        })
    }
    /// `Func_01ABD4`: every `rate + 1` ticks the next step pose and shift.
    fn step(&self, e: &mut Enemy, table: &[(u16, i32); 4]) -> bool {
        if count_down(&mut e.vars[S_DELAY]) {
            return e.vars[S_COUNT] >= 0;
        }
        e.vars[S_DELAY] = e.vars[S_RATE];
        let (mapping, dx) = table[(e.vars[S_INDEX] & 3) as usize];
        e.vars[S_INDEX] += 1;
        e.mapping = mapping;
        e.x += if e.facing_left { -dx } else { dx };
        e.vars[S_COUNT] -= 1;
        e.vars[S_COUNT] >= 0
    }
    fn walk(&self, e: &mut Enemy, count: i32) {
        e.vars[S_COUNT] = count;
        e.vars[S_INDEX] = 0;
        e.vars[S_DELAY] = self.rate;
        e.vars[S_RATE] = self.rate;
    }
    /// `Func_01AC12`: the slash wave.
    fn wave(&self, cx: &mut Ctx, e: &Enemy) {
        let upside_down = e.vars[S_FLAGS] & 1 != 0;
        let dy = if upside_down { 32 } else { -32 };
        let mut wave = Projectile::new(
            e.x,
            e.y + dy,
            if e.facing_left { 8 } else { -8 },
            true,
            self.slash,
            self.slash_damage,
        );
        wave.passes_terrain = true;
        wave.life = 11;
        cx.shoot(wave);
    }
}
impl Behavior for Slasher {
    fn label(&self, _kind: u16) -> String {
        "EnemyE4 $1AA48".into()
    }
    fn activate(&self, _cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn) {
        e.state = EnemyState::Waiting;
        e.vars[S_FLAGS] = i32::from(spawn.flags | 0x8000);
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, _spawn: &Spawn, terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        cx.process_hit(e);
        loop {
            match e.routine {
                0 => {
                    // $1AA5A: Object_SetTileBase, then flag bit 0 hangs
                    // it upside down (`ror.w #4`: V flip).
                    stop(e);
                    let flip = if e.vars[S_FLAGS] & 1 != 0 { 0x1000 } else { 0 };
                    e.tile = Some(cx.tile_base(0xe4) | flip);
                    e.mapping = self.stand;
                    e.death_routine = 0x18;
                    e.hurt_routine = 0x20;
                    e.vars[S_HOME] = e.x;
                    e.routine = 4;
                }
                4 => {
                    // $1AA9C: facing the player (flipped while it is right).
                    e.facing_left = !cx.player_left(e.x);
                    if cx.player_dx(e.x) >= self.range {
                        return;
                    }
                    self.walk(e, self.steps);
                    e.routine = 8;
                    return;
                }
                8 => {
                    // $1AAD4
                    if cx.player_dx(e.x) >= self.close && self.step(e, &self.advance_steps) {
                        return;
                    }
                    e.vars[S_TAKEN] = e.vars[S_COUNT];
                    e.mapping = self.stand;
                    e.vars[S_COUNT] = self.pause;
                    e.vars[S_INDEX] = 0;
                    e.routine = 0x0c;
                    return;
                }
                0x0c => {
                    // $1AB10: the swing script.
                    if e.mapping == self.trigger.0 && e.vars[S_COUNT] == self.trigger.1 {
                        self.wave(cx, e);
                    }
                    if count_down(&mut e.vars[S_COUNT]) {
                        return;
                    }
                    let index = e.vars[S_INDEX] as usize;
                    e.vars[S_INDEX] += 1;
                    let (mapping, ticks) =
                        self.swing.get(index).copied().unwrap_or((self.stand, -1));
                    e.mapping = mapping;
                    e.vars[S_COUNT] = ticks;
                    if ticks >= 0 {
                        return;
                    }
                    e.vars[S_COUNT] = self.pause;
                    e.routine = 0x10;
                }
                0x10 => {
                    // $1AB5A
                    if count_down(&mut e.vars[S_COUNT]) {
                        return;
                    }
                    let taken = e.vars[S_TAKEN];
                    let back = if taken >= 0 {
                        self.steps - taken
                    } else {
                        self.steps
                    };
                    self.walk(e, back);
                    e.routine = 0x14;
                }
                0x14 => {
                    // $1AB86: back to the post.
                    if self.step(e, &self.retreat_steps) {
                        return;
                    }
                    e.x = e.vars[S_HOME];
                    e.mapping = self.stand;
                    e.routine = 4;
                    return;
                }
                0x18 => {
                    // $1ABAA
                    cx.death_arc(e, self.dead);
                    e.routine = 0x1c;
                    return;
                }
                0x1c => {
                    // $1ABBC
                    if cx.landed_blast(e, terrain) {
                        Ctx::free(e);
                    }
                    return;
                }
                _ => {
                    // $1ABCC
                    e.routine = e.saved_routine;
                    return;
                }
            }
        }
    }
}
