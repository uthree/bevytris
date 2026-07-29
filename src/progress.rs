//! Stage progression: which stages are unlocked and the best grade earned
//! on each. Persisted as RON next to the settings file.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::ai::MAX_STAGE;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grade {
    D,
    C,
    B,
    A,
    S,
}

impl Grade {
    pub fn letter(self) -> &'static str {
        match self {
            Grade::S => "S",
            Grade::A => "A",
            Grade::B => "B",
            Grade::C => "C",
            Grade::D => "D",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Grade::S => Color::srgb(1.0, 0.85, 0.2),
            Grade::A => Color::srgb(0.4, 0.95, 1.0),
            Grade::B => Color::srgb(0.4, 0.95, 0.5),
            Grade::C => Color::srgb(0.8, 0.8, 0.85),
            Grade::D => Color::srgb(0.7, 0.55, 0.45),
        }
    }

    fn rank(self) -> u32 {
        match self {
            Grade::D => 0,
            Grade::C => 1,
            Grade::B => 2,
            Grade::A => 3,
            Grade::S => 4,
        }
    }

    pub fn better_than(self, other: Grade) -> bool {
        self.rank() > other.rank()
    }
}

#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    /// Highest stage the player may enter (1-based).
    pub unlocked: u32,
    pub grades: HashMap<u32, Grade>,
    /// Zone battle runs its own 30-stage campaign, tracked separately.
    #[serde(default = "first_stage")]
    pub zone_unlocked: u32,
    #[serde(default)]
    pub zone_grades: HashMap<u32, Grade>,
    /// Best sprint (40 lines) finish, in milliseconds.
    #[serde(default)]
    pub best_sprint_ms: Option<u64>,
    /// Lines cleared in zen across every session, ever. Zen has no score
    /// to beat and no run to finish, so this total (and the level it
    /// implies) is the whole of its progression.
    #[serde(default)]
    pub zen_lines: u64,
}

/// Lines per zen level. The level is cosmetic — it never touches gravity
/// — so it can climb forever without making the mode harder.
pub const ZEN_LINES_PER_LEVEL: u64 = 10;

/// Zen level for a lifetime line total (1-based).
pub fn zen_level_for(lines: u64) -> u64 {
    lines / ZEN_LINES_PER_LEVEL + 1
}

fn first_stage() -> u32 {
    1
}

impl Default for Progress {
    fn default() -> Self {
        Self {
            unlocked: 1,
            grades: HashMap::new(),
            zone_unlocked: 1,
            zone_grades: HashMap::new(),
            best_sprint_ms: None,
            zen_lines: 0,
        }
    }
}

impl Progress {
    pub fn is_unlocked(&self, stage: u32) -> bool {
        stage <= self.unlocked
    }

    pub fn is_zone_unlocked(&self, stage: u32) -> bool {
        stage <= self.zone_unlocked
    }

    /// Record a stage clear; returns true if this grade is a new best.
    pub fn record_clear(&mut self, stage: u32, grade: Grade) -> bool {
        self.unlocked = self.unlocked.max((stage + 1).min(MAX_STAGE));
        let new_best = match self.grades.get(&stage) {
            Some(old) => grade.better_than(*old),
            None => true,
        };
        if new_best {
            self.grades.insert(stage, grade);
        }
        save_progress(self);
        new_best
    }

    /// Record a zone battle stage clear; returns true on a new best grade.
    pub fn record_zone_clear(&mut self, stage: u32, grade: Grade) -> bool {
        self.zone_unlocked = self.zone_unlocked.max((stage + 1).min(MAX_STAGE));
        let new_best = match self.zone_grades.get(&stage) {
            Some(old) => grade.better_than(*old),
            None => true,
        };
        if new_best {
            self.zone_grades.insert(stage, grade);
        }
        save_progress(self);
        new_best
    }

    /// Record a finished sprint run; returns true on a new best time.
    pub fn record_sprint(&mut self, ms: u64) -> bool {
        let new_best = self.best_sprint_ms.map_or(true, |best| ms < best);
        if new_best {
            self.best_sprint_ms = Some(ms);
        }
        save_progress(self);
        new_best
    }

    pub fn zen_level(&self) -> u64 {
        zen_level_for(self.zen_lines)
    }

    /// Lines cleared toward the next zen level.
    pub fn zen_level_progress(&self) -> u64 {
        self.zen_lines % ZEN_LINES_PER_LEVEL
    }

    /// Add cleared lines to the lifetime zen total. Returns true when
    /// that crossed a level boundary.
    ///
    /// Zen has no finish line to save at, so the total is written to disk
    /// on every level-up: often enough that quitting mid-run costs at
    /// most a few lines, rare enough not to touch the disk on every
    /// clear.
    pub fn add_zen_lines(&mut self, lines: u64) -> bool {
        if lines == 0 {
            return false;
        }
        let before = self.zen_level();
        self.zen_lines += lines;
        let leveled = self.zen_level() > before;
        if leveled {
            save_progress(self);
        }
        leveled
    }
}

fn progress_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("bevytris").join("progress.ron"))
}

pub fn load_progress() -> Progress {
    let Some(path) = progress_path() else {
        return Progress::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => match ron::from_str(&text) {
            Ok(progress) => progress,
            Err(err) => {
                // Starting fresh means the next save overwrites this file,
                // and a campaign is not something to lose to a parse error
                // nobody saw. Keep the unreadable original beside it so it
                // can be salvaged by hand.
                let backup = path.with_extension("ron.bak");
                match std::fs::rename(&path, &backup) {
                    Ok(()) => warn!("failed to parse {path:?}: {err}; kept it at {backup:?}"),
                    Err(io) => {
                        warn!("failed to parse {path:?}: {err}; could not back it up either: {io}")
                    }
                }
                Progress::default()
            }
        },
        Err(_) => Progress::default(),
    }
}

pub fn save_progress(progress: &Progress) {
    let Some(path) = progress_path() else { return };
    let Some(dir) = path.parent() else { return };
    if let Err(err) = std::fs::create_dir_all(dir) {
        warn!("could not create config dir {dir:?}: {err}");
        return;
    }
    match ron::ser::to_string_pretty(progress, Default::default()) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                warn!("could not write progress to {path:?}: {err}");
            }
        }
        Err(err) => warn!("could not serialize progress: {err}"),
    }
}

/// Debug helper: BEVYTRIS_UNLOCK_ALL=1 opens every stage for this run
/// (not persisted unless a stage is cleared).
pub fn load_progress_with_env() -> Progress {
    let mut progress = load_progress();
    if std::env::var("BEVYTRIS_UNLOCK_ALL").is_ok() {
        progress.unlocked = MAX_STAGE;
        progress.zone_unlocked = MAX_STAGE;
    }
    progress
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files written before zen existed still list `best_dig_ms`. Serde
    /// ignores unknown fields, but this mode's removal must not be the
    /// thing that quietly resets someone's 30-stage campaign.
    #[test]
    fn a_pre_zen_progress_file_still_loads() {
        let legacy = r#"(
            unlocked: 14,
            grades: {7: S, 8: A},
            zone_unlocked: 3,
            zone_grades: {1: B},
            best_sprint_ms: Some(48210),
            best_dig_ms: Some(61000),
        )"#;
        let p: Progress = ron::from_str(legacy).expect("legacy file must still parse");
        assert_eq!(p.unlocked, 14);
        assert_eq!(p.grades.get(&7), Some(&Grade::S));
        assert_eq!(p.zone_unlocked, 3);
        assert_eq!(p.best_sprint_ms, Some(48210));
        // No zen history yet: a fresh counter, not a lost one.
        assert_eq!(p.zen_lines, 0);
        assert_eq!(p.zen_level(), 1);
    }

    #[test]
    fn zen_levels_climb_every_ten_lines() {
        assert_eq!(zen_level_for(0), 1);
        assert_eq!(zen_level_for(9), 1);
        assert_eq!(zen_level_for(10), 2);
        assert_eq!(zen_level_for(1_234), 124);
    }

    #[test]
    fn adding_lines_reports_only_real_level_ups() {
        let mut p = Progress::default();
        p.zen_lines = 8;
        assert!(!p.add_zen_lines(1), "8 -> 9 stays on level 1");
        assert!(p.add_zen_lines(1), "9 -> 10 is a level up");
        assert_eq!(p.zen_level(), 2);
        assert_eq!(p.zen_level_progress(), 0);
        // A single clear that vaults several levels still reports once.
        assert!(p.add_zen_lines(40));
        assert_eq!(p.zen_level(), 6);
        assert!(!p.add_zen_lines(0));
    }
}
