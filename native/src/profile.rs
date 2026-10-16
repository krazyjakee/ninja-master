//! Deterministic run journal and optional, local-only persistent player profile.
mod catalog;
mod presentation;
mod screen;
pub use catalog::{Achievement, CATALOG};
pub use screen::{ProfileScreen, ScreenKind};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub ticks: u64,
    pub completions: u64,
    pub rounds: u64,
    pub deaths: u64,
    pub continues: u64,
    pub enemies: u64,
    pub shurikens: u64,
    pub melee_hits: u64,
    pub ninjutsu: u64,
    pub pickups: u64,
}
impl Stats {
    /// Credit only progress beyond each run's persisted high-water marks.
    fn credit(&mut self, now: &Self, high: &mut Self) {
        macro_rules! credit { ($($field:ident),*) => {$(
            self.$field = self.$field.saturating_add(now.$field.saturating_sub(high.$field));
            high.$field = high.$field.max(now.$field);
        )*}; }
        credit!(
            ticks,
            completions,
            rounds,
            deaths,
            continues,
            enemies,
            shurikens,
            melee_hits,
            ninjutsu,
            pickups
        );
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitKind {
    Shuriken,
    Melee,
    DeathKick,
    Board,
    Magic,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub kind: HitKind,
    pub target: u16,
    pub defeated: bool,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct CombatJournal {
    pub shurikens: u64,
    pub ninjutsu: u64,
    pub mijin: bool,
    pub hits: Vec<Hit>,
}
#[derive(Clone, Debug)]
pub(crate) struct Encounter {
    pub achievement: usize,
    pub start: Stats,
    pub jumped: bool,
    pub floor: bool,
    pub wrong_attack: bool,
    pub damaged: bool,
    pub mijin: bool,
    pub hits: u64,
}
/// Carried with gameplay and reconstructed by save-history replay.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub id: u64,
    pub stats: Stats,
    pub practice: bool,
    pub level: u8,
    pub candidates: u64,
    pub cleared: u8,
    pub best_rounds: [Option<u64>; 7],
    pub completed_stages: BTreeSet<String>,
    pub round_ticks: u64,
    pub round_died: bool,
    pub round_start_ammo: u16,
    pub initial_ammo: u16,
    #[serde(skip)]
    pub(crate) encounter: Option<Encounter>,
    #[serde(skip)]
    pub(crate) clear_recorded: bool,
    #[serde(skip)]
    pub(crate) encounter_finished: bool,
}
impl Run {
    pub(crate) fn award(&mut self, index: usize) {
        self.candidates |= 1u64 << index;
    }
    pub(crate) fn hits(&mut self, hits: &[Hit]) {
        for hit in hits {
            self.stats.enemies += u64::from(hit.defeated);
            self.stats.melee_hits += u64::from(matches!(
                hit.kind,
                HitKind::Melee | HitKind::DeathKick | HitKind::Board
            ));
            if let Some(e) = &mut self.encounter {
                // Boss parts count as encounter hits; ordinary enemies do not
                // invalidate the restricted attacks used on a midboss.
                let target = match e.achievement {
                    8 => 0xb8,
                    9 => 0x1c,
                    10 => 0xfc,
                    11 => 0x18,
                    12 => 0x10,
                    13 => 0xf8,
                    _ => crate::bosses::BOSS_KIND,
                };
                if hit.target == target {
                    e.hits += 1;
                    e.wrong_attack |= match e.achievement {
                        11 => hit.kind != HitKind::Board && hit.kind != HitKind::Magic,
                        12 => hit.kind != HitKind::DeathKick,
                        _ => false,
                    };
                }
            }
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub version: u32,
    pub next_run: u64,
    pub lifetime: Stats,
    pub practice: Stats,
    pub unlocked: BTreeMap<usize, u64>,
    pub credited: BTreeMap<u64, Stats>,
    pub tainted: BTreeSet<u64>,
    pub best_rounds: [Option<u64>; 7],
    pub completed_stages: BTreeSet<String>,
    pub last_run: Option<Run>,
    pub time_trials: BTreeMap<String, crate::practice::Record>,
}
#[derive(Default)]
pub struct Profile {
    pub data: Data,
    path: Option<PathBuf>,
    pub status: String,
    readonly: bool,
    dirty: bool,
    last_flush: u64,
    pub(crate) notices: VecDeque<usize>,
    pub(crate) notice_ticks: u16,
}
impl Profile {
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut profile = Self::default();
        profile.path = path;
        if let Some(path) = &profile.path {
            match std::fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<Data>(&bytes) {
                    Ok(data) if data.version <= 1 => profile.data = data,
                    Ok(_) => {
                        profile.status = "NEWER PROFILE VERSION - READ ONLY".into();
                        profile.readonly = true;
                    }
                    Err(_) => {
                        profile.status = "PROFILE DAMAGED - FILE PRESERVED".into();
                        profile.readonly = true;
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    profile.status = format!("PROFILE READ ERROR: {error}");
                    profile.readonly = true;
                }
            }
        }
        profile
    }
    pub(crate) fn begin(&mut self, run: &mut Run) {
        self.data.next_run = self.data.next_run.saturating_add(1).max(1);
        run.id = self.data.next_run;
        self.dirty = true;
    }
    pub fn observe(&mut self, run: &Run) {
        if run.id == 0 {
            return;
        } // Legacy saves and demos have no profile identity.
        self.data.next_run = self.data.next_run.max(run.id);
        if run.practice {
            self.data.tainted.insert(run.id);
        }
        let practice = run.practice || self.data.tainted.contains(&run.id);
        let high = self.data.credited.entry(run.id).or_default();
        if practice {
            self.data.practice.credit(&run.stats, high);
        } else {
            self.data.lifetime.credit(&run.stats, high);
        }
        if !practice {
            self.data
                .completed_stages
                .extend(run.completed_stages.iter().cloned());
            for index in 0..CATALOG.len() {
                if run.candidates & (1u64 << index) != 0 && !self.data.unlocked.contains_key(&index)
                {
                    self.data.unlocked.insert(index, timestamp());
                    self.notices.push_back(index);
                }
            }
            for (best, &time) in self.data.best_rounds.iter_mut().zip(&run.best_rounds) {
                if let Some(time) = time {
                    *best = Some(best.map_or(time, |old| old.min(time)));
                }
            }
        }
        let mut snapshot = run.clone();
        snapshot.practice = practice;
        self.data.last_run = Some(snapshot);
        self.dirty = true;
    }
    pub(crate) fn record_trial(
        &mut self,
        stage: crate::stage::StageId,
        attempt: crate::practice::Attempt,
    ) -> bool {
        if !attempt.completed || !attempt.valid || !attempt.record.valid() {
            return false;
        }
        let key = crate::practice::stage_key(stage);
        if self
            .data
            .time_trials
            .get(&key)
            .is_some_and(|old| old.valid() && old.ticks <= attempt.record.ticks)
        {
            return false;
        }
        self.data.time_trials.insert(key, attempt.record);
        self.dirty = true;
        self.flush();
        true
    }
    pub(crate) fn taint(&mut self, run: &mut Run) {
        // Keep pre-modification progress, then make eligibility sticky across
        // rewinds, even to a slot saved before the modification.
        run.practice = true;
        if run.id != 0 {
            self.data.tainted.insert(run.id);
        }
        self.observe(run);
        self.flush();
    }
    pub fn flush(&mut self) {
        if !self.dirty || self.readonly {
            return;
        }
        let Some(path) = &self.path else {
            return;
        };
        let result = (|| -> Result<(), String> {
            let parent = path.parent().ok_or("profile path has no parent")?;
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            self.data.version = 1;
            let bytes = serde_json::to_vec_pretty(&self.data).map_err(|e| e.to_string())?;
            let temporary = path.with_extension("json.tmp");
            use std::io::Write;
            let mut file = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            std::fs::rename(temporary, path).map_err(|e| e.to_string())?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.dirty = false;
                self.status.clear();
            }
            Err(error) => self.status = format!("PROFILE SAVE ERROR: {error}"),
        }
    }
    pub(crate) fn tick(&mut self) {
        self.advance(true);
    }
    pub(crate) fn tick_without_notices(&mut self) {
        self.advance(false);
    }
    fn advance(&mut self, show_notices: bool) {
        if show_notices {
            if self.notice_ticks > 0 {
                self.notice_ticks -= 1;
                if self.notice_ticks == 0 {
                    self.notices.pop_front();
                }
            }
            if self.notice_ticks == 0 && !self.notices.is_empty() {
                self.notice_ticks = 240;
            }
        }
        self.last_flush += 1;
        if self.last_flush >= 300 {
            self.last_flush = 0;
            self.flush();
        }
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        self.flush();
    }
}
fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
pub fn duration(ticks: u64) -> String {
    let seconds = ticks / 60;
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests;
