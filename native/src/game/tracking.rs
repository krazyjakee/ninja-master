//! Native event predicates for the local achievement set. See native/ACHIEVEMENTS.txt.
use super::*;
use crate::{combat::EnemyState, profile::Encounter};
impl Game {
    pub(super) fn track_begin(&mut self) {
        self.run.stats.ticks += 1;
        self.run.round_ticks += 1;
        self.run.practice |= self.options.infinite
            || self.options.invincible
            || self.options.level != self.run.level;
        if self.run.encounter.is_none() && !self.run.encounter_finished && self.death.is_none() {
            let boss = self.combat.enemies.iter().any(|e| {
                self.combat.spawns[e.spawn].kind == crate::bosses::BOSS_KIND
                    && e.health > 0
                    && e.live()
            });
            let mini = [0xb8, 0x1c, 0xc0, 0x18, 0x10, 0xf8]
                .get(self.stage_id.round_index())
                .copied();
            let mini_active = self.stage_id.substage == 1
                && mini.is_some_and(|kind| {
                    self.combat.enemies.iter().any(|e| {
                        self.combat.spawns[e.spawn].kind == kind
                            && e.live()
                            && e.state != EnemyState::Dormant
                            && e.health > 0
                            && (kind != 0xc0 || e.routine >= 8)
                            && !e.dying
                    })
                });
            if boss || mini_active {
                let index = if boss {
                    14 + self.stage_id.round_index()
                } else {
                    8 + self.stage_id.round_index()
                };
                self.run.encounter = Some(Encounter {
                    achievement: index,
                    start: self.run.stats.clone(),
                    jumped: false,
                    floor: false,
                    wrong_attack: false,
                    damaged: false,
                    mijin: false,
                    hits: 0,
                });
            }
        }
    }
    pub(super) fn track_end(&mut self) {
        let journal = std::mem::take(&mut self.combat.journal);
        self.run.stats.shurikens += journal.shurikens;
        self.run.stats.ninjutsu += journal.ninjutsu;
        self.run.hits(&journal.hits);
        if let Some(e) = &mut self.run.encounter {
            e.jumped |= self.player.jump_requested || self.player.wall_kicked;
            e.floor |= self.player.grounded;
            e.mijin |= journal.mijin;
        }
        // The capsule encounter has a scripted victory before the floor sinks.
        let capsules_cleared = self
            .run
            .encounter
            .as_ref()
            .is_some_and(|e| e.achievement == 10)
            && self
                .combat
                .enemies
                .iter()
                .any(|e| self.combat.spawns[e.spawn].kind == 0xc0 && e.routine >= 0x10);
        let next = self.requests & REQUEST_NEXT_SUBSTAGE != 0;
        if capsules_cleared || next {
            self.finish_encounter();
        }
        self.track_thresholds();
        if next && !self.run.clear_recorded {
            self.run.clear_recorded = true;
            self.run
                .completed_stages
                .insert(crate::practice::stage_key(self.stage_id));
            if self
                .next_stage()
                .is_none_or(|id| id.round != self.stage_id.round)
            {
                let round = self.stage_id.round_index();
                if round < 7 {
                    self.run.stats.rounds += 1;
                    self.run.cleared |= 1 << round;
                    self.run.award(round);
                    if !self.round_flags.ninjutsu {
                        self.run.award(24 + round);
                    }
                    if self.combat.hud.health == self.combat.hud.max_health
                        && self.combat.hud.health > 0
                    {
                        self.run.award(32);
                    }
                    if !self.round_flags.thrown && self.run.round_start_ammo >= 10 {
                        self.run.award(33);
                    }
                    if !self.run.round_died
                        && !self.round_flags.damaged
                        && !self.round_flags.thrown
                        && !self.round_flags.ninjutsu
                    {
                        self.run.award(34);
                    }
                    let time = self.run.round_ticks;
                    self.run.best_rounds[round] =
                        Some(self.run.best_rounds[round].map_or(time, |old| old.min(time)));
                    if round == 6 {
                        self.run.stats.completions += 1;
                        if self.run.cleared == 0x7f {
                            if self.run.level == 3 && self.run.stats.continues == 0 {
                                self.run.award(7);
                            }
                            if self.run.stats.ninjutsu == 0 {
                                self.run.award(31);
                            }
                        }
                    }
                }
            }
        }
        self.track_secrets();
    }
    pub(super) fn track_thresholds(&mut self) {
        for (i, extra) in [100, 200, 300].into_iter().enumerate() {
            if self.combat.hud.ammo >= self.run.initial_ammo.saturating_add(extra) {
                self.run.award(21 + i);
            }
        }
        if self.run.level == 3 && self.combat.hud.lives >= 9 {
            self.run.award(35);
        }
        if self.run.level != 1 {
            for (i, points) in [50_000, 150_000, 300_000].into_iter().enumerate() {
                if self.combat.hud.score >= points {
                    self.run.award(36 + i);
                }
            }
        }
    }
    fn finish_encounter(&mut self) {
        let Some(e) = self.run.encounter.take() else {
            return;
        };
        self.run.encounter_finished = true;
        let stars = self.run.stats.shurikens > e.start.shurikens;
        let magic = self.run.stats.ninjutsu > e.start.ninjutsu;
        let died = self.run.stats.deaths > e.start.deaths;
        let pass = match e.achievement {
            8 => !e.damaged && !died && !stars && !magic && !e.jumped,
            10 => !e.damaged && !died && !magic && !e.floor,
            11 => !died && !stars && !e.mijin && !e.wrong_attack && e.hits > 0,
            12 => !e.damaged && !died && !magic && !stars && !e.wrong_attack && e.hits > 0,
            14 => !e.damaged && !died && !stars && !magic,
            _ => !e.damaged && !died && !magic,
        };
        if pass {
            self.run.award(e.achievement);
        }
    }
    pub(super) fn track_pickups(&mut self, pickups: &[(u8, i32, i32)]) {
        // The high pipe in 4-2's final part holds the hidden 1-Up.
        if self.stage_id == StageId::new(4, 2, 2)
            && pickups
                .iter()
                .any(|&(kind, x, y)| kind == 3 && (656..720).contains(&x) && (-64..96).contains(&y))
        {
            self.run.award(43);
        }
    }
    fn track_secrets(&mut self) {
        if self.death.is_some() || self.combat.hud.health == 0 {
            return;
        }
        let (x, y) = self.player.position();
        for secret in SECRETS {
            if self.stage_id == secret.stage && secret.contains(x, y) {
                self.run.award(secret.achievement);
            }
        }
    }
}
/// Feet coordinates inside the optional rooms, away from their entrance shafts.
/// These regions are bounded by the ROM layouts and their list-6 item records;
/// all three supported regional images use the same world coordinates.
#[derive(Clone, Copy)]
struct Secret {
    achievement: usize,
    stage: StageId,
    x: (i32, i32),
    y: (i32, i32),
}
impl Secret {
    fn contains(self, x: i32, y: i32) -> bool {
        (self.x.0..self.x.1).contains(&x) && (self.y.0..self.y.1).contains(&y)
    }
}
const SECRETS: [Secret; 9] = [
    Secret {
        achievement: 39,
        stage: StageId::new(1, 2, 0),
        x: (800, 944),
        y: (96, 193),
    },
    Secret {
        achievement: 40,
        stage: StageId::new(1, 2, 2),
        x: (1664, 1744),
        y: (32, 129),
    },
    Secret {
        achievement: 41,
        stage: StageId::new(2, 2, 0),
        x: (768, 912),
        y: (16, 81),
    },
    Secret {
        achievement: 42,
        stage: StageId::new(3, 2, 0),
        x: (2624, 2816),
        y: (184, 257),
    },
    Secret {
        achievement: 44,
        stage: StageId::new(5, 2, 0),
        x: (32, 288),
        y: (336, 385),
    },
    Secret {
        achievement: 45,
        stage: StageId::new(5, 2, 1),
        x: (0, 128),
        y: (48, 97),
    },
    Secret {
        achievement: 46,
        stage: StageId::new(5, 2, 2),
        x: (0, 128),
        y: (64, 113),
    },
    Secret {
        achievement: 47,
        stage: StageId::new(5, 2, 3),
        x: (416, 496),
        y: (1584, 1649),
    },
    Secret {
        achievement: 48,
        stage: StageId::new(7, 2, 2),
        x: (1616, 1696),
        y: (16, 65),
    },
];

#[cfg(test)]
#[path = "tracking_tests.rs"]
mod tests;
