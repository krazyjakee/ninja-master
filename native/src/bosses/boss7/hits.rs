//! Weapon hits (`Func_03417C`): the parry, the wound and the knock-back.
use super::*;

impl Boss7 {
    /// `Func_03417C` hit part: the reaction recorded by the weapon pass.
    pub(super) fn hits(&self, cx: &mut Ctx, e: &mut Enemy) {
        let Some(reaction) = e.reaction.take() else {
            return;
        };
        if e.state == EnemyState::Hurt {
            e.state = EnemyState::Waiting;
        }
        match reaction {
            Reaction::Deflect => {
                let parrying = matches!(e.routine, 0x0c | 0x14);
                if e.routine == 0x0c {
                    Self::face(cx, e);
                }
                // Func_034380: spark and effect 8, at most every 15 ticks.
                if Self::misc(e, M_DEFLECT) == 0 {
                    Self::set_misc(e, M_DEFLECT, self.deflect_cooldown);
                    cx.spark(e, false);
                    cx.effect_sound(self.deflect_sound);
                }
                if parrying {
                    self.parry(cx, e);
                }
            }
            Reaction::Hurt | Reaction::Death => {
                let damage = if e.vars[FLAGS] & SECOND != 0 {
                    e.vars[HEALTH] - (i32::from(e.health) - 1)
                } else {
                    i32::from(POOL) - i32::from(e.health)
                };
                self.wound(cx, e, damage.max(0));
            }
        }
    }
    /// `$3425C`: a blocked weapon in routines `$C`/`$14`.
    fn parry(&self, cx: &mut Ctx, e: &mut Enemy) {
        if e.vars[FLAGS] & BROKEN != 0 {
            return;
        }
        e.routine = 0x14;
        Self::face(cx, e);
        e.vars[SHOVE_TIMER] = 30;
        // $34286: thrown shurikens select a guard pose but cannot break it.
        let melee = cx.player_state == Some(crate::movement::state::AIR_STAB)
            || cx
                .player_state
                .is_some_and(|state| self.player_classes.get(state) == Some(&1));
        let parries = Self::misc(e, M_PARRIES) + i32::from(melee);
        Self::set_misc(e, M_PARRIES, parries);
        if parries >= 2 {
            e.vars[FLAGS] |= BROKEN;
            Self::start(e, self.broken_script);
            self.voice(cx, self.broken_voice);
            return;
        }
        // High or low guard by the height of the attack object.
        let attack_y = cx
            .attacks
            .iter()
            .filter(|&&(_, _, high_priority)| high_priority == cx.current_high_priority())
            .min_by_key(|&&(x, y, _)| (x - e.x).abs() + (y - e.y).abs())
            .map_or(cx.player.1 - 27, |&(_, y, _)| y);
        let low = ((e.y - attack_y) as u16) < self.low_range as u16;
        Self::start(
            e,
            if low {
                self.high_script
            } else {
                self.low_script
            },
        );
    }
    /// `Func_03455E` + `Func_0345FC`: health, the 4-hit combo, the flash.
    fn wound(&self, cx: &Ctx, e: &mut Enemy, damage: i32) {
        e.vars[SHOVE_TIMER] = 15;
        e.flash = if e.mapping == self.guard[0] {
            self.flash.1
        } else {
            self.flash.0
        };
        if e.vars[FLAGS] & MIJIN != 0 {
            return;
        }
        e.vars[HEALTH] -= damage;
        if e.vars[FLAGS] & SECOND != 0 {
            e.vars[HEALTH] = i32::from(e.health) - 1;
        }
        if e.vars[HEALTH] < 0 {
            self.knock(cx, e);
            return;
        }
        let combo = if Self::misc(e, M_COMBO_TIMER) == 0 {
            1
        } else {
            Self::misc(e, M_COMBO) + 1
        };
        Self::set_misc(e, M_COMBO_TIMER, self.combo_window);
        Self::set_misc(e, M_COMBO, combo.min(15));
        if combo >= self.combo_max {
            self.knock(cx, e);
        }
    }
    /// `Func_0345C4`: knocked back away from the player.
    pub(super) fn knock(&self, cx: &Ctx, e: &mut Enemy) {
        Self::face(cx, e);
        e.routine = 0x20;
        e.motion.vx = -Self::sign(e) * self.knock_vx;
        e.motion.vy = self.knock_vy;
        e.motion.gravity = self.gravity;
        e.motion.ax = 0;
        e.vars[COUNT] = self.knock_wait;
    }
}
