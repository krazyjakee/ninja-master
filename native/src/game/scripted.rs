//! Repeatable traversal input for play without a pad.
use super::Game;
use crate::input::{Button, Input};

/// Repeatable traversal input shared by desktop and headless play mode.
/// Attacks come mid-jump: ground attacks and landings stop the original's
/// horizontal motion, which would stall the traversal.
pub fn scripted_input(tick: u64) -> Input {
    let phase = tick % 90;
    let direction = if tick == 1 { 0 } else { Button::Right.mask() };
    let attack = if phase == 40 || phase == 70 {
        Button::Attack.mask()
    } else {
        0
    };
    let held = direction
        | if (24..54).contains(&phase) {
            Button::Jump.mask()
        } else {
            0
        }
        | attack;
    Input {
        held,
        pressed: (if phase == 24 { Button::Jump.mask() } else { 0 }) | attack,
    }
}

impl Game {
    /// State-aware deterministic tape for the native traversal demonstration.
    /// It faces nearby enemies and fights from melee range; manual input is untouched.
    pub fn scripted_input(&self, tick: u64) -> Input {
        let mut input = scripted_input(tick);
        let (x, y) = self.player.position();
        let target = self
            .combat
            .enemies
            .iter()
            .filter(|enemy| {
                enemy.state != crate::combat::EnemyState::Defeated
                    && (enemy.y - y).abs() < 72
                    && (enemy.x - x).abs() < 192
            })
            .min_by_key(|enemy| (enemy.x - x).abs());
        if let Some(enemy) = target {
            let left = enemy.x < x;
            // The samurai's independent body pusher rejects a stance
            // inside its mapping box. Stay outside it, within sword reach.
            let distance = if matches!(self.combat.spawns[enemy.spawn].kind, 0xb4 | 0xb8) {
                56
            } else {
                40
            };
            input.held &= !(Button::Left.mask() | Button::Right.mask());
            if (enemy.x - x).abs() > distance || self.player.facing_left != left {
                input.held |= if left {
                    Button::Left.mask()
                } else {
                    Button::Right.mask()
                };
            }
            // Land and hold a reproducible attack stance instead of jumping
            // away from a stationary encounter every ninety ticks.
            input.held &= !Button::Jump.mask();
            input.pressed &= !Button::Jump.mask();
            input.held &= !Button::Attack.mask();
            input.pressed &= !Button::Attack.mask();
            // Ground attacks stand still ($20CDA), so only swing in reach.
            if tick.is_multiple_of(20)
                && (enemy.x - x).abs() <= 64
                && self.player.facing_left == left
            {
                input.held |= Button::Attack.mask();
                input.pressed |= Button::Attack.mask();
                // Alternate crouching sword attacks to conserve shurikens,
                // as the full-game tape's forest controller does.
                if (tick / 20).is_multiple_of(2) {
                    input.held |= Button::Down.mask();
                }
            }
        }
        input
    }
}
