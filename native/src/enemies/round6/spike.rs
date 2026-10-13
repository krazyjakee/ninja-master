//! Types `$8C` and `$90`, `Enemy8C` (`$17E02`) / `Enemy90` (`$17F28`): floor spikes.
use crate::{
    combat::{Enemy, Spawn},
    enemies::{Behavior, Ctx, HitResponse},
    game::Terrain,
    rom::{Result, byte, signed, word, word_i},
};

/// `$2A` countdown, `$2C` thrust ticks, `$2E` rest ticks, `$30`/`$32`
/// thrust and retract steps, `$34` home Y.
const COUNT: usize = 0;
const THRUST: usize = 1;
const REST: usize = 2;
const UP: usize = 3;
const DOWN: usize = 4;
const HOME: usize = 5;

/// `$8C`/`$90`: a spike (`$377`) hidden 80 px below its spawn point. It
/// thrusts up 8 px/tick for N + 1 ticks (spawn flags bits 4-7), holds 61
/// ticks, sinks back 1 px/tick and rests 60 + 32 * M + 1 ticks (bits 8-11)
/// before the next thrust. `$90` waits for the player within 56 px before
/// each thrust. Contact damage 1; it cannot be destroyed.
pub(crate) struct Spike {
    handler: usize,
    mapping: u16,
    depth: i32,
    up: i32,
    down: i32,
    hold: i32,
    rest_base: i32,
    /// `$90`: the thrust waits for the player within this distance.
    reach: Option<i32>,
    sound: u8,
}
impl Spike {
    pub(super) fn load(rom: &[u8], handler: usize) -> Result<Self> {
        // $8C and $90 share one layout; $90's routine 4 ($17FD2) adds the
        // trigger test, moving the later immediates by 16 bytes.
        let at = |offset: usize| handler + offset;
        let trigger = handler == 0x17f28;
        let later = |offset: usize| {
            if trigger {
                at(offset) + 0x10
            } else {
                at(offset)
            }
        };
        Ok(Self {
            handler,
            mapping: word(rom, handler - 2)?,
            depth: word_i(rom, at(0x76))?,
            up: signed(rom, at(0x7a))?,
            down: signed(rom, at(0x7e))?,
            hold: word_i(rom, later(0xce))?,
            rest_base: word_i(rom, at(0x6e))?,
            reach: if trigger {
                Some(word_i(rom, 0x17fdc)?)
            } else {
                None
            },
            sound: byte(rom, later(0xb5))?,
        })
    }
}
impl Behavior for Spike {
    fn label(&self, _kind: u16) -> String {
        if self.reach.is_some() {
            format!("Enemy90 ${:05X}", self.handler)
        } else {
            format!("Enemy8C ${:05X}", self.handler)
        }
    }
    fn update(&self, cx: &mut Ctx, e: &mut Enemy, spawn: &Spawn, _terrain: &dyn Terrain) {
        e.timer = e.timer.wrapping_add(1);
        // $12880: the body only (see the module notes). The death routine
        // follows the current routine, so it is never killed.
        e.death_routine = e.routine;
        match e.routine {
            0 => {
                // $17E32: flags N (bits 4-7) and M (bits 8-11); bit 0 hangs
                // it from the ceiling (vertical flip, reversed steps).
                e.motion.stop();
                e.timer = 0;
                e.mapping = self.mapping;
                // Enemy8C/90 never call Object_SetTileBase: obj_tile stays
                // zero from the cleared object record, and mapping $377's
                // $528/$52A attributes address resident art directly.
                e.tile = Some(0);
                e.vars[THRUST] = i32::from((spawn.flags >> 4) & 0xf);
                e.vars[REST] = (i32::from(spawn.flags & 0xf00) >> 3) + self.rest_base;
                let hanging = spawn.flags & 1 != 0;
                let sign = if hanging { -1 } else { 1 };
                e.y += sign * self.depth;
                e.vars[HOME] = e.y;
                e.vars[UP] = sign * self.up;
                e.vars[DOWN] = sign * self.down;
                e.routine = 4;
            }
            4 => {
                // $17EAC (+ $17FD2 for $90): arm the thrust.
                if let Some(reach) = self.reach
                    && cx.player_dx(e.x) >= reach
                {
                    return;
                }
                e.vars[COUNT] = e.vars[THRUST];
                e.routine = 8;
                cx.visible_sound(e.x, e.y, self.sound);
            }
            8 => {
                // $17EBE: thrust.
                e.y += e.vars[UP];
                e.vars[COUNT] -= 1;
                if e.vars[COUNT] >= 0 {
                    return;
                }
                e.vars[COUNT] = self.hold;
                e.routine = 0x0c;
            }
            0x0c => {
                // $17EDA: hold.
                e.vars[COUNT] -= 1;
                if e.vars[COUNT] >= 0 {
                    return;
                }
                e.routine = 0x10;
            }
            0x10 => {
                // $17EE8: retract to the home Y.
                e.y += e.vars[DOWN];
                let hanging = spawn.flags & 1 != 0;
                if (e.y < e.vars[HOME]) != hanging && e.y != e.vars[HOME] {
                    return;
                }
                e.y = e.vars[HOME];
                e.vars[COUNT] = e.vars[REST];
                e.routine = 0x14;
            }
            _ => {
                // $17F16: rest.
                e.vars[COUNT] -= 1;
                if e.vars[COUNT] >= 0 {
                    return;
                }
                e.routine = 4;
            }
        }
    }
    fn hit(&self, enemy: &Enemy, _kind: u16) -> HitResponse {
        if enemy.routine == 0 {
            HitResponse::Pass
        } else {
            HitResponse::Ignore
        }
    }
    fn integrates(&self) -> bool {
        // Moved by whole pixels; Object_Integrate has no velocity to add.
        false
    }
}
